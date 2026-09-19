# Approval Gate Specification

**Document:** `docs/specs/APPROVAL_GATE_SPEC.md`
**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-06
**Owner:** CodeBro Engineering
**Related ADR:** ADR-009 (Configuration Versioning)

---

## 1. Purpose

This specification defines the complete approval gate system for P6 adaptive behavior. Every adaptive action that modifies user state, executes tools, installs components, or changes configuration must pass through an approval gate before execution. The approval gate is the primary safety mechanism that prevents unauthorized or unexpected changes.

---

## 2. Approval Gate States

```
                    ┌──────────────┐
                    │   PENDING    │
                    │  (awaiting   │
                    │   user)      │
                    └──────┬───────┘
                           │
              ┌────────────┼────────────┐
              ▼                       ▼
    ┌─────────────────┐      ┌─────────────────┐
    │   ACCEPTED      │      │   REJECTED      │
    │  (action runs)  │      │  (action dies)  │
    └────────┬────────┘      └────────┬────────┘
             │                        │
             ▼                        ▼
    ┌─────────────────┐      ┌─────────────────┐
    │   COMPLETED     │      │   CANCELLED     │
    │  (tracked)      │      │  (logged)       │
    └─────────────────┘      └─────────────────┘
             │
             │  (before timeout)
             ▼
    ┌─────────────────┐
    │   TIMEOUT       │
    │  (treated as    │
    │   rejection)    │
    └─────────────────┘
```

---

## 3. Approval Gate Lifecycle

### 3.1 Gate Creation

When an adaptive subsystem requests an action that requires approval:

1. **Gate ID generation**: A UUIDv4 is generated for the approval gate.
2. **Action serialization**: The action is serialized with all parameters.
3. **Risk classification**: The action is classified by risk level (see Section 4).
4. **Gate insertion**: The gate is inserted into the active gates registry.
5. **User notification**: The TUI displays the approval request.
6. **Timeout start**: The timeout timer begins (default: 60 seconds).

```rust
pub struct ApprovalGate {
    pub gate_id: Uuid,
    pub action: SerializedAction,
    pub risk_level: RiskLevel,
    pub created_at: DateTime<Utc>,
    pub timeout_at: DateTime<Utc>,
    pub state: GateState,
    pub user_id: Option<String>,
    pub session_id: String,
}

pub enum GateState {
    Pending,
    Accepted,
    Rejected,
    Timeout,
    Cancelled,
    Completed,
}

pub enum RiskLevel {
    Safe,        // Read-only, no user state change
    Ask,         // Requires explicit approval
    Dangerous,   // Destructive, requires double confirmation
}
```

### 3.2 Approval Accepted

When the user accepts an approval request:

1. **State transition**: `Pending` → `Accepted`
2. **Action execution**: The serialized action is deserialized and executed.
3. **Result capture**: The execution result is captured.
4. **Gate completion**: State transitions to `Completed`.
5. **Log entry**: An activity log entry is created.
6. **Registry removal**: The gate is removed from active gates.
7. **History persistence**: The completed gate is persisted to `~/.codebro/approval_history.json`.

```rust
impl ApprovalGate {
    pub fn accept(&mut self) -> Result<ActionResult> {
        self.state = GateState::Accepted;
        let result = self.action.execute()?;
        self.state = GateState::Completed;
        self.persist_to_history()?;
        Ok(result)
    }
}
```

### 3.3 Approval Rejected

When the user rejects an approval request:

1. **State transition**: `Pending` → `Rejected`
2. **Action abort**: The requested action is not executed.
3. **Gate completion**: State transitions to `Cancelled`.
4. **Log entry**: A rejection log entry is created.
5. **Registry removal**: The gate is removed from active gates.
6. **History persistence**: The rejected gate is persisted.

```rust
impl ApprovalGate {
    pub fn reject(&mut self) {
        self.state = GateState::Rejected;
        self.state = GateState::Cancelled;
        self.persist_to_history();
    }
}
```

