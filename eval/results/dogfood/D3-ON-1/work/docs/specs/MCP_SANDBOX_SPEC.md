# MCP Sandbox Specification

**Document:** `docs/specs/MCP_SANDBOX_SPEC.md`
**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-06
**Owner:** CodeBro Engineering
**Related ADR:** ADR-009 (Configuration Versioning)

---

## 1. Purpose

This specification defines the **lifecycle-only** design for the MCP (Model Context Protocol) sandbox. No implementation code is written in this phase. The sandbox ensures that MCP servers are discovered, validated, approved, and monitored before being activated in the CodeBro runtime.

---

## 2. Lifecycle Overview

```
Discovery
    ↓
Sandbox
    ↓
Permission Review
    ↓
Validation
    ↓
Approval
    ↓
Activation
    ↓
Monitoring
    ↓
Removal
```

Each stage has explicit entry/exit criteria and produces artifacts that feed into the next stage.

---

## 3. Stage 1: Discovery

### 3.1 Purpose

Identify MCP servers available in the workspace and user environment.

### 3.2 Inputs

- Workspace root path
- User configuration (`~/.codebro/config.toml`)
- System environment variables (`PATH`, `MCP_SERVERS`)

### 3.3 Discovery Methods

| Method | Source | Priority |
|--------|--------|----------|
| Workspace config | `.codebro/mcp.json` in workspace root | 1 |
| User config | `~/.codebro/config.toml` → `workspace.mcp_servers` | 2 |
| Environment | `MCP_SERVERS` env var (JSON array) | 3 |
| System path | `which <server-name>` for known servers | 4 |

### 3.4 Discovery Output

```rust
pub struct McpDiscoveryResult {
    pub servers: Vec<McpServerDiscovery>,
    pub discovered_at: DateTime<Utc>,
    pub workspace_root: PathBuf,
}

pub struct McpServerDiscovery {
    pub name: String,
    pub transport: McpTransport,
    pub available: bool,
    pub approved: bool,
    pub source: DiscoverySource,
}

pub enum McpTransport {
    Stdio { command: String, args: Vec<String> },
    Sse { url: String },
}

pub enum DiscoverySource {
    WorkspaceConfig,
    UserConfig,
    Environment,
    SystemPath,
}
```

### 3.5 Exit Criteria

- [ ] All discovery sources have been scanned
- [ ] Duplicate server names are merged (user config overrides workspace config)
- [ ] Results are persisted to `~/.codebro/mcp_discovery.json`
- [ ] No servers are auto-approved during discovery

---

## 4. Stage 2: Sandbox

### 4.1 Purpose

Isolate MCP servers in a controlled environment before granting any permissions.

### 4.2 Sandbox Environment

| Resource | Restriction | Rationale |
|----------|-------------|-----------|
| Filesystem | Read-only access to workspace root | Prevent unauthorized writes |
| Network | Outbound only to configured URLs | Prevent data exfiltration |
| Environment | Scoped to server's declared env vars | Prevent credential leakage |
| Process | Limited to server's declared command | Prevent arbitrary execution |
| Time | Max 30 seconds per invocation | Prevent hangs |
| Memory | Max 256MB per server | Prevent memory exhaustion |

### 4.3 Sandbox Implementation

```rust
pub struct McpSandbox {
    pub server_name: String,
    pub transport: McpTransport,
    pub sandbox_config: SandboxConfig,
    pub invocation_count: u64,
    pub max_invocations: u64,
}

pub struct SandboxConfig {
    pub read_only_dirs: Vec<PathBuf>,
    pub allowed_network: Vec<String>,
    pub env_whitelist: Vec<String>,
    pub timeout_ms: u64,
    pub max_memory_mb: u64,
    pub max_invocations: u64,
}
```

### 4.4 Sandbox Actions

During sandboxing, the server is tested with:
1. **Health check**: Connect and verify server responds.
2. **Tool enumeration**: List available tools without executing.
3. **Permission probe**: Attempt restricted operations (expected to fail).
4. **Resource limit test**: Verify timeout and memory limits work.

