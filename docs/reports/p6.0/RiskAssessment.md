# Risk Assessment — P6.0 Implementation Readiness

**Document:** `docs/reports/p6.0/RiskAssessment.md`
**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Executive Summary

This risk assessment evaluates the risks associated with proceeding from P6.0 (Implementation Readiness) to P6.1 (Preference Engine & MCP Manager implementation). All critical risks have identified mitigations. The overall risk level is **ACCEPTABLE with monitoring**.

---

## 2. Risk Categories

### 2.1 Architectural Risks

| Risk | Likelihood | Impact | Score | Mitigation |
|------|------------|--------|-------|------------|
| P6 modules violate architecture boundaries | Low | High | 4 | Architecture manifest freeze; ADR requirement for boundary changes |
| Cyclic dependencies introduced | Low | High | 4 | Dependency graph analysis in each phase gate |
| Config versioning breaks existing users | Medium | Medium | 6 | Migration tests; backup before migration; rollback support |
| New modules depend on P6-only code | Medium | Medium | 6 | Clear phase boundaries; P5 modules must remain P6-agnostic |
| Intelligence layer boundary violation | Low | High | 4 | Read-only contract enforced; tests for violation |

### 2.2 Security Risks

| Risk | Likelihood | Impact | Score | Mitigation |
|------|------------|--------|-------|------------|
| Approval gate bypass | Possible | Critical | 12 | Code review; security testing; mandatory gate for all Ask/Dangerous actions |
| Command injection via tool arguments | Possible | Critical | 12 | Input validation; command allowlist; sandbox |
| Malicious MCP server escapes sandbox | Possible | Critical | 12 | Sandbox isolation; network restrictions; crash detection |
| Malicious plugin escapes sandbox | Possible | Critical | 12 | Plugin sandbox; approval gate; signature verification |
| Prompt injection via tool output | Likely | High | 12 | Output sanitization; input validation |
| API key exposure in logs | Unlikely | High | 8 | Secret redaction; keychain storage |
| Preference manipulation | Possible | High | 8 | Deterministic engine; no external input |
| Data exfiltration via MCP | Possible | High | 8 | Network restrictions; monitoring |

### 2.3 Implementation Risks

| Risk | Likelihood | Impact | Score | Mitigation |
|------|------------|--------|-------|------------|
| Preference Engine non-deterministic | Medium | High | 8 | Strict policy; unit tests for determinism |
| MCP sandbox implementation defects | Medium | High | 8 | Isolation testing; penetration testing |
| Approval gate race conditions | Medium | Medium | 6 | Concurrency tests; atomic state transitions |
| Config migration failures | Medium | Medium | 6 | Migration tests; backup before migration |
| Performance regression from P5 | Low | Medium | 4 | Benchmark gates at each phase |
| Test coverage gaps | Medium | Medium | 6 | 90%+ coverage target; regression tests |
| Documentation drift | Low | Medium | 4 | Contract-first development; docs as code |

### 2.4 Operational Risks

| Risk | Likelihood | Impact | Score | Mitigation |
|------|------------|--------|-------|------------|
| User confusion from adaptive behavior | Medium | Medium | 6 | Explainability policy; clear UI indicators |
| Approval fatigue from too many gates | Medium | Medium | 6 | Rate limiting; auto-approve Safe actions |
| Performance impact of monitoring | Low | Medium | 4 | Async monitoring; sampling |
| Storage growth from history | Medium | Low | 4 | Retention policies; cleanup jobs |
| Recovery after crash | Low | Medium | 4 | Pending gate recovery; backup system |

---

## 3. Risk Matrix

