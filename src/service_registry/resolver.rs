#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Service Resolver — deterministic lookup, version negotiation,
//! capability matching, and dependency validation.
//!
//! Resolution order:
//! 1. Priority (Critical > High > Medium > Low)
//! 2. Version (higher is better)
//! 3. Registration Order (earlier is better)
//!
//! Never random.

use std::fmt;

use crate::observability::{CorrelationId, Event, EventBus, EventType};
use crate::service_registry::registry::ServiceRegistry;
use crate::service_registry::service::Service;
use crate::service_registry::types::*;

/// Resolver for finding the best matching service.
#[derive(Clone)]
pub struct ServiceResolver {
    registry: ServiceRegistry,
    event_bus: Option<EventBus>,
}

impl ServiceResolver {
    pub fn new(registry: ServiceRegistry) -> Self {
        ServiceResolver {
            registry,
            event_bus: None,
        }
    }

    pub fn with_event_bus(mut self, event_bus: EventBus) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    /// Resolve a service by name with capability requirements.
    ///
    /// Returns the best matching service according to resolution rules.
    pub fn resolve(
        &self,
        name: &str,
        requester: &str,
        required_capability: Option<&Capability>,
    ) -> ResolutionResult {
        let candidates = self.resolve_candidates(name, required_capability);

        if candidates.is_empty() {
            return ResolutionResult::NotFound {
                name: name.to_string(),
            };
        }

        // Sort by: priority (desc), version (desc), registration order (asc)
        let mut sorted = candidates;
        sorted.sort_by(|a, b| {
            // Priority: higher is better
            let pa = priority_order(&a.priority);
            let pb = priority_order(&b.priority);
            pb.cmp(&pa)
                .then_with(|| b.version.cmp(&a.version))
                .then_with(|| a.registration_order.cmp(&b.registration_order))
        });

        let best = &sorted[0];

        // Permission check
        if let Some(ref svc) = self.registry.get(&best.service_id) {
            if !svc.has_permission_for(requester, &AccessLevel::Read) {
                self.emit_event(RegistryDiagnosticEvent::PermissionDenied {
                    requester: requester.to_string(),
                    service_id: best.service_id.clone(),
                    required_access: AccessLevel::Read,
                });
                return ResolutionResult::PermissionDenied {
                    requester: requester.to_string(),
                    service_id: best.service_id.clone(),
                    required_access: AccessLevel::Read,
                };
            }
        }

        self.emit_event(RegistryDiagnosticEvent::ServiceResolved {
            service_id: best.service_id.clone(),
            version: best.version.clone(),
            requester: requester.to_string(),
            resolution_time_ms: 0.0,
        });

        ResolutionResult::Found {
            service_id: best.service_id.clone(),
            version: best.version.clone(),
            provider: best.provider.clone(),
            priority: best.priority.clone(),
            registration_order: best.registration_order,
        }
    }

    /// Resolve by exact service ID.
    pub fn resolve_by_id(&self, service_id: &ServiceId, requester: &str) -> ResolutionResult {
        match self.registry.get(service_id) {
            Some(svc) => {
                if !svc.has_permission_for(requester, &AccessLevel::Read) {
                    return ResolutionResult::PermissionDenied {
                        requester: requester.to_string(),
                        service_id: service_id.clone(),
                        required_access: AccessLevel::Read,
                    };
                }
                ResolutionResult::Found {
                    service_id: svc.id.clone(),
                    version: svc.version.clone(),
                    provider: svc.provider.clone(),
                    priority: svc.priority.clone(),
                    registration_order: svc.registration_order,
                }
            }
            None => ResolutionResult::NotFound {
                name: service_id.to_string(),
            },
        }
    }

