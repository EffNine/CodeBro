# User Preference Model — P6 Design Specification

**Document:** `docs/design/USER_PREFERENCE_MODEL.md`
**Version:** 1.0.0
**Phase:** P6 — Adaptive Intelligence
**Status:** Proposed — Design Summit
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Purpose

The Preference Engine is the foundational data store for all adaptive behavior. It holds the developer's explicit and implicitly-approved preferences. It is **deterministic** — no LLM is used to read, write, or interpret preferences.

The Preference Engine answers: *What does this developer prefer?*

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Preference Engine                        │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │                  PreferenceStore                     │   │
│  │                                                      │   │
│  │  ┌─────────────┐  ┌─────────────┐  ┌────────────┐  │   │
│  │  │ Coding      │  │ Cost        │  │ Provider   │  │   │
│  │  │ Preferences │  │ Preferences │  │ Preferences│  │   │
│  │  └─────────────┘  └─────────────┘  └────────────┘  │   │
│  │  ┌─────────────┐  ┌─────────────┐  ┌────────────┐  │   │
│  │  │ Language    │  │ Workflow    │  │ Profile    │  │   │
│  │  │ Preferences │  │ Preferences │  │ Defaults   │  │   │
│  │  └─────────────┘  └─────────────┘  └────────────┘  │   │
│  │                                                      │   │
│  │  [Validation Layer] — all writes are validated      │   │
│  │  [Audit Layer]      — all writes are logged         │   │
│  └─────────────────────────────────────────────────────┘   │
│                             │                             │
│          ┌──────────────────┼──────────────────┐          │
│          ▼                  ▼                  ▼          │
│    Intent Engine       Recommendation    Cost Policy       │
│    (reads/writes)      Engine (reads)     (reads)          │
└─────────────────────────────────────────────────────────────┘
```

---

## 3. Preference Schema

### 3.1 Top-Level Structure

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preferences {
    pub version: u32,
    pub last_updated: String,
    pub coding: CodingPreferences,
    pub cost: CostPreferences,
    pub provider: ProviderPreferences,
    pub language: LanguagePreferences,
    pub workflow: WorkflowPreferences,
    pub overrides: OverrideMap,
    pub audit: AuditLog,
}
```

### 3.2 Coding Preferences

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodingPreferences {
    /// Default style: "idiomatic", "explicit", "minimal", "defensive"
    pub style: Option<String>,

    /// Whether to add comments to generated code
    pub add_comments: Option<bool>,

    /// Whether to prefer functional or imperative style
    pub paradigm_preference: Option<String>,

    /// Maximum line length for generated code
    pub max_line_length: Option<usize>,

    /// Whether to include tests with generated code
    pub include_tests: Option<bool>,

    /// Whether to include documentation
    pub include_docs: Option<bool>,

    /// Preferred error handling style
    pub error_handling: Option<String>,
}
```

### 3.3 Cost Preferences

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostPreferences {
    /// Daily spending limit in USD
    pub daily_limit_usd: Option<f64>,

    /// Per-session spending limit in USD
    pub session_limit_usd: Option<f64>,

    /// Whether to warn before exceeding 80% of limit
    pub warning_threshold: Option<f32>,

    /// Whether to block when limit is reached
    pub hard_limit_enforcement: Option<bool>,

    /// Preferred cost tier: "minimal", "balanced", "quality"
    pub preferred_tier: Option<String>,

    /// Maximum cost per single task in USD
    pub max_per_task_usd: Option<f64>,
}
```

### 3.4 Provider Preferences

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPreferences {
    /// Default provider for all tasks
    pub default_provider: Option<String>,

    /// Per-role model overrides
    pub role_overrides: HashMap<String, String>,

    /// Whether to prefer local models when available
    pub prefer_local: Option<bool>,

    /// Minimum quality threshold before falling back to cloud
    pub min_quality_threshold: Option<f32>,
}
```

### 3.5 Language Preferences

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguagePreferences {
    /// Primary working language
    pub primary_language: Option<String>,

    /// Additional languages
    pub additional_languages: Vec<String>,

    /// Framework preferences per language
    pub framework_preferences: HashMap<String, String>,

    /// Whether to detect language from project automatically
    pub auto_detect_language: Option<bool>,
}
```

### 3.6 Workflow Preferences

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowPreferences {
    /// Whether to ask before installing any tool/package
    pub ask_before_install: Option<bool>,

    /// Whether to ask before running shell commands
    pub ask_before_command: Option<bool>,

    /// Whether to show diffs before file edits
    pub show_diff_before_edit: Option<bool>,

    /// Whether to run tests before committing
    pub run_tests_before_commit: Option<bool>,

    /// Preferred review style: "strict", "light", "safety-focused"
    pub review_style: Option<String>,

    /// Whether to suggest workflow automations
    pub suggest_workflows: Option<bool>,
}
```

### 3.7 Override Map

```rust
/// Per-project or per-workspace overrides
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OverrideMap {
    pub project_overrides: HashMap<PathBuf, Preferences>,
    pub session_overrides: HashMap<String, Preferences>,
}
```

### 3.8 Audit Log

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuditLog {
    pub entries: Vec<AuditEntry>,
    pub max_entries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp: String,
    pub key: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub source: ChangeSource,
    pub approved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChangeSource {
    Manual,           // User changed via TUI
    Intent,           // Intent Engine parsed from natural language
    Recommendation,   // User approved a recommendation
    ProfileSwitch,    // User switched profile
}
```

