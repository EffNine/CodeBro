# MCP Lifecycle — P6 Design Specification

**Document:** `docs/design/MCP_LIFECYCLE.md`
**Version:** 1.0.0
**Phase:** P6 — Adaptive Intelligence
**Status:** Proposed — Design Summit
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Purpose

The MCP (Model Context Protocol) Lifecycle manages the discovery, recommendation, installation, validation, updates, and removal of MCP servers. Every step requires explicit user approval.

---

## 2. Architecture

```
┌───────────────────────────────────────────────────────────────┐
│                    MCP Lifecycle Manager                      │
│                                                               │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐   │
│  │  Discovery   │  │  Recommendation│  │  Installation    │   │
│  │  Engine      │  │  Engine       │  │  Manager         │   │
│  └──────┬───────┘  └──────┬───────┘  └────────┬─────────┘   │
│         │                 │                    │             │
│  ┌──────▼───────┐  ┌──────▼───────┐  ┌────────▼─────────┐   │
│  │  Validation  │  │  Update      │  │  Removal         │   │
│  │  Engine      │  │  Manager     │  │  Manager         │   │
│  └──────────────┘  └──────────────┘  └──────────────────┘   │
│                                                               │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │              MCP Registry (persistent)                  │ │
│  │  ~/.codebro/adaptive/mcp_registry.json                  │ │
│  └─────────────────────────────────────────────────────────┘ │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
                        Approval Gate
```

---

## 3. Lifecycle Stages

### 3.1 Stage 1: Discovery

MCP servers are discovered through:

| Source | Method | Frequency |
|--------|--------|-----------|
| Local filesystem | Scan `~/.codebro/mcp-servers/` and project `.codebro/mcp-servers/` | On startup |
| Registry | Check `~/.codebro/mcp-registry.json` for known servers | On startup + periodic |
| Project config | Read `mcp_servers` from workspace config | On workspace change |

```rust
pub struct McpDiscoveryResult {
    pub servers: Vec<DiscoveredMcpServer>,
    pub discovered_at: String,
}

pub struct DiscoveredMcpServer {
    pub name: String,
    pub transport: McpTransport,
    pub description: String,
    pub available_tools: Vec<String>,
    pub source: McpDiscoverySource,
    pub is_trusted: bool,
}

pub enum McpTransport {
    Stdio { command: String, args: Vec<String> },
    Sse { url: String },
}

pub enum McpDiscoverySource {
    LocalFile,
    Registry,
    ProjectConfig,
}
```

### 3.2 Stage 2: Recommendation

Discovered servers are evaluated and recommended:

```rust
pub struct McpRecommendation {
    pub id: String,
    pub server: DiscoveredMcpServer,
    pub confidence: f32,
    pub reasoning: String,
    pub evidence: Vec<String>,
    pub cost_impact: Option<CostImpact>,
    pub security_assessment: SecurityAssessment,
    pub required_approval: bool,
}
```

#### Recommendation Criteria

| Criterion | Weight | Description |
|-----------|--------|-------------|
| Tool relevance | 0.3 | How many tools match current project needs |
| Trust score | 0.2 | Server's trust rating (known authors = higher) |
| Community adoption | 0.1 | Number of users (from registry) |
| Security rating | 0.2 | No suspicious commands, no network access unless expected |
| Cost impact | 0.2 | Estimated additional cost |

### 3.3 Stage 3: Installation

Installation requires explicit approval:

```
User approves recommendation
        ↓
Validate server (run in sandbox)
        ↓
Download/clone server code (if needed)
        ↓
Install to ~/.codebro/mcp-servers/<name>/
        ↓
Register in mcp_registry.json
        ↓
Enable in current session
```

#### Installation Validation

Before installation, the server is validated:

```rust
pub struct McpValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub sandbox_test_result: Option<SandboxTestResult>,
}

pub struct SandboxTestResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub tools_available: Vec<String>,
}
```

### 3.4 Stage 4: Validation

Post-installation validation ensures the server works correctly:

```rust
pub struct McpValidationCheck {
    pub server_name: String,
    pub connected: bool,
    pub tools_count: usize,
    pub health_check_passed: bool,
    pub last_error: Option<String>,
    pub validated_at: String,
}
```

### 3.5 Stage 5: Updates

Updates are checked periodically and recommended:

```rust
pub struct McpUpdateInfo {
    pub server_name: String,
    pub current_version: String,
    pub available_version: String,
    pub changelog: String,
    pub breaking_changes: Vec<String>,
    pub recommended: bool,
}
```

Update flow:
1. Check registry for newer versions
2. If update found → Generate `McpRecommendation` with update details
3. User approves → Download and install update
4. Re-validate after update

### 3.6 Stage 6: Removal

Removal requires explicit approval:

```
User requests removal (or recommends via TUI)
        ↓
Check if any active sessions use this server
        ↓
If active sessions exist → Warn user
        ↓
User confirms removal
        ↓
Disable server
        ↓
Remove from ~/.codebro/mcp-servers/<name>/
        ↓
Remove from mcp_registry.json
```

