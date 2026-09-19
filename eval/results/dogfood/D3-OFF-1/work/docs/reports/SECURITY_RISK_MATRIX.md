# Security Risk Matrix

**Document:** `docs/reports/SECURITY_RISK_MATRIX.md`
**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-06
**Owner:** CodeBro Engineering
**Phase:** P6.0 Implementation Readiness

---

## 1. Risk Assessment Methodology

| Rating | Likelihood | Impact | Risk Score |
|--------|------------|--------|------------|
| **Critical** | Likely | Critical | 12–16 |
| **High** | Possible | High | 8–11 |
| **Medium** | Unlikely | Medium | 4–7 |
| **Low** | Rare | Low | 1–3 |

Risk Score = Likelihood (1–4) × Impact (1–4)

---

## 2. Risk Register

### 2.1 API Key & Credential Risks

| ID | Risk | Likelihood | Impact | Score | Category | Mitigation | P6 Phase |
|----|------|------------|--------|-------|----------|------------|----------|
| R-001 | API key stored in plaintext config file | Possible | High | 8 | Credential | chmod 600; keychain support | P6.1 |
| R-002 | API key leaked in logs | Unlikely | High | 6 | Credential | Secret redaction in all log paths | P6.1 |
| R-003 | API key leaked in memory dump | Unlikely | Medium | 4 | Credential | Keys passed by reference | P6.2 |
| R-004 | Environment variable exposure via /proc | Unlikely | Medium | 4 | Credential | Use keychain, not env vars | P6.1 |
| R-005 | Provider credential mixing | Unlikely | High | 4 | Credential | One provider per session | P6.2 |

### 2.2 Permission & Access Risks

| ID | Risk | Likelihood | Impact | Score | Category | Mitigation | P6 Phase |
|----|------|------------|--------|-------|----------|------------|----------|
| R-006 | Approval gate bypass | Possible | Critical | 12 | Permission | Mandatory approval for all Ask/Dangerous actions | P6.0 |
| R-007 | Permission confusion in TUI | Possible | Medium | 6 | Permission | Clear risk-level indicators | P6.1 |
| R-008 | Permission chaining attack | Possible | High | 8 | Permission | Each step validated independently | P6.1 |
| R-009 | Duplicate approval exploited | Unlikely | Medium | 4 | Permission | Duplicate detection with hash | P6.0 |
| R-010 | Concurrent approval overflow | Unlikely | Medium | 4 | Permission | Concurrency limits enforced | P6.0 |

### 2.3 Shell Execution Risks

| ID | Risk | Likelihood | Impact | Score | Category | Mitigation | P6 Phase |
|----|------|------------|--------|-------|----------|------------|----------|
| R-011 | Command injection | Possible | Critical | 12 | Shell | Input validation + allowlist | P6.1 |
| R-012 | Privilege escalation via sudo | Unlikely | Critical | 8 | Shell | Explicit sudo prevention in sandbox | P6.1 |
| R-013 | Resource exhaustion via loops | Possible | Medium | 6 | Shell | Timeout + invocation limits | P6.1 |
| R-014 | Data exfiltration via shell | Possible | High | 8 | Shell | Network restrictions in sandbox | P6.1 |
| R-015 | Environment variable leakage | Possible | Medium | 6 | Shell | Env var whitelist | P6.1 |

### 2.4 Filesystem Risks

| ID | Risk | Likelihood | Impact | Score | Category | Mitigation | P6 Phase |
|----|------|------------|--------|-------|----------|------------|----------|
| R-016 | Path traversal attack | Possible | High | 8 | Filesystem | Path canonicalization | P6.1 |
| R-017 | Symlink escape | Possible | High | 8 | Filesystem | Symlink validation | P6.1 |
| R-018 | Unauthorized file overwrite | Unlikely | Medium | 4 | Filesystem | Ask permission on write | P6.1 |
| R-019 | Sensitive file access | Possible | Medium | 6 | Filesystem | Denylist for sensitive paths | P6.1 |
| R-020 | File permission bypass | Unlikely | High | 4 | Filesystem | Respect OS permissions | P6.2 |

### 2.5 MCP Execution Risks

