# Service Registry Architecture Report

**P9.4 — Service Registry Foundation**
**Date:** 2026-08-07
**Status:** Complete

---

## 1. Executive Summary

The Service Registry is the official communication layer between plugins in CodeBro. It enforces the architectural rule that plugins MUST NOT keep direct references to each other, call each other directly, or share mutable state. All inter-plugin communication flows exclusively through the registry.

---

## 2. Module Tree

```
src/service_registry/
├── mod.rs           (51 lines)   — Module declaration and public re-exports
├── types.rs         (730 lines)  — Core type definitions
├── service.rs       (446 lines)  — Service struct and builder pattern
├── registry.rs      (443 lines)  — Core register/unregister/activate/deactivate/enumerate
├── resolver.rs      (652 lines)  — Deterministic lookup and version negotiation
├── discovery.rs     (374 lines)  — Metadata queries, filtering, search
├── permissions.rs   (488 lines)  — Ownership, visibility, access validation
├── lifecycle.rs     (507 lines)  — State machine: Registered <-> Activated <-> Deactivated <-> Error
└── diagnostics.rs   (370 lines)  — Statistics, failed lookups, violations, events
```

**Total:** 4,061 lines of Rust code across 9 modules.

---

## 3. Public APIs

### ServiceRegistry (registry.rs)
```rust
pub struct ServiceRegistry { /* Clone via Arc<Mutex<>> */ }

impl ServiceRegistry {
    pub fn new() -> Self;
    pub fn with_event_bus(event_bus: EventBus) -> Self;
    pub fn register(&mut self, service: Service) -> Result<u64, RegistryError>;
    pub fn unregister(&mut self, service_id: &ServiceId) -> Result<Service, RegistryError>;
    pub fn activate(&mut self, service_id: &ServiceId) -> Result<(), RegistryError>;
    pub fn deactivate(&mut self, service_id: &ServiceId) -> Result<(), RegistryError>;
    pub fn get(&self, service_id: &ServiceId) -> Option<Service>;
    pub fn enumerate(&self, status: Option<&ServiceStatus>) -> Vec<Service>;
    pub fn enumerate_by_name(&self, name: &str) -> Vec<Service>;
    pub fn count(&self) -> usize;
    pub fn contains(&self, service_id: &ServiceId) -> bool;
}
```

### ServiceResolver (resolver.rs)
```rust
pub struct ServiceResolver { /* Clone via Arc */ }

impl ServiceResolver {
    pub fn new(registry: ServiceRegistry) -> Self;
    pub fn with_event_bus(self, event_bus: EventBus) -> Self;
    pub fn resolve(&self, name: &str, requester: &str, required_capability: Option<&Capability>) -> ResolutionResult;
    pub fn resolve_by_id(&self, service_id: &ServiceId, requester: &str) -> ResolutionResult;
    pub fn resolve_with_version(&self, name: &str, requester: &str, min_version: &ServiceVersion, max_version: Option<&ServiceVersion>, required_capability: Option<&Capability>) -> ResolutionResult;
    pub fn validate_dependencies(&self, service_id: &ServiceId) -> Result<(), DependencyCheckError>;
    pub fn resolve_all(&self, name: &str) -> Vec<AmbiguousCandidate>;
}
```

### ServiceDiscovery (discovery.rs)
```rust
pub struct ServiceDiscovery { /* Clone via Arc */ }

impl ServiceDiscovery {
    pub fn new(registry: ServiceRegistry) -> Self;
    pub fn search_by_name(&self, prefix: &str) -> DiscoveryResult;
    pub fn search_by_provider(&self, provider: &str) -> DiscoveryResult;
    pub fn search_by_capability(&self, capability: &Capability) -> DiscoveryResult;
    pub fn search(&self, filter: &DiscoveryFilter) -> DiscoveryResult;
    pub fn get_metadata(&self, service_id: &ServiceId) -> Option<ServiceMetadata>;
    pub fn get_manifest(&self, service_id: &ServiceId) -> Option<Service>;
    pub fn services_by_provider(&self, provider: &str) -> Vec<Service>;
    pub fn activated_services(&self) -> Vec<Service>;
    pub fn count_by_name(&self, name: &str) -> usize;
    pub fn list_names(&self) -> Vec<String>;
    pub fn find_by_metadata(&self, key: &str, value: &str) -> DiscoveryResult;
}
```

### ServicePermissions (permissions.rs)
```rust
pub struct ServicePermissions { /* Clone via Arc */ }

impl ServicePermissions {
    pub fn new(registry: ServiceRegistry) -> Self;
    pub fn check_access(&self, service_id: &ServiceId, requester: &str, required: AccessLevel) -> AccessResult;
    pub fn can_write(&self, service_id: &ServiceId, requester: &str) -> bool;
    pub fn can_read(&self, service_id: &ServiceId, requester: &str) -> bool;
    pub fn is_owner(&self, service_id: &ServiceId, plugin_id: &str) -> bool;
    pub fn effective_access(&self, service_id: &ServiceId, requester: &str) -> AccessLevel;
    pub fn list_permissions(&self, service_id: &ServiceId) -> Vec<ServicePermission>;
    pub fn validate_permissions(&self, service_id: &ServiceId) -> Result<(), PermissionValidationError>;
    pub fn grant_access(&self, service_id: &ServiceId, grantee: &str, access: AccessLevel, description: &str) -> Result<(), PermissionError>;
    pub fn revoke_access(&self, service_id: &ServiceId, grantee: &str) -> Result<(), PermissionError>;
}
```

