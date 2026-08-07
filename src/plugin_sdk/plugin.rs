#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Plugin trait and plugin state machine.
//!
//! Every plugin implements the `Plugin` trait. The plugin lifecycle is
//! managed by `PluginLifecycle` and tracked through `PluginState`.

use super::types::*;
use std::fmt;

/// Current state of a plugin in its lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    /// Plugin has been discovered but not yet validated.
    Discovered,
    /// Plugin manifest has been validated.
    Validated,
    /// Plugin has been loaded into memory.
    Loaded,
    /// Plugin has been initialized.
    Initialized,
    /// Plugin is registered and active.
    Active,
    /// Plugin is shutting down.
    ShuttingDown,
    /// Plugin has been shut down.
    Shutdown,
    /// Plugin encountered an error.
    Error(String),
}

impl fmt::Display for PluginState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginState::Discovered => write!(f, "discovered"),
            PluginState::Validated => write!(f, "validated"),
            PluginState::Loaded => write!(f, "loaded"),
            PluginState::Initialized => write!(f, "initialized"),
            PluginState::Active => write!(f, "active"),
            PluginState::ShuttingDown => write!(f, "shutting_down"),
            PluginState::Shutdown => write!(f, "shutdown"),
            PluginState::Error(msg) => write!(f, "error({msg})"),
        }
    }
}

/// Errors that can occur during plugin lifecycle.
#[derive(Debug, Clone)]
pub enum PluginError {
    ManifestInvalid(String),
    DependencyMissing(String),
    LoadFailed(String),
    InitFailed(String),
    HookFailed(String),
    PermissionDenied(String),
    SandboxViolation(String),
    ShutdownFailed(String),
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginError::ManifestInvalid(msg) => write!(f, "Manifest invalid: {msg}"),
            PluginError::DependencyMissing(dep) => write!(f, "Dependency missing: {dep}"),
            PluginError::LoadFailed(msg) => write!(f, "Load failed: {msg}"),
            PluginError::InitFailed(msg) => write!(f, "Init failed: {msg}"),
            PluginError::HookFailed(msg) => write!(f, "Hook failed: {msg}"),
            PluginError::PermissionDenied(msg) => write!(f, "Permission denied: {msg}"),
            PluginError::SandboxViolation(msg) => write!(f, "Sandbox violation: {msg}"),
            PluginError::ShutdownFailed(msg) => write!(f, "Shutdown failed: {msg}"),
        }
    }
}

impl std::error::Error for PluginError {}

/// The core plugin trait.
///
/// Every plugin must implement this trait. The trait is deliberately
/// small to minimize the extension surface.
pub trait Plugin: Send + Sync {
    /// Returns the manifest for this plugin.
    fn manifest(&self) -> &PluginManifest;

    /// Initialize the plugin. Called after loading, before registration.
    /// Must be idempotent.
    fn init(&mut self) -> Result<(), PluginError>;

    /// Called when a hook fires. Return false to short-circuit.
    fn on_hook(
        &mut self,
        phase: &HookPhase,
        context: &HookContext,
    ) -> Result<HookResponse, PluginError>;

    /// Shutdown the plugin. Must release all resources.
    fn shutdown(&mut self) -> Result<(), PluginError>;

    /// Get a human-readable name for diagnostics.
    fn name(&self) -> &str {
        &self.manifest().name
    }

    /// Clone the plugin into a boxed trait object.
    fn clone_box(&self) -> Box<dyn Plugin>;
}

impl Clone for Box<dyn Plugin> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// Context passed to plugin hooks.
#[derive(Debug, Clone)]
pub struct HookContext {
    pub correlation_id: String,
    pub trace_id: Option<String>,
    pub phase: HookPhase,
    pub metadata: std::collections::HashMap<String, String>,
}

impl HookContext {
    pub fn new(correlation_id: &str, phase: HookPhase) -> Self {
        HookContext {
            correlation_id: correlation_id.to_string(),
            trace_id: None,
            phase,
            metadata: std::collections::HashMap::new(),
        }
    }
}

/// Response from a plugin hook.
#[derive(Debug, Clone)]
pub enum HookResponse {
    /// Hook completed successfully, continue pipeline.
    Ok,
    /// Hook completed with modifications.
    Modified {
        changes: std::collections::HashMap<String, String>,
    },
    /// Hook blocked the action (e.g., denied approval).
    Blocked { reason: String },
}

impl fmt::Display for HookResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HookResponse::Ok => write!(f, "ok"),
            HookResponse::Modified { .. } => write!(f, "modified"),
            HookResponse::Blocked { reason } => write!(f, "blocked: {reason}"),
        }
    }
}

/// A no-op plugin implementation for testing.
#[derive(Debug, Clone)]
pub struct NoOpPlugin {
    manifest: PluginManifest,
}

impl NoOpPlugin {
    pub fn new(manifest: PluginManifest) -> Self {
        NoOpPlugin { manifest }
    }
}