    /// Resolve with version constraint.
    pub fn resolve_with_version(
        &self,
        name: &str,
        requester: &str,
        min_version: &ServiceVersion,
        max_version: Option<&ServiceVersion>,
        required_capability: Option<&Capability>,
    ) -> ResolutionResult {
        let candidates = self.resolve_candidates(name, required_capability);

        let filtered: Vec<&AmbiguousCandidate> = candidates
            .iter()
            .filter(|c| {
                let svc = self.registry.get(&c.service_id);
                if let Some(s) = svc {
                    if &s.version < min_version {
                        return false;
                    }
                    if let Some(max) = max_version {
                        if s.version > *max {
                            return false;
                        }
                    }
                    if !s.has_permission_for(requester, &AccessLevel::Read) {
                        return false;
                    }
                    true
                } else {
                    false
                }
            })
            .collect();

        if filtered.is_empty() {
            let available: Vec<ServiceVersion> = candidates
                .iter()
                .filter_map(|c| self.registry.get(&c.service_id).map(|s| s.version))
                .collect();
            return ResolutionResult::VersionConflict {
                available_versions: available,
                requested: min_version.clone(),
            };
        }

        // Sort by priority, version, registration order
        let mut sorted = filtered.clone();
        sorted.sort_by(|a, b| {
            let pa = priority_order(&a.priority);
            let pb = priority_order(&b.priority);
            pb.cmp(&pa)
                .then_with(|| b.version.cmp(&a.version))
                .then_with(|| a.registration_order.cmp(&b.registration_order))
        });

        let best = sorted[0];
        self.emit_event(RegistryDiagnosticEvent::ServiceResolved {
            service_id: best.service_id.clone(),
            version: best.version.clone(),
            requester: requester.to_string(),
            resolution_time_ms: 0.0,
        });

        ResolutionResult::Found {
            service_id: best.service_id.clone(),
            version: best.version.clone(),
            provider: best.provider.clone(),
            priority: best.priority.clone(),
            registration_order: best.registration_order,
        }
    }

    /// Check if all dependencies of a service are satisfied.
    pub fn validate_dependencies(
        &self,
        service_id: &ServiceId,
    ) -> Result<(), DependencyCheckError> {
        let svc = self
            .registry
            .get(service_id)
            .ok_or(DependencyCheckError::NotFound(service_id.clone()))?;

        let all_services = self.registry.enumerate(None);
        let missing = svc.check_dependencies_satisfied(&all_services);

        if !missing.is_empty() {
            for dep in &missing {
                self.emit_event(RegistryDiagnosticEvent::DependencyViolation {
                    service_id: service_id.clone(),
                    missing_dependency: dep.clone(),
                });
            }
            return Err(DependencyCheckError::MissingDependencies(missing));
        }

        Ok(())
    }

    /// Resolve all services matching a name (returns all versions).
    pub fn resolve_all(&self, name: &str) -> Vec<AmbiguousCandidate> {
        self.resolve_candidates(name, None)
    }

    fn resolve_candidates(
        &self,
        name: &str,
        required_capability: Option<&Capability>,
    ) -> Vec<AmbiguousCandidate> {
        let services = self.registry.enumerate_by_name(name);
        services
            .into_iter()
            .enumerate()
            .filter(|(_, svc)| {
                if let Some(cap) = required_capability {
                    svc.has_capability(cap)
                } else {
                    true
                }
            })
            .map(|(order, svc)| AmbiguousCandidate {
                service_id: svc.id.clone(),
                version: svc.version.clone(),
                provider: svc.provider.clone(),
                priority: svc.priority.clone(),
                registration_order: svc.registration_order,
            })
            .collect()
    }

    fn emit_event(&self, event: RegistryDiagnosticEvent) {
        if let Some(ref bus) = self.event_bus {
            bus.emit(&Event::new(
                EventType::Custom(event.to_string()),
                CorrelationId::new(),
                "service_resolver",
                &event.to_string(),
            ));
        }
    }
}

fn priority_order(p: &ServicePriority) -> u32 {
    match p {
        ServicePriority::Critical => 4,
        ServicePriority::High => 3,
        ServicePriority::Medium => 2,
        ServicePriority::Low => 1,
        ServicePriority::Custom(n) => *n,
    }
}

