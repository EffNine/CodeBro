# Architecture Report — P5 Developer Experience Platform

## Overview

Phase P5 introduces the Developer Experience Platform as a new architectural layer that sits between the user interface (TUI) and the existing core system. This report documents the architectural decisions, module boundaries, and integration points.

---

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                        TUI Layer                                 │
│  ┌───────────┐  ┌──────────────┐  ┌─────────────────────────┐  │
│  │  App      │  │  Dashboard   │  │  Events / Shortcuts     │  │
│  │  (P5      │  │  (unchanged  │  │  (Ctrl+P, slash cmds)   │  │
│  │   fields) │  │    + P5)     │  │                         │  │
│  └─────┬─────┘  └──────┬───────┘  └──────────┬──────────────┘  │
│        │               │                      │                │
└────────┼───────────────┼──────────────────────┼────────────────┘
         │               │                      │
┌────────▼───────────────▼──────────────────────▼────────────────┐
│                    P5 Platform Layer                            │
│                                                                 │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │  Settings    │  │  Provider    │  │  Workspace           │  │
│  │  Manager     │  │  Manager     │  │  Discovery           │  │
│  │              │  │              │  │                      │  │
│  │  - 14        │  │  - 5 built-   │  │  - 14 discovery      │
│  │    settings  │  │    in providers│  │    kinds             │
│  │  - 5         │  │  - Health     │  │  - Integration       │
│  │    sections  │  │    checking   │  │    proposals         │
│  │  - Pending   │  │  - Model      │  │  - MCP discovery     │
│  │    changes   │  │    picker     │  │                      │  │
│  └──────┬───────┘  └──────┬───────┘  └──────────┬───────────┘  │
│         │                 │                      │              │
│  ┌──────▼───────┐  ┌──────▼───────┐  ┌──────────▼───────────┐  │
│  │  Capability  │  │  Onboarding  │  │  Configuration       │  │
│  │  Discovery   │  │  Manager     │  │  Model               │  │
│  │              │  │              │  │                      │  │
│  │  - Tool      │  │  - 9-step    │  │  - Stable internal   │
│  │    detection │  │    wizard    │  │    Config struct     │  │
│  │  - Runtime   │  │  - CLI &     │  │  - TOML persistence  │  │
│  │    detection │  │    TUI modes │  │  - Versioned format  │  │
│  │  -           │  │  - First-run │  │                      │  │
│  │    recommend │  │    detection │  │  Re-exported via     │  │
│  │   ations     │  │              │  │  config::Config      │  │
│  └──────────────┘  └──────────────┘  └──────────────────────┘  │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
         │                                      │
         ▼                                      ▼
┌─────────────────────┐            ┌────────────────────────────┐
│  Existing Core      │            │  External Systems          │
│  (P0-P4.5)          │            │                            │
│  - Agent            │            │  - API providers           │
│  - Tools            │            │  - Git / Cargo / npm       │
│  - Intelligence     │            │  - Filesystem              │
│  - Reliability      │            │  - Terminal (crossterm)    │
└─────────────────────┘            └────────────────────────────┘
```

---

## Module Dependencies

```
main.rs
  ├── settings        (no internal deps)
  ├── provider_manager (depends on: providers, anyhow, serde)
  ├── workspace_discovery (depends on: serde, chrono)
  ├── capability_discovery (depends on: serde, std)
  ├── onboarding      (depends on: provider_manager, workspace_discovery, capability_discovery)
  └── cli             (depends on: onboarding, provider_manager)
```

**Key design decision**: P5 modules have **no dependencies on the existing agent/runtime code**. This ensures the platform layer can be tested and evolved independently.

---

## Module Specifications

### SettingsManager

```rust
pub struct SettingsManager {
    settings: Vec<Setting>,
    config: Config,
    config_dir: PathBuf,
    pending_changes: Vec<usize>,
}