impl Plugin for NoOpPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn init(&mut self) -> Result<(), PluginError> {
        Ok(())
    }

    fn on_hook(
        &mut self,
        _phase: &HookPhase,
        _context: &HookContext,
    ) -> Result<HookResponse, PluginError> {
        Ok(HookResponse::Ok)
    }

    fn shutdown(&mut self) -> Result<(), PluginError> {
        Ok(())
    }

    fn clone_box(&self) -> Box<dyn Plugin> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_id_validation() {
        let id = PluginId::new("author/plugin-name").unwrap();
        assert_eq!(id.author(), Some("author"));
        assert_eq!(id.name(), Some("plugin-name"));

        assert!(PluginId::new("").is_err());
        assert!(PluginId::new("no-slash").is_err());
    }

    #[test]
    fn test_plugin_version() {
        let v = PluginVersion::new("1.2.3").unwrap();
        assert_eq!(v.major(), 1);
        assert_eq!(v.minor(), 2);
        assert_eq!(v.patch(), 3);
        assert_eq!(v.to_string(), "1.2.3");
    }

    #[test]
    fn test_version_range() {
        let min = PluginVersion::new("1.0.0").unwrap();
        let max = PluginVersion::new("2.0.0").unwrap();
        let range = SupportedVersionRange::new(min, max);
        assert!(range.contains(&PluginVersion::new("1.5.0").unwrap()));
        assert!(!range.contains(&PluginVersion::new("0.9.0").unwrap()));
        assert!(!range.contains(&PluginVersion::new("2.1.0").unwrap()));
    }

    #[test]
    fn test_required_sdk_version() {
        let v = PluginVersion::new("1.0.0").unwrap();
        assert!(RequiredSdkVersion::Exact(v.clone()).meets(&v));
        assert!(!RequiredSdkVersion::Exact(v.clone()).meets(&PluginVersion::new("1.1.0").unwrap()));
        assert!(RequiredSdkVersion::Minimum(v.clone()).meets(&v));
        assert!(RequiredSdkVersion::Minimum(v.clone()).meets(&PluginVersion::new("1.1.0").unwrap()));
        assert!(
            !RequiredSdkVersion::Minimum(v.clone()).meets(&PluginVersion::new("0.9.0").unwrap())
        );
    }

    #[test]
    fn test_plugin_state_transitions() {
        assert_eq!(PluginState::Discovered.to_string(), "discovered");
        assert_eq!(PluginState::Active.to_string(), "active");
        assert_eq!(
            PluginState::Error("test".to_string()).to_string(),
            "error(test)"
        );
    }

    #[test]
    fn test_hook_context() {
        let ctx = HookContext::new("corr-1", HookPhase::IntentResolved);
        assert_eq!(ctx.correlation_id, "corr-1");
        assert_eq!(ctx.phase, HookPhase::IntentResolved);
    }

    #[test]
    fn test_noop_plugin() {
        let manifest = PluginManifest::new(
            PluginId::new("test/noop").unwrap(),
            "NoOp",
            "A no-op plugin",
            PluginVersion::new("1.0.0").unwrap(),
            Author::new("test"),
            License::MIT,
            RequiredSdkVersion::Minimum(PluginVersion::new("1.0.0").unwrap()),
            SupportedVersionRange::new(
                PluginVersion::new("1.0.0").unwrap(),
                PluginVersion::new("2.0.0").unwrap(),
            ),
        );
        let mut plugin = NoOpPlugin::new(manifest);
        assert!(plugin.init().is_ok());
        assert!(plugin
            .on_hook(
                &HookPhase::IntentResolved,
                &HookContext::new("c", HookPhase::IntentResolved)
            )
            .is_ok());
        assert!(plugin.shutdown().is_ok());
    }

    #[test]
    fn test_hook_response_display() {
        assert_eq!(HookResponse::Ok.to_string(), "ok");
        assert_eq!(
            HookResponse::Blocked {
                reason: "denied".to_string()
            }
            .to_string(),
            "blocked: denied"
        );
    }

    #[test]
    fn test_manifest_validate() {
        let manifest = PluginManifest::new(
            PluginId::new("test/valid").unwrap(),
            "Valid",
            "A valid plugin",
            PluginVersion::new("1.0.0").unwrap(),
            Author::new("test"),
            License::MIT,
            RequiredSdkVersion::Minimum(PluginVersion::new("1.0.0").unwrap()),
            SupportedVersionRange::new(
                PluginVersion::new("1.0.0").unwrap(),
                PluginVersion::new("2.0.0").unwrap(),
            ),
        );
        assert!(manifest.validate().is_ok());

        let bad = PluginManifest::new(
            PluginId::new("test/empty").unwrap(),
            "",
            "desc",
            PluginVersion::new("1.0.0").unwrap(),
            Author::new("test"),
            License::MIT,
            RequiredSdkVersion::Minimum(PluginVersion::new("1.0.0").unwrap()),
            SupportedVersionRange::new(
                PluginVersion::new("1.0.0").unwrap(),
                PluginVersion::new("2.0.0").unwrap(),
            ),
        );
        assert!(bad.validate().is_err());
    }

    #[test]
    fn test_hook_phase_display() {
        assert_eq!(HookPhase::PipelineStarted.to_string(), "pipeline_started");
        assert_eq!(HookPhase::IntentResolved.to_string(), "intent_resolved");
        assert_eq!(
            HookPhase::Custom("custom".to_string()).to_string(),
            "custom"
        );
    }

    #[test]
    fn test_permission_level() {
        assert_eq!(
            format!("{}", PermissionLevel::Read),
            format!("{}", PermissionLevel::Read)
        );
        let perm = Permission::new(
            SecurityDomain::Observability,
            PermissionLevel::Read,
            "read events",
        );
        assert_eq!(perm.domain.to_string(), "observability");
        assert_eq!(perm.description, "read events");
    }
}