---

## 4. MCP Registry

The registry persists MCP state:

```json
{
  "version": 1,
  "servers": [
    {
      "name": "filesystem",
      "transport": { "type": "stdio", "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem"] },
      "description": "Filesystem operations",
      "status": "Installed",
      "version": "1.0.0",
      "installed_at": "2026-08-06T00:00:00Z",
      "last_validated_at": "2026-08-06T00:00:00Z",
      "tools": ["read_file", "write_file", "list_directory"],
      "trusted": true,
      "auto_enabled": false
    }
  ],
  "discovery_history": []
}
```

---

## 5. Trait Contract

```rust
pub trait McPLifecycleTrait: Send + Sync {
    /// Discover available MCP servers
    fn discover(&self) -> McpDiscoveryResult;

    /// Get recommendations for discovered servers
    fn get_recommendations(&self) -> Vec<&McpRecommendation>;

    /// Install an MCP server (requires approval)
    fn install(&mut self, recommendation_id: &str) -> Result<McpInstallationResult>;

    /// Validate an installed MCP server
    fn validate(&self, server_name: &str) -> McpValidationResult;

    /// Check for available updates
    fn check_updates(&self) -> Vec<McpUpdateInfo>;

    /// Update an MCP server (requires approval)
    fn update(&mut self, server_name: &str) -> Result<McpUpdateResult>;

    /// Remove an MCP server (requires approval)
    fn remove(&mut self, server_name: &str) -> Result<()>;

    /// Get all installed servers
    fn get_installed(&self) -> Vec<&McpServerRecord>;

    /// Enable/disable a server
    fn set_enabled(&mut self, server_name: &str, enabled: bool) -> Result<()>;
}

pub struct McpInstallationResult {
    pub success: bool,
    pub server_name: String,
    pub validation_result: McpValidationResult,
    pub tools_added: Vec<String>,
}

pub struct McpUpdateResult {
    pub success: bool,
    pub server_name: String,
    pub old_version: String,
    pub new_version: String,
    pub validation_result: McpValidationResult,
}

pub struct McpServerRecord {
    pub name: String,
    pub transport: McpTransport,
    pub description: String,
    pub status: McpServerStatus,
    pub version: String,
    pub installed_at: String,
    pub last_validated_at: Option<String>,
    pub tools: Vec<String>,
    pub trusted: bool,
    pub enabled: bool,
}

pub enum McpServerStatus {
    Discovered,
    Recommended,
    Installed,
    Validated,
    Error(String),
}
```

---

## 6. Security Model

### 6.1 Trust Levels

| Trust Level | Criteria | Restrictions |
|-------------|----------|--------------|
| **Trusted** | From known registry, verified author | Full access |
| **Unverified** | Discovered locally, not in registry | Sandbox validation required |
| **Blocked** | Known malicious pattern | Cannot be installed |

### 6.2 Sandbox Validation

Unverified servers run in a sandbox before installation:

```rust
pub struct SandboxConfig {
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub allowed_network: bool,
    pub allowed_file_access: Vec<PathBuf>,
    pub environment_vars: HashMap<String, String>,
}
```

### 6.3 Forbidden Patterns

Servers matching these patterns are automatically blocked:

- Commands that download and execute arbitrary code
- Servers that access sensitive paths (`~/.ssh`, `~/.gnupg`, `/etc/shadow`)
- Servers that make outbound network calls without explicit configuration
- Servers with obfuscated command strings

---

## 7. TUI Integration

### 7.1 View: `/mcp`

```
┌─────────────────────────────────────────────┐
│  MCP SERVERS                                │
├─────────────────────────────────────────────┤
│  Installed (1)                              │
│  ─────────────────────────────────          │
│  ✓ filesystem     v1.0.0  Trusted         │
│                                             │
│  Discoveries (2)                            │
│  ─────────────────────────────────          │
│  ? github-api     v2.1.0  Unverified       │
│    Tools: get_issue, create_pr, list_repos  │
│    [Install] [Dismiss]                      │
│                                             │
│  ? database     v1.0.0  Blocked            │
│    Reason: Accesses /etc/shadow             │
│                                             │
│  [Scan Again]  [Close]                      │
└─────────────────────────────────────────────┘
```

---

## 8. Anti-Patterns

```rust
// NEVER: Auto-install an MCP server without user approval
// ALWAYS: Present as a recommendation first

// NEVER: Allow unverified servers full filesystem access
// ALWAYS: Sandbox validation before installation

// NEVER: Silently update MCP servers
// ALWAYS: Notify user of updates and require approval
```

---

## 9. References

- [ADAPTIVE_PLATFORM_SPEC.md](./ADAPTIVE_PLATFORM_SPEC.md)
- [COST_POLICY.md](./COST_POLICY.md)
- [TRUST_MODEL.md](./TRUST_MODEL.md)

---

## 10. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
