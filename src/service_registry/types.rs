#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Core types for the Service Registry.
//!
//! Every service declares:
//! - Service ID, Name, Version
//! - Provider Plugin
//! - Capabilities
//! - Permissions
//! - Dependencies
//! - SDK Version
//! - Metadata

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

// =========================================================================
// Identifiers
// =========================================================================

/// Unique identifier for a service.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServiceId(pub String);

impl ServiceId {
    pub fn new(id: &str) -> Result<Self, ServiceIdError> {
        if id.is_empty() {
            return Err(ServiceIdError::Empty);
        }
        if id.contains(' ') {
            return Err(ServiceIdError::InvalidChar(' '));
        }
        Ok(ServiceId(id.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceIdError {
    Empty,
    InvalidChar(char),
}

impl fmt::Display for ServiceIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServiceIdError::Empty => write!(f, "ServiceId cannot be empty"),
            ServiceIdError::InvalidChar(c) => write!(f, "ServiceId contains invalid character: {c}"),
        }
    }
}

impl std::error::Error for ServiceIdError {}

/// Service name — human-readable identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServiceName(pub String);

impl ServiceName {
    pub fn new(name: &str) -> Result<Self, ServiceNameError> {
        if name.is_empty() {
            return Err(ServiceNameError::Empty);
        }
        Ok(ServiceName(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServiceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceNameError {
    Empty,
}

impl fmt::Display for ServiceNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ServiceName cannot be empty")
    }
}

impl std::error::Error for ServiceNameError {}

/// Semantic version for a service.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ServiceVersion(pub String);

impl ServiceVersion {
    pub fn new(v: &str) -> Result<Self, VersionError> {
        let parts: Vec<&str> = v.split('.').collect();
        if parts.len() != 3 {
            return Err(VersionError::InvalidFormat(v.to_string()));
        }
        for part in &parts {
            if part.parse::<u32>().is_err() {
                return Err(VersionError::InvalidPart(part.to_string()));
            }
        }
        Ok(ServiceVersion(v.to_string()))
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

impl fmt::Display for ServiceVersion {
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
// Capabilities
// =========================================================================

/// A capability that a service provides.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Data read capability.
    Read,
    /// Data write capability.
    Write,
    /// Executable action.
    Execute,
    /// Streaming data.
    Stream,
    /// Plugin hook integration.
    Hook,
    /// Tool execution.
    Tool,
    /// Provider integration.
    Provider,
    /// Agent coordination.
    Agent,
    /// File system access.
    FileSystem,
    /// Network access.
    Network,
    /// Custom capability.
    Custom(String),
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Capability::Read => write!(f, "read"),
            Capability::Write => write!(f, "write"),
            Capability::Execute => write!(f, "execute"),
            Capability::Stream => write!(f, "stream"),
            Capability::Hook => write!(f, "hook"),
            Capability::Tool => write!(f, "tool"),
            Capability::Provider => write!(f, "provider"),
            Capability::Agent => write!(f, "agent"),
            Capability::FileSystem => write!(f, "filesystem"),
            Capability::Network => write!(f, "network"),
            Capability::Custom(s) => write!(f, "{s}"),
        }
    }
}

// =========================================================================
// Service Status
// =========================================================================

/// Lifecycle status of a registered service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceStatus {
    Registered,
    Activated,
    Deactivated,
    Error(String),
}

impl fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServiceStatus::Registered => write!(f, "registered"),
            ServiceStatus::Activated => write!(f, "activated"),
            ServiceStatus::Deactivated => write!(f, "deactivated"),
            ServiceStatus::Error(s) => write!(f, "error({s})"),
        }
    }
}

// =========================================================================
// Priority
// =========================================================================

/// Resolution priority for services with the same name and version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServicePriority {
    Critical,
    High,
    Medium,
    Low,
    Custom(u32),
}

impl PartialOrd for ServicePriority {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ServicePriority {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.order_value().cmp(&other.order_value())
    }
}

impl ServicePriority {
    fn order_value(&self) -> u32 {
        match self {
            ServicePriority::Critical => 4,
            ServicePriority::High => 3,
            ServicePriority::Medium => 2,
            ServicePriority::Low => 1,
            ServicePriority::Custom(n) => *n,
        }
    }
}

impl fmt::Display for ServicePriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServicePriority::Critical => write!(f, "critical"),
            ServicePriority::High => write!(f, "high"),
            ServicePriority::Medium => write!(f, "medium"),
            ServicePriority::Low => write!(f, "low"),
            ServicePriority::Custom(n) => write!(f, "custom:{n}"),
        }
    }
}

impl Default for ServicePriority {
    fn default() -> Self {
        ServicePriority::Medium
    }
}

// =========================================================================
// Visibility
// =========================================================================

/// Who can discover and resolve this service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    /// Visible to all plugins.
    Public,
    /// Visible only to the provider plugin and explicitly granted plugins.
    Private,
    /// Visible within the same namespace.
    Namespace(String),
}

