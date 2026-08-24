//! Tool Hook System
//!
//! Defines interfaces for pre-execution permission hooks and post-execution
//! rollback hooks. Hooks can be attached per-tool or globally.

use super::capabilities::PermissionPolicy;
use super::context::{ToolContext, ToolResult};
use anyhow::Result;

/// Decision returned by a permission hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Tool execution is allowed.
    Allowed { reason: Option<String> },
    /// Tool execution requires user confirmation.
    Ask {
        reason: String,
        tool_name: String,
        args: String,
    },
    /// Tool execution is denied.
    Denied { reason: String },
}

impl PermissionDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, PermissionDecision::Allowed { .. })
    }

    pub fn requires_ask(&self) -> bool {
        matches!(self, PermissionDecision::Ask { .. })
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, PermissionDecision::Denied { .. })
    }
}

/// Pre-execution hook interface.
///
/// Called before a tool executes. If the hook denies or asks, the tool
/// should not run (or must wait for user confirmation).
pub trait PermissionHook: Send + Sync + std::any::Any + 'static {
    /// Check permission for a tool execution.
    fn check(&self, context: &ToolContext) -> PermissionDecision;
}

/// Post-execution hook interface.
///
/// Called after a tool executes. Used for audit logging, state capture,
/// and rollback preparation.
pub trait RollbackHook: Send + Sync + std::any::Any + 'static {
    /// Called before execution to capture state for potential rollback.
    fn before_execute(&self, context: &mut ToolContext) -> Result<()>;

    /// Called after execution to record outcome and prepare rollback if needed.
    fn after_execute(&self, context: &ToolContext, result: &ToolResult) -> Result<()>;
}

/// Built-in permission hook that enforces capability-based policies.
#[derive(Debug, Clone, Default)]
pub struct CapabilityPermissionHook;

impl PermissionHook for CapabilityPermissionHook {
    fn check(&self, context: &ToolContext) -> PermissionDecision {
        match context.tool_capabilities.permission_policy() {
            PermissionPolicy::AutoAllow => PermissionDecision::Allowed {
                reason: Some("Capability policy: auto-allow".to_string()),
            },
            PermissionPolicy::RequireConfirmation => PermissionDecision::Ask {
                reason: format!(
                    "Tool '{}' requires confirmation (capabilities: {})",
                    context.tool_name,
                    context.tool_capabilities.format()
                ),
                tool_name: context.tool_name.clone(),
                args: context.args.clone(),
            },
            PermissionPolicy::Blocked => PermissionDecision::Denied {
                reason: format!(
                    "Tool '{}' is blocked by capability policy",
                    context.tool_name
                ),
            },
            PermissionPolicy::External => PermissionDecision::Allowed {
                reason: Some("Delegated to external policy".to_string()),
            },
        }
    }
}

/// Built-in rollback hook that tracks mutating tools.
pub struct DefaultRollbackHook {
    snapshots: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl Default for DefaultRollbackHook {
    fn default() -> Self {
        DefaultRollbackHook {
            snapshots: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl Clone for DefaultRollbackHook {
    fn clone(&self) -> Self {
        // Clone the snapshot map
        let snapshots = self.snapshots.lock().unwrap().clone();
        DefaultRollbackHook {
            snapshots: std::sync::Mutex::new(snapshots),
        }
    }
}

impl std::fmt::Debug for DefaultRollbackHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultRollbackHook")
            .field("snapshots_count", &self.snapshots.lock().unwrap().len())
            .finish()
    }
}

impl DefaultRollbackHook {
    /// Get a snapshot of state before execution.
    pub fn snapshot(&self, key: &str, state: &str) {
        self.snapshots
            .lock()
            .unwrap()
            .insert(key.to_string(), state.to_string());
    }

    /// Get the last snapshot for a key.
    pub fn get_snapshot(&self, key: &str) -> Option<String> {
        self.snapshots.lock().unwrap().get(key).cloned()
    }

    /// Clear all snapshots.
    pub fn clear(&self) {
        self.snapshots.lock().unwrap().clear();
    }
}

impl RollbackHook for DefaultRollbackHook {
    fn before_execute(&self, context: &mut ToolContext) -> Result<()> {
        // No-op for generic hook; specific tools override.
        Ok(())
    }

    fn after_execute(&self, _context: &ToolContext, _result: &ToolResult) -> Result<()> {
        Ok(())
    }
}

/// A hook container that holds both permission and rollback hooks for a tool.
#[derive(Default)]
pub struct ToolHooks {
    pub permission: Option<Box<dyn PermissionHook>>,
    pub rollback: Option<Box<dyn RollbackHook>>,
}

impl std::fmt::Debug for ToolHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolHooks")
            .field(
                "permission",
                &if self.permission.is_some() {
                    "Some(...)"
                } else {
                    "None"
                },
            )
            .field(
                "rollback",
                &if self.rollback.is_some() {
                    "Some(...)"
                } else {
                    "None"
                },
            )
            .finish()
    }
}

