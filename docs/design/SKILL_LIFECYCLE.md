# Skill Lifecycle — P6 Design Specification

**Document:** `docs/design/SKILL_LIFECYCLE.md`
**Version:** 1.0.0
**Phase:** P6 — Adaptive Intelligence
**Status:** Proposed — Design Summit
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Purpose

The Skill Lifecycle manages the discovery, recommendation, installation, validation, updates, and removal of skills. Skills are reusable workflows that encode successful patterns. This spec extends the existing `SkillManager` with a formal lifecycle that integrates with the Adaptive Platform.

---

## 2. Architecture

```
┌───────────────────────────────────────────────────────────────┐
│                    Skill Lifecycle Manager                    │
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
│  │          Existing SkillManager (src/agent/skill.rs)     │ │
│  │  - Skill storage and retrieval                          │ │
│  │  - Confidence tracking                                  │ │
│  │  - Usage statistics                                     │ │
│  └─────────────────────────────────────────────────────────┘ │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
                        Approval Gate
```

---

## 3. Lifecycle Stages

### 3.1 Stage 1: Discovery

Skills are discovered from:

| Source | Method |
|--------|--------|
| Built-in | Ship with CodeBro binary |
| Registry | Check `~/.codebro/skill-registry.json` |
| Community | Scan known community repositories |
| Generated | Auto-create from Workflow Engine patterns |

```rust
pub struct SkillDiscoveryResult {
    pub skills: Vec<DiscoveredSkill>,
    pub discovered_at: String,
}

pub struct DiscoveredSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub trigger_conditions: Vec<String>,
    pub workflow: Vec<String>,
    pub tools_used: Vec<String>,
    pub language: Option<String>,
    pub framework: Option<String>,
    pub source: SkillDiscoverySource,
    pub author: Option<String>,
    pub version: String,
    pub download_url: Option<String>,
}

pub enum SkillDiscoverySource {
    BuiltIn,
    Registry,
    Community,
    Generated,
}
```

### 3.2 Stage 2: Recommendation

Discovered skills are recommended based on relevance:

```rust
pub struct SkillRecommendation {
    pub id: String,
    pub skill: DiscoveredSkill,
    pub confidence: f32,
    pub reasoning: String,
    pub evidence: Vec<String>,
    pub cost_impact: Option<CostImpact>,
    pub required_approval: bool,
}
```

#### Recommendation Scoring

| Factor | Weight | Description |
|--------|--------|-------------|
| Trigger match | 0.3 | How well trigger conditions match current task |
| Language match | 0.2 | Skill language matches project language |
| Framework match | 0.1 | Skill framework matches project framework |
| Author trust | 0.1 | Known/trusted author |
| Community rating | 0.1 | Rating from other users |
| Success rate | 0.2 | Historical success rate |

### 3.3 Stage 3: Installation

Installation flow:

```
User approves recommendation
        ↓
Download skill package (if external)
        ↓
Validate skill structure
        ↓
Store in ~/.codebro/skills/<id>.json
        ↓
Register in SkillManager
        ↓
Run initial validation test
```

#### Validation

```rust
pub struct SkillValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub test_results: Vec<SkillTestResult>,
}

pub struct SkillTestResult {
    pub test_name: String,
    pub passed: bool,
    pub output: String,
}
```

### 3.4 Stage 4: Validation

Post-installation, the skill undergoes validation:

```rust
pub struct SkillValidationCheck {
    pub skill_id: String,
    pub structure_valid: bool,
    pub triggers_valid: bool,
    pub workflow_executable: bool,
    pub last_test_result: Option<SkillTestResult>,
    pub validated_at: String,
}
```

### 3.5 Stage 5: Updates

Skills may be updated when new versions are available:

```rust
pub struct SkillUpdateInfo {
    pub skill_id: String,
    pub skill_name: String,
    pub current_version: String,
    pub available_version: String,
    pub changelog: String,
    pub breaking_changes: Vec<String>,
    pub recommended: bool,
}
```

### 3.6 Stage 6: Removal

Removal flow:

```
User requests removal
        ↓
Check if skill is currently in use
        ↓
If in use → Warn user, wait for completion
        ↓
Remove from ~/.codebro/skills/<id>.json
        ↓
Remove from SkillManager
        ↓
Update skill-registry.json
```

---

## 4. Skill Registry

```json
{
  "version": 1,
  "skills": [
    {
      "id": "rust-test-pattern",
      "name": "Rust Test Pattern",
      "description": "Standard pattern for writing Rust unit tests",
      "version": "1.0.0",
      "author": "codebro-community",
      "language": "rust",
      "framework": null,
      "trigger_conditions": ["test", "unit test", "write test"],
      "workflow": ["list_files", "read_file", "create_file"],
      "tools_used": ["list_files", "read_file", "create_file"],
      "status": "Trusted",
      "confidence": 0.92,
      "usage_count": 47,
      "success_count": 45,
      "installed_at": "2026-08-01T00:00:00Z",
      "last_used": "2026-08-06T00:00:00Z"
    }
  ]
}
```

