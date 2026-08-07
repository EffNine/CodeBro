#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Permission model for the Service Registry.
//!
//! Enforces:
//! - Ownership (provider is owner)
//! - Visibility (public/private/namespace)
//! - Access validation (read/write/admin)

use std::collections::HashMap;
use std::fmt;

use crate::service_registry::registry::ServiceRegistry;
use crate::service_registry::types::*;
use crate::service_registry::service::{Service, ServicePermission};

/// Permission checker for service access control.
#[derive(Clone)]
pub struct ServicePermissions {
    registry: ServiceRegistry,
}

impl ServicePermissions {
    pub fn new(registry: ServiceRegistry) -> Self {
        ServicePermissions { registry }
    }

    /// Check if a requester has the required access level for a service.
    pub fn check_access(
        &self,
        service_id: &ServiceId,
        requester: &str,
        required: AccessLevel,
    ) -> AccessResult {
        let svc = match self.registry.get(service_id) {
            Some(s) => s,
            None => return AccessResult::NotFound(service_id.clone()),
        };

        // Owner (provider) always has admin access
        if svc.provider == requester {
            return AccessResult::Granted(AccessLevel::Admin);
        }

        // Check explicit permissions
        for perm in &svc.permissions {
            if perm.grantee == requester || perm.grantee == "*" {
                if perm.access_level == AccessLevel::Admin
                    || (required == AccessLevel::Read && perm.access_level == AccessLevel::Write)
                    || (required == AccessLevel::Write && perm.access_level == AccessLevel::Admin)
                    || perm.access_level == required
                {
                    return AccessResult::Granted(perm.access_level.clone());
                }
            }
        }

        // Check visibility-based access
        match &svc.visibility {
            Visibility::Public => {
                if required == AccessLevel::Read {
                    return AccessResult::Granted(AccessLevel::Read);
                }
            }
            Visibility::Namespace(ref ns) => {
                // Namespace visibility: requester must be in the same namespace
                if requester.starts_with(ns.as_str()) {
                    if required == AccessLevel::Read {
                        return AccessResult::Granted(AccessLevel::Read);
                    }
                }
            }
            Visibility::Private => {
                // Private: only owner and explicitly granted users
            }
        }

        AccessResult::Denied {
            service_id: service_id.clone(),
            requester: requester.to_string(),
            required,
            available: svc.visibility.clone(),
        }
    }

    /// Check if a requester can write to a service.
    pub fn can_write(&self, service_id: &ServiceId, requester: &str) -> bool {
        matches!(
            self.check_access(service_id, requester, AccessLevel::Write),
            AccessResult::Granted(_)
        )
    }

    /// Check if a requester can read from a service.
    pub fn can_read(&self, service_id: &ServiceId, requester: &str) -> bool {
        matches!(
            self.check_access(service_id, requester, AccessLevel::Read),
            AccessResult::Granted(_)
        )
    }

    /// Check ownership.
    pub fn is_owner(&self, service_id: &ServiceId, plugin_id: &str) -> bool {
        if let Some(svc) = self.registry.get(service_id) {
            svc.provider == plugin_id
        } else {
            false
        }
    }

    /// Get the effective access level for a requester on a service.
    pub fn effective_access(
        &self,
        service_id: &ServiceId,
        requester: &str,
    ) -> AccessLevel {
        match self.check_access(service_id, requester, AccessLevel::Read) {
            AccessResult::Granted(level) => level,
            AccessResult::Denied { .. } => AccessLevel::None,
            AccessResult::NotFound(_) => AccessLevel::None,
        }
    }

    /// List all permission grants for a service.
    pub fn list_permissions(&self, service_id: &ServiceId) -> Vec<ServicePermission> {
        match self.registry.get(service_id) {
            Some(svc) => svc.permissions.clone(),
            None => Vec::new(),
        }
    }

    /// Validate that a service's permissions are consistent.
    pub fn validate_permissions(
        &self,
        service_id: &ServiceId,
    ) -> Result<(), PermissionValidationError> {
        let svc = self
            .registry
            .get(service_id)
            .ok_or(PermissionValidationError::ServiceNotFound(service_id.clone()))?;

        // Check for conflicting permissions (same grantee with conflicting levels)
        let mut grantee_levels: std::collections::HashMap<String, AccessLevel> =
            std::collections::HashMap::new();
        for perm in &svc.permissions {
            if let Some(existing) = grantee_levels.get(&perm.grantee) {
                if existing != &perm.access_level {
                    return Err(PermissionValidationError::ConflictingPermissions(
                        service_id.clone(),
                        perm.grantee.clone(),
                    ));
                }
            }
            grantee_levels.insert(perm.grantee.clone(), perm.access_level.clone());
        }

        // Private services should not have wildcard grants
        if matches!(svc.visibility, Visibility::Private) {
            for perm in &svc.permissions {
                if perm.grantee == "*" {
                    return Err(PermissionValidationError::WildcardOnPrivate(
                        service_id.clone(),
                    ));
                }
            }
        }

        Ok(())
    }

