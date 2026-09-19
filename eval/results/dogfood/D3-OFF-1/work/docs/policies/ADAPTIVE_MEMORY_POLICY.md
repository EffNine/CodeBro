# Adaptive Memory Policy

**Document:** `docs/policies/ADAPTIVE_MEMORY_POLICY.md`
**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-06
**Owner:** CodeBro Engineering
**Phase:** P6.0 Implementation Readiness

---

## 1. Purpose

This policy defines what CodeBro may remember, what it must never remember, and how memory is managed across sessions. The Preference Engine must remain deterministic — memory influences recommendations but never overrides user intent.

---

## 2. What May Be Remembered

### 2.1 Local Memory (User-Owned)

| Data Type | Location | Retention | Purpose |
|-----------|----------|-----------|---------|
| User preferences | `~/.codebro/preferences.json` | Indefinite | Personalize recommendations |
| Project context | `~/.codebro/project_memory.json` | Per-project | Understand workspace |
| Session history | `~/.codebro/sessions/` | 90 days | Context continuity |
| Tool execution history | `~/.codebro/tool_history.json` | 30 days | Pattern recognition |
| Approval history | `~/.codebro/approval_history.json` | 90 days | Audit trail |
| Skill confidence scores | `~/.codebro/skills.json` | Indefinite | Skill ranking |
| Reflection insights | `~/.codebro/reflections.json` | Indefinite | Learning |

### 2.2 Provider Memory (Provider-Owned)

| Data Type | Location | Retention | Purpose |
|-----------|----------|-----------|---------|
| Provider API keys | Keychain / env var | Until rotated | Authentication |
| Provider health status | In-memory | Session lifetime | Availability |
| Provider model info | `~/.codebro/providers.json` | Indefinite | Provider selection |

### 2.3 Ephemeral Memory (In-Memory Only)

| Data Type | Location | Retention | Purpose |
|-----------|----------|-----------|---------|
| Short-term context | In-memory | Session lifetime | Current task context |
| Reasoning traces | In-memory | Task lifetime | Debugging |
| Tool execution state | In-memory | Task lifetime | Pipeline state |

---

## 3. What Must Never Be Remembered

### 3.1 Absolute Prohibitions

| Data Type | Reason | Enforcement |
|-----------|--------|-------------|
| API keys (plaintext) | Credential exposure | Stored only in keychain/env |
| Full request/response bodies | Privacy + token cost | Only summaries stored |
| Source code content | IP protection | Only symbols/metadata stored |
| User passwords | Security | Never collected |
| Personal identification | Privacy | Never collected |
| Financial data | Security | Never collected |
| Health data | Privacy (HIPAA) | Never collected |
| Authentication tokens (raw) | Credential exposure | Hashed or keychain-only |

### 3.2 Conditional Prohibitions

| Data Type | Condition | Exception |
|-----------|-----------|-----------|
| File contents | Read tools | Only symbol metadata stored, not full file content |
| Command output | Shell tools | Only success/failure + duration stored |
| Network requests | Provider calls | Only model + tokens + latency stored |
| Error details | All tools | Stack traces not persisted |

### 3.3 Prohibited Memory Patterns

```
NEVER store:
- Raw LLM responses (only summaries)
- Full tool arguments containing secrets
- Raw user input beyond current session
- Complete file contents (only symbols/metadata)
- Any data marked as sensitive by the user
```

---

## 4. Retention Policy

### 4.1 Retention Schedule

| Memory Type | Retention Period | Cleanup Trigger |
|-------------|-----------------|-----------------|
| Session history | 90 days | Age-based + size limit |
| Tool execution history | 30 days | Age-based |
| Approval history | 90 days | Age-based |
| Preferences | Indefinite | User deletion |
| Project memory | Indefinite | User deletion |
| Skill confidence | Indefinite | Manual reset |
| Reflections | Indefinite | User deletion |
| Short-term context | Session lifetime | Session end |
| Provider health | Session lifetime | Session end |

### 4.2 Size Limits

| Storage | Max Size | Action When Exceeded |
|---------|----------|---------------------|
| Session files | 100 MB total | Oldest sessions deleted first |
| Tool history | 50 MB | Oldest entries archived |
| Approval history | 10 MB | Oldest entries archived |
| Project memory | 5 MB | Low-confidence entries removed |
| Preferences | 1 MB | Rarely-used preferences archived |

### 4.3 Cleanup Process

1. **Daily**: Age-based cleanup runs on startup.
2. **On demand**: User can trigger cleanup via `/cleanup` command.
3. **On low disk**: Automatic cleanup when disk < 10% free.

---

## 5. Deletion Policy

### 5.1 User-Initiated Deletion

| Action | Command | Effect |
|--------|---------|--------|
| Delete session | `/sessions:delete <id>` | Remove session file |
| Clear history | `/history:clear` | Clear tool/approval history |
| Reset preferences | `/preferences:reset` | Clear all user preferences |
| Clear project memory | `/memory:clear` | Clear project-level memory |
| Wipe all data | `codebro wipe` | Delete all `.codebro/` data |

### 5.2 Automatic Deletion

| Data | Trigger | Action |
|------|---------|--------|
| Expired sessions | Age > 90 days | Delete file |
| Expired history | Age > 30/90 days | Archive then delete |
| Low-confidence patterns | Confidence < 0.3 | Remove from memory |
| Orphaned symbols | No file reference | Remove from index |

### 5.3 Deletion Guarantees

- Deletion is **immediate** (not soft-delete).
- Deleted data is **not recoverable** (no trash).
- Deletion is **logged** for audit.
- Deletion is **atomic** (all-or-nothing).

---