### ServiceLifecycle (lifecycle.rs)
```rust
pub struct ServiceLifecycle { /* Clone via Arc */ }

impl ServiceLifecycle {
    pub fn new(registry: ServiceRegistry) -> Self;
    pub fn with_event_bus(self, event_bus: EventBus) -> Self;
    pub fn activate(&mut self, service_id: &ServiceId, reason: &str) -> Result<LifecycleTransition, LifecycleError>;
    pub fn deactivate(&mut self, service_id: &ServiceId, reason: &str) -> Result<LifecycleTransition, LifecycleError>;
    pub fn error(&self, service_id: &ServiceId, error_msg: &str) -> Result<LifecycleTransition, LifecycleError>;
    pub fn recover(&self, service_id: &ServiceId, reason: &str) -> Result<LifecycleTransition, LifecycleError>;
    pub fn current_state(&self, service_id: &ServiceId) -> Result<LifecycleState, LifecycleError>;
    pub fn transition_log(&self) -> Vec<LifecycleTransition>;
    pub fn recent_transitions(&self, service_id: &ServiceId, limit: usize) -> Vec<LifecycleTransition>;
    pub fn clear_log(&self);
}
```

### ServiceDiagnostics (diagnostics.rs)
```rust
pub struct ServiceDiagnostics { /* Clone via Arc */ }

impl ServiceDiagnostics {
    pub fn new() -> Self;
    pub fn with_capacity(max_history: usize) -> Self;
    pub fn record_registration(&self, event: &RegistryDiagnosticEvent);
    pub fn record_resolution(&self, event: &RegistryDiagnosticEvent);
    pub fn record_failure(&self, query_name: &str, reason: &str);
    pub fn record_permission_violation(&self, event: &RegistryDiagnosticEvent);
    pub fn record_lifecycle(&self, event: &RegistryDiagnosticEvent);
    pub fn statistics(&self) -> RegistryStatistics;
    pub fn recent_failures(&self) -> Vec<ResolutionFailureRecord>;
    pub fn recent_violations(&self) -> Vec<RegistryDiagnosticEvent>;
    pub fn recent_events(&self, limit: usize) -> Vec<RegistryDiagnosticEvent>;
    pub fn snapshot(&self) -> DiagnosticSnapshot;
    pub fn clear(&self);
}
```

---

## 4. Core Types

| Type | Description |
|------|-------------|
| `ServiceId` | Unique identifier (`String` wrapper with validation) |
| `ServiceName` | Human-readable name |
| `ServiceVersion` | Semantic version (`MAJOR.MINOR.PATCH`) |
| `ServicePriority` | Critical, High, Medium, Low, Custom(u32) |
| `ServiceStatus` | Registered, Activated, Deactivated, Error(String) |
| `Capability` | Read, Write, Execute, Stream, Hook, Tool, Provider, Agent, FileSystem, Network, Custom |
| `Visibility` | Public, Private, Namespace(String) |
| `AccessLevel` | None, Read, Write, Admin |
| `ServiceDependency` | service_id, min_version, capability_required |
| `ServiceMetadata` | HashMap<String, String> |
| `DiscoveryFilter` | name_prefix, provider, capabilities, version range, visibility, status, metadata |
| `ResolutionResult` | Found, Ambiguous, VersionConflict, CapabilityMismatch, PermissionDenied, NotFound |
| `LifecycleState` | Registered, Activated, Deactivated, Error(String), ShuttingDown |
| `LifecycleTransition` | service_id, from, to, timestamp, reason |

---

## 5. Design Principles

1. **No Direct Plugin Communication** — All inter-plugin calls go through the registry
2. **Deterministic Resolution** — Priority → Version → Registration Order (never random)
3. **Thread-Safe** — All public types are `Send + Sync + Clone` via `Arc<Mutex<>>`
4. **Observable** — Emits events via P9.2 observability platform
5. **Permission Enforced** — Access checks on every resolution
6. **Immutable Services** — Services are immutable after construction
7. **Builder Pattern** — All services constructed via fluent builder API
8. **Future Compatible** — Supports AI Runtime, LLM Providers, Enterprise, Marketplace, Cloud, Remote services without redesign

---

## 6. Observability Integration

Events emitted (P9.2 compliant):
- `ServiceRegistered`
- `ServiceUnregistered`
- `ServiceActivated`
- `ServiceDeactivated`
- `ServiceResolved`
- `ResolutionFailed`
- `PermissionDenied`
- `DependencyViolation`

All events flow through the shared `EventBus` when provided at construction time.

---

## 7. Security Model

- **Ownership**: The provider plugin is the owner and has admin access
- **Visibility**: Public (all), Private (owner only), Namespace (same namespace)
- **Explicit Grants**: Owner can grant specific access levels to specific plugins
- **Wildcard**: `*` grantee matches all plugins (not allowed on private services)
- **No Shared Mutable State**: Services are immutable; registry state is behind `Arc<Mutex<>>`

---

## 8. Acceptance Criteria Status

| Criterion | Status |
|-----------|--------|
| Public API unchanged | ✅ No changes to existing modules |
| Existing engines unchanged | ✅ No changes to existing modules |
| Plugin SDK unchanged | ✅ No changes to plugin_sdk/ |
| Thread-safe | ✅ All types Send + Sync + Clone, tested with 10-thread spawns |
| Deterministic | ✅ Priority → Version → Registration order |
| Observable | ✅ Events emitted via EventBus integration |
| Permission enforced | ✅ Access checks on every resolution |
| Zero regressions | ✅ 1642 tests pass, 0 failures |