### 3.4 Approval Timeout

When the timeout expires before user action:

1. **State transition**: `Pending` → `Timeout`
2. **Equivalent to rejection**: The action is not executed.
3. **Log entry**: A timeout log entry is created with severity `Warning`.
4. **Registry removal**: The gate is removed from active gates.
5. **History persistence**: The timed-out gate is persisted.

```rust
impl ApprovalGate {
    pub fn on_timeout(&mut self) {
        self.state = GateState::Timeout;
        tracing::warn!(
            "Approval gate {} timed out after {}s",
            self.gate_id,
            self.timeout_at.duration_since(self.created_at).as_secs()
        );
        self.persist_to_history();
    }
}
```

---

## 4. Risk Classification

| Risk Level | Description | Timeout | User Display |
|------------|-------------|---------|--------------|
| `Safe` | Read-only, no state change | N/A (auto-approve) | Info notification |
| `Ask` | Modifies user state, reversible | 60s | Approval dialog with accept/reject |
| `Dangerous` | Destructive, irreversible | 30s | Double confirmation required |

### 4.1 Action Classification Rules

| Action Type | Default Risk | Reason |
|-------------|--------------|--------|
| Read file | `Safe` | No state change |
| List directory | `Safe` | No state change |
| Git status | `Safe` | No state change |
| Create file | `Ask` | Modifies filesystem |
| Edit file | `Ask` | Modifies filesystem |
| Run command | `Ask` | May have side effects |
| Delete file | `Dangerous` | Irreversible |
| Install MCP server | `Dangerous` | Modifies system |
| Change API key | `Ask` | Modifies credentials |
| Auto-approve safe ops | `Safe` | User-configured |

---

## 5. Duplicate Approval Detection

### 5.1 Detection Algorithm

When a new approval request is created, the system checks for duplicates:

1. **Action hash**: Compute a hash of the serialized action.
2. **Time window**: Check for gates with the same hash created within the last 5 seconds.
3. **Session match**: Only compare gates within the same session.

```rust
impl ApprovalGateRegistry {
    pub fn check_duplicate(&self, action: &SerializedAction, session_id: &str) -> Option<Uuid> {
        let hash = action.hash();
        let window_start = Utc::now() - Duration::seconds(5);

        self.active_gates
            .iter()
            .filter(|g| g.session_id == session_id && g.created_at > window_start)
            .find(|g| g.action.hash() == hash)
            .map(|g| g.gate_id)
    }
}
```

### 5.2 Duplicate Handling

If a duplicate is detected:
1. The new gate is **not created**.
2. The existing gate's timeout is **reset to 60 seconds**.
3. The user is **not notified again** (already waiting).
4. A debug log entry is recorded.

---

## 6. Concurrent Approval Handling

### 6.1 Concurrency Model

Multiple approval gates can exist concurrently:
- Each gate has a unique ID.
- Each gate operates independently.
- The TUI displays all pending gates in a stacked dialog.

### 6.2 Concurrency Limits

| Limit | Value | Reason |
|-------|-------|--------|
| Max concurrent gates | 10 | Prevent UI overload |
| Max gates per session | 50 | Prevent memory issues |
| Max gates per minute | 5 | Prevent approval fatigue |

If the concurrent limit is reached:
1. The new request is **queued**.
2. The requesting subsystem is notified of the queue position.
3. When a gate completes, the next queued request is processed.

```rust
pub struct ApprovalQueue {
    pending: VecDeque<ApprovalRequest>,
    max_concurrent: usize,
    max_per_minute: usize,
}

impl ApprovalQueue {
    pub fn try_enqueue(&mut self, request: ApprovalRequest) -> Result<QueueResult> {
        if self.active_count() >= self.max_concurrent {
            return Err(ApprovalError::ConcurrencyLimitReached);
        }
        if self.recent_count() >= self.max_per_minute {
            return Err(ApprovalError::RateLimitReached);
        }
        self.pending.push_back(request);
        Ok(QueueResult::Queued(self.pending.len()))
    }
}
```