| ID | Risk | Likelihood | Impact | Score | Category | Mitigation | P6 Phase |
|----|------|------------|--------|-------|----------|------------|----------|
| R-021 | Malicious MCP server | Possible | Critical | 12 | MCP | Sandbox isolation + approval | P6.1 |
| R-022 | MCP tool injection | Possible | High | 8 | MCP | Tool validation + signature check | P6.1 |
| R-023 | MCP data exfiltration | Possible | High | 8 | MCP | Network restrictions in sandbox | P6.1 |
| R-024 | MCP privilege escalation | Unlikely | Critical | 8 | MCP | No host filesystem access | P6.1 |
| R-025 | MCP server crash DoS | Possible | Medium | 6 | MCP | Crash detection + auto-termination | P6.1 |
| R-026 | MCP connection hijacking | Unlikely | High | 4 | MCP | TLS for SSE; stdio isolation | P6.1 |

### 2.6 Plugin Execution Risks

| ID | Risk | Likelihood | Impact | Score | Category | Mitigation | P6 Phase |
|----|------|------------|--------|-------|----------|------------|----------|
| R-027 | Malicious plugin | Possible | Critical | 12 | Plugin | Plugin sandbox + approval | P6.1 |
| R-028 | Plugin code injection | Possible | High | 8 | Plugin | Plugin validation + sandbox | P6.1 |
| R-029 | Plugin privilege escalation | Unlikely | Critical | 8 | Plugin | Plugin sandbox isolation | P6.1 |
| R-030 | Plugin data exfiltration | Possible | High | 8 | Plugin | Network restrictions in sandbox | P6.1 |
| R-031 | Plugin supply chain attack | Unlikely | Critical | 8 | Plugin | Plugin signature verification | P6.2 |

### 2.7 Prompt Injection Risks

| ID | Risk | Likelihood | Impact | Score | Category | Mitigation | P6 Phase |
|----|------|------------|--------|-------|----------|------------|----------|
| R-032 | Prompt injection via tool output | Likely | High | 12 | Injection | Output sanitization layer | P6.1 |
| R-033 | Prompt injection via user input | Possible | Medium | 6 | Injection | Input validation | P6.1 |
| R-034 | Prompt injection via file content | Possible | High | 8 | Injection | File content sanitization | P6.1 |
| R-035 | Jailbreak via adaptive behavior | Unlikely | Critical | 8 | Injection | Deterministic preference engine | P6.2 |
| R-036 | System prompt manipulation | Possible | High | 8 | Injection | System prompt protection | P6.2 |

### 2.8 Tool Injection Risks

| ID | Risk | Likelihood | Impact | Score | Category | Mitigation | P6 Phase |
|----|------|------------|--------|-------|----------|------------|----------|
| R-037 | Malicious tool registration | Unlikely | High | 4 | Injection | Tool validation on registration | P6.1 |
| R-038 | Tool signature forgery | Unlikely | High | 4 | Injection | Signature verification | P6.1 |
| R-039 | Tool name collision | Possible | Medium | 6 | Injection | Name uniqueness enforcement | P6.1 |
| R-040 | Tool behavior manipulation | Unlikely | High | 4 | Injection | Tool behavior auditing | P6.2 |

### 2.9 Adaptive Behavior Risks

| ID | Risk | Likelihood | Impact | Score | Category | Mitigation | P6 Phase |
|----|------|------------|--------|-------|----------|------------|----------|
| R-041 | Preference manipulation | Possible | High | 8 | Adaptive | Deterministic engine; no external input | P6.2 |
| R-042 | Preference injection via history | Possible | High | 8 | Adaptive | History validation | P6.2 |
| R-043 | Preference corruption | Unlikely | Medium | 4 | Adaptive | Backup + validation | P6.2 |
| R-044 | Intent hijacking | Unlikely | High | 4 | Adaptive | Intent validation | P6.2 |
| R-045 | Intent confusion | Unlikely | Medium | 4 | Adaptive | Clear intent boundaries | P6.2 |
| R-046 | Workflow injection | Unlikely | High | 4 | Adaptive | Workflow validation | P6.2 |
| R-047 | Workflow loops | Possible | Medium | 6 | Adaptive | Loop detection | P6.2 |
| R-048 | Workflow privilege escalation | Unlikely | Critical | 8 | Adaptive | Each step through approval gate | P6.2 |