// Public API:
impl SettingsManager {
    pub fn new(config: Config, config_dir: PathBuf) -> Self;
    pub fn get_setting(&self, key: &str) -> Option<&Setting>;
    pub fn set_string(&mut self, key: &str, value: &str) -> Result<()>;
    pub fn set_integer(&mut self, key: &str, value: i64) -> Result<()>;
    pub fn set_boolean(&mut self, key: &str, value: bool) -> Result<()>;
    pub fn apply_changes(&mut self) -> Result<()>;
    pub fn discard_changes(&mut self);
    pub fn summary(&self) -> String;
}
```

### ProviderManager

```rust
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct ProviderManager {
    providers: HashMap<String, ProviderEntry>,
    active_provider: Option<String>,
    config_dir: PathBuf,
}

// Public API:
impl ProviderManager {
    pub fn new(config_dir: PathBuf) -> Self;
    pub fn register_builtin(&mut self);
    pub fn set_active(&mut self, provider_id: &str) -> Result<()>;
    pub fn set_api_key(&mut self, provider_id: &str, key: &str) -> Result<()>;
    pub fn api_key_masked(&self, provider_id: &str) -> Option<String>;
    pub async fn check_health(&mut self, provider_id: &str) -> Result<HealthStatus>;
    pub async fn check_all_health(&mut self) -> Vec<(String, HealthStatus, Option<u64>)>;
    pub fn list_providers(&self) -> Vec<(&str, &ProviderEntry)>;
    pub fn persist(&self) -> Result<()>;
    pub fn load(&mut self) -> Result<()>;
}
```

### WorkspaceDiscovery

```rust
pub struct DiscoveryEngine {
    root: PathBuf,
}

pub struct WorkspaceDiscovery {
    pub root: PathBuf,
    pub findings: Vec<DiscoveryFinding>,
    pub proposals: Vec<IntegrationProposal>,
    pub language: String,
    pub framework: Option<String>,
    pub build_system: Option<String>,
    pub package_manager: Option<String>,
    pub testing_framework: Option<String>,
}

// Public API:
impl DiscoveryEngine {
    pub fn new(root: PathBuf) -> Self;
    pub fn discover(&self) -> WorkspaceDiscovery;
}

pub fn discover_mcp_servers(root: &Path) -> Vec<McpServerInfo>;
```

### CapabilityDiscovery

```rust
pub struct CapabilityScanner {
    workspace_root: PathBuf,
}

pub struct CapabilityDiscovery {
    pub capabilities: Vec<Capability>,
    pub recommendations: Vec<String>,
    pub workspace_root: PathBuf,
}

// Public API:
impl CapabilityScanner {
    pub fn new(workspace_root: PathBuf) -> Self;
    pub fn scan(&self) -> CapabilityDiscovery;
}
```

### OnboardingManager

```rust
pub struct OnboardingManager {
    pub config_dir: PathBuf,
    pub session: OnboardingSession,
}

// Public API:
impl OnboardingManager {
    pub fn new(config_dir: PathBuf) -> Self;
    pub fn check_first_run(&self) -> bool;
    pub fn start(&mut self);
    pub fn complete(&mut self, workspace_root: &PathBuf) -> Result<OnboardingResult>;
    pub fn step_info(&self) -> (&'static str, &'static str);
}
```

---

## Configuration Model

The stable internal configuration model (see `docs/vision/CONFIGURATION_MODEL.md`) defines:

```
Config
├── provider: ProviderConfig
│   ├── provider_id: String
│   ├── base_url: String
│   ├── model: String
│   ├── api_key_source: ApiKeySource
│   └── health: ProviderHealth
├── workspace: WorkspaceConfig
│   ├── root: PathBuf
│   ├── language: String
│   ├── framework: Option<String>
│   ├── build_system: Option<String>
│   ├── package_manager: Option<String>
│   ├── testing_framework: Option<String>
│   ├── integrations: Vec<Integration>
│   └── mcp_servers: Vec<McpServerDiscovery>
├── features: FeatureFlags
│   ├── show_coordination: bool
│   ├── show_task_graph: bool
│   ├── show_metrics: bool
│   ├── show_memory_notifications: bool
│   ├── show_skill_notifications: bool
│   ├── auto_approve_safe: bool
│   ├── skill_auto_apply_threshold: f32
│   ├── context_token_budget: usize
│   └── max_tool_iterations: u32
└── metadata: ConfigMetadata
    ├── format_version: u32
    ├── last_modified: DateTime<Utc>
    ├── codebro_version: String
    ├── onboarding_complete: bool
    └── onboarding_completed_at: Option<DateTime<Utc>>