---

## 7. Cancelled Approval

### 7.1 Cancellation Trigger

An approval gate can be cancelled when:
1. The user explicitly cancels (ESC key).
2. The session ends.
3. The workspace changes.
4. The parent action is cancelled.

### 7.2 Cancellation Flow

1. **State transition**: `Pending` → `Cancelled`
2. **Resource cleanup**: Any allocated resources are released.
3. **Log entry**: A cancellation log entry is created.
4. **Registry removal**: The gate is removed from active gates.

```rust
impl ApprovalGate {
    pub fn cancel(&mut self, reason: CancelReason) {
        self.state = GateState::Cancelled;
        self.cleanup();
        self.log_cancellation(reason);
    }
}
```

---

## 8. Interrupted Approval (Crash/Restart Recovery)

### 8.1 Recovery on Restart

When CodeBro restarts:

1. **Scan for pending gates**: Read `~/.codebro/pending_gates.json`.
2. **Validate timestamps**: Remove gates older than 5 minutes.
3. **Timeout remaining gates**: All remaining gates are marked as `Timeout`.
4. **Log recovery**: A recovery log entry is created.

```rust
impl ApprovalGateRegistry {
    pub fn recover_from_restart(&mut self) {
        let pending = self.load_pending_gates();
        let now = Utc::now();

        for mut gate in pending {
            if now.duration_since(gate.created_at).unwrap_or_default() > Duration::minutes(5) {
                gate.state = GateState::Timeout;
                gate.persist_to_history();
            } else {
                // Gate is too recent to be stale; keep it pending
                self.active_gates.push(gate);
            }
        }
    }
}
```

### 8.2 Persistence Format

```json
{
  "version": 1,
  "gates": [
    {
      "gate_id": "550e8400-e29b-41d4-a716-446655440000",
      "action": { ... },
      "risk_level": "Ask",
      "created_at": "2026-08-06T10:00:00Z",
      "timeout_at": "2026-08-06T10:01:00Z",
      "state": "Pending",
      "session_id": "abc123"
    }
  ]
}
```

---

## 9. Recovery After Restart

### 9.1 Full Recovery Procedure

1. **Load pending gates** from `~/.codebro/pending_gates.json`.
2. **Filter stale gates** (older than 5 minutes → timeout).
3. **Re-notify user** for valid pending gates (with note that session restarted).
4. **Log recovery event** in activity log.
5. **Clear pending gates file** after recovery.

### 9.2 User Notification

When recovering a pending gate after restart:
- Display: "Approval request resumed after restart. This request was created before the restart."
- The timeout is **not extended** — the original timeout still applies.
- If the original timeout has expired, the gate is marked as `Timeout`.

---

## 10. TUI Integration

### 10.1 Approval Dialog

```
┌─────────────────────────────────────────────────────┐
│  APPROVAL REQUIRED                                  │
├─────────────────────────────────────────────────────┤
│  Risk: ⚠️  Ask                                      │
│  Action: Edit file: src/main.rs                     │
│  Change: Replace "hello" with "world" in line 42    │
│                                                     │
│  Created: 2 seconds ago                             │
│  Timeout: 58 seconds remaining                      │
│                                                     │
│  [ Accept ]  [ Reject ]  [ Cancel All ]             │
└─────────────────────────────────────────────────────┘
```

### 10.2 Multiple Pending Gates

When multiple gates are pending, they are displayed as a stack:

```
┌─────────────────────────────────────────────────────┐
│  PENDING APPROVALS (3)                              │
├─────────────────────────────────────────────────────┤
│  1. ⚠️  Edit file: src/main.rs (58s)               │
│  2. ℹ️   Run command: cargo test (45s)             │
│  3. 🚨  Delete file: build/out.o (20s)            │
│                                                     │
│  [ Accept All ]  [ Reject All ]  [ Cancel ]         │
└─────────────────────────────────────────────────────┘
```

