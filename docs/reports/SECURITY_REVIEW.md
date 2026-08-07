# Security Review

**Document:** `docs/reports/SECURITY_REVIEW.md`
**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-06
**Owner:** CodeBro Engineering
**Phase:** P6.0 Implementation Readiness

---

## 1. Executive Summary

This security review evaluates the CodeBro architecture against P6 adaptive behavior requirements. The review covers API key storage, provider credentials, permission boundaries, shell execution, filesystem access, MCP execution, plugin execution, prompt injection, tool injection, and privilege escalation.

**Overall Security Posture: READY with conditions.**

The current architecture provides a solid foundation for secure adaptive behavior. Key conditions must be met before P6 implementation begins (see Section 8).

---

## 2. Review Methodology

| Aspect | Method |
|--------|--------|
| Code analysis | Manual review of `src/` modules |
| Architecture review | Analysis of module boundaries and data flow |
| Contract review | Validation of tool/provider/permission contracts |
| Threat modeling | STRIDE analysis for each subsystem |
| Dependency review | `cargo audit` for known vulnerabilities |

---

## 3. API Key Storage

### 3.1 Current State

| Aspect | Status | Findings |
|--------|--------|----------|
| Storage location | `~/.codebro/config.toml` | ✅ File permissions 600 enforced by `SettingsManager` |
| In-memory handling | Passed by reference | ✅ Never returned from provider methods |
| Environment variable | `CODEBRO_API_KEY` | ✅ Highest priority, not persisted |
| Keychain support | `ApiKeySource::Keychain` | ⚠️ Stub only — not implemented |
| Logging exposure | Secret redaction in shell tools | ✅ `redact_secrets()` function exists |

### 3.2 Threats

| Threat | Likelihood | Impact | Mitigation |
|--------|------------|--------|------------|
| Config file read by other users | Low | High | chmod 600 enforced |
| API key in logs | Low | High | Redaction function exists |
| API key in memory dumps | Low | Medium | Keys passed by reference, never stored |
| API key in environment dump | Medium | High | Use keychain for production |

### 3.3 Recommendations

1. **P6.1**: Implement `ApiKeySource::Keychain` using `keyring` crate.
2. **P6.1**: Add `chmod 600` on config file write.
3. **P6.1**: Add log sanitization in all logging paths.

---

## 4. Provider Credentials

### 4.1 Current State

| Aspect | Status | Findings |
|--------|--------|----------|
| Provider trait | `api_key()` returns `Option<&str>` | ✅ Reference-only access |
| Provider isolation | One provider per session | ✅ Prevents credential mixing |
| Health check exposure | No credentials in health responses | ✅ |
| Provider switching | Credentials not shared across providers | ✅ |

### 4.2 Threats

| Threat | Likelihood | Impact | Mitigation |
|--------|------------|--------|------------|
| Credential leakage via health check | Low | Medium | Health responses exclude credentials |
| Cross-provider credential mixing | Low | High | One provider per session enforced |
| Provider config file exposure | Low | High | Config file permissions enforced |

### 4.3 Recommendations

1. **P6.2**: Add credential rotation support in provider trait.
2. **P6.2**: Add provider-specific secret scanning in logs.

---

## 5. Permission Boundaries

### 5.1 Current State

| Aspect | Status | Findings |
|--------|--------|----------|
| Tool permission model | `Ask`/`Allow`/`Dangerous` | ✅ Defined in tool contract |
| Permission hooks | `PermissionHook` trait | ✅ Implemented |
| Approval gate | Pending specification | ⚠️ ADR-009 + APPROVAL_GATE_SPEC created |
| Permission escalation prevention | No cross-tool permission inheritance | ✅ |

### 5.2 Threats

| Threat | Likelihood | Impact | Mitigation |
|--------|------------|--------|------------|
| Permission bypass via tool chaining | Medium | High | Approval gate must validate each step |
| Privilege escalation via env vars | Low | High | Env var whitelist in sandbox |
| Permission confusion in TUI | Medium | Medium | Clear risk-level indicators required |

### 5.3 Recommendations

1. **P6.0**: Complete approval gate implementation (APPROVAL_GATE_SPEC).
2. **P6.1**: Add permission chaining analysis.
3. **P6.1**: Add TUI risk-level color coding.

---

## 6. Shell Execution

### 6.1 Current State

| Aspect | Status | Findings |
|--------|--------|----------|
| Command execution | `RunCommand` tool | ✅ Through tool pipeline |
| Timeout | Configurable | ✅ Default 30s |
| Output redaction | `redact_secrets()` | ✅ |
| Working directory | Configurable | ✅ |
| Environment isolation | None | ⚠️ All env vars accessible |
| Child process spawn | Allowed | ⚠️ No restriction |

### 6.2 Threats

| Threat | Likelihood | Impact | Mitigation |
|--------|------------|--------|------------|
| Command injection | Medium | High | Input validation required |
| Privilege escalation via sudo | Low | Critical | Sudo not allowed in sandbox |
| Resource exhaustion via loops | Medium | Medium | Timeout + invocation limit |
| Data exfiltration via network | Medium | High | Network restrictions in MCP sandbox |

