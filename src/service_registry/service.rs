#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Service definition and builder.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

use super::types::*;

// =========================================================================
// Service
// =========================================================================

/// A registered service in the registry.
///
/// Immutable after construction. All fields are set at registration time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub id: ServiceId,
    pub name: ServiceName,
    pub version: ServiceVersion,
    pub provider: String,
    pub capabilities: Vec<Capability>,
    pub permissions: Vec<ServicePermission>,
    pub dependencies: Vec<ServiceDependency>,
    pub sdk_version: ServiceVersion,
    pub priority: ServicePriority,
    pub visibility: Visibility,
    pub metadata: ServiceMetadata,
    pub registration_order: u64,
    pub status: ServiceStatus,
}

impl Service {
    pub fn builder() -> ServiceBuilder {
        ServiceBuilder::new()
    }

    pub fn has_capability(&self, cap: &Capability) -> bool {
        self.capabilities.contains(cap)
    }

    pub fn has_permission_for(&self, requester: &str, required: &AccessLevel) -> bool {
        for perm in &self.permissions {
            if perm.grantee == requester || perm.grantee == "*" {
                if perm.access_level == *required || perm.access_level == AccessLevel::Admin {
                    return true;
                }
            }
        }
        // Public services allow read access by default
        if matches!(self.visibility, Visibility::Public) && *required == AccessLevel::Read {
            return true;
        }
        false
    }

    pub fn check_dependencies_satisfied(&self, registered: &[Service]) -> Vec<ServiceId> {
        let mut missing = Vec::new();
        for dep in &self.dependencies {
            let found = registered.iter().any(|s| {
                s.id == dep.service_id
                    && s.version >= dep.min_version
                    && s.has_capability(&dep.capability_required)
                    && matches!(
                        s.status,
                        ServiceStatus::Activated | ServiceStatus::Registered
                    )
            });
            if !found {
                missing.push(dep.service_id.clone());
            }
        }
        missing
    }
}

// =========================================================================
// Service Permission
// =========================================================================

/// Permission grant for a specific service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePermission {
    pub grantee: String,
    pub access_level: AccessLevel,
    pub description: String,
}

impl ServicePermission {
    pub fn new(grantee: &str, access: AccessLevel, desc: &str) -> Self {
        ServicePermission {
            grantee: grantee.to_string(),
            access_level: access,
            description: desc.to_string(),
        }
    }
}

// =========================================================================
// Service Builder
// =========================================================================

/// Builder for constructing Service instances.
pub struct ServiceBuilder {
    id: Option<ServiceId>,
    name: Option<ServiceName>,
    version: Option<ServiceVersion>,
    provider: Option<String>,
    capabilities: Vec<Capability>,
    permissions: Vec<ServicePermission>,
    dependencies: Vec<ServiceDependency>,
    sdk_version: ServiceVersion,
    priority: ServicePriority,
    visibility: Visibility,
    metadata: ServiceMetadata,
    registration_order: u64,
    status: ServiceStatus,
}

impl ServiceBuilder {
    pub fn new() -> Self {
        ServiceBuilder {
            id: None,
            name: None,
            version: None,
            provider: None,
            capabilities: Vec::new(),
            permissions: Vec::new(),
            dependencies: Vec::new(),
            sdk_version: ServiceVersion::new("1.0.0").unwrap(),
            priority: ServicePriority::default(),
            visibility: Visibility::Public,
            metadata: ServiceMetadata::new(),
            registration_order: 0,
            status: ServiceStatus::Registered,
        }
    }

    pub fn with_id(mut self, id: ServiceId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn with_name(mut self, name: ServiceName) -> Self {
        self.name = Some(name);
        self
    }

    pub fn with_version(mut self, version: ServiceVersion) -> Self {
        self.version = Some(version);
        self
    }

    pub fn with_provider(mut self, provider: &str) -> Self {
        self.provider = Some(provider.to_string());
        self
    }

    pub fn with_capabilities(mut self, caps: Vec<Capability>) -> Self {
        self.capabilities = caps;
        self
    }

    pub fn add_capability(mut self, cap: Capability) -> Self {
        self.capabilities.push(cap);
        self
    }

    pub fn with_permissions(mut self, perms: Vec<ServicePermission>) -> Self {
        self.permissions = perms;
        self
    }

    pub fn add_permission(mut self, perm: ServicePermission) -> Self {
        self.permissions.push(perm);
        self
    }

    pub fn with_dependencies(mut self, deps: Vec<ServiceDependency>) -> Self {
        self.dependencies = deps;
        self
    }

    pub fn add_dependency(mut self, dep: ServiceDependency) -> Self {
        self.dependencies.push(dep);
        self
    }

    pub fn with_sdk_version(mut self, version: ServiceVersion) -> Self {
        self.sdk_version = version;
        self
    }

    pub fn with_priority(mut self, priority: ServicePriority) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_visibility(mut self, vis: Visibility) -> Self {
        self.visibility = vis;
        self
    }

    pub fn with_metadata(mut self, meta: ServiceMetadata) -> Self {
        self.metadata = meta;
        self
    }

    pub fn with_registration_order(mut self, order: u64) -> Self {
        self.registration_order = order;
        self
    }

    pub fn with_status(mut self, status: ServiceStatus) -> Self {
        self.status = status;
        self
    }

    pub fn build(self) -> Result<Service, ServiceBuildError> {
        let id = self
            .id
            .ok_or(ServiceBuildError::MissingField("id".to_string()))?;
        let name = self
            .name
            .ok_or(ServiceBuildError::MissingField("name".to_string()))?;
        let version = self
            .version
            .ok_or(ServiceBuildError::MissingField("version".to_string()))?;
        let provider = self
            .provider
            .clone()
            .ok_or(ServiceBuildError::MissingField("provider".to_string()))?;

        Ok(Service {
            id,
            name,
            version,
            provider,
            capabilities: self.capabilities,
            permissions: self.permissions,
            dependencies: self.dependencies,
            sdk_version: self.sdk_version,
            priority: self.priority,
            visibility: self.visibility,
            metadata: self.metadata,
            registration_order: self.registration_order,
            status: self.status,
        })
    }
}

impl Default for ServiceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum ServiceBuildError {
    MissingField(String),
}

impl fmt::Display for ServiceBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServiceBuildError::MissingField(field) => {
                write!(f, "Missing required field: {field}")
            }
        }
    }
}

