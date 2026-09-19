# ADR-009: Configuration Versioning

**ADR Number:** ADR-009
**Title:** Configuration Versioning
**Author:** CodeBro Engineering
**Status:** Proposed
**Created:** 2026-08-06
**Updated:** 2026-08-06
**Supersedes:** None
**Related RFC:** None

---

## 1. Context

### 1.1 Background

CodeBro's configuration system currently uses a flat TOML format with no versioning. The internal `Config` struct (defined in `src/config/mod.rs`) has evolved through P0–P5 with ad-hoc additions:

- P0–P3: `provider`, `base_url`, `model`, `api_key`
- P4: intelligence layer toggles added implicitly
- P5: `ProviderConfig`, `WorkspaceConfig`, `FeatureFlags`, `ConfigMetadata` added

The current `Config` struct in `src/config/mod.rs` does not include `format_version`, `last_modified`, or `codebro_version` fields — these exist only in the `CONFIGURATION_MODEL.md` vision document but are not yet implemented in the runtime.

As P6 introduces adaptive intelligence, configuration will grow significantly:

- Preference Engine settings (user preferences, learned patterns)
- Intent Engine settings (context windows, routing rules)
- Workflow Engine settings (automation rules, triggers)
- Recommendation Engine settings (scoring parameters)
- MCP Server configurations
- Plugin configurations

Without a formal versioning strategy, these additions will create silent incompatibilities, corrupted state, and unpredictable behavior.

### 1.2 Constraints

- Must respect the existing `~/.codebro/config.toml` persistence format.
- Must not break existing P0–P5 configurations on upgrade.
- Must support rollback to previous versions.
- Must handle corrupted configuration files gracefully.
- Must maintain backward compatibility within major versions.
- Must not require user intervention for minor version upgrades.

### 1.3 Stakeholders

- **Users**: Affected by configuration migrations and compatibility.
- **P6 Implementation**: Will add new configuration fields that must integrate cleanly.
- **Testing**: Must validate migrations across all supported versions.
- **Support**: Must handle corrupted configuration recovery.

---

## 2. Decision

### 2.1 Decision Statement

CodeBro adopts a **semantic configuration versioning system** with explicit schema version, migration pipeline, compatibility guarantees, and corruption recovery. The `Config` struct will include a `format_version` field, and all configuration reads will go through a versioned loader that applies migrations and validates schema before use.

### 2.2 Rationale

1. **Explicit versioning prevents silent corruption** — A missing or incorrect `format_version` triggers migration rather than undefined behavior.
2. **Idempotent migrations enable safe upgrades** — Each migration is reversible and produces the same result regardless of how many times it runs.
3. **Compatibility policy protects users** — Major version bumps require explicit user action; minor versions auto-migrate.
4. **Corruption recovery prevents hard failures** — A corrupted config falls back to defaults with a warning, never crashes the application.

### 2.3 Principles Applied

- **No Hidden Automation** — Migrations are logged and visible to the user.
- **Human Approval** — Major version migrations require user confirmation.
- **Developer First** — Minor version migrations are automatic and fast (< 10ms).
- **Observable AI Actions** — Migration results are recorded in the activity log.

---

## 3. Consequences

### 3.1 Positive Consequences

- Configuration schema is formally tracked and validated.
- New features can add configuration fields without breaking old configs.
- Rollback to previous configuration states is possible.
- Corrupted configurations are recoverable without data loss.
- Backward compatibility is guaranteed within major versions.

### 3.2 Negative Consequences

- Migration code must be maintained alongside feature code.
- Schema validation adds a small startup cost (~5ms).
- Deprecated fields generate warnings during migration.
- Migration tests add to the test surface.

### 3.3 Trade-offs

| Aspect | Trade-off | Mitigation |
|--------|-----------|------------|
| Migration complexity | More code to maintain | Each migration is a pure function; well-tested |
| Startup latency | Schema validation adds ~5ms | Validated to be under 10ms; negligible |
| Deprecated fields | Warnings clutter logs | Warnings suppressed after user acknowledgment |
| Rollback storage | Backup configs consume disk | Backups are compressed and rotated (max 5) |

### 3.4 Impact on Architecture

| Module | Impact |
|--------|--------|
| `config/mod.rs` | Add `format_version`, `last_modified`, `codebro_version` fields; add `load_with_migration()` and `validate()` methods |
| `settings/mod.rs` | Add migration registry; run migrations on load |
| `agent/recovery.rs` | Add corrupted config recovery path |
| `tui/ui.rs` | Show migration warnings in activity log |
| `provider_manager/mod.rs` | Migrate old provider format to new `ProviderConfig` |

### 3.5 Impact on Future Work

- P6 subsystems must declare which config fields they add and in which version.
- New ADRs for P6 subsystems must reference the configuration versioning policy.
- Migration tests must cover all supported version transitions.

---

## 4. Alternatives Considered