### 6.3 Recommendations

1. **P6.1**: Add command allowlist/denylist.
2. **P6.1**: Add sandbox for shell commands (no sudo, no network).
3. **P6.1**: Add invocation limits per session.

---

## 7. Filesystem Access

### 7.1 Current State

| Aspect | Status | Findings |
|--------|--------|----------|
| Read access | `ReadFile`, `ListFiles` | ✅ Approved tools |
| Write access | `CreateFile`, `EditFile` | ✅ Ask permission |
| Delete access | Not implemented | ✅ Not a threat yet |
| Path traversal | Not validated | ⚠️ Missing validation |
| Symlink following | Allowed | ⚠️ Could escape workspace |
| File permissions | Respects OS | ✅ |

### 7.2 Threats

| Threat | Likelihood | Impact | Mitigation |
|--------|------------|--------|------------|
| Path traversal attack | Medium | High | Path normalization required |
| Symlink escape | Medium | High | Symlink validation required |
| Unauthorized file overwrite | Low | Medium | Ask permission on write |
| Sensitive file access | Low | Medium | Denylist for sensitive paths |

### 7.3 Recommendations

1. **P6.1**: Add path traversal validation (canonicalize paths).
2. **P6.1**: Add symlink validation (refuse outside workspace).
3. **P6.1**: Add sensitive file denylist (`.env`, `.ssh/`, etc.).

---

## 8. MCP Execution

### 8.1 Current State

| Aspect | Status | Findings |
|--------|--------|----------|
| MCP discovery | Implemented | ✅ P5 workspace discovery |
| MCP sandbox | Specified | ⚠️ APPROVAL_GATE_SPEC + MCP_SANDBOX_SPEC created |
| MCP approval | Pending | ⚠️ Approval gate required |
| MCP monitoring | Specified | ⚠️ Monitoring stage in sandbox spec |
| MCP removal | Specified | ⚠️ Removal stage in sandbox spec |

### 8.2 Threats

| Threat | Likelihood | Impact | Mitigation |
|--------|------------|--------|------------|
| Malicious MCP server | Medium | High | Sandbox isolation required |
| MCP tool injection | Medium | High | Tool validation required |
| MCP data exfiltration | Medium | High | Network restrictions required |
| MCP privilege escalation | Low | Critical | No host filesystem access |

### 8.3 Recommendations

1. **P6.1**: Implement MCP sandbox per MCP_SANDBOX_SPEC.
2. **P6.1**: Implement approval gate per APPROVAL_GATE_SPEC.
3. **P6.2**: Add MCP-specific security monitoring.

---

## 9. Plugin Execution

### 9.1 Current State

| Aspect | Status | Findings |
|--------|--------|----------|
| Plugin system | RFC-002 defined | ✅ Architecture defined |
| Plugin sandbox | Not specified | ⚠️ Must be defined before P6 |
| Plugin approval | Not specified | ⚠️ Must be defined before P6 |
| Plugin permissions | Not defined | ⚠️ Must be defined before P6 |

### 9.2 Threats

| Threat | Likelihood | Impact | Mitigation |
|--------|------------|--------|------------|
| Malicious plugin | Medium | High | Plugin sandbox required |
| Plugin code injection | Medium | High | Plugin validation required |
| Plugin privilege escalation | Low | Critical | Plugin sandbox isolation required |
| Plugin data exfiltration | Medium | High | Network restrictions required |

### 9.3 Recommendations

1. **P6.0**: Create PLUGIN_SANDBOX_SPEC (similar to MCP_SANDBOX_SPEC).
2. **P6.1**: Implement plugin sandbox.
3. **P6.1**: Implement plugin approval gate.

---

## 10. Prompt Injection

### 10.1 Current State

| Aspect | Status | Findings |
|--------|--------|----------|
| Input sanitization | None | ⚠️ No sanitization |
| System prompt protection | None | ⚠️ No protection |
| Tool output sanitization | Partial | ⚠️ Secrets redacted, but no injection prevention |
| User input validation | None | ⚠️ No validation |

### 10.2 Threats

| Threat | Likelihood | Impact | Mitigation |
|--------|------------|--------|------------|
| Prompt injection via tool output | High | High | Output sanitization required |
| Prompt injection via user input | Medium | Medium | Input validation required |
| Prompt injection via file content | Medium | High | File content sanitization required |
| Jailbreak via adaptive behavior | Low | Critical | Preference Engine must not be manipolvable |

### 10.3 Recommendations

1. **P6.1**: Add input sanitization layer.
2. **P6.1**: Add output sanitization for tool results.
3. **P6.2**: Add prompt injection detection in Preference Engine.
4. **P6.2**: Add system prompt protection.

---

## 11. Tool Injection

### 11.1 Current State

