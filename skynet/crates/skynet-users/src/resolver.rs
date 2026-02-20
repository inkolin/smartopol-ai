use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use skynet_core::types::UserRole;
use tracing::{debug, info};

use crate::error::{Result, UserError};
use crate::identity::{add_identity, create_user, find_user_by_identity};
use crate::types::User;

/// Maximum number of (channel, identifier) → user_id pairs kept in the
/// in-process cache. Simple eviction: when full, drop the oldest half.
const CACHE_MAX: usize = 256;

/// Result of resolving a (channel, identifier) pair.
#[derive(Debug)]
pub enum ResolvedUser {
    Known(User),
    /// Auto-created on first contact. Caller should trigger onboarding flow.
    NewlyCreated {
        user: User,
        needs_onboarding: bool,
    },
}

impl ResolvedUser {
    pub fn user(&self) -> &User {
        match self {
            ResolvedUser::Known(u) => u,
            ResolvedUser::NewlyCreated { user, .. } => user,
        }
    }
}

/// Translates external channel identities into first-class Skynet users.
///
/// Hot path: every inbound message calls resolve(). We cache the (channel,
/// identifier) → user_id mapping in memory to avoid a DB round-trip on
/// every message for known users.
pub struct UserResolver {
    db: Arc<Mutex<Connection>>,
    /// Key: (channel, identifier), Value: user_id.
    /// Stored in insertion order via Vec-backed eviction (simple; good enough
    /// until we have profiling data that justifies a real LRU crate).
    cache: Mutex<HashMap<(String, String), String>>,
    /// Insertion-order key list for eviction — parallel to the HashMap.
    cache_order: Mutex<Vec<(String, String)>>,
}

impl UserResolver {
    pub fn new(db: Arc<Mutex<Connection>>) -> Self {
        Self {
            db,
            cache: Mutex::new(HashMap::new()),
            cache_order: Mutex::new(Vec::new()),
        }
    }

    /// Resolve a (channel, identifier) pair to a user.
    ///
    /// On first contact the user is auto-created with role=User so the agent
    /// can respond immediately; an onboarding message is triggered by the
    /// NewlyCreated variant.
    pub fn resolve(&self, channel: &str, identifier: &str) -> Result<ResolvedUser> {
        let key = (channel.to_string(), identifier.to_string());

        // Fast path: cache hit avoids a DB lock.
        if let Some(user_id) = self.cache_lookup(&key) {
            debug!(channel, identifier, user_id, "cache hit");
            let conn = self.db.lock().unwrap();
            if let Some(user) = crate::identity::get_user(&conn, &user_id)? {
                return Ok(ResolvedUser::Known(user));
            }
            // User was deleted externally; fall through to DB query.
            self.cache_remove(&key);
        }

        // Slow path: full DB lookup.
        let conn = self.db.lock().unwrap();
        if let Some(user) = find_user_by_identity(&conn, channel, identifier)? {
            self.cache_insert(key, user.id.clone());
            return Ok(ResolvedUser::Known(user));
        }

        // Unknown identity — auto-create a new user and link the identity.
        info!(channel, identifier, "new identity; creating user");
        let display_name = format!("{}:{}", channel, identifier);
        let user = create_user(&conn, &display_name, UserRole::User)?;
        add_identity(&conn, &user.id, channel, identifier)?;
        self.cache_insert(key, user.id.clone());

        Ok(ResolvedUser::NewlyCreated {
            user,
            needs_onboarding: true,
        })
    }

    /// Re-assign an existing channel identity to a different (target) user.
    /// Used when an admin manually links two accounts.
    pub fn link_identity(
        &self,
        admin_id: &str,
        channel: &str,
        identifier: &str,
        target_user_id: &str,
    ) -> Result<()> {
        let conn = self.db.lock().unwrap();

        // Verify the admin user actually exists and is admin.
        let admin = crate::identity::get_user(&conn, admin_id)?
            .ok_or_else(|| UserError::NotFound(admin_id.to_string()))?;
        if !admin.role.is_admin() {
            return Err(UserError::PermissionDenied(
                "only admins can re-link identities".to_string(),
            ));
        }

        // Upsert: update if the (channel, identifier) pair exists, else insert.
        let now = chrono::Utc::now().to_rfc3339();
        let rows = conn.execute(
            "UPDATE user_identities
             SET user_id=?3, linked_by=?4, linked_at=?5
             WHERE channel=?1 AND identifier=?2",
            rusqlite::params![channel, identifier, target_user_id, admin_id, now],
        )?;

        if rows == 0 {
            add_identity(&conn, target_user_id, channel, identifier)?;
        }

        // Invalidate both the old and new user's cache entries.
        self.invalidate_channel(channel, identifier);
        Ok(())
    }