---

## 5. Integration with Existing SkillManager

The Skill Lifecycle Manager wraps the existing `SkillManager`:

```rust
pub struct SkillLifecycleManager {
    skill_manager: SkillManager,
    registry_path: PathBuf,
    discovery_sources: Vec<Box<dyn SkillDiscoverySource>>,
}

impl SkillLifecycleManager {
    /// Discover skills from all sources
    pub fn discover(&self) -> SkillDiscoveryResult {
        // Aggregate from all discovery sources
    }

    /// Generate recommendations for undiscovered skills
    pub fn get_recommendations(&self, context: &SkillContext) -> Vec<SkillRecommendation> {
        // Score and rank discovered skills
    }

    /// Install a recommended skill
    pub fn install(&mut self, recommendation_id: &str) -> Result<SkillInstallationResult> {
        // Download, validate, store
    }

    /// Update a skill to a new version
    pub fn update(&mut self, skill_id: &str) -> Result<SkillUpdateResult> {
        // Download new version, validate, replace
    }

    /// Remove a skill
    pub fn remove(&mut self, skill_id: &str) -> Result<()> {
        // Delete from disk and registry
    }
}
```

---

## 6. Trait Contract

```rust
pub trait SkillLifecycleTrait: Send + Sync {
    /// Discover available skills
    fn discover(&self) -> SkillDiscoveryResult;

    /// Get recommendations for the current context
    fn get_recommendations(&self, context: &SkillContext) -> Vec<SkillRecommendation>;

    /// Install a recommended skill
    fn install(&mut self, recommendation_id: &str) -> Result<SkillInstallationResult>;

    /// Validate an installed skill
    fn validate(&self, skill_id: &str) -> SkillValidationResult;

    /// Check for available updates
    fn check_updates(&self) -> Vec<SkillUpdateInfo>;

    /// Update a skill
    fn update(&mut self, skill_id: &str) -> Result<SkillUpdateResult>;

    /// Remove a skill
    fn remove(&mut self, skill_id: &str) -> Result<()>;

    /// Get all installed skills
    fn get_installed(&self) -> Vec<&Skill>;

    /// Export skill registry as JSON
    fn export_registry(&self) -> Result<String>;

    /// Import skill registry from JSON
    fn import_registry(&mut self, registry_json: &str) -> Result<()>;
}

pub struct SkillContext {
    pub task_description: String,
    pub project_language: Option<String>,
    pub project_framework: Option<String>,
    pub current_preferences: HashMap<String, String>,
}

pub struct SkillInstallationResult {
    pub success: bool,
    pub skill_id: String,
    pub validation_result: SkillValidationResult,
}

pub struct SkillUpdateResult {
    pub success: bool,
    pub skill_id: String,
    pub old_version: String,
    pub new_version: String,
    pub validation_result: SkillValidationResult,
}
```

---

## 7. TUI Integration

### 7.1 View: `/skills`

```
┌─────────────────────────────────────────────┐
│  SKILLS                                     │
├─────────────────────────────────────────────┤
│  Installed (3)                              │
│  ─────────────────────────────────          │
│  ✓ Rust Test Pattern    v1.0  47 uses     │
│  ✓ Python FastAPI       v2.1  23 uses     │
│  ✓ Go HTTP Handler      v1.0  12 uses     │
│                                             │
│  Recommendations (2)                        │
│  ─────────────────────────────────          │
│  ? TypeScript React    v1.0  Unverified    │
│    Triggers: "react", "component", "hook"   │
│    [Install] [Dismiss]                      │
│                                             │
│  ? Docker Setup       v1.0  Blocked        │
│    Reason: Requires sudo access             │
│                                             │
│  [Discover]  [Update All]  [Close]          │
└─────────────────────────────────────────────┘
```

---

## 8. Anti-Patterns

```rust
// NEVER: Auto-install a skill without user approval
// ALWAYS: Present as a recommendation first

// NEVER: Allow skills that require sudo or root access
// ALWAYS: Block and report such skills

// NEVER: Silently update skills
// ALWAYS: Notify user of updates and require approval
```

---

## 9. References

- [ADAPTIVE_PLATFORM_SPEC.md](./ADAPTIVE_PLATFORM_SPEC.md)
- [WORKFLOW_ENGINE_SPEC.md](./WORKFLOW_ENGINE_SPEC.md)
- [COST_POLICY.md](./COST_POLICY.md)

---

## 10. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