impl fmt::Display for Visibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Visibility::Public => write!(f, "public"),
            Visibility::Private => write!(f, "private"),
            Visibility::Namespace(n) => write!(f, "namespace:{n}"),
        }
    }
}

// =========================================================================
// Permission Level
// =========================================================================

/// Access level for service resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessLevel {
    None,
    Read,
    Write,
    Admin,
}

impl fmt::Display for AccessLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccessLevel::None => write!(f, "none"),
            AccessLevel::Read => write!(f, "read"),
            AccessLevel::Write => write!(f, "write"),
            AccessLevel::Admin => write!(f, "admin"),
        }
    }
}

// =========================================================================
// Dependencies
// =========================================================================

/// A dependency that a service requires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceDependency {
    pub service_id: ServiceId,
    pub min_version: ServiceVersion,
    pub capability_required: Capability,
}

impl ServiceDependency {
    pub fn new(
        service_id: ServiceId,
        min_version: ServiceVersion,
        capability_required: Capability,
    ) -> Self {
        ServiceDependency {
            service_id,
            min_version,
            capability_required,
        }
    }
}

// =========================================================================
// Service Metadata
// =========================================================================

/// Arbitrary key-value metadata attached to a service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceMetadata(pub HashMap<String, String>);

impl ServiceMetadata {
    pub fn new() -> Self {
        ServiceMetadata(HashMap::new())
    }

    pub fn with(mut self, key: &str, value: &str) -> Self {
        self.0.insert(key.to_string(), value.to_string());
        self
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|s| s.as_str())
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }
}

impl Default for ServiceMetadata {
    fn default() -> Self {
        Self::new()
    }
}

// =========================================================================
// Service Resolution
// =========================================================================

/// Result of a service resolution attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionResult {
    Found {
        service_id: ServiceId,
        version: ServiceVersion,
        provider: String,
        priority: ServicePriority,
        registration_order: u64,
    },
    Ambiguous {
        candidates: Vec<AmbiguousCandidate>,
    },
    VersionConflict {
        available_versions: Vec<ServiceVersion>,
        requested: ServiceVersion,
    },
    CapabilityMismatch {
        required: Capability,
        available: Vec<Capability>,
    },
    PermissionDenied {
        requester: String,
        service_id: ServiceId,
        required_access: AccessLevel,
    },
    NotFound {
        name: String,
    },
}

impl ResolutionResult {
    pub fn is_success(&self) -> bool {
        matches!(self, ResolutionResult::Found { .. })
    }