    /// Look up a user by their Skynet user ID (primary key).
    ///
    /// Returns None if no user exists with this ID.
    pub fn get_user(&self, user_id: &str) -> Result<Option<crate::types::User>> {
        let conn = self.db.lock().unwrap();
        crate::identity::get_user(&conn, user_id)
    }

    /// List all identities linked to a Skynet user.
    ///
    /// Returns the list of (channel, identifier) pairs for prompt injection.
    pub fn list_identities(&self, user_id: &str) -> Result<Vec<crate::types::UserIdentity>> {
        let conn = self.db.lock().unwrap();
        crate::identity::list_identities_for_user(&conn, user_id)
    }

    /// Self-service identity linking: merge a source identity into the target user.
    ///
    /// Unlike `link_identity()` which requires admin privileges, this is called
    /// after the verification code flow has been validated. It:
    /// 1. Moves the source identity to point to `target_user_id`
    /// 2. Invalidates caches for both the old and new user
    ///
    /// The caller is responsible for validating the verification code.
    pub fn self_link(
        &self,
        source_channel: &str,
        source_identifier: &str,
        target_user_id: &str,
    ) -> Result<()> {
        let conn = self.db.lock().unwrap();

        // Verify the target user exists.
        let _target = crate::identity::get_user(&conn, target_user_id)?
            .ok_or_else(|| UserError::NotFound(target_user_id.to_string()))?;

        // Update the identity to point to the target user.
        let now = chrono::Utc::now().to_rfc3339();
        let rows = conn.execute(
            "UPDATE user_identities
             SET user_id=?3, linked_by=?4, linked_at=?5
             WHERE channel=?1 AND identifier=?2",
            rusqlite::params![
                source_channel,
                source_identifier,
                target_user_id,
                "self_link",
                now
            ],
        )?;

        if rows == 0 {
            // Identity doesn't exist yet — create it.
            crate::identity::add_identity(
                &conn,
                target_user_id,
                source_channel,
                source_identifier,
            )?;
        }

        // Invalidate caches for both users.
        self.invalidate_channel(source_channel, source_identifier);
        self.invalidate_user(target_user_id);

        info!(
            channel = source_channel,
            identifier = source_identifier,
            target_user_id,
            "identity self-linked"
        );
        Ok(())
    }

    /// Drop all cache entries that belong to `user_id`.
    /// Call this after updating a user's role or capabilities.
    pub fn invalidate_user(&self, user_id: &str) {
        let mut cache = self.cache.lock().unwrap();
        let mut order = self.cache_order.lock().unwrap();
        order.retain(|k| {
            if cache.get(k).map(|v| v.as_str()) == Some(user_id) {
                cache.remove(k);
                false
            } else {
                true
            }
        });
    }

    // ── cache helpers ─────────────────────────────────────────────────────────

    fn cache_lookup(&self, key: &(String, String)) -> Option<String> {
        self.cache.lock().unwrap().get(key).cloned()
    }

    fn cache_remove(&self, key: &(String, String)) {
        let mut cache = self.cache.lock().unwrap();
        let mut order = self.cache_order.lock().unwrap();
        cache.remove(key);
        order.retain(|k| k != key);
    }

    fn cache_insert(&self, key: (String, String), user_id: String) {
        let mut cache = self.cache.lock().unwrap();
        let mut order = self.cache_order.lock().unwrap();

        if let std::collections::hash_map::Entry::Occupied(mut e) = cache.entry(key.clone()) {
            // Refresh value in-place; no order change needed for our simple eviction.
            e.insert(user_id);
            return;
        }

        // Evict oldest half when at capacity to prevent unbounded growth.
        if cache.len() >= CACHE_MAX {
            let evict_count = CACHE_MAX / 2;
            let to_remove: Vec<_> = order.drain(..evict_count).collect();
            for k in to_remove {
                cache.remove(&k);
            }
        }

        order.push(key.clone());
        cache.insert(key, user_id);
    }

    fn invalidate_channel(&self, channel: &str, identifier: &str) {
        let key = (channel.to_string(), identifier.to_string());
        self.cache_remove(&key);
    }
}
