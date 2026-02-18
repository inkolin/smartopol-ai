use std::sync::{Arc, RwLock};
use std::time::Instant;

use tracing::{debug, error, warn};

use crate::types::{HookAction, HookContext, HookDefinition, HookResult, HookTiming};

/// Central registry and dispatcher for all hooks in the system.
///
/// Designed to be cheaply cloneable via Arc — a single HookEngine instance
/// should be shared across the whole process (pass as Arc<HookEngine>).
pub struct HookEngine {
    /// Sorted by priority ascending after every registration.
    hooks: RwLock<Vec<HookDefinition>>,
}

impl HookEngine {
    pub fn new() -> Self {
        Self { hooks: RwLock::new(Vec::new()) }
    }

    /// Register a hook. Re-sorts the list so priority order is always correct.
    pub fn register(&self, hook: HookDefinition) {
        let mut hooks = self.hooks.write().expect("hook registry poisoned");
        hooks.push(hook);
        // Stable sort preserves registration order within the same priority.
        hooks.sort_by_key(|h| h.priority);
        debug!(name = %hooks.last().unwrap().name, "hook registered");
    }

    /// Remove a hook by name. Silent no-op if the name is not found.
    pub fn unregister(&self, name: &str) {
        let mut hooks = self.hooks.write().expect("hook registry poisoned");
        let before = hooks.len();
        hooks.retain(|h| h.name != name);
        if hooks.len() < before {
            debug!(name, "hook unregistered");
        }
    }

    /// Emit an event: run Before hooks (blocking), then After hooks (fire-and-forget).
    ///
    /// Returns the combined result — callers should check `action` to decide
    /// whether to continue or abort their operation.
    pub fn emit(&self, mut ctx: HookContext) -> HookResult {
        let action = self.emit_before(&mut ctx);

        // If a Before hook blocked, skip After hooks — the event never happened.
        if matches!(action, HookAction::Block { .. }) {
            return HookResult { action, duration_ms: 0 };
        }

        self.emit_after(ctx);

        HookResult { action, duration_ms: 0 }
    }

    /// Run all Before hooks in priority order.
    ///
    /// Stops at the first Block. Modify updates the context payload in-place
    /// so subsequent hooks see the mutated version.
    pub fn emit_before(&self, ctx: &mut HookContext) -> HookAction {
        let hooks = self.hooks.read().expect("hook registry poisoned");

        for hook in hooks.iter().filter(|h| h.event == ctx.event && h.timing == HookTiming::Before)
        {
            let t = Instant::now();
            let result = hook.handler.handle(ctx);
            let elapsed_ms = t.elapsed().as_millis() as u64;

            debug!(
                hook = %hook.name,
                duration_ms = elapsed_ms,
                "before hook completed"
            );

            match result.action {
                HookAction::Block { ref reason } => {
                    warn!(hook = %hook.name, reason, "hook blocked event");
                    return result.action;
                }
                HookAction::Modify { ref payload } => {
                    // Propagate payload mutation so the next hook sees updated data.
                    ctx.payload = payload.clone();
                }
                HookAction::Allow => {}
            }
        }

        HookAction::Allow
    }

    /// Spawn all After hooks concurrently — errors are logged, never propagated.
    pub fn emit_after(&self, ctx: HookContext) {
        let hooks = self.hooks.read().expect("hook registry poisoned");

        for hook in hooks.iter().filter(|h| h.event == ctx.event && h.timing == HookTiming::After)
        {
            let ctx_clone = ctx.clone();
            // Clone Arc — cheap pointer bump, not a deep copy of the handler.
            let handler = Arc::clone(&hook.handler);
            let hook_name = hook.name.clone();

            tokio::spawn(async move {
                let t = Instant::now();
                let result = handler.handle(&ctx_clone);
                let elapsed_ms = t.elapsed().as_millis() as u64;

                if let HookAction::Block { reason } = result.action {
                    // After hooks cannot actually block — log the misconfiguration.
                    error!(
                        hook = %hook_name,
                        duration_ms = elapsed_ms,
                        reason,
                        "after hook returned Block — ignored (use Before timing to block)"
                    );
                } else {
                    debug!(hook = %hook_name, duration_ms = elapsed_ms, "after hook completed");
                }
            });
        }
    }
}

impl Default for HookEngine {
    fn default() -> Self {
        Self::new()
    }
}