### 4.5 Exit Criteria

- [ ] Health check passes
- [ ] Tool list is retrieved
- [ ] All restricted operations are properly blocked
- [ ] Timeouts and memory limits are enforced
- [ ] Sandbox report is generated

---

## 5. Stage 3: Permission Review

### 5.1 Purpose

Analyze the sandbox results and determine what permissions the server should have.

### 5.2 Permission Analysis

For each tool discovered in the sandbox:

| Tool Property | Permission Implication |
|---------------|----------------------|
| Reads files | `Read` permission |
| Writes files | `Write` permission (requires approval) |
| Executes commands | `Execute` permission (requires approval) |
| Accesses network | `Network` permission (requires approval) |
| Modifies state | `StateChange` permission (requires approval) |

### 5.3 Permission Categories

| Category | Tools | Auto-Approved? |
|----------|-------|----------------|
| `Read` | File reads, git status, diagnostics | Yes (Safe) |
| `Write` | File creation, file edits | No (Ask) |
| `Execute` | Shell commands, build commands | No (Ask) |
| `Network` | API calls, web fetches | No (Ask) |
| `StateChange` | Config changes, database writes | No (Dangerous) |

### 5.4 Permission Review Output

```rust
pub struct PermissionReview {
    pub server_name: String,
    pub recommended_permissions: Vec<ToolPermission>,
    pub risk_score: f32,
    pub review_notes: Vec<String>,
}

pub struct ToolPermission {
    pub tool_name: String,
    pub permission: PermissionLevel,
    pub justification: String,
}

pub enum PermissionLevel {
    Read,
    Write,
    Execute,
    Network,
    StateChange,
}
```

### 5.5 Exit Criteria

- [ ] All tools have been classified
- [ ] Risk score is calculated (0.0–1.0)
- [ ] Review notes explain each classification
- [ ] No permissions are granted without review

---

## 6. Stage 4: Validation

### 6.1 Purpose

Validate that the server conforms to security and quality requirements.

### 6.2 Validation Checks

| Check | Severity | Description |
|-------|----------|-------------|
| Schema compliance | `Error` | Server must conform to MCP spec |
| Tool naming | `Warning` | Tool names must be alphanumeric + underscore |
| Tool descriptions | `Warning` | Each tool must have a non-empty description |
| Input validation | `Info` | Check for input sanitization |
| Error handling | `Info` | Check for graceful error responses |
| Resource limits | `Error` | Server must respect sandbox limits |
| No auto-execution | `Error` | Server must not execute without user request |
| No persistence | `Warning` | Server should not write to disk unrequested |

### 6.3 Validation Output

```rust
pub struct ValidationReport {
    pub server_name: String,
    pub passed: bool,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
    pub info: Vec<ValidationInfo>,
}

pub enum ValidationError {
    SchemaViolation(String),
    MissingToolDescription(String),
    AutoExecutionDetected,
    ResourceLimitBypass,
}
```

### 6.4 Exit Criteria

- [ ] All `Error`-severity checks pass
- [ ] `Warning`-severity checks are documented
- [ ] Validation report is persisted
- [ ] Server is marked as `validated: true` or `validated: false`

---

## 7. Stage 5: Approval

### 7.1 Purpose

Present the server and its permissions to the user for final approval.

### 7.2 Approval Request

```rust
pub struct McpApprovalRequest {
    pub server_name: String,
    pub transport: McpTransport,
    pub tools: Vec<McpToolSummary>,
    pub permissions: Vec<ToolPermission>,
    pub risk_score: f32,
    pub validation_report: ValidationReport,
    pub approval_id: Uuid,
}
```

### 7.3 TUI Display