impl Clone for ToolHooks {
    fn clone(&self) -> Self {
        ToolHooks {
            permission: self.permission.as_deref().and_then(clone_box_permission),
            rollback: self.rollback.as_deref().and_then(clone_box_rollback),
        }
    }
}

impl ToolHooks {
    pub fn new() -> Self {
        ToolHooks::default()
    }

    pub fn with_permission(mut self, hook: Box<dyn PermissionHook>) -> Self {
        self.permission = Some(hook);
        self
    }

    pub fn with_rollback(mut self, hook: Box<dyn RollbackHook>) -> Self {
        self.rollback = Some(hook);
        self
    }

    /// Check permissions, falling back to capability-based default.
    pub fn check_permission(&self, context: &ToolContext) -> PermissionDecision {
        if let Some(ref hook) = self.permission {
            hook.check(context)
        } else {
            CapabilityPermissionHook.check(context)
        }
    }

    /// Run before-execute hooks.
    pub fn before_execute(&self, context: &mut ToolContext) -> Result<()> {
        if let Some(ref hook) = self.rollback {
            hook.before_execute(context)?;
        }
        Ok(())
    }

    /// Run after-execute hooks.
    pub fn after_execute(&self, context: &ToolContext, result: &ToolResult) -> Result<()> {
        if let Some(ref hook) = self.rollback {
            hook.after_execute(context, result)?;
        }
        Ok(())
    }
}

/// Global hook manager that can apply hooks to all tools.
#[derive(Default)]
pub struct HookManager {
    global_permission: Option<Box<dyn PermissionHook>>,
    global_rollback: Option<Box<dyn RollbackHook>>,
    per_tool_hooks: std::collections::HashMap<String, ToolHooks>,
}

impl std::fmt::Debug for HookManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookManager")
            .field(
                "global_permission",
                &if self.global_permission.is_some() {
                    "Some(...)"
                } else {
                    "None"
                },
            )
            .field(
                "global_rollback",
                &if self.global_rollback.is_some() {
                    "Some(...)"
                } else {
                    "None"
                },
            )
            .field("per_tool_hooks", &self.per_tool_hooks.len())
            .finish()
    }
}

impl HookManager {
    pub fn new() -> Self {
        HookManager::default()
    }

    pub fn set_global_permission(&mut self, hook: Box<dyn PermissionHook>) {
        self.global_permission = Some(hook);
    }

    pub fn set_global_rollback(&mut self, hook: Box<dyn RollbackHook>) {
        self.global_rollback = Some(hook);
    }

    pub fn set_tool_hooks(&mut self, tool_name: &str, hooks: ToolHooks) {
        self.per_tool_hooks.insert(tool_name.to_string(), hooks);
    }

    /// Resolve the effective hooks for a tool: per-tool hooks take
    /// precedence; otherwise the global hooks (if any); otherwise empty
    /// (which falls back to the capability-based default).
    pub fn get_tool_hooks(&self, tool_name: &str) -> ToolHooks {
        if let Some(hooks) = self.per_tool_hooks.get(tool_name) {
            return ToolHooks {
                permission: hooks.permission.as_deref().and_then(clone_box_permission),
                rollback: hooks.rollback.as_deref().and_then(clone_box_rollback),
            };
        }
        ToolHooks {
            permission: self
                .global_permission
                .as_deref()
                .and_then(clone_box_permission),
            rollback: self.global_rollback.as_deref().and_then(clone_box_rollback),
        }
    }

    /// Check permission for a tool, combining per-tool and global hooks.
    pub fn check_permission(&self, context: &ToolContext) -> PermissionDecision {
        if let Some(hooks) = self.per_tool_hooks.get(&context.tool_name) {
            if let Some(ref hook) = hooks.permission {
                return hook.check(context);
            }
        }
        if let Some(ref hook) = self.global_permission {
            return hook.check(context);
        }
        CapabilityPermissionHook.check(context)
    }

    /// Run before-execute hooks.
    pub fn before_execute(&self, context: &mut ToolContext) -> Result<()> {
        if let Some(hooks) = self.per_tool_hooks.get(&context.tool_name) {
            if let Some(ref hook) = hooks.rollback {
                return hook.before_execute(context);
            }
        }
        if let Some(ref hook) = self.global_rollback {
            return hook.before_execute(context);
        }
        Ok(())
    }

    /// Run after-execute hooks.
    pub fn after_execute(&self, context: &ToolContext, result: &ToolResult) -> Result<()> {
        if let Some(hooks) = self.per_tool_hooks.get(&context.tool_name) {
            if let Some(ref hook) = hooks.rollback {
                return hook.after_execute(context, result);
            }
        }
        if let Some(ref hook) = self.global_rollback {
            return hook.after_execute(context, result);
        }
        Ok(())
    }
}