```

---

## TUI Integration Points

The P5 platform integrates into the existing TUI through:

1. **TuiApp fields** (app.rs):
   - `settings: Option<SettingsManager>`
   - `provider_manager: Option<ProviderManager>`
   - `settings_panel: SettingsPanel`
   - `provider_panel: ProviderPanel`
   - `workspace_panel: WorkspacePanel`

2. **AppEvent extensions** (events.rs):
   - `AppEvent::ProviderHealthResults(...)`
   - `AppEvent::WorkspaceDiscovered {...}`

3. **Slash command extensions** (ui.rs):
   - `/settings`, `/settings:apply`, `/settings:discard`
   - `/providers`, `/health`
   - `/discover`, `/workspace`, `/onboard`

4. **CLI extensions** (cli/mod.rs):
   - `Commands::Onboard` subcommand
   - `run_onboarding_wizard()` async function
   - First-run detection in `run()`

---

## Design Decisions

### D1: No Direct Agent Dependencies
P5 modules do not import from `agent::` or `runtime::`. This ensures:
- Independent testability
- Clear layer boundaries
- P6 can add adaptive intelligence without P5 changes

### D2: ProviderManager is Serializable
The `ProviderManager` derives `Serialize` and `Deserialize` for JSON persistence. This allows:
- Configuration backup/restore
- Multi-device sync (future)
- Testing with serialized fixtures

### D3: Workspace Discovery is Read-Only in P5
MCP server detection returns available servers but does not auto-install them. This:
- Respects the "No Hidden Automation" principle
- Prepares for P6's auto-install capability
- Keeps P5 scope focused on discovery

### D4: Settings Use Pending Changes Pattern
Settings changes are staged before applying. This:
- Allows users to review before committing
- Supports `/settings:discard` for cancellation
- Prevents accidental configuration changes

### D5: Onboarding Has CLI and TUI Paths
The onboarding wizard works both as a CLI command (`codebro onboard`) and can be triggered from the TUI (`/onboard`). This:
- Supports headless deployment
- Allows re-onboarding at any time
- Keeps the TUI clean (wizard runs in CLI)

---

## File Layout

```
codebro/
├── src/
│   ├── settings/
│   │   └── mod.rs          # SettingsManager, SettingsPanel
│   ├── provider_manager/
│   │   └── mod.rs          # ProviderManager, ProviderId, WizardState
│   ├── workspace_discovery/
│   │   └── mod.rs          # DiscoveryEngine, WorkspaceDiscovery
│   ├── capability_discovery/
│   │   └── mod.rs          # CapabilityScanner, CapabilityDiscovery
│   ├── onboarding/
│   │   └── mod.rs          # OnboardingManager, OnboardingSession
│   ├── cli/
│   │   └── mod.rs          # Extended with Onboard command
│   └── tui/
│       ├── app.rs          # Extended with P5 fields
│       ├── events.rs       # Extended with P5 events
│       └── ui.rs           # Extended with P5 commands
├── docs/
│   └── vision/
│       ├── CODEBRO_VISION.md
│       ├── DX_PRINCIPLES.md
│       ├── FIRST_RUN_EXPERIENCE.md
│       └── CONFIGURATION_MODEL.md
└── docs/reports/p5/
    ├── DeveloperExperienceReport-P5.md
    ├── ArchitectureReport-P5.md
    ├── ValidationReport-P5.md
    ├── BenchmarkReport-P5.md
    ├── RegressionReport-P5.md
    └── FutureCompatibilityReport-P5.md
```