| Aspect | Status | Findings |
|--------|--------|----------|
| Tool registration | Registry-based | ✅ `ToolRegistry` |
| Tool validation | None | ⚠️ No validation of tool source |
| Tool signature check | None | ⚠️ No signature verification |
| Tool allowlist | None | ⚠️ No allowlist |

### 11.2 Threats

| Threat | Likelihood | Impact | Mitigation |
|--------|------------|--------|------------|
| Malicious tool registration | Low | High | Tool validation required |
| Tool signature forgery | Low | High | Signature verification required |
| Tool name collision | Medium | Medium | Name uniqueness enforced |
| Tool behavior manipulation | Low | High | Tool behavior audit required |

### 11.3 Recommendations

1. **P6.1**: Add tool registration validation.
2. **P6.1**: Add tool signature verification.
3. **P6.2**: Add tool behavior auditing.

---

## 12. Privilege Escalation

### 12.1 Current State

| Aspect | Status | Findings |
|--------|--------|----------|
| User privilege model | None | ⚠️ All operations run as user |
| Sudo prevention | Not explicit | ⚠️ No explicit prevention |
| Container isolation | Not implemented | ⚠️ No container isolation |
| Capability dropping | Not implemented | ⚠️ No capability dropping |

### 12.2 Threats

| Threat | Likelihood | Impact | Mitigation |
|--------|------------|--------|------------|
| Privilege escalation via shell | Low | Critical | Shell sandbox required |
| Privilege escalation via MCP | Low | Critical | MCP sandbox required |
| Privilege escalation via plugin | Low | Critical | Plugin sandbox required |
| Privilege escalation via config | Medium | High | Config validation required |

### 12.3 Recommendations

1. **P6.1**: Add explicit sudo prevention in shell sandbox.
2. **P6.1**: Add config validation to prevent privilege changes.
3. **P6.2**: Evaluate container isolation for P7.

---

## 13. Adaptive Behavior Security

### 13.1 Preference Engine

| Threat | Likelihood | Impact | Mitigation |
|--------|------------|--------|------------|
| Preference manipulation | Medium | High | Deterministic engine only |
| Preference injection via history | Medium | High | History validation required |
| Preference corruption | Low | Medium | Backup + validation |

### 13.2 Intent Engine

| Threat | Likelihood | Impact | Mitigation |
|--------|------------|--------|------------|
| Intent hijacking | Low | High | Intent validation required |
| Intent injection via context | Medium | Medium | Context sanitization |
| Intent confusion | Low | Medium | Clear intent boundaries |

### 13.3 Workflow Engine

| Threat | Likelihood | Impact | Mitigation |
|--------|------------|--------|------------|
| Workflow injection | Low | High | Workflow validation |
| Workflow loops | Medium | Medium | Loop detection |
| Workflow privilege escalation | Low | Critical | Each step through approval gate |

---

## 14. Summary of Findings

| Category | Severity | Status | P6 Phase |
|----------|----------|--------|----------|
| API key storage | High | ⚠️ Keychain not implemented | P6.1 |
| Provider credentials | Medium | ✅ Mostly secure | P6.2 |
| Permission boundaries | High | ⚠️ Approval gate needed | P6.0 |
| Shell execution | High | ⚠️ Sandbox needed | P6.1 |
| Filesystem access | Medium | ⚠️ Path validation needed | P6.1 |
| MCP execution | High | ⚠️ Sandbox specified, not built | P6.1 |
| Plugin execution | High | ⚠️ Sandbox not yet specified | P6.0 |
| Prompt injection | High | ⚠️ No sanitization | P6.1 |
| Tool injection | Medium | ⚠️ No validation | P6.1 |
| Privilege escalation | High | ⚠️ No sandboxing | P6.1 |
| Adaptive behavior | Medium | ⚠️ Needs validation | P6.2 |

---

## 15. Conditions for P6 Implementation

All conditions below must be met before P6.1 implementation begins:

1. ✅ ADR-009 Configuration Versioning created
2. ✅ APPROVAL_GATE_SPEC.md created
3. ✅ MCP_SANDBOX_SPEC.md created
4. ✅ SECURITY_RISK_MATRIX.md created (this document)
5. ⚠️ API key keychain implementation planned (P6.1)
6. ⚠️ Shell sandbox design planned (P6.1)
7. ⚠️ File path traversal validation planned (P6.1)
8. ⚠️ Prompt injection mitigation planned (P6.1)
9. ⚠️ Plugin sandbox spec to be created (P6.0)
10. ✅ No implementation code added in P6.0

---

## 16. References

- [APPROVAL_GATE_SPEC.md](../specs/APPROVAL_GATE_SPEC.md)
- [MCP_SANDBOX_SPEC.md](../specs/MCP_SANDBOX_SPEC.md)
- [ADR-009: Configuration Versioning](../ADR/adr-009-configuration-versioning.md)
- [Tool Contract](../contracts/tool_contract.md)
- [DX Principles](../vision/DX_PRINCIPLES.md)

---

## 17. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