#[derive(Debug, Clone)]
pub enum DependencyCheckError {
    NotFound(ServiceId),
    MissingDependencies(Vec<ServiceId>),
}

impl fmt::Display for DependencyCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DependencyCheckError::NotFound(id) => write!(f, "Service not found: {id}"),
            DependencyCheckError::MissingDependencies(deps) => {
                write!(f, "Missing dependencies: {:?}", deps)
            }
        }
    }
}

impl std::error::Error for DependencyCheckError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_svc(
        id: &str,
        name: &str,
        version: &str,
        provider: &str,
        caps: Vec<Capability>,
    ) -> Service {
        Service::builder()
            .with_id(ServiceId::new(id).unwrap())
            .with_name(ServiceName::new(name).unwrap())
            .with_version(ServiceVersion::new(version).unwrap())
            .with_provider(provider)
            .with_capabilities(caps)
            .build()
            .unwrap()
    }

    fn make_registry() -> (ServiceRegistry, ServiceResolver) {
        let reg = ServiceRegistry::new();
        let resolver = ServiceResolver::new(reg.clone());
        (reg, resolver)
    }

    #[test]
    fn test_resolve_found() {
        let (mut reg, resolver) = make_registry();
        reg.register(make_svc(
            "s1",
            "data",
            "1.0.0",
            "p1",
            vec![Capability::Read],
        ))
        .unwrap();
        let result = resolver.resolve("data", "plugin-a", None);
        match result {
            ResolutionResult::Found { service_id, .. } => {
                assert_eq!(service_id.as_str(), "s1");
            }
            _ => panic!("Expected Found"),
        }
    }

    #[test]
    fn test_resolve_not_found() {
        let (mut reg, resolver) = make_registry();
        let result = resolver.resolve("nonexistent", "plugin-a", None);
        match result {
            ResolutionResult::NotFound { name } => assert_eq!(name, "nonexistent"),
            _ => panic!("Expected NotFound"),
        }
    }

    #[test]
    fn test_resolve_priority_wins() {
        let (mut reg, resolver) = make_registry();
        let low = Service::builder()
            .with_id(ServiceId::new("s-low").unwrap())
            .with_name(ServiceName::new("data").unwrap())
            .with_version(ServiceVersion::new("2.0.0").unwrap())
            .with_provider("p1")
            .with_capabilities(vec![Capability::Read])
            .with_priority(ServicePriority::Low)
            .build()
            .unwrap();
        let high = Service::builder()
            .with_id(ServiceId::new("s-high").unwrap())
            .with_name(ServiceName::new("data").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p2")
            .with_capabilities(vec![Capability::Read])
            .with_priority(ServicePriority::High)
            .build()
            .unwrap();
        reg.register(low).unwrap();
        reg.register(high).unwrap();

        let result = resolver.resolve("data", "plugin-a", None);
        match result {
            ResolutionResult::Found { service_id, .. } => {
                assert_eq!(service_id.as_str(), "s-high");
            }
            _ => panic!("Expected Found"),
        }
    }

    #[test]
    fn test_resolve_version_wins_tiebreaker() {
        let (mut reg, resolver) = make_registry();
        let v1 = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("data").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p1")
            .with_capabilities(vec![Capability::Read])
            .build()
            .unwrap();
        let v2 = Service::builder()
            .with_id(ServiceId::new("s2").unwrap())
            .with_name(ServiceName::new("data").unwrap())
            .with_version(ServiceVersion::new("2.0.0").unwrap())
            .with_provider("p2")
            .with_capabilities(vec![Capability::Read])
            .build()
            .unwrap();
        reg.register(v1).unwrap();
        reg.register(v2).unwrap();

        let result = resolver.resolve("data", "plugin-a", None);
        match result {
            ResolutionResult::Found { service_id, .. } => {
                assert_eq!(service_id.as_str(), "s2");
            }
            _ => panic!("Expected Found"),
        }
    }

    #[test]
    fn test_resolve_registration_order_wins_tiebreaker() {
        let (mut reg, resolver) = make_registry();
        let s1 = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("data").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p1")
            .with_capabilities(vec![Capability::Read])
            .build()
            .unwrap();
        let s2 = Service::builder()
            .with_id(ServiceId::new("s2").unwrap())
            .with_name(ServiceName::new("data").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p2")
            .with_capabilities(vec![Capability::Read])
            .build()
            .unwrap();
        reg.register(s1).unwrap();
        reg.register(s2).unwrap();

        let result = resolver.resolve("data", "plugin-a", None);
        match result {
            ResolutionResult::Found { service_id, .. } => {
                assert_eq!(service_id.as_str(), "s1");
            }
            _ => panic!("Expected Found"),
        }
    }

    #[test]
    fn test_resolve_capability_filter() {
        let (mut reg, resolver) = make_registry();
        let read_only = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("data").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p1")
            .with_capabilities(vec![Capability::Read])
            .build()
            .unwrap();
        let read_write = Service::builder()
            .with_id(ServiceId::new("s2").unwrap())
            .with_name(ServiceName::new("data").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p2")
            .with_capabilities(vec![Capability::Read, Capability::Write])
            .build()
            .unwrap();
        reg.register(read_only).unwrap();
        reg.register(read_write).unwrap();

        let result = resolver.resolve("data", "plugin-a", Some(&Capability::Write));
        match result {
            ResolutionResult::Found { service_id, .. } => {
                assert_eq!(service_id.as_str(), "s2");
            }
            _ => panic!("Expected Found s2"),
        }
    }

    #[test]
    fn test_resolve_permission_denied() {
        let (mut reg, resolver) = make_registry();
        use crate::service_registry::ServicePermission;
        let perm = ServicePermission::new("admin", AccessLevel::Admin, "admin only");
        let svc = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("secret").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p1")
            .with_capabilities(vec![Capability::Read])
            .with_permissions(vec![perm])
            .with_visibility(Visibility::Private)
            .build()
            .unwrap();
        reg.register(svc).unwrap();

        let result = resolver.resolve("secret", "unauthorized", None);
        match result {
            ResolutionResult::PermissionDenied { requester, .. } => {
                assert_eq!(requester, "unauthorized");
            }
            _ => panic!("Expected PermissionDenied"),
        }
    }

    #[test]
    fn test_resolve_with_version_constraint() {
        let (mut reg, resolver) = make_registry();
        let v1 = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("data").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p1")
            .with_capabilities(vec![Capability::Read])
            .build()
            .unwrap();
        let v2 = Service::builder()
            .with_id(ServiceId::new("s2").unwrap())
            .with_name(ServiceName::new("data").unwrap())
            .with_version(ServiceVersion::new("2.0.0").unwrap())
            .with_provider("p1")
            .with_capabilities(vec![Capability::Read])
            .build()
            .unwrap();
        reg.register(v1).unwrap();
        reg.register(v2).unwrap();

        let result = resolver.resolve_with_version(
            "data",
            "plugin-a",
            &ServiceVersion::new("1.5.0").unwrap(),
            None,
            None,
        );
        match result {
            ResolutionResult::Found { service_id, .. } => {
                assert_eq!(service_id.as_str(), "s2");
            }
            _ => panic!("Expected Found s2"),
        }
    }

    #[test]
    fn test_resolve_version_conflict() {
        let (mut reg, resolver) = make_registry();
        let v1 = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("data").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p1")
            .with_capabilities(vec![Capability::Read])
            .build()
            .unwrap();
        reg.register(v1).unwrap();

        let result = resolver.resolve_with_version(
            "data",
            "plugin-a",
            &ServiceVersion::new("3.0.0").unwrap(),
            None,
            None,
        );
        match result {
            ResolutionResult::VersionConflict {
                available_versions, ..
            } => {
                assert_eq!(available_versions.len(), 1);
                assert_eq!(available_versions[0].to_string(), "1.0.0");
            }
            _ => panic!("Expected VersionConflict"),
        }
    }

    #[test]
    fn test_dependency_validation_satisfied() {
        let (mut reg, resolver) = make_registry();
        let dep = Service::builder()
            .with_id(ServiceId::new("dep1").unwrap())
            .with_name(ServiceName::new("dep-svc").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p1")
            .with_capabilities(vec![Capability::Read])
            .build()
            .unwrap();
        let main = Service::builder()
            .with_id(ServiceId::new("main").unwrap())
            .with_name(ServiceName::new("main-svc").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p2")
            .with_capabilities(vec![Capability::Read])
            .with_dependencies(vec![ServiceDependency::new(
                ServiceId::new("dep1").unwrap(),
                ServiceVersion::new("1.0.0").unwrap(),
                Capability::Read,
            )])
            .build()
            .unwrap();
        reg.register(dep).unwrap();
        reg.register(main).unwrap();

        let result = resolver.validate_dependencies(&ServiceId::new("main").unwrap());
        assert!(result.is_ok());
    }

    #[test]
    fn test_dependency_validation_missing() {
        let (mut reg, resolver) = make_registry();
        let main = Service::builder()
            .with_id(ServiceId::new("main").unwrap())
            .with_name(ServiceName::new("main-svc").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p2")
            .with_capabilities(vec![Capability::Read])
            .with_dependencies(vec![ServiceDependency::new(
                ServiceId::new("missing").unwrap(),
                ServiceVersion::new("1.0.0").unwrap(),
                Capability::Read,
            )])
            .build()
            .unwrap();
        reg.register(main).unwrap();

        let result = resolver.validate_dependencies(&ServiceId::new("main").unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_by_id() {
        let (mut reg, resolver) = make_registry();
        let svc = make_svc("s1", "data", "1.0.0", "p1", vec![Capability::Read]);
        reg.register(svc).unwrap();

        let result = resolver.resolve_by_id(&ServiceId::new("s1").unwrap(), "plugin-a");
        match result {
            ResolutionResult::Found { service_id, .. } => {
                assert_eq!(service_id.as_str(), "s1");
            }
            _ => panic!("Expected Found"),
        }
    }

    #[test]
    fn test_resolve_all_versions() {
        let (mut reg, resolver) = make_registry();
        reg.register(make_svc(
            "s1",
            "data",
            "1.0.0",
            "p1",
            vec![Capability::Read],
        ))
        .unwrap();
        reg.register(make_svc(
            "s2",
            "data",
            "2.0.0",
            "p2",
            vec![Capability::Read],
        ))
        .unwrap();
        reg.register(make_svc(
            "s3",
            "data",
            "1.5.0",
            "p3",
            vec![Capability::Read],
        ))
        .unwrap();

        let all = resolver.resolve_all("data");
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_deterministic_resolution() {
        let (mut reg, resolver) = make_registry();
        for i in 0..5 {
            let svc = Service::builder()
                .with_id(ServiceId::new(&format!("s{i}")).unwrap())
                .with_name(ServiceName::new("data").unwrap())
                .with_version(ServiceVersion::new("1.0.0").unwrap())
                .with_provider("p1")
                .with_capabilities(vec![Capability::Read])
                .build()
                .unwrap();
            reg.register(svc).unwrap();
        }

        let r1 = resolver.resolve("data", "plugin-a", None);
        let r2 = resolver.resolve("data", "plugin-a", None);
        match (&r1, &r2) {
            (
                ResolutionResult::Found {
                    service_id: id1, ..
                },
                ResolutionResult::Found {
                    service_id: id2, ..
                },
            ) => {
                assert_eq!(id1, id2);
            }
            _ => panic!("Expected deterministic resolution"),
        }
    }
}