| Alternative | Description | Pros | Cons | Why Rejected |
|-------------|-------------|------|------|--------------|
| A: No versioning | Keep flat TOML, add fields silently | Simplest | Silent incompatibilities; no rollback; corruption risk | Unacceptable for adaptive system |
| B: JSON Schema + Validation | Use JSON Schema for validation | Standard; tooling support | Requires JSON instead of TOML; more complex migrations | Violates existing TOML convention |
| C: Versioned TOML (chosen) | `format_version` field + migration functions | Simple; backward compatible; explicit | Requires migration code | Best balance of simplicity and safety |
| D: Database-backed config | Store config in SQLite | Queryable; versioned | Overkill for user config; breaks simplicity | Violates P0–P5 design |

---

## 5. Implementation Notes

### 5.1 Config Struct Update

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConfigMetadata {
    pub format_version: u32,
    pub last_modified: chrono::DateTime<chrono::Utc>,
    pub codebro_version: String,
    pub onboarding_complete: bool,
    pub onboarding_completed_at: Option<chrono::DateTime<chrono::Utc>>,
}
```

### 5.2 Migration Registry Pattern

```rust
pub struct MigrationRegistry {
    migrations: Vec<Box<dyn Migration>>,
}

pub trait Migration: Send + Sync {
    fn from_version(&self) -> u32;
    fn to_version(&self) -> u32;
    fn migrate(&self, config: Config) -> Result<Config>;
    fn is_reversible(&self) -> bool;
}

impl MigrationRegistry {
    pub fn run(&self, config: Config, target_version: u32) -> Result<Config> {
        let mut current = config;
        let mut version = current.metadata.format_version;

        while version < target_version {
            let migration = self.migration_for(version)?;
            current = migration.migrate(current)?;
            current.metadata.format_version = version + 1;
            version += 1;
        }

        Ok(current)
    }
}
```

### 5.3 Schema Validation

```rust
impl Config {
    pub fn validate(&self) -> Result<(), ConfigValidationError> {
        // 1. Check format_version is supported
        if self.metadata.format_version > CURRENT_FORMAT_VERSION {
            return Err(ConfigValidationError::FutureVersion);
        }

        // 2. Check provider is valid
        if self.provider.provider_id.is_empty() {
            return Err(ConfigValidationError::EmptyProvider);
        }

        // 3. Check required fields
        if self.provider.base_url.is_empty() {
            return Err(ConfigValidationError::EmptyBaseUrl);
        }

        // 4. Check feature flag ranges
        if self.features.skill_auto_apply_threshold < 0.0
            || self.features.skill_auto_apply_threshold > 1.0
        {
            return Err(ConfigValidationError::InvalidThreshold);
        }

        Ok(())
    }
}
```

### 5.4 Corruption Recovery

```rust
impl Config {
    pub fn load_with_recovery() -> Result<Self> {
        let config_dir = Self::config_dir();
        let config_path = config_dir.join("config.toml");

        // Try normal load first
        if let Ok(config) = Self::load() {
            if let Err(e) = config.validate() {
                tracing::warn!("Config validation failed: {}", e);
                return Self::load_backup_or_defaults();
            }
            return Ok(config);
        }

        // Load corrupted config as backup
        Self::load_backup_or_defaults()
    }

    fn load_backup_or_defaults() -> Result<Self> {
        // Try to restore from latest backup
        if let Some(backup) = Self::find_latest_backup() {
            tracing::warn!("Loading corrupted config backup: {}", backup.display());
            let config = Self::load_from_path(&backup)?;
            if config.validate().is_ok() {
                return Ok(config);
            }
        }

        // Fall back to defaults
        tracing::warn!("No valid config found. Using defaults.");
        Ok(Self::default())
    }
}
```

### 5.5 Backup Policy

- Backups are stored in `~/.codebro/backups/config-v{N}.toml.gz`
- Maximum 5 backups retained (oldest deleted on new backup)
- Backup created before every migration
- Backup is a gzip-compressed TOML snapshot

### 5.6 Anti-Patterns

```rust
// DO NOT: Skip validation on load
pub fn load() -> Result<Self> {
    let content = fs::read_to_string(&config_path)?;
    Ok(toml::from_str(&content)?)  // No validation!
}

// DO: Always validate after load
pub fn load() -> Result<Self> {
    let config = Self::load_raw()?;
    config.validate()?;
    Ok(config)
}
```

---

## 6. Migration Strategy

### 6.1 Version Compatibility Matrix

| Current Version | Supported Upgrade Paths | Notes |
|----------------|------------------------|-------|
| 0 (legacy) | → 1 | Convert flat TOML to structured config |
| 1 | → 2 | Add workspace config section |
| 2 | → 3 | Add feature flags section |
| 3 | → 4 | Add metadata section |
| 4 | → 5 | Add MCP discovery config |
| 5 | → 6 | Add P6 adaptive preferences |

### 6.2 Migration Rules

1. **Minor migrations** (same major version): Automatic, no user interaction.
2. **Major migrations** (major version bump): Require user confirmation via TUI dialog.
3. **All migrations** must be logged to the activity log.
4. **All migrations** must create a backup before proceeding.
5. **All migrations** must be idempotent.

---

## 7. References

- [Configuration Model](../vision/CONFIGURATION_MODEL.md)
- [Architecture Manifest v1.0](../architecture/architecture_manifest_v1.md)
- [SOP v1.0](../SOP/codebro_sop_v1.md)
- [DX Principles](../vision/DX_PRINCIPLES.md)

---

## 8. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