```
┌─────────────────────────────────────────────────────┐
│  NEW MCP SERVER: code-sandbox                       │
├─────────────────────────────────────────────────────┤
│  Transport: stdio                                   │
│  Command: npx @codebra/sandbox-agent                 │
│  Risk Score: 0.3 (Low)                              │
│                                                     │
│  Available Tools (4):                               │
│  • read_file (Read) - Read file contents            │
│  • write_file (Write) - Create or edit files        │
│  • run_test (Execute) - Run test suite              │
│  • fetch_docs (Network) - Fetch documentation       │
│                                                     │
│  Validation: ✓ Passed (0 errors, 1 warning)         │
│                                                     │
│  [ Approve ]  [ Reject ]  [ Approve with Limits ]   │
└─────────────────────────────────────────────────────┘
```

### 7.4 Approval Outcomes

| Outcome | Action |
|---------|--------|
| Approve | Server is activated with full permissions |
| Approve with Limits | Server is activated with restricted permissions |
| Reject | Server is removed from pending list |
| Timeout | Server is removed from pending list (treated as reject) |

### 7.5 Exit Criteria

- [ ] User has made a decision
- [ ] Decision is logged
- [ ] Server state is updated

---

## 8. Stage 6: Activation

### 8.1 Purpose

Connect the approved server to the CodeBro runtime.

### 8.2 Activation Process

1. **Launch server**: Start the MCP server process in the sandbox.
2. **Establish connection**: Connect via stdio or SSE.
3. **Handshake**: Complete MCP protocol handshake.
4. **Register tools**: Add server tools to the tool registry.
5. **Start monitoring**: Begin activity monitoring.
6. **Log activation**: Record activation in session log.

### 8.3 Activation State

```rust
pub struct McpServerActive {
    pub server_name: String,
    pub connection: McpConnection,
    pub registered_tools: Vec<String>,
    pub activated_at: DateTime<Utc>,
    pub invocation_count: u64,
    pub error_count: u64,
    pub last_error: Option<String>,
}
```

### 8.4 Exit Criteria

- [ ] Server is connected and responsive
- [ ] All approved tools are registered
- [ ] Tool execution routing is established
- [ ] Monitoring is active

---

## 9. Stage 7: Monitoring

### 9.1 Purpose

Continuously monitor active MCP servers for security and performance.

### 9.2 Monitored Metrics

| Metric | Threshold | Action |
|--------|-----------|--------|
| Invocation count | > 100/hour | Warning |
| Error rate | > 10% | Warning |
| Latency p95 | > 5s | Warning |
| Memory usage | > 256MB | Terminate |
| Unexpected network | Any | Terminate + Alert |
| Unexpected file writes | Any | Terminate + Alert |
| Timeout violations | > 3 | Terminate |

### 9.3 Monitoring Actions

| Condition | Action |
|-----------|--------|
| Warning threshold reached | Log warning, notify user |
| Error threshold reached | Log error, suggest review |
| Critical threshold reached | Terminate server, log incident |
| Unexpected behavior detected | Terminate server, alert user |

### 9.4 Monitoring Output

```rust
pub struct McpMonitoringReport {
    pub server_name: String,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub invocation_count: u64,
    pub error_count: u64,
    pub avg_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub peak_memory_mb: f64,
    pub incidents: Vec<McpIncident>,
}

pub struct McpIncident {
    pub timestamp: DateTime<Utc>,
    pub severity: IncidentSeverity,
    pub description: String,
    pub action_taken: String,
}

pub enum IncidentSeverity {
    Warning,
    Error,
    Critical,
}
```

---

## 10. Stage 8: Removal

### 10.1 Purpose

Gracefully remove an MCP server from the runtime.

### 10.2 Removal Triggers

| Trigger | Cause |
|---------|-------|
| User request | User explicitly removes server |
| Timeout | Server inactive for 24 hours |
| Error threshold | Error rate exceeds 50% |
| Security incident | Server detected as malicious |
| Configuration change | Server removed from config |

### 10.3 Removal Process

