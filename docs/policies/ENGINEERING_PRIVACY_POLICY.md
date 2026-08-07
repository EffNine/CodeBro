# Engineering Privacy Policy

**Document:** `docs/policies/ENGINEERING_PRIVACY_POLICY.md`
**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-06
**Owner:** CodeBro Engineering
**Phase:** P6.0 Implementation Readiness

---

## 1. Purpose

This policy defines the privacy boundaries for CodeBro's engineering operations. It establishes what data is stored locally, what data is sent to providers, what telemetry is collected, and who owns what data.

**Core principle: The user owns their data. CodeBro is a tool, not a service.**

---

## 2. Local Storage

### 2.1 What Is Stored Locally

| Data | Location | Purpose | Owner |
|------|----------|---------|-------|
| Configuration | `~/.codebro/config.toml` | Runtime settings | User |
| Session history | `~/.codebro/sessions/` | Context continuity | User |
| Project memory | `~/.codebro/project_memory.json` | Workspace understanding | User |
| Global memory | `~/.codebro/memory.json` | Cross-project knowledge | User |
| Tool history | `~/.codebro/tool_history.json` | Pattern recognition | User |
| Approval history | `~/.codebro/approval_history.json` | Audit trail | User |
| Preferences | `~/.codebro/preferences.json` | Personalization | User |
| Skills | `~/.codebro/skills.json` | Skill management | User |
| Traces | `~/.codebro/traces/` | Debugging | User |
| Code index | `~/.codebro/code_index.db` | Symbol search | User |
| Backups | `~/.codebro/backups/` | Recovery | User |

### 2.2 Local Storage Security