---

## 4. Trait Contract

```rust
pub trait PreferenceEngineTrait: Send + Sync {
    /// Read a preference value by key
    fn get(&self, key: &str) -> Option<String>;

    /// Read all preferences in a category
    fn get_category(&self, category: &str) -> Option<HashMap<String, String>>;

    /// Write a preference (requires approval if source is not Manual)
    fn set(&mut self, key: &str, value: &str, source: ChangeSource) -> Result<PreferenceChange>;

    /// Get all preferences as a flat map
    fn get_all(&self) -> HashMap<String, String>;

    /// Check if a preference has changed since last save
    fn has_changed(&self, key: &str, new_value: &str) -> bool;

    /// Save preferences to disk
    fn save(&self) -> Result<()>;

    /// Load preferences from disk
    fn load(&mut self) -> Result<()>;

    /// Get the audit log
    fn get_audit_log(&self) -> &[AuditEntry];

    /// Get preference change history for a key
    fn get_change_history(&self, key: &str) -> Vec<&AuditEntry>;

    /// Validate a proposed preference value
    fn validate(&self, key: &str, value: &str) -> Result<(), ValidationError>;

    /// Get effective preferences for a given workspace and session
    fn get_effective(&self, workspace: Option<&Path>, session: Option<&str>) -> Preferences;
}
```

---

## 5. Key Resolution Order

When resolving a preference, the engine uses this priority:

1. **Session override** — highest priority, specific to current session
2. **Project override** — specific to current workspace
3. **Explicit preference** — user-set value in global preferences
4. **Profile default** — value from active profile
5. **System default** — built-in default

```
get_effective(workspace, session) →
  merge(session_overrides, project_overrides, global_preferences, profile_defaults, system_defaults)
```

---

## 6. Validation Rules

All preference writes pass through validation:

| Key | Validation Rule |
|-----|----------------|
| `cost.daily_limit_usd` | Must be positive, must not exceed $1000 |
| `cost.session_limit_usd` | Must be positive, must not exceed daily limit |
| `cost.warning_threshold` | Must be between 0.0 and 1.0 |
| `coding.style` | Must be one of: idiomatic, explicit, minimal, defensive |
| `coding.max_line_length` | Must be between 70 and 200 |
| `provider.prefer_local` | Boolean only |
| `workflow.ask_before_install` | Boolean only |
| `language.primary_language` | Must be a supported language |

Invalid values are rejected with a descriptive error; no partial writes occur.

---

## 7. Persistence

### 7.1 Storage Location

```
~/.codebro/adaptive/preferences.json
```

### 7.2 Format

```json
{
  "version": 1,
  "last_updated": "2026-08-06T00:00:00Z",
  "coding": {
    "style": "idiomatic",
    "include_tests": true,
    "add_comments": false
  },
  "cost": {
    "daily_limit_usd": 5.0,
    "preferred_tier": "balanced"
  },
  "provider": {
    "default_provider": "openai",
    "prefer_local": false
  },
  "language": {
    "primary_language": "rust",
    "auto_detect_language": true
  },
  "workflow": {
    "ask_before_install": true,
    "show_diff_before_edit": true
  },
  "overrides": {
    "project_overrides": {},
    "session_overrides": {}
  },
  "audit": {
    "entries": [],
    "max_entries": 1000
  }
}
```

### 7.3 Write Semantics

- Writes are atomic: content is written to a temp file, then renamed
- On write failure, the original file is preserved
- A backup (`preferences.json.bak`) is kept before each write

---

## 8. TUI Integration

### 8.1 View: `/preferences`

Displays all preferences in a tabular format with columns:

| Column | Content |
|--------|---------|
| Category | coding, cost, provider, language, workflow |
| Key | Preference key |
| Value | Current value |
| Source | Manual, Intent, Recommendation, Profile |
| Editable | Yes/No indicator |

### 8.2 Interactions

- Arrow keys navigate rows
- `Enter` opens edit mode for the selected preference
- `Escape` cancels editing
- `A` toggles the "show all sources" filter
- `R` shows the audit log for the selected preference

### 8.3 Change Notification

When a preference changes, the TUI displays a brief notification:

```
[Preference] cost.daily_limit_usd: 3.0 → 5.0 (approved by user)
```

---

## 9. Anti-Patterns

```rust
// NEVER: Read preferences directly from config.toml
// ALWAYS: Use the PreferenceEngineTrait

// NEVER: Bypass validation on write
// ALWAYS: Pass through validate() before set()

// NEVER: Store secrets in preferences
// ALWAYS: API keys remain in environment/secure storage
```

---

## 10. References

- [ADAPTIVE_PLATFORM_SPEC.md](./ADAPTIVE_PLATFORM_SPEC.md)
- [Architecture Manifest v1.0](../architecture/architecture_manifest_v1.md)
- [Configuration Model](../vision/CONFIGURATION_MODEL.md)

---

## 11. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