    pub fn service_id(&self) -> Option<&ServiceId> {
        match self {
            ResolutionResult::Found { service_id, .. } => Some(service_id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbiguousCandidate {
    pub service_id: ServiceId,
    pub version: ServiceVersion,
    pub provider: String,
    pub priority: ServicePriority,
    pub registration_order: u64,
}

// =========================================================================
// Discovery Filters
// =========================================================================

/// Filters for service discovery queries.
#[derive(Debug, Clone, Default)]
pub struct DiscoveryFilter {
    pub name_prefix: Option<String>,
    pub provider: Option<String>,
    pub capabilities: Vec<Capability>,
    pub min_version: Option<ServiceVersion>,
    pub max_version: Option<ServiceVersion>,
    pub visibility: Option<Visibility>,
    pub status: Option<ServiceStatus>,
    pub metadata_contains: HashMap<String, String>,
}

impl DiscoveryFilter {
    pub fn new() -> Self {
        DiscoveryFilter::default()
    }

    pub fn by_name_prefix(mut self, prefix: &str) -> Self {
        self.name_prefix = Some(prefix.to_string());
        self
    }

    pub fn by_provider(mut self, provider: &str) -> Self {
        self.provider = Some(provider.to_string());
        self
    }

    pub fn with_capabilities(mut self, caps: Vec<Capability>) -> Self {
        self.capabilities = caps;
        self
    }

    pub fn with_version_range(
        mut self,
        min: ServiceVersion,
        max: ServiceVersion,
    ) -> Self {
        self.min_version = Some(min);
        self.max_version = Some(max);
        self
    }

    pub fn with_visibility(mut self, vis: Visibility) -> Self {
        self.visibility = Some(vis);
        self
    }

    pub fn with_status(mut self, status: ServiceStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn matches(&self, service: &crate::service_registry::Service) -> bool {
        if let Some(ref prefix) = self.name_prefix {
            if !service.name.0.as_str().starts_with(prefix) {
                return false;
            }
        }
        if let Some(ref provider) = self.provider {
            if &service.provider != provider {
                return false;
            }
        }
        if !self.capabilities.is_empty() {
            let service_caps: Vec<&Capability> =
                service.capabilities.iter().collect();
            if !self
                .capabilities
                .iter()
                .all(|c| service_caps.contains(&c))
            {
                return false;
            }
        }
        if let Some(ref min_v) = self.min_version {
            if service.version < *min_v {
                return false;
            }
        }
        if let Some(ref max_v) = self.max_version {
            if service.version > *max_v {
                return false;
            }
        }
        if let Some(ref vis) = self.visibility {
            if &service.visibility != vis {
                return false;
            }
        }
        if let Some(ref status) = self.status {
            if &service.status != status {
                return false;
            }
        }
        for (k, v) in &self.metadata_contains {
            if service.metadata.get(k) != Some(v.as_str()) {
                return false;
            }
        }
        true
    }
}

// =========================================================================
// Diagnostics Events
// =========================================================================

/// Recorded diagnostic event from the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RegistryDiagnosticEvent {
    ServiceRegistered {
        service_id: ServiceId,
        version: ServiceVersion,
        provider: String,
    },
    ServiceUnregistered {
        service_id: ServiceId,
    },
    ServiceActivated {
        service_id: ServiceId,
    },
    ServiceDeactivated {
        service_id: ServiceId,
    },
    ServiceResolved {
        service_id: ServiceId,
        version: ServiceVersion,
        requester: String,
        resolution_time_ms: f64,
    },
    ResolutionFailed {
        query_name: String,
        reason: String,
    },
    PermissionDenied {
        requester: String,
        service_id: ServiceId,
        required_access: AccessLevel,
    },
    DependencyViolation {
        service_id: ServiceId,
        missing_dependency: ServiceId,
    },
}

impl fmt::Display for RegistryDiagnosticEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryDiagnosticEvent::ServiceRegistered {
                service_id,
                version,
                provider,
            } => write!(
                f,
                "ServiceRegistered(id={service_id}, version={version}, provider={provider})"
            ),
            RegistryDiagnosticEvent::ServiceUnregistered { service_id } => {
                write!(f, "ServiceUnregistered(id={service_id})")
            }
            RegistryDiagnosticEvent::ServiceActivated { service_id } => {
                write!(f, "ServiceActivated(id={service_id})")
            }
            RegistryDiagnosticEvent::ServiceDeactivated { service_id } => {
                write!(f, "ServiceDeactivated(id={service_id})")
            }
            RegistryDiagnosticEvent::ServiceResolved {
                service_id,
                version,
                requester,
                resolution_time_ms,
            } => write!(
                f,
                "ServiceResolved(id={service_id}, version={version}, requester={requester}, time={resolution_time_ms}ms)"
            ),
            RegistryDiagnosticEvent::ResolutionFailed {
                query_name,
                reason,
            } => write!(
                f,
                "ResolutionFailed(name={query_name}, reason={reason})"
            ),
            RegistryDiagnosticEvent::PermissionDenied {
                requester,
                service_id,
                required_access,
            } => write!(
                f,
                "PermissionDenied(requester={requester}, service={service_id}, access={required_access})"
            ),
            RegistryDiagnosticEvent::DependencyViolation {
                service_id,
                missing_dependency,
            } => write!(
                f,
                "DependencyViolation(service={service_id}, missing={missing_dependency})"
            ),
        }
    }
}

// =========================================================================
// Registry Statistics
// =========================================================================

/// Aggregate statistics for the service registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryStatistics {
    pub total_registered: usize,
    pub total_activated: usize,
    pub total_deactivated: usize,
    pub total_errors: usize,
    pub resolution_count: u64,
    pub resolution_success_count: u64,
    pub resolution_failure_count: u64,
    pub permission_violations: u64,
    pub dependency_violations: u64,
    pub avg_resolution_time_ms: f64,
    pub recent_failures: Vec<ResolutionFailureRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionFailureRecord {
    pub query_name: String,
    pub reason: String,
    pub timestamp: String,
}

impl RegistryStatistics {
    pub fn new() -> Self {
        RegistryStatistics {
            total_registered: 0,
            total_activated: 0,
            total_deactivated: 0,
            total_errors: 0,
            resolution_count: 0,
            resolution_success_count: 0,
            resolution_failure_count: 0,
            permission_violations: 0,
            dependency_violations: 0,
            avg_resolution_time_ms: 0.0,
            recent_failures: Vec::new(),
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "RegistryStatistics {{\n  total_registered: {}\n  total_activated: {}\n  total_deactivated: {}\n  total_errors: {}\n  resolution_count: {}\n  resolution_success: {}\n  resolution_failure: {}\n  permission_violations: {}\n  dependency_violations: {}\n  avg_resolution_ms: {:.2}\n}}",
            self.total_registered,
            self.total_activated,
            self.total_deactivated,
            self.total_errors,
            self.resolution_count,
            self.resolution_success_count,
            self.resolution_failure_count,
            self.permission_violations,
            self.dependency_violations,
            self.avg_resolution_time_ms,
        )
    }
}

impl Default for RegistryStatistics {
    fn default() -> Self {
        Self::new()
    }
}