| Measure | Implementation |
|---------|---------------|
| Directory permissions | `chmod 700 ~/.codebro/` |
| File permissions | `chmod 600` for all sensitive files |
| API key storage | Platform keychain (optional) or environment variable |
| Encryption at rest | Not enforced (user's OS responsibility) |
| Access control | OS-level user isolation |

### 2.3 Local Storage Guarantees

1. All local data is **user-owned**.
2. No local data is transmitted without explicit user action.
3. Users can **delete all local data** at any time.
4. Users can **export all local data** at any time.
5. Local data is **never shared** with third parties.

---

## 3. Cloud Requests

### 3.1 What Is Sent to Providers

| Data | Purpose | Provider |
|------|---------|----------|
| User message | LLM inference | Configured provider |
| Tool output | LLM context | Configured provider |
| System prompt | LLM context | Configured provider |
| Model name | API routing | Configured provider |

### 3.2 What Is NOT Sent to Providers

| Data | Reason |
|------|--------|
| API keys | Never transmitted in requests |
| Full file contents | Only relevant symbols/snippets |
| Source code | Only code intelligence metadata |
| User preferences | Stored locally only |
| Session history | Stored locally only |
| Approval history | Stored locally only |
| Tool history | Stored locally only |

### 3.3 Provider Boundaries

1. **One provider per session** — credentials are not shared across providers.
2. **Provider selection is user-controlled** — CodeBro does not auto-switch providers.
3. **Provider responses are not stored** — Only summaries are kept locally.
4. **Provider errors are handled locally** — No error details sent to third parties.
5. **Provider health checks use minimal data** — Only a test message is sent.

### 3.4 Network Security

| Aspect | Implementation |
|--------|---------------|
| Transport | HTTPS/TLS 1.2+ for all provider connections |
| Certificate validation | Enforced |
| Proxy support | Respects `HTTPS_PROXY`, `HTTP_PROXY` env vars |
| Connection timeout | 30 seconds |
| Retry policy | 3 retries with exponential backoff |

---

## 4. Telemetry Policy

### 4.1 Current State: NO TELEMETRY

CodeBro currently collects **zero telemetry**. This is a deliberate design decision.

### 4.2 Future Telemetry (If Implemented)

If telemetry is ever added, the following policy applies:

| Telemetry Type | Required? | Anonymized? | Opt-out? |
|---------------|-----------|-------------|----------|
| Crash reports | No | Yes | Yes |
| Performance metrics | No | Yes | Yes |
| Feature usage | No | Yes | Yes |
| Error rates | No | Yes | Yes |
| Location data | Never | N/A | N/A |
| User content | Never | N/A | N/A |
| API keys | Never | N/A | N/A |

### 4.3 Telemetry Opt-Out

If telemetry is added in the future:
1. Opt-out must be available via `/telemetry:off` command.
2. Opt-out must be respected immediately.
3. Opt-out must persist across restarts.
4. Opt-out must be documented in the TUI.

**Current status:** No telemetry is collected. This policy exists to prevent future telemetry without explicit user consent.

---

## 5. Diagnostics Policy

### 5.1 What Diagnostics Collect

| Diagnostic | Data Collected | Stored Locally? |
|------------|---------------|-----------------|
| Tool execution | Tool name, duration, success/failure | Yes |
| Provider calls | Model, tokens, latency, success/failure | Yes |
| Memory operations | Operation type, duration, success/failure | Yes |
| Crash dumps | Stack trace, OS info, CodeBro version | Yes (optional) |

### 5.2 What Diagnostics Do NOT Collect

| Data | Reason |
|------|--------|
| User messages | Privacy |
| Tool arguments (secrets) | Security |
| File contents | IP protection |
| Source code | IP protection |
| API keys | Security |
| Personal data | Privacy |

### 5.3 Diagnostic Export

Users can export diagnostics via `/diagnostics:export`. Exported diagnostics:
- Are anonymized (no user content).
- Include only error/recovery-relevant data.
- Are user-controlled (never auto-sent).

---

## 6. Crash Reports

### 6.1 Current State: NO CRASH REPORTS

CodeBro does **not** send crash reports anywhere. Crash information is stored locally only.

### 6.2 Local Crash Storage

If a crash occurs:
1. Stack trace is written to `~/.codebro/crash_<timestamp>.log`.
2. OS information is recorded.
3. CodeBro version is recorded.
4. No user data is included.

### 6.3 Future Crash Reports (If Implemented)

If crash reporting is added:
1. Must be **opt-in only**.
2. Must be **anonymized**.
3. Must **never include user data**.
4. Must be **transmitted over HTTPS**.
5. Must allow **opt-out at any time**.

---

## 7. Provider Boundaries

### 7.1 Provider Data Flow

```
User Input
    ↓
CodeBro (local processing)
    ↓
Provider Request (minimal data)
    ↓
Provider Response
    ↓
CodeBro (local processing)
    ↓
User Output
```

### 7.2 Data Boundary Rules

| Boundary | Rule |
|----------|------|
| User → CodeBro | Full user input accepted |
| CodeBro → Provider | Only what's needed for inference |
| Provider → CodeBro | Full response received |
| CodeBro → User | Filtered, safe output |

### 7.3 Provider Isolation

1. Each provider is isolated — credentials and state are not shared.
2. Provider switching does not leak data between providers.
3. Provider health checks do not send user data.
4. Provider errors are handled locally.

---

## 8. User Ownership

### 8.1 Data Ownership Statement

| Data Type | Owner | Control |
|-----------|-------|---------|
| All local files | User | Full control (read/write/delete) |
| All provider requests | User | Can review, can revoke |
| All provider responses | User | Stored locally, user-owned |
| All preferences | User | Full control |
| All history | User | Full control |

### 8.2 User Rights

| Right | Implementation |
|-------|---------------|
| Right to access | `codebro export` command |
| Right to rectify | Direct file editing or `/preferences` command |
| Right to erasure | `codebro wipe` or per-item deletion |
| Right to portability | JSON export format |
| Right to object | Opt-out of any future telemetry |
| Right to restriction | Local-only mode available |

### 8.3 No Third-Party Sharing

CodeBro **never**:
- Sells user data.
- Shares user data with third parties.
- Uses user data for advertising.
- Uses user data for model training.
- Transmits user data without explicit purpose.

---

## 9. Compliance

### 9.1 GDPR

| Requirement | CodeBro Status |
|-------------|---------------|
| Right to erasure | ✅ `codebro wipe` |
| Right to access | ✅ `codebro export` |
| Right to rectification | ✅ Direct file editing |
| Data minimization | ✅ Only necessary data stored |
| Purpose limitation | ✅ Data used only for CodeBro functionality |
| Storage limitation | ✅ Retention policies enforced |
| Integrity and confidentiality | ✅ Local-only, encrypted in transit |

### 9.2 CCPA

| Requirement | CodeBro Status |
|-------------|---------------|
| Right to know | ✅ User can export all data |
| Right to delete | ✅ `codebro wipe` |
| Right to opt-out of sale | ✅ No data is sold |
| Non-discrimination | ✅ No different treatment for exercising rights |

### 9.3 Internal Privacy Standards

| Standard | Status |
|----------|--------|
| No telemetry without consent | ✅ Current: none; Future: opt-in |
| No user content in logs | ✅ Secrets redacted |
| No third-party data sharing | ✅ Never |
| User owns all data | ✅ Explicitly stated |
| Data minimization | ✅ Only what's needed |

---

## 10. Privacy by Design

### 10.1 Principles Applied

1. **Privacy as default**: No data leaves the machine unless explicitly requested.
2. **Privacy as configuration**: Users control what is stored and shared.
3. **Privacy as transparency**: Users can see exactly what data is collected.
4. **Privacy as user control**: Users can delete, export, or modify any data.

### 10.2 Future Considerations

If CodeBro adds cloud features in the future:
1. Privacy policy must be updated before release.
2. User consent must be obtained explicitly.
3. Opt-out must remain available.
4. Data minimization must remain enforced.

---

## 11. References

- [ADAPTIVE_MEMORY_POLICY.md](./ADAPTIVE_MEMORY_POLICY.md)
- [SECURITY_REVIEW.md](../reports/SECURITY_REVIEW.md)
- [DX Principles](../vision/DX_PRINCIPLES.md)

---

## 12. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