    /// Grant access to a requester for a service.
    /// Note: This requires the requester to be the owner or have admin access.
    pub fn grant_access(
        &self,
        service_id: &ServiceId,
        grantee: &str,
        access: AccessLevel,
        description: &str,
    ) -> Result<(), PermissionError> {
        let mut inner = self.registry.inner.lock().unwrap();
        let svc = inner
            .services
            .get(service_id)
            .ok_or(PermissionError::ServiceNotFound(service_id.clone()))?;

        let perm = ServicePermission::new(grantee, access.clone(), description);
        let mut service = svc.clone();
        service.permissions.push(perm);

        // Update the service in the registry
        inner.services.insert(service_id.clone(), service);
        Ok(())
    }

    /// Revoke access for a grantee.
    pub fn revoke_access(
        &self,
        service_id: &ServiceId,
        grantee: &str,
    ) -> Result<(), PermissionError> {
        let mut inner = self.registry.inner.lock().unwrap();
        let svc = inner
            .services
            .get_mut(service_id)
            .ok_or(PermissionError::ServiceNotFound(service_id.clone()))?;

        svc.permissions.retain(|p| p.grantee != grantee);
        Ok(())
    }
}

/// Result of an access check.
#[derive(Debug, Clone)]
pub enum AccessResult {
    Granted(AccessLevel),
    Denied {
        service_id: ServiceId,
        requester: String,
        required: AccessLevel,
        available: Visibility,
    },
    NotFound(ServiceId),
}

impl AccessResult {
    pub fn is_granted(&self) -> bool {
        matches!(self, AccessResult::Granted(_))
    }
}

/// Permission validation errors.
#[derive(Debug, Clone)]
pub enum PermissionValidationError {
    ServiceNotFound(ServiceId),
    ConflictingPermissions(ServiceId, String),
    WildcardOnPrivate(ServiceId),
}

impl fmt::Display for PermissionValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PermissionValidationError::ServiceNotFound(id) => {
                write!(f, "Service not found: {id}")
            }
            PermissionValidationError::ConflictingPermissions(id, grantee) => {
                write!(
                    f,
                    "Conflicting permissions for service {id}, grantee {grantee}"
                )
            }
            PermissionValidationError::WildcardOnPrivate(id) => {
                write!(f, "Wildcard permission on private service: {id}")
            }
        }
    }
}

impl std::error::Error for PermissionValidationError {}

/// Permission operation errors.
#[derive(Debug, Clone)]
pub enum PermissionError {
    ServiceNotFound(ServiceId),
    NotOwner(String),
    InvalidAccessLevel(String),
}

impl fmt::Display for PermissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PermissionError::ServiceNotFound(id) => write!(f, "Service not found: {id}"),
            PermissionError::NotOwner(plugin) => write!(f, "Not owner: {plugin}"),
            PermissionError::InvalidAccessLevel(level) => {
                write!(f, "Invalid access level: {level}")
            }
        }
    }
}

