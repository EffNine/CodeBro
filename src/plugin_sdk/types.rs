#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Core types for the Plugin SDK Foundation.
//!
//! All types are immutable, serializable, and deterministic.
//! No runtime mutation of core state is permitted.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::RangeInclusive;

// =========================================================================
// Identifiers
// =========================================================================

/// Unique identifier for a plugin.
///
/// Format: `<author>/<name>` (e.g., "codebro/core-tools").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PluginId(pub String);

impl PluginId {
    pub fn new(id: &str) -> Result<Self, PluginIdError> {
        if id.is_empty() {
            return Err(PluginIdError::Empty);
        }
        if id.chars().any(|c| c == ' ') {
            return Err(PluginIdError::InvalidChar(' '));
        }
        // Must contain at least one '/' for author/name format
        if !id.contains('/') {
            return Err(PluginIdError::MissingSlash);
        }
        Ok(PluginId(id.to_string()))
    }

    pub fn author(&self) -> Option<&str> {
        self.0.split('/').next()
    }

    pub fn name(&self) -> Option<&str> {
        self.0.split('/').last()
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginIdError {
    Empty,
    InvalidChar(char),
    MissingSlash,
}

impl fmt::Display for PluginIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginIdError::Empty => write!(f, "PluginId cannot be empty"),
            PluginIdError::InvalidChar(c) => write!(f, "PluginId contains invalid character: {c}"),
            PluginIdError::MissingSlash => write!(f, "PluginId must be in author/name format"),
        }
    }
}

impl std::error::Error for PluginIdError {}

/// Semantic version for plugins.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Hash, Serialize, Deserialize)]
pub struct PluginVersion(pub String);

impl PluginVersion {
    pub fn new(v: &str) -> Result<Self, VersionError> {
        // Basic semantic version check: MAJOR.MINOR.PATCH
        let parts: Vec<&str> = v.split('.').collect();
        if parts.len() != 3 {
            return Err(VersionError::InvalidFormat(v.to_string()));
        }
        for part in &parts {
            if part.parse::<u32>().is_err() {
                return Err(VersionError::InvalidPart(part.to_string()));
            }
        }
        Ok(PluginVersion(v.to_string()))
    }

    pub fn major(&self) -> u32 {
        self.0
            .split('.')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    pub fn minor(&self) -> u32 {
        self.0
            .split('.')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    pub fn patch(&self) -> u32 {
        self.0
            .split('.')
            .nth(2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }
}

impl fmt::Display for PluginVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionError {
    InvalidFormat(String),
    InvalidPart(String),
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionError::InvalidFormat(v) => write!(f, "Invalid version format: {v}"),
            VersionError::InvalidPart(p) => write!(f, "Invalid version part: {p}"),
        }
    }
}

impl std::error::Error for VersionError {}

// =========================================================================
// Version Ranges
// =========================================================================

/// A range of supported CodeBro versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupportedVersionRange {
    pub min: PluginVersion,
    pub max: PluginVersion,
}

impl SupportedVersionRange {
    pub fn new(min: PluginVersion, max: PluginVersion) -> Self {
        SupportedVersionRange { min, max }
    }

    pub fn contains(&self, version: &PluginVersion) -> bool {
        version >= &self.min && version <= &self.max
    }
}

/// Required SDK version specifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequiredSdkVersion {
    Exact(PluginVersion),
    Minimum(PluginVersion),
    Range(SupportedVersionRange),
}

impl RequiredSdkVersion {
    pub fn meets(&self, sdk_version: &PluginVersion) -> bool {
        match self {
            RequiredSdkVersion::Exact(v) => v == sdk_version,
            RequiredSdkVersion::Minimum(v) => sdk_version >= v,
            RequiredSdkVersion::Range(r) => r.contains(sdk_version),
        }
    }
}

impl fmt::Display for RequiredSdkVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequiredSdkVersion::Exact(v) => write!(f, "={v}"),
            RequiredSdkVersion::Minimum(v) => write!(f, ">={v}"),
            RequiredSdkVersion::Range(r) => write!(f, "{}..={}", r.min, r.max),
        }
    }
}

// =========================================================================
// Author & License
// =========================================================================

