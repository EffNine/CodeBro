# Permission Model Report

**P9.4 — Service Registry Foundation**
**Module:** `src/service_registry/permissions.rs`

---

## 1. Overview

The permission model enforces strict access control on service resolution. It prevents plugins from accessing services they do not have explicit or implicit permission to use.

---

## 2. Access Levels

| Level | Description |
|-------|-------------|
| `None` | No access |
| `Read` | Can discover and resolve the service |
| `Write` | Can resolve and request state changes |
| `Admin` | Full access including permission management |

---

## 3. Visibility Modes

| Mode | Description | Default Access |
|------|-------------|----------------|
| `Public` | Visible to all plugins | Read for everyone |
| `Private` | Visible only to owner and explicit grantees | None (except owner) |
| `Namespace(String)` | Visible within a namespace | Read for namespace members |

---

## 4. Permission Rules

### Rule 1: Owner Always Has Admin
The provider plugin (service owner) automatically has `Admin` access regardless of visibility.

### Rule 2: Explicit Grants Override Visibility
Services can grant specific access to specific plugins via `ServicePermission` entries.

### Rule 3: Public Services Allow Read by Default
Public services allow `Read` access to any requester without explicit grants.

### Rule 4: Private Services Require Explicit Grant
Private services deny all access except to the owner and explicitly granted plugins.

### Rule 5: Namespace Services Allow Read Within Namespace
Services with `Namespace("team")` allow read access to any plugin whose ID starts with `"team"`.

### Rule 6: Wildcard Grants Forbidden on Private Services
`*` as a grantee on a `Private` service is a validation error.

---

## 5. Public API

```rust
// Access checks
fn check_access(&self, service_id: &ServiceId, requester: &str, required: AccessLevel) -> AccessResult
fn can_write(&self, service_id: &ServiceId, requester: &str) -> bool
fn can_read(&self, service_id: &ServiceId, requester: &str) -> bool
fn is_owner(&self, service_id: &ServiceId, plugin_id: &str) -> bool
fn effective_access(&self, service_id: &ServiceId, requester: &str) -> AccessLevel

// Permission management
fn list_permissions(&self, service_id: &ServiceId) -> Vec<ServicePermission>
fn validate_permissions(&self, service_id: &ServiceId) -> Result<(), PermissionValidationError>
fn grant_access(&self, service_id: &ServiceId, grantee: &str, access: AccessLevel, description: &str) -> Result<(), PermissionError>
fn revoke_access(&self, service_id: &ServiceId, grantee: &str) -> Result<(), PermissionError>
```

---

## 6. AccessResult

```rust
pub enum AccessResult {
    Granted(AccessLevel),
    Denied { service_id, requester, required, available },
    NotFound(ServiceId),
}
```

---

## 7. Validation Rules

The `validate_permissions` method checks:
1. **No conflicting grants**: Same grantee cannot have different access levels
2. **No wildcard on private**: `*` grantee is invalid on `Private` visibility services

---

## 8. ServicePermission Model

```rust
pub struct ServicePermission {
    pub grantee: String,      // Plugin ID or "*" for all
    pub access_level: AccessLevel,
    pub description: String,
}
```

---

## 9. Test Coverage

**13 tests** covering:
- Public read allowed
- Public write denied (no explicit grant)
- Owner has admin access
- Explicit grant grants write
- Private service blocks unauthorized access
- Namespace visibility allows same-namespace access
- Clean permission validation passes
- Conflicting permissions detected
- Wildcard on private service rejected
- Effective access computation
- Permission listing
- Ownership check
- AccessResult semantics

---

## 10. Security Guarantees

1. **No direct communication**: Plugins cannot bypass the registry to communicate
2. **No shared mutable state**: Services are immutable; permissions are checked at resolution time
3. **Owner isolation**: Providers retain full control over their services
4. **Minimal privilege**: Default is deny; access must be explicitly granted (except for public read)
5. **Audit trail**: Permission violations are logged via diagnostics
