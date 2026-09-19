# Security Model Report — P9.3

**Date:** 2026-08-06

## Overview

The Plugin SDK enforces strict security boundaries. Plugins are isolated from core state and can only interact through approved interfaces.

## Security Guarantees

### 1. No Direct Memory Access
Plugins cannot access or modify core memory directly. All interactions go through:
- `HookDispatcher` (read-only event observation)
- `CapabilityModel` (capability queries)
- `PluginSandbox` (permission checks)

### 2. Approval Gate Enforcement
The sandbox explicitly blocks:
- `approval_bypass` → `SandboxViolation::ApprovalGateBypass`
- `validation_bypass` → `SandboxViolation::ValidationBypass`
- `deterministic_change` → `SandboxViolation::DeterministicBehaviorChange`

### 3. Permission Domains
Each plugin declares required permissions:

| Domain | Level | Description |
|--------|-------|-------------|
| `Observability` | Read | Read observability events |
| `Preferences` | Read | Read preference data |
| `PreferencesWrite` | Write | Modify preferences (requires approval) |
| `Pipeline` | Read | Read pipeline state |
| `Tools` | Read | Read tool registry |
| `Providers` | Read | Read provider registry |
| `Agent` | Read | Read agent state |

### 4. Sandbox Policy

Default sandbox policy:
- **Memory limit**: 64 MB per plugin
- **Execution timeout**: 5 seconds per hook
- **File I/O**: Disabled by default
- **Network**: Disabled by default
- **Environment access**: Disabled by default

### 5. Violation Tracking

All sandbox violations are recorded and queryable:
```rust
let violations = sandbox.violations();
sandbox.record_violation(SandboxViolation::DomainNotAuthorized(...));
```

## Security Test Coverage

6 tests:
- Default sandbox is restrictive (no domains allowed)
- Allowed domain check passes
- Denied domain check fails
- Approval bypass is blocked
- Validation bypass is blocked
- Violation recording and clearing
