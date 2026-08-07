# ADR-006: Tool Lifecycle Management

**Document:** `docs/ADR/adr-006-tool-lifecycle-management.md`
**Version:** 1.0.0
**Part of:** CodeBro P3 Tool Platform
**Status:** Proposed
**Created:** 2026-08-05
**Updated:** 2026-08-05
**Related RFC:** RFC-002

---

## 1. Context

### 1.1 Background

The current tool system has no lifecycle management. Tools are either registered or not. There is no concept of:
- Temporarily disabling a tool
- Deprecating a tool with a warning
- Tracking tool state over time
- Auditing tool availability changes

### 1.2 Decision

Introduce a six-state lifecycle machine with explicit transitions.

---

## 2. Decision

### 2.1 Lifecycle States

```
Unregistered → Registered → Enabled
                         → Disabled → Enabled
                         → Deprecating → Removed
```

### 2.2 State Transition Rules

| From | To | Valid |
|------|-----|-------|
| Unregistered | Registered | Yes |
| Registered | Enabled | Yes |
| Registered | Disabled | Yes |
| Enabled | Disabled | Yes |
| Disabled | Enabled | Yes |
| Enabled | Deprecating | Yes |
| Registered | Deprecating | Yes |
| Deprecating | Removed | Yes |
| Removed | (any) | No |

### 2.3 Implementation

Each tool in the registry carries its lifecycle state. Transitions are validated before application.

---

## 3. References

- [ADR-005: Tool Capability Model](adr-005-tool-capability-model.md)

---

## 4. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-05 | Created | CodeBro Engineering |
