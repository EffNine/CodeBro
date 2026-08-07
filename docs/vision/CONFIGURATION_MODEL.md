# Configuration Model

## Overview

This document defines the stable internal configuration model for CodeBro. The TUI is the primary interface; configuration files are implementation details.

---

## Internal Configuration Model

All configuration is represented by a single `Config` struct internally:

```rust
pub struct Config {
    // Provider settings
    pub provider: ProviderConfig,
    
    // Workspace settings
    pub workspace: WorkspaceConfig,
    
    // Feature flags
    pub features: FeatureFlags,
    
    // Metadata
    pub metadata: ConfigMetadata,
}
```

### ProviderConfig

```rust
pub struct ProviderConfig {
    /// Provider identifier (openai, openrouter, deepseek, ollama, lmstudio)
    pub provider_id: String,
    
    /// Base URL for the provider API
    pub base_url: String,
    
    /// Currently selected model
    pub model: String,
    
    /// API key (never persisted in plain text to config file)
    pub api_key_source: ApiKeySource,
    
    /// Provider health status
    pub health: ProviderHealth,
    
    /// Last health check timestamp
    pub last_health_check: Option<chrono::DateTime<chrono::Utc>>,
}

pub enum ApiKeySource {
    /// Key stored in secure keychain (platform-dependent)
    Keychain,
    /// Key provided via environment variable
    Environment,
    /// Key provided at runtime (not persisted)
    Runtime,
}

pub struct ProviderHealth {
    pub status: HealthStatus,
    pub last_error: Option<String>,
    pub latency_ms: Option<u64>,
}

pub enum HealthStatus {
    Healthy,
    Unhealthy(String),
    Unknown,
}
```

### WorkspaceConfig

```rust
pub struct WorkspaceConfig {
    /// Root path of the workspace
    pub root: PathBuf,
    
    /// Detected language
    pub language: String,
    
    /// Detected framework (if any)
    pub framework: Option<String>,
    
    /// Detected build system
    pub build_system: Option<String>,
    
    /// Detected package manager
    pub package_manager: Option<String>,
    
    /// Detected testing framework
    pub testing_framework: Option<String>,
    
    /// Enabled integrations
    pub integrations: Vec<Integration>,
    
    /// MCP servers (discovery only in P5)
    pub mcp_servers: Vec<McpServerDiscovery>,
}

pub struct Integration {
    pub name: String,
    pub enabled: bool,
    pub approved: bool,
    pub description: String,
}

pub struct McpServerDiscovery {
    pub name: String,
    pub transport: McpTransport,
    pub available: bool,
    pub approved: bool,
}

pub enum McpTransport {
    Stdio { command: String, args: Vec<String> },
    Sse { url: String },
}
```

### FeatureFlags

```rust
pub struct FeatureFlags {
    /// Enable agent coordination view
    pub show_coordination: bool,
    
    /// Enable task graph view
    pub show_task_graph: bool,
    
    /// Enable metrics panel
    pub show_metrics: bool,
    
    /// Enable memory notifications
    pub show_memory_notifications: bool,
    
    /// Enable skill notifications
    pub show_skill_notifications: bool,
    
    /// Auto-approve safe operations
    pub auto_approve_safe: bool,
    
    /// Minimum confidence for auto-applying skills
    pub skill_auto_apply_threshold: f32,
    
    /// Token budget for context building
    pub context_token_budget: usize,
    
    /// Max tool pipeline iterations
    pub max_tool_iterations: u32,
}
```

### ConfigMetadata

```rust
pub struct ConfigMetadata {
    /// Version of the config format
    pub format_version: u32,
    
    /// When the config was last modified
    pub last_modified: chrono::DateTime<chrono::Utc>,
    
    /// CodeBro version that created/last modified this config
    pub codebro_version: String,
    
    /// Whether onboarding is complete
    pub onboarding_complete: bool,
    
    /// Onboarding completion timestamp
    pub onboarding_completed_at: Option<chrono::DateTime<chrono::Utc>>,
}
```

---

## Persistence Format (TOML)

The internal model is persisted to `~/.codebro/config.toml`:

```toml
format_version = 1

[provider]
provider_id = "openai"
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
api_key_source = "environment"  # or "keychain"

[workspace]
root = "/path/to/project"
language = "rust"
framework = "axum"
build_system = "cargo"
package_manager = "cargo"
testing_framework = "cargo_test"

[integrations]
git = { enabled = true, approved = true, description = "Git status and diff tracking" }
cargo = { enabled = true, approved = true, description = "Cargo build and test runner" }

[features]
show_coordination = true
show_task_graph = false
show_metrics = true
show_memory_notifications = true
show_skill_notifications = true
auto_approve_safe = false
skill_auto_apply_threshold = 0.8
context_token_budget = 8000
max_tool_iterations = 5

[metadata]
last_modified = "2024-01-01T00:00:00Z"
codebro_version = "0.7.0"
onboarding_complete = true
onboarding_completed_at = "2024-01-01T00:00:00Z"
```

---

## Configuration Abstraction Layer

The `SettingsManager` module provides a stable internal API that abstracts over:

1. **Config loading** — from file, env vars, or defaults
2. **Config saving** — to file with atomic writes
3. **Config validation** — schema checks before persisting
4. **Config migration** — handling format version changes
5. **Config diffing** — showing what changed before applying

This means the TUI and all other modules interact with `Config` struct, never directly with TOML files.

---

## Stability Guarantees

1. **Internal model stability**: The `Config` struct fields are stable across P5 releases
2. **Format versioning**: TOML format has a version number; older formats are migrated
3. **Backward compatibility**: New fields are optional with sensible defaults
4. **Forward compatibility**: Unknown fields are preserved when round-tripping

---

## Migration Strategy

When the config format version changes:

1. `SettingsManager::load()` detects format version
2. If version < current, runs migration functions
3. Each migration is idempotent and reversible
4. Migration results are logged

Example migration (v1 → v2):
```rust
fn migrate_v1_to_v2(config: Config) -> Config {
    Config {
        features: FeatureFlags {
            show_coordination: config.features.show_coordination,
            show_task_graph: config.features.show_task_graph,
            show_metrics: true,  // newly enabled by default
            ..config.features
        },
        ..config
    }
}
```