### 2.10 Configuration Risks

| ID | Risk | Likelihood | Impact | Score | Category | Mitigation | P6 Phase |
|----|------|------------|--------|-------|----------|------------|----------|
| R-049 | Corrupted config file | Possible | Medium | 6 | Config | Corruption recovery (ADR-009) | P6.1 |
| R-050 | Config injection via migration | Unlikely | High | 4 | Config | Migration validation | P6.1 |
| R-051 | Config format bypass | Unlikely | Medium | 4 | Config | Schema validation | P6.1 |
| R-052 | Privilege change via config | Unlikely | Critical | 4 | Config | Config validation | P6.1 |

---

## 3. Risk Heat Map

```
Critical ┤  R-006 R-011 R-021 R-027 R-032 R-048
         │
High     ┤  R-001 R-006 R-012 R-016 R-017 R-022 R-023 R-028 R-029 R-030 R-034 R-036 R-041 R-042 R-047
         │
Medium   ┤  R-003 R-004 R-007 R-009 R-010 R-013 R-015 R-018 R-019 R-025 R-033 R-039 R-043 R-045 R-046 R-049 R-051 R-052
         │
Low      ┤  R-002 R-005 R-008 R-020 R-024 R-026 R-031 R-035 R-037 R-038 R-040 R-044
         └──────────────────────────────────────────────────────
           Rare    Unlikely   Possible    Likely
                      Likelihood
```

---

## 4. Critical Risks Requiring Immediate Mitigation

| Rank | Risk ID | Risk | P6 Phase | Mitigation |
|------|---------|------|----------|------------|
| 1 | R-006 | Approval gate bypass | P6.0 | Implement approval gate per APPROVAL_GATE_SPEC |
| 2 | R-011 | Command injection | P6.1 | Input validation + command allowlist |
| 3 | R-021 | Malicious MCP server | P6.1 | Sandbox isolation per MCP_SANDBOX_SPEC |
| 4 | R-027 | Malicious plugin | P6.1 | Plugin sandbox + approval |
| 5 | R-032 | Prompt injection via tool output | P6.1 | Output sanitization layer |
| 6 | R-048 | Workflow privilege escalation | P6.2 | Each step through approval gate |

---

## 5. Risk Acceptance

| Risk ID | Accepted? | Rationale |
|---------|-----------|-----------|
| R-003 | Yes | Keys passed by reference; memory dumps are developer-side |
| R-004 | Yes | Keychain will be implemented in P6.1 |
| R-024 | Yes | MCP sandbox prevents host filesystem access |
| R-031 | Yes | Plugin supply chain is a lower priority than direct injection |
| R-035 | Yes | Deterministic engine prevents external manipulation |
| R-044 | Yes | Intent validation catches hijacking attempts |

**Unacceptable risks (must mitigate before P6.1):**
- R-006, R-011, R-021, R-027, R-032, R-048

---

## 6. Risk Monitoring

### 6.1 Ongoing Monitoring

| Metric | Threshold | Action |
|--------|-----------|--------|
| Failed approval attempts | > 10/hour | Alert security team |
| Sandbox violations | > 0 | Terminate server immediately |
| Prompt injection detections | > 5/hour | Review sanitization layer |
| Configuration corruption events | > 0 | Review migration code |
| Tool registration failures | > 0 | Review tool validation |

### 6.2 Periodic Review

| Review | Frequency | Owner |
|--------|-----------|-------|
| Security risk register update | Quarterly | Security lead |
| Dependency audit (`cargo audit`) | Before each release | CI/CD |
| Architecture security review | Before each major phase | Architecture review board |

---

## 7. References

- [SECURITY_REVIEW.md](./SECURITY_REVIEW.md)
- [APPROVAL_GATE_SPEC.md](../specs/APPROVAL_GATE_SPEC.md)
- [MCP_SANDBOX_SPEC.md](../specs/MCP_SANDBOX_SPEC.md)
- [ADR-009: Configuration Versioning](../ADR/adr-009-configuration-versioning.md)

---

## 8. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