/// Plugin author information.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    pub email: Option<String>,
    pub url: Option<String>,
}

impl Author {
    pub fn new(name: &str) -> Self {
        Author {
            name: name.to_string(),
            email: None,
            url: None,
        }
    }
}

/// Plugin license type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum License {
    MIT,
    Apache2,
    GPL3,
    LGPL3,
    BSD2,
    BSD3,
    MPL2,
    Proprietary,
    Custom(String),
}

impl fmt::Display for License {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            License::MIT => write!(f, "MIT"),
            License::Apache2 => write!(f, "Apache-2.0"),
            License::GPL3 => write!(f, "GPL-3.0"),
            License::LGPL3 => write!(f, "LGPL-3.0"),
            License::BSD2 => write!(f, "BSD-2-Clause"),
            License::BSD3 => write!(f, "BSD-3-Clause"),
            License::MPL2 => write!(f, "MPL-2.0"),
            License::Proprietary => write!(f, "Proprietary"),
            License::Custom(s) => write!(f, "{s}"),
        }
    }
}

// =========================================================================
// Plugin Manifest
// =========================================================================

/// Manifest for a plugin — declares its identity, capabilities, and requirements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: PluginId,
    pub name: String,
    pub description: String,
    pub version: PluginVersion,
    pub author: Author,
    pub license: License,
    pub required_sdk_version: RequiredSdkVersion,
    pub supported_codebro_versions: SupportedVersionRange,
    pub capabilities: Vec<String>,
    pub permissions: Vec<Permission>,
    pub dependencies: Vec<PluginId>,
    pub hooks: Vec<String>,
}

impl PluginManifest {
    pub fn new(
        id: PluginId,
        name: &str,
        description: &str,
        version: PluginVersion,
        author: Author,
        license: License,
        required_sdk_version: RequiredSdkVersion,
        supported_codebro_versions: SupportedVersionRange,
    ) -> Self {
        PluginManifest {
            id,
            name: name.to_string(),
            description: description.to_string(),
            version,
            author,
            license,
            required_sdk_version,
            supported_codebro_versions,
            capabilities: Vec::new(),
            permissions: Vec::new(),
            dependencies: Vec::new(),
            hooks: Vec::new(),
        }
    }

    pub fn with_capabilities(mut self, capabilities: Vec<String>) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_permissions(mut self, permissions: Vec<Permission>) -> Self {
        self.permissions = permissions;
        self
    }

    pub fn with_dependencies(mut self, dependencies: Vec<PluginId>) -> Self {
        self.dependencies = dependencies;
        self
    }

    pub fn with_hooks(mut self, hooks: Vec<String>) -> Self {
        self.hooks = hooks;
        self
    }

    /// Validate the manifest for basic consistency.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.name.is_empty() {
            return Err(ManifestError::EmptyName);
        }
        if self.description.is_empty() {
            return Err(ManifestError::EmptyDescription);
        }
        // Check for permission conflicts
        let has_read = self
            .permissions
            .iter()
            .any(|p| matches!(p.level, PermissionLevel::Read));
        let has_write = self
            .permissions
            .iter()
            .any(|p| matches!(p.level, PermissionLevel::Write));
        if has_read && has_write {
            // This is allowed; read and write are different permissions
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    EmptyName,
    EmptyDescription,
    InvalidVersion,
    MissingDependency,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::EmptyName => write!(f, "Plugin name cannot be empty"),
            ManifestError::EmptyDescription => write!(f, "Plugin description cannot be empty"),
            ManifestError::InvalidVersion => write!(f, "Invalid plugin version"),
            ManifestError::MissingDependency => write!(f, "Missing required dependency"),
        }
    }
}

impl std::error::Error for ManifestError {}

// =========================================================================
// Permissions
// =========================================================================

/// The level of access a plugin requires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionLevel {
    /// Read-only access to observability data.
    Read,
    /// Read and write to plugin state (not core state).
    Write,
    /// Access to plugin hooks only (cannot read/write core data).
    HookOnly,
}

impl fmt::Display for PermissionLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PermissionLevel::Read => write!(f, "read"),
            PermissionLevel::Write => write!(f, "write"),
            PermissionLevel::HookOnly => write!(f, "hook_only"),
        }
    }
}