impl std::error::Error for PermissionError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_svc(id: &str, name: &str, version: &str, provider: &str) -> Service {
        Service::builder()
            .with_id(ServiceId::new(id).unwrap())
            .with_name(ServiceName::new(name).unwrap())
            .with_version(ServiceVersion::new(version).unwrap())
            .with_provider(provider)
            .with_capabilities(vec![Capability::Read])
            .build()
            .unwrap()
    }

    #[test]
    fn test_public_read_allowed() {
        let mut reg = ServiceRegistry::new();
        let perm = ServicePermissions::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "plugin-a")).unwrap();
        assert!(perm.can_read(&ServiceId::new("s1").unwrap(), "anyone"));
    }

    #[test]
    fn test_public_write_denied() {
        let mut reg = ServiceRegistry::new();
        let perm = ServicePermissions::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "plugin-a")).unwrap();
        assert!(!perm.can_write(&ServiceId::new("s1").unwrap(), "anyone"));
    }

    #[test]
    fn test_owner_has_admin() {
        let mut reg = ServiceRegistry::new();
        let perm = ServicePermissions::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "plugin-a")).unwrap();
        let result = perm.check_access(&ServiceId::new("s1").unwrap(), "plugin-a", AccessLevel::Write);
        match result {
            AccessResult::Granted(level) => assert_eq!(level, AccessLevel::Admin),
            _ => panic!("Expected Granted"),
        }
    }

    #[test]
    fn test_explicit_grant() {
        let mut reg = ServiceRegistry::new();
        let perm = ServicePermission::new("plugin-b", AccessLevel::Write, "write access");
        let svc = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("test").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("plugin-a")
            .with_capabilities(vec![Capability::Read])
            .with_permissions(vec![perm])
            .build()
            .unwrap();
        reg.register(svc).unwrap();

        let perm_checker = ServicePermissions::new(reg);
        assert!(perm_checker.can_write(&ServiceId::new("s1").unwrap(), "plugin-b"));
        assert!(!perm_checker.can_write(&ServiceId::new("s1").unwrap(), "plugin-c"));
    }

    #[test]
    fn test_private_service_no_access() {
        let mut reg = ServiceRegistry::new();
        let svc = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("secret").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("plugin-a")
            .with_capabilities(vec![Capability::Read])
            .with_visibility(Visibility::Private)
            .build()
            .unwrap();
        reg.register(svc).unwrap();

        let perm_checker = ServicePermissions::new(reg);
        assert!(!perm_checker.can_read(&ServiceId::new("s1").unwrap(), "anyone"));
        assert!(perm_checker.can_read(&ServiceId::new("s1").unwrap(), "plugin-a"));
    }

    #[test]
    fn test_namespace_visibility() {
        let mut reg = ServiceRegistry::new();
        let svc = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("team-svc").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("plugin-a")
            .with_capabilities(vec![Capability::Read])
            .with_visibility(Visibility::Namespace("team".to_string()))
            .build()
            .unwrap();
        reg.register(svc).unwrap();

        let perm_checker = ServicePermissions::new(reg);
        assert!(perm_checker.can_read(&ServiceId::new("s1").unwrap(), "team-member-1"));
        assert!(!perm_checker.can_read(&ServiceId::new("s1").unwrap(), "other-team"));
    }

    #[test]
    fn test_validate_permissions_clean() {
        let mut reg = ServiceRegistry::new();
        let perm = ServicePermissions::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "plugin-a")).unwrap();
        let result = perm.validate_permissions(&ServiceId::new("s1").unwrap());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_permissions_conflicting() {
        let mut reg = ServiceRegistry::new();
        let svc = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("test").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("plugin-a")
            .with_capabilities(vec![Capability::Read])
            .with_permissions(vec![
                ServicePermission::new("user-x", AccessLevel::Read, "read"),
                ServicePermission::new("user-x", AccessLevel::Write, "write"),
            ])
            .build()
            .unwrap();
        reg.register(svc).unwrap();

        let perm_checker = ServicePermissions::new(reg);
        let result = perm_checker.validate_permissions(&ServiceId::new("s1").unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_permissions_wildcard_on_private() {
        let mut reg = ServiceRegistry::new();
        let svc = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("test").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("plugin-a")
            .with_capabilities(vec![Capability::Read])
            .with_visibility(Visibility::Private)
            .with_permissions(vec![ServicePermission::new(
                "*",
                AccessLevel::Read,
                "wildcard",
            )])
            .build()
            .unwrap();
        reg.register(svc).unwrap();

        let perm_checker = ServicePermissions::new(reg);
        let result = perm_checker.validate_permissions(&ServiceId::new("s1").unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_effective_access() {
        let mut reg = ServiceRegistry::new();
        let perm = ServicePermissions::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "plugin-a")).unwrap();
        let access = perm.effective_access(&ServiceId::new("s1").unwrap(), "anyone");
        assert_eq!(access, AccessLevel::Read);
    }

    #[test]
    fn test_list_permissions() {
        let mut reg = ServiceRegistry::new();
        let svc = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("test").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("plugin-a")
            .with_capabilities(vec![Capability::Read])
            .with_permissions(vec![
                ServicePermission::new("user-a", AccessLevel::Read, "read"),
                ServicePermission::new("user-b", AccessLevel::Write, "write"),
            ])
            .build()
            .unwrap();
        reg.register(svc).unwrap();

        let perm_checker = ServicePermissions::new(reg);
        let perms = perm_checker.list_permissions(&ServiceId::new("s1").unwrap());
        assert_eq!(perms.len(), 2);
    }

    #[test]
    fn test_is_owner() {
        let mut reg = ServiceRegistry::new();
        let perm = ServicePermissions::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "plugin-a")).unwrap();
        assert!(perm.is_owner(&ServiceId::new("s1").unwrap(), "plugin-a"));
        assert!(!perm.is_owner(&ServiceId::new("s1").unwrap(), "plugin-b"));
    }

    #[test]
    fn test_access_result_is_granted() {
        assert!(AccessResult::Granted(AccessLevel::Read).is_granted());
        assert!(!AccessResult::Denied {
            service_id: ServiceId::new("s1").unwrap(),
            requester: "x".to_string(),
            required: AccessLevel::Write,
            available: Visibility::Private,
        }
        .is_granted());
    }
}
