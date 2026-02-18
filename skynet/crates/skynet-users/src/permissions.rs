use chrono::Utc;
use rusqlite::{params, Connection};
use skynet_core::types::UserRole;

use crate::error::{Result, UserError};
use crate::types::User;

/// All capabilities that can be checked in one place. Adding a new capability
/// here forces the compiler to ensure check() handles it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Permission {
    InstallSoftware,
    ExecuteCommands,
    UseBrowser,
    SendMessages,
    AccessMemory,
    /// Allows reading other users' memory (admin-only).
    AccessAllMemory,
    ManageUsers,
    ApproveRequests,
    ViewCostReports,
}

/// Result of a permission check. Callers pattern-match this rather than
/// catching errors so they can distinguish "hard no" from "ask admin".
#[derive(Debug, Clone)]
pub enum PermissionCheck {
    Allowed,
    Denied {
        reason: String,
    },
    /// The action is allowed but must be queued for admin sign-off first.
    NeedsApproval {
        action_type: String,
    },
    BudgetExceeded {
        used: u64,
        limit: u64,
    },
}

pub struct PermissionChecker;

impl PermissionChecker {
    /// Evaluate whether `user` may perform `permission`.
    ///
    /// Precedence: Admin > role-level restriction > per-user flags.
    /// Admin users bypass every check — this mirrors typical RBAC practice
    /// where the system owner must always have an escape hatch.
    pub fn check(user: &User, permission: &Permission) -> PermissionCheck {
        // Admins bypass all checks.
        if user.role == UserRole::Admin {
            return PermissionCheck::Allowed;
        }

        // Child role: lock down everything dangerous.
        if user.role == UserRole::Child {
            match permission {
                Permission::SendMessages | Permission::AccessMemory => {
                    return PermissionCheck::Allowed
                }
                _ => {
                    return PermissionCheck::Denied {
                        reason: "child accounts cannot perform this action".to_string(),
                    }
                }
            }
        }

        // Standard user: check individual capability flags.
        match permission {
            Permission::SendMessages | Permission::AccessMemory => PermissionCheck::Allowed,

            Permission::InstallSoftware => {
                if user.can_install_software {
                    maybe_needs_approval(user, "install_software")
                } else {
                    PermissionCheck::Denied {
                        reason: "install_software not enabled for this user".to_string(),
                    }
                }
            }

            Permission::ExecuteCommands => {
                if user.can_exec_commands {
                    maybe_needs_approval(user, "exec_commands")
                } else {
                    PermissionCheck::Denied {
                        reason: "exec_commands not enabled for this user".to_string(),
                    }
                }
            }

            Permission::UseBrowser => {
                if user.can_use_browser {
                    PermissionCheck::Allowed
                } else {
                    PermissionCheck::Denied {
                        reason: "use_browser not enabled for this user".to_string(),
                    }
                }
            }

            // Non-admin users cannot access other users' memory or manage users.
            Permission::AccessAllMemory | Permission::ManageUsers | Permission::ApproveRequests => {
                PermissionCheck::Denied {
                    reason: "admin role required".to_string(),
                }
            }

            Permission::ViewCostReports => PermissionCheck::Denied {
                reason: "admin role required".to_string(),
            },
        }
    }

    /// Update daily token counter and check against the user's budget.
    ///
    /// Resets the counter when the wall-clock date changes so the quota is
    /// truly per-calendar-day in the user's stored timezone (approximated as
    /// UTC here; full tz support can be added in Phase 5).
    pub fn record_token_usage(
        conn: &Connection,
        user_id: &str,
        tokens: u64,
    ) -> Result<PermissionCheck> {
        let today = Utc::now().format("%Y-%m-%d").to_string();

        // Load current counters — minimal fetch, not the full user row.
        let (mut used_today, reset_date, limit): (u64, Option<String>, Option<u64>) = conn
            .query_row(
                "SELECT tokens_used_today, tokens_reset_date, max_tokens_per_day
                 FROM users WHERE id = ?1",
                params![user_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(UserError::DatabaseError)?;

        // Roll over the daily counter when the date changes.
        let reset_needed = reset_date.as_deref() != Some(&today);
        if reset_needed {
            used_today = 0;
        }

        let new_total = used_today.saturating_add(tokens);

        // Persist before checking so the usage is recorded even on a failed request.
        conn.execute(
            "UPDATE users SET tokens_used_today=?2, tokens_reset_date=?3,
                              total_tokens_used = total_tokens_used + ?4,
                              updated_at=?5
             WHERE id=?1",
            params![user_id, new_total, today, tokens, Utc::now().to_rfc3339()],
        )
        .map_err(UserError::DatabaseError)?;

        if let Some(cap) = limit {
            if new_total > cap {
                return Ok(PermissionCheck::BudgetExceeded {
                    used: new_total,
                    limit: cap,
                });
            }
        }

        Ok(PermissionCheck::Allowed)
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Wrap an otherwise-allowed action in NeedsApproval when the user profile
/// requires it. Avoids repeating this conditional in every match arm.
fn maybe_needs_approval(user: &User, action_type: &str) -> PermissionCheck {
    if user.requires_admin_approval {
        PermissionCheck::NeedsApproval {
            action_type: action_type.to_string(),
        }
    } else {
        PermissionCheck::Allowed
    }
}