## 6. Export Policy

### 6.1 Exportable Data

| Data | Format | Purpose |
|------|--------|---------|
| Sessions | JSON | Backup, analysis |
| Preferences | JSON | Migration, backup |
| Project memory | JSON | Backup, sharing |
| Tool history | JSONL | Analysis |
| Approval history | JSON | Audit |
| Skills | JSON | Backup, migration |

### 6.2 Export Process

```rust
pub struct DataExporter {
    pub data_type: ExportDataType,
    pub format: ExportFormat,
    pub output_path: PathBuf,
}

pub enum ExportDataType {
    Sessions,
    Preferences,
    ProjectMemory,
    ToolHistory,
    ApprovalHistory,
    Skills,
    All,
}

pub enum ExportFormat {
    Json,
    Jsonl,
    Csv,
}
```

### 6.3 Export Restrictions

- Exports **never include** API keys or sensitive credentials.
- Exports **never include** raw LLM responses.
- Exports **never include** source code content.
- Exported data is **user-owned** — no restrictions on downstream use.

---

## 7. Reset Policy

### 7.1 Partial Reset

| Reset Type | Scope | Command |
|------------|-------|---------|
| Reset preferences | User preferences only | `/preferences:reset` |
| Reset project memory | Project-level memory only | `/memory:reset` |
| Reset skills | Skill confidence only | `/skills:reset` |
| Reset history | Tool + approval history | `/history:reset` |

### 7.2 Full Reset

| Reset Type | Scope | Command |
|------------|-------|---------|
| Full reset | All local data | `codebro reset` |
| Factory reset | All data + config | `codebro reset --factory` |

### 7.3 Reset Guarantees

- Reset is **irreversible**.
- Reset requires **confirmation** (double-confirm for full reset).
- Reset is **logged**.
- Reset does **not** affect provider credentials (stored in keychain).

---

## 8. Local vs Provider Data

### 8.1 Data Ownership

| Data | Owner | Storage | Access |
|------|-------|---------|--------|
| Preferences | User | Local | User only |
| Project memory | User | Local | User + CodeBro |
| Session history | User | Local | User + CodeBro |
| Provider credentials | User | Keychain | CodeBro (read-only) |
| Provider responses | Provider | Provider | Provider only |
| Tool execution | CodeBro | Local | CodeBro |

### 8.2 Data Flow

```
User → Local Storage (preferences, memory, history)
User → Provider (requests, credentials)
Provider → User (responses)
CodeBro → Local Storage (tool history, approval history)
CodeBro → Provider (requests with credentials)
```

### 8.3 Boundary Rules

1. **Local data never leaves the machine** without explicit user export.
2. **Provider data is never stored locally** beyond session context.
3. **Credentials are never transmitted** except to the configured provider.
4. **User can delete any local data** at any time.
5. **User can revoke provider access** by rotating credentials.

---

## 9. Preference Engine Determinism

### 9.1 Core Principle

The Preference Engine is **deterministic**. Given the same input (preferences + context), it always produces the same output (recommendations). It does not learn from interactions in a way that changes its core behavior.

### 9.2 Determinism Guarantees

| Aspect | Guarantee |
|--------|-----------|
| Input → Output | Same input always produces same output |
| No stochastic learning | No random behavior changes |
| No external influence | Preferences are user-defined only |
| Reproducible | Recommendations can be reproduced |
| Auditable | All preference changes are logged |

### 9.3 What the Preference Engine Does NOT Do

- It does **not** learn from user corrections (that would be non-deterministic).
- It does **not** modify its own weights based on interaction patterns.
-它 does **not** make recommendations based on external data sources.
- It does **not** change behavior without explicit user configuration.

### 9.4 Preference Update Rules

1. Preferences are updated **only** by explicit user action.
2. Preference updates are **logged** with timestamp and value change.
3. Preference updates are **validated** before applying.
4. Preference updates are **reversible** (undo supported).

---

## 10. Memory Security

### 10.1 Storage Security

| Storage | Security Measure |
|---------|-----------------|
| `~/.codebro/` directory | chmod 700 |
| Config files | chmod 600 |
| Session files | chmod 600 |
| Memory files | chmod 600 |
| Keychain entries | Platform keychain (OS-managed) |

### 10.2 Access Control

- Only the owning user can read/write `.codebro/` data.
- No network access to local storage.
- No sharing of local data between users.

### 10.3 Encryption

| Data | Encrypted? | Method |
|------|------------|--------|
| Config file | No | File permissions (600) |
| Session files | No | File permissions (600) |
| Memory files | No | File permissions (600) |
| API keys | Yes (optional) | Platform keychain |
| Sensitive preferences | Yes (opt-in) | AES-256 encryption |

---

## 11. Compliance

### 11.1 GDPR

- **Right to erasure**: Users can delete all local data.
- **Right to access**: Users can export all local data.
- **Right to rectification**: Users can modify preferences.
- **Data minimization**: Only necessary data is stored.

### 11.2 CCPA

- **Right to know**: Users can see what data is stored.
- **Right to delete**: Users can delete all local data.
- **Right to opt-out**: No data is sold or shared.

### 11.3 Internal Compliance

- All memory operations are logged.
- All deletions are logged.
- All exports are user-initiated.
- No telemetry without explicit consent.

---

## 12. References

- [Memory Contract](../contracts/memory_contract.md)
- [ADR-009: Configuration Versioning](../ADR/adr-009-configuration-versioning.md)
- [APPROVAL_GATE_SPEC.md](../specs/APPROVAL_GATE_SPEC.md)
- [DX Principles](../vision/DX_PRINCIPLES.md)

---

## 13. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