/// A permission granted to a plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permission {
    pub domain: SecurityDomain,
    pub level: PermissionLevel,
    pub description: String,
}

impl Permission {
    pub fn new(domain: SecurityDomain, level: PermissionLevel, description: &str) -> Self {
        Permission {
            domain,
            level,
            description: description.to_string(),
        }
    }
}

/// Security domain for permissions.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SecurityDomain {
    /// Read observability events (non-sensitive).
    Observability,
    /// Read preference data (read-only).
    Preferences,
    /// Modify preferences (requires approval).
    PreferencesWrite,
    /// Access to pipeline state (read-only).
    Pipeline,
    /// Access to tool registry (read-only).
    Tools,
    /// Access to provider registry (read-only).
    Providers,
    /// Access to agent state (read-only).
    Agent,
    /// Custom domain.
    Custom(String),
}

impl fmt::Display for SecurityDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecurityDomain::Observability => write!(f, "observability"),
            SecurityDomain::Preferences => write!(f, "preferences"),
            SecurityDomain::PreferencesWrite => write!(f, "preferences:write"),
            SecurityDomain::Pipeline => write!(f, "pipeline"),
            SecurityDomain::Tools => write!(f, "tools"),
            SecurityDomain::Providers => write!(f, "providers"),
            SecurityDomain::Agent => write!(f, "agent"),
            SecurityDomain::Custom(s) => write!(f, "{s}"),
        }
    }
}

// =========================================================================
// Plugin Source
// =========================================================================

/// Where a plugin is loaded from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginSource {
    /// Local file path.
    Local(String),
    /// Remote URL (marketplace).
    Remote(String),
    /// Embedded in binary (internal).
    Internal,
    /// Generated by AI (AI plugin).
    AiGenerated,
}

impl fmt::Display for PluginSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginSource::Local(p) => write!(f, "local:{p}"),
            PluginSource::Remote(u) => write!(f, "remote:{u}"),
            PluginSource::Internal => write!(f, "internal"),
            PluginSource::AiGenerated => write!(f, "ai-generated"),
        }
    }
}

// =========================================================================
// Hook Types
// =========================================================================

/// Phase at which a hook fires in the pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookPhase {
    /// Before the pipeline starts.
    PipelineStarted,
    /// After intent is resolved.
    IntentResolved,
    /// After recommendations are generated.
    RecommendationsGenerated,
    /// After a workflow is created.
    WorkflowCreated,
    /// After validation completes.
    ValidationCompleted,
    /// After approval is granted.
    ApprovalGranted,
    /// After a preference is applied.
    PreferenceApplied,
    /// After the pipeline finishes.
    PipelineFinished,
    /// On any observability event.
    ObservabilityEvent,
    /// Custom phase.
    Custom(String),
}

impl fmt::Display for HookPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HookPhase::PipelineStarted => write!(f, "pipeline_started"),
            HookPhase::IntentResolved => write!(f, "intent_resolved"),
            HookPhase::RecommendationsGenerated => write!(f, "recommendations_generated"),
            HookPhase::WorkflowCreated => write!(f, "workflow_created"),
            HookPhase::ValidationCompleted => write!(f, "validation_completed"),
            HookPhase::ApprovalGranted => write!(f, "approval_granted"),
            HookPhase::PreferenceApplied => write!(f, "preference_applied"),
            HookPhase::PipelineFinished => write!(f, "pipeline_finished"),
            HookPhase::ObservabilityEvent => write!(f, "observability_event"),
            HookPhase::Custom(s) => write!(f, "{s}"),
        }
    }
}

// =========================================================================
// CodeBro Version
// =========================================================================

/// Semantic version for CodeBro itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeBroVersion(pub String);

impl CodeBroVersion {
    pub fn current() -> Self {
        CodeBroVersion("1.0.0".to_string())
    }

    pub fn major(&self) -> u32 {
        self.0
            .split('.')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    pub fn minor(&self) -> u32 {
        self.0
            .split('.')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    pub fn patch(&self) -> u32 {
        self.0
            .split('.')
            .nth(2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }
}

impl fmt::Display for CodeBroVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