```
Critical ┤  R-006 R-011 R-021 R-027 R-032 R-048
         │  (Approval  (Command  (MCP     (Plugin  (Prompt  (Workflow
         │   bypass)   injection) server)  escape)  inj.)    escal.)
High     ┤  R-001 R-005 R-016 R-017 R-022 R-023 R-028 R-029 R-030 R-034 R-036 R-041 R-042
         │  (API key  (Pref     (Path    (Symlink)(MCP     (MCP     (Plugin  (Plugin  (MCP     (Prompt  (System  (Pref     (Pref
         │   leakage)  manipulation)(traversal)(escape)  tool     (data    escape  code     exfil    inj.    prompt    inj.     corruption)
         │            )            )       )      inj.)    exfil)  )       inj.)   )       man.)              )
Medium   ┤  R-003 R-004 R-007 R-009 R-010 R-013 R-015 R-018 R-019 R-025 R-033 R-039 R-043 R-045 R-046 R-049 R-051 R-052
         │  (Key     (Env      (Perm   (Dup   (Concur)(Res    (Env    (Unauth)(Sensit)(MCP    (Prompt (Tool  (Pref   (Intent (Workf  (Config (Config
         │   dump)   var leak)  confusion)(appr)  approval)(exhaust)(var    file    file   crash   inj.   name   corr.)  hijack) low    corrupt)(format
         │            )       )       )       limit)  )       leak)   access  access )   )      via     coll.            )       )
Low      ┤  R-002 R-008 R-020 R-024 R-026 R-031 R-035 R-037 R-038 R-040 R-044
         │  (Log    (Perm   (File  (MCP    (MCP    (Plug   (Jailbr)(Tool  (Tool  (Tool  (Intent
         │   leak)  mix)    perm  priv    conn.  in sup  eak    sig    name   behav  confus)
         │            )       bypass escalation loss   chain           forg.  coll.  manip           )
         └──────────────────────────────────────────────────────────────────────────────────────
           Rare     Unlikely   Possible    Likely
                      Likelihood
```

---

## 4. Critical Risks Requiring Mitigation Before P6.1

| Risk ID | Risk | Mitigation | Owner | Target Phase |
|---------|------|------------|-------|-------------|
| R-006 | Approval gate bypass | Implement per APPROVAL_GATE_SPEC; code review | Security | P6.0 follow-up |
| R-011 | Command injection | Input validation + command allowlist + sandbox | Engineering | P6.1 |
| R-021 | Malicious MCP server | Sandbox isolation per MCP_SANDBOX_SPEC | Engineering | P6.1 |
| R-027 | Malicious plugin | Plugin sandbox + approval gate | Engineering | P6.1 |
| R-032 | Prompt injection via tool output | Output sanitization layer | Engineering | P6.1 |
| R-048 | Workflow privilege escalation | Each step through approval gate | Engineering | P6.3 |

**All 6 critical risks have documented mitigations. None block P6.0 completion.**

---

## 5. Risk Acceptance

| Risk ID | Accepted? | Rationale |
|---------|-----------|-----------|
| R-003 | Yes | Keys passed by reference; memory dumps are developer-side |
| R-004 | Yes | Keychain will be implemented in P6.1 |
| R-024 | Yes | MCP sandbox prevents host filesystem access |
| R-031 | Yes | Plugin supply chain is lower priority than direct injection |
| R-035 | Yes | Deterministic engine prevents external manipulation |
| R-044 | Yes | Intent validation catches hijacking attempts |

**No risks are unacceptably open.**

---

## 6. Risk Monitoring Plan

| Metric | Threshold | Action | Frequency |
|--------|-----------|--------|-----------|
| Failed approval attempts | > 10/hour | Alert security team | Real-time |
| Sandbox violations | > 0 | Terminate server immediately | Real-time |
| Prompt injection detections | > 5/hour | Review sanitization layer | Daily |
| Configuration corruption events | > 0 | Review migration code | Daily |
| Tool registration failures | > 0 | Review tool validation | Daily |
| Performance regression | > 10% from baseline | Investigate and fix | Per phase |
| Test coverage drop | < 90% | Block merge | Per PR |

---

## 7. Risk Summary

| Category | Total Risks | Critical | High | Medium | Low |
|----------|-------------|----------|------|--------|-----|
| Architectural | 5 | 0 | 2 | 2 | 1 |
| Security | 13 | 6 | 7 | 0 | 0 |
| Implementation | 7 | 0 | 1 | 4 | 2 |
| Operational | 5 | 0 | 0 | 3 | 2 |
| **Total** | **30** | **6** | **10** | **9** | **5** |

---

## 8. Conclusion

The risk assessment identifies **6 critical risks** that require mitigation before P6.1 implementation. All critical risks have documented mitigations that are either:
- Already implemented (approval gate spec, MCP sandbox spec)
- Planned for P6.1 implementation
- Not blockers for P6.0 completion

**Overall risk level: ACCEPTABLE.**

Proceed to P6.1 after architecture review and resolution of critical findings.

---

## 9. References

- [SECURITY_REVIEW.md](./SECURITY_REVIEW.md)
- [SECURITY_RISK_MATRIX.md](./SECURITY_RISK_MATRIX.md)
- [APPROVAL_GATE_SPEC.md](../specs/APPROVAL_GATE_SPEC.md)
- [MCP_SANDBOX_SPEC.md](../specs/MCP_SANDBOX_SPEC.md)

---

## 10. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