/// Clone a permission hook by concrete type. Built-in hook types are
/// cloned; unknown external hooks fall back to `None` (degrading to the
/// capability-based default rather than failing).
fn clone_box_permission(hook: &dyn PermissionHook) -> Option<Box<dyn PermissionHook>> {
    let any = hook as &dyn std::any::Any;
    if let Some(cap) = any.downcast_ref::<CapabilityPermissionHook>() {
        return Some(Box::new(cap.clone()));
    }
    None
}

/// Clone a rollback hook by concrete type. Built-in hook types are cloned;
/// unknown external hooks fall back to `None`.
fn clone_box_rollback(hook: &dyn RollbackHook) -> Option<Box<dyn RollbackHook>> {
    let any = hook as &dyn std::any::Any;
    if let Some(def) = any.downcast_ref::<DefaultRollbackHook>() {
        return Some(Box::new(def.clone()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_decision_methods() {
        let allowed = PermissionDecision::Allowed { reason: None };
        assert!(allowed.is_allowed());
        assert!(!allowed.requires_ask());
        assert!(!allowed.is_denied());

        let ask = PermissionDecision::Ask {
            reason: "need confirmation".to_string(),
            tool_name: "t".to_string(),
            args: "a".to_string(),
        };
        assert!(!ask.is_allowed());
        assert!(ask.requires_ask());
        assert!(!ask.is_denied());

        let denied = PermissionDecision::Denied {
            reason: "blocked".to_string(),
        };
        assert!(!denied.is_allowed());
        assert!(!denied.requires_ask());
        assert!(denied.is_denied());
    }

    #[test]
    fn test_capability_hook_auto_allow() {
        let hook = CapabilityPermissionHook;
        let ctx = ToolContext::new("list_files", ".");
        let decision = hook.check(&ctx);
        assert!(decision.is_allowed());
    }

    #[test]
    fn test_capability_hook_require_confirmation() {
        let hook = CapabilityPermissionHook;
        let ctx = ToolContext::builder("run_command", "rm -rf /")
            .with_capabilities(crate::tools::ToolCapabilities {
                executes_commands: true,
                writes_files: true,
                requires_confirmation: true,
                ..Default::default()
            })
            .build();
        let decision = hook.check(&ctx);
        assert!(decision.requires_ask());
    }

    #[test]
    fn test_tool_hooks_fallback() {
        let hooks = ToolHooks::new();
        let ctx = ToolContext::new("test", "args");
        let decision = hooks.check_permission(&ctx);
        assert!(decision.is_allowed());
    }

    #[test]
    fn test_hook_manager_global_and_per_tool() {
        let mut mgr = HookManager::new();

        let custom_hook = CapabilityPermissionHook;
        mgr.set_global_permission(Box::new(custom_hook));

        let tool_hooks = ToolHooks::new().with_permission(Box::new(CapabilityPermissionHook));
        mgr.set_tool_hooks("special_tool", tool_hooks);

        // Per-tool hook takes precedence
        let ctx = ToolContext::new("special_tool", "args");
        assert!(mgr.check_permission(&ctx).is_allowed());

        // Global hook applies to other tools
        let ctx2 = ToolContext::new("other_tool", "args");
        assert!(mgr.check_permission(&ctx2).is_allowed());
    }

    /// A denying permission hook used to prove global hooks are enforced.
    struct DenyAllHook;

    impl PermissionHook for DenyAllHook {
        fn check(&self, context: &ToolContext) -> PermissionDecision {
            PermissionDecision::Denied {
                reason: format!("deny-all for {}", context.tool_name),
            }
        }
    }

    #[test]
    fn test_global_permission_hook_is_enforced() {
        // Sprint 30C relies on a global permission hook (the research
        // read-only boundary). Prove the hook is actually consulted by the
        // registry's permission path.
        let mut mgr = HookManager::new();
        mgr.set_global_permission(Box::new(DenyAllHook));

        let ctx = ToolContext::new("read_file", "src/main.rs");
        let decision = mgr.check_permission(&ctx);
        assert!(
            decision.is_denied(),
            "global permission hook must deny, got {:?}",
            decision
        );
    }

    #[test]
    fn test_per_tool_permission_hook_overrides_global() {
        // Per-tool hooks take precedence over a global hook.
        struct AllowAllHook;

        impl PermissionHook for AllowAllHook {
            fn check(&self, _context: &ToolContext) -> PermissionDecision {
                PermissionDecision::Allowed { reason: None }
            }
        }

        let mut mgr = HookManager::new();
        mgr.set_global_permission(Box::new(DenyAllHook));
        mgr.set_tool_hooks(
            "read_file",
            ToolHooks::new().with_permission(Box::new(AllowAllHook)),
        );

        let ctx = ToolContext::new("read_file", "src/main.rs");
        assert!(
            mgr.check_permission(&ctx).is_allowed(),
            "per-tool hook must override the global deny"
        );
        let other = ToolContext::new("list_files", ".");
        assert!(
            mgr.check_permission(&other).is_denied(),
            "global deny still applies to other tools"
        );
    }
}