---

## 11. Audit Trail

### 11.1 History Format

All approval gates are persisted to `~/.codebro/approval_history.json`:

```json
{
  "version": 1,
  "entries": [
    {
      "gate_id": "550e8400-...",
      "action": { ... },
      "risk_level": "Ask",
      "result": "Accepted",
      "created_at": "2026-08-06T10:00:00Z",
      "completed_at": "2026-08-06T10:00:05Z",
      "duration_ms": 5000,
      "session_id": "abc123"
    }
  ]
}
```

### 11.2 History Retention

- History is retained for **90 days**.
- Entries older than 90 days are archived to `~/.codebro/approval_history_archive.jsonl`.
- Archive is gzip-compressed.

---

## 12. Configuration

### 12.1 Config Fields

```rust
pub struct ApprovalConfig {
    /// Default timeout for approval gates (seconds)
    pub default_timeout_secs: u64,

    /// Maximum concurrent approval gates
    pub max_concurrent_gates: usize,

    /// Maximum gates per minute
    pub max_gates_per_minute: usize,

    /// Auto-approve actions at or below this risk level
    pub auto_approve_risk_level: RiskLevel,

    /// Enable approval history
    pub history_enabled: bool,

    /// History retention days
    pub history_retention_days: u32,
}
```

### 12.2 Defaults

| Field | Default | Notes |
|-------|---------|-------|
| `default_timeout_secs` | 60 | Can be overridden per risk level |
| `max_concurrent_gates` | 10 | Hard limit |
| `max_gates_per_minute` | 5 | Rate limit |
| `auto_approve_risk_level` | `Safe` | Only Safe actions are auto-approved |
| `history_enabled` | `true` | Always on |
| `history_retention_days` | 90 | Configurable |

---

## 13. Error Handling

| Error | Cause | Handling |
|-------|-------|----------|
| `DuplicateGate` | Same action within 5s | Merge with existing gate |
| `ConcurrencyLimitReached` | 10+ active gates | Queue request |
| `RateLimitReached` | 5+ gates in 1 minute | Queue request |
| `TimeoutExpired` | User didn't respond | Treat as rejection |
| `InvalidAction` | Action serialization failed | Log error, reject gate |
| `CorruptHistory` | History file is invalid | Reset history, log warning |

---

## 14. Testing Requirements

### 14.1 Unit Tests

| Test | Description |
|------|-------------|
| `test_accept_flow` | Verify accepted gate executes action |
| `test_reject_flow` | Verify rejected gate does not execute |
| `test_timeout_flow` | Verify timed-out gate is treated as rejection |
| `test_duplicate_detection` | Verify duplicate gates are merged |
| `test_concurrency_limit` | Verify queue behavior at limit |
| `test_rate_limit` | Verify rate limiting |
| `test_cancel_flow` | Verify cancellation cleanup |
| `test_restart_recovery` | Verify pending gates after restart |
| `test_stale_gate_filtering` | Verify old gates are timed out on recovery |
| `test_history_persistence` | Verify history is saved correctly |
| `test_history_retention` | Verify old entries are archived |

### 14.2 Integration Tests

| Test | Description |
|------|-------------|
| `test_tui_approval_dialog` | Verify TUI displays approval correctly |
| `test_multi_gate_display` | Verify multiple pending gates display |
| `test_auto_approve_safe` | Verify Safe actions are auto-approved |
| `test_dangerous_double_confirm` | Verify Dangerous actions require double confirm |

---

## 15. References

- [ADR-009: Configuration Versioning](../ADR/adr-009-configuration-versioning.md)
- [DX Principles](../vision/DX_PRINCIPLES.md)
- [Tool Contract](../contracts/tool_contract.md)

---

## 16. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