1. **Stop accepting new invocations**
2. **Wait for in-flight requests** (max 10 seconds)
3. **Close connection**
4. **Terminate process** (if stdio)
5. **Remove from tool registry**
6. **Log removal**
7. **Update persistence**

### 10.4 Removal Output

```rust
pub struct McpRemovalReport {
    pub server_name: String,
    pub removed_at: DateTime<Utc>,
    pub reason: RemovalReason,
    pub pending_invocations: u64,
    pub total_invocations: u64,
    pub total_errors: u64,
}

pub enum RemovalReason {
    UserRequest,
    Timeout,
    ErrorThreshold,
    SecurityIncident,
    ConfigChange,
}
```

---

## 11. State Machine

```
                        ┌─────────────┐
                        │  DISCOVERED  │
                        └──────┬──────┘
                               │
                    ┌──────────┼──────────┐
                    ▼                     ▼
          ┌─────────────────┐    ┌─────────────────┐
          │     SANDBOXED    │    │   REMOVED       │
          │  (testing only) │    │  (by user)      │
          └────────┬────────┘    └─────────────────┘
                   │
           ┌───────┼───────┐
           ▼               ▼
  ┌─────────────────┐ ┌─────────────────┐
  │ PERMISSIONS     │ │  VALIDATION     │
  │  REVIEWED       │ │   FAILED        │
  └────────┬────────┘ └─────────────────┘
           │
           ▼
  ┌─────────────────┐
  │   APPROVED      │
  │  (user consent) │
  └────────┬────────┘
           │
           ▼
  ┌─────────────────┐
  │   ACTIVE        │
  │  (monitored)    │
  └────────┬────────┘
           │
    ┌──────┼──────┐
    ▼              ▼
┌─────────┐  ┌──────────┐
│ REMOVED │  │ MONITORED│
│(user)   │  │  (normal)│
└─────────┘  └──────────┘
           │
           ▼
      ┌─────────┐
      │ REMOVED │
      └─────────┘
```

---

## 12. Data Persistence

### 12.1 Files

| File | Purpose |
|------|---------|
| `~/.codebro/mcp_discovery.json` | Discovery results |
| `~/.codebro/mcp_sandbox_reports/` | Sandbox validation reports |
| `~/.codebro/mcp_pending.json` | Pending approval servers |
| `~/.codebro/mcp_active.json` | Currently active servers |
| `~/.codebro/mcp_history.json` | Historical server lifecycle events |

### 12.2 Format

All files use JSON with schema version 1. Each file includes a `format_version` field.

---

## 13. Security Constraints

### 13.1sandbox Isolation

- MCP servers **cannot** access the host filesystem outside the sandbox.
- MCP servers **cannot** make outbound network connections outside allowed URLs.
- MCP servers **cannot** spawn child processes.
- MCP servers **cannot** modify CodeBro configuration.
- MCP servers **cannot** access environment variables outside the whitelist.

### 13.2 Tool Execution Boundaries

- Tool execution goes through the standard tool pipeline.
- All tool calls are logged with arguments (secrets redacted).
- Tool output is subject to the 32KB limit.
- Tool execution is subject to the approval gate.

### 13.3 Network Security

- SSE connections must use HTTPS.
- Stdio connections are isolated per-process.
- Connection strings are not logged.

---

## 14. Error Recovery

### 14.1 Server Crash

If an active server crashes:
1. Monitoring detects the crash.
2. Server is marked as `terminated`.
3. User is notified.
4. Server is removed from the active list.
5. If the crash was unexpected, a security incident is logged.

### 14.2 Connection Loss

If connection is lost:
1. Server is marked as `disconnected`.
2. Reconnection attempts: 3 (with exponential backoff).
3. If all attempts fail, server is removed and user is notified.

---

## 15. References

- [ADR-009: Configuration Versioning](../ADR/adr-009-configuration-versioning.md)
- [Tool Contract](../contracts/tool_contract.md)
- [MCP Specification](https://modelcontextprotocol.io)

---

## 16. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
