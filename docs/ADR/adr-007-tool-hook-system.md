# ADR-007: Tool Hook System

**Document:** `docs/ADR/adr-007-tool-hook-system.md`
**Version:** 1.0.0
**Part of:** CodeBro P3 Tool Platform
**Status:** Proposed
**Created:** 2026-08-05
**Updated:** 2026-08-05
**Related RFC:** RFC-002

---

## 1. Context

### 1.1 Background

Tools need extensibility points for:
- Pre-execution permission checks
- Post-execution rollback/audit
- Streaming output interception
- Diagnostic data collection

### 1.2 Decision

Use trait-based hooks that can be attached per-tool or globally.

---

## 2. Decision

### 2.1 Hook Types

| Hook | Timing | Purpose |
|------|--------|---------|
| `PermissionHook` | Pre-execution | Validate tool usage against policy |
| `RollbackHook` | Pre/Post | Capture state for potential reversal |
| `StreamHook` | During execution | Intercept streaming output |
| `DiagnosticHook` | Post-execution | Record metrics and health data |

### 2.2 Hook Attachment

Hooks can be attached:
1. Per-tool (stored in registry per tool name)
2. Globally (applied to all tools)

---

## 3. References

- [ADR-005: Tool Capability Model](adr-005-tool-capability-model.md)

---

## 4. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-05 | Created | CodeBro Engineering |