impl std::error::Error for ServiceBuildError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_service(id: &str, name: &str, version: &str, provider: &str) -> Service {
        Service::builder()
            .with_id(ServiceId::new(id).unwrap())
            .with_name(ServiceName::new(name).unwrap())
            .with_version(ServiceVersion::new(version).unwrap())
            .with_provider(provider)
            .with_capabilities(vec![Capability::Read, Capability::Write])
            .with_sdk_version(ServiceVersion::new("1.0.0").unwrap())
            .build()
            .unwrap()
    }

    #[test]
    fn test_build_service() {
        let svc = make_service("s1", "test-service", "1.0.0", "plugin-a");
        assert_eq!(svc.id.as_str(), "s1");
        assert_eq!(svc.name.as_str(), "test-service");
        assert_eq!(svc.version.to_string(), "1.0.0");
        assert_eq!(svc.provider, "plugin-a");
        assert_eq!(svc.capabilities.len(), 2);
        assert!(svc.has_capability(&Capability::Read));
        assert!(svc.has_capability(&Capability::Write));
        assert!(!svc.has_capability(&Capability::Execute));
    }

    #[test]
    fn test_builder_missing_fields() {
        let result = Service::builder().build();
        assert!(result.is_err());
    }

    #[test]
    fn test_capability_check() {
        let svc = make_service("s1", "test", "1.0.0", "p");
        assert!(svc.has_capability(&Capability::Read));
        assert!(svc.has_capability(&Capability::Write));
        assert!(!svc.has_capability(&Capability::Execute));
    }

    #[test]
    fn test_permission_public_read() {
        let svc = make_service("s1", "test", "1.0.0", "p");
        assert!(svc.has_permission_for("anyone", &AccessLevel::Read));
    }

    #[test]
    fn test_permission_granted() {
        let perm = ServicePermission::new("plugin-b", AccessLevel::Write, "write access");
        let svc = make_service("s1", "test", "1.0.0", "p").clone();
        // Service is immutable after build; test via a new builder
        let svc = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("test").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p")
            .with_capabilities(vec![Capability::Read])
            .with_permissions(vec![perm])
            .build()
            .unwrap();
        assert!(svc.has_permission_for("plugin-b", &AccessLevel::Write));
        assert!(!svc.has_permission_for("plugin-c", &AccessLevel::Write));
    }

    #[test]
    fn test_dependency_check_satisfied() {
        let dep = ServiceDependency::new(
            ServiceId::new("dep1").unwrap(),
            ServiceVersion::new("1.0.0").unwrap(),
            Capability::Read,
        );
        let svc = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("test").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p")
            .with_capabilities(vec![Capability::Read])
            .with_dependencies(vec![dep])
            .build()
            .unwrap();
        let dep_svc = Service::builder()
            .with_id(ServiceId::new("dep1").unwrap())
            .with_name(ServiceName::new("dep").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p")
            .with_capabilities(vec![Capability::Read])
            .build()
            .unwrap();
        let missing = svc.check_dependencies_satisfied(&[dep_svc]);
        assert!(missing.is_empty());
    }

    #[test]
    fn test_dependency_check_missing() {
        let dep = ServiceDependency::new(
            ServiceId::new("dep1").unwrap(),
            ServiceVersion::new("1.0.0").unwrap(),
            Capability::Read,
        );
        let svc = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("test").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p")
            .with_capabilities(vec![Capability::Read])
            .with_dependencies(vec![dep])
            .build()
            .unwrap();
        let missing = svc.check_dependencies_satisfied(&[]);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].as_str(), "dep1");
    }

    #[test]
    fn test_service_metadata() {
        let meta = ServiceMetadata::new()
            .with("env", "production")
            .with("region", "us-east-1");
        let svc = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("test").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p")
            .with_capabilities(vec![Capability::Read])
            .with_metadata(meta)
            .build()
            .unwrap();
        assert_eq!(svc.metadata.get("env"), Some("production"));
        assert_eq!(svc.metadata.get("region"), Some("us-east-1"));
        assert_eq!(svc.metadata.get("missing"), None);
    }

    #[test]
    fn test_visibility_private() {
        let svc = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("test").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p")
            .with_capabilities(vec![Capability::Read])
            .with_visibility(Visibility::Private)
            .build()
            .unwrap();
        assert!(!svc.has_permission_for("anyone", &AccessLevel::Read));
    }

    #[test]
    fn test_service_version_comparison() {
        let v1 = ServiceVersion::new("1.0.0").unwrap();
        let v2 = ServiceVersion::new("1.1.0").unwrap();
        let v3 = ServiceVersion::new("2.0.0").unwrap();
        assert!(v2 > v1);
        assert!(v3 > v2);
        assert!(v1 < v2);
    }

    #[test]
    fn test_priority_ordering() {
        assert!(ServicePriority::Critical > ServicePriority::High);
        assert!(ServicePriority::High > ServicePriority::Medium);
        assert!(ServicePriority::Medium > ServicePriority::Low);
    }
}
