# Profile Engine — P6 Design Specification

**Document:** `docs/design/PROFILE_ENGINE_SPEC.md`
**Version:** 1.0.0
**Phase:** P6 — Adaptive Intelligence
**Status:** Proposed — Design Summit
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Purpose

The Profile Engine manages developer profiles — named collections of preferences optimized for different contexts (coding, reviewing, researching, etc.). Profiles are fully editable inside the TUI and can be switched on demand.

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Profile Engine                         │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │                  Profile Store                       │   │
│  │                                                      │   │
│  │  ┌───────────┐  ┌───────────┐  ┌──────────────┐   │   │
│  │  │ Coding    │  │ Review    │  │ Research     │   │   │
│  │  │ Profile   │  │ Profile   │  │ Profile      │   │   │
│  │  └───────────┘  └───────────┘  └──────────────┘   │   │
│  │  ┌───────────┐  ┌───────────┐  ┌──────────────┐   │   │
│  │  │ Planning  │  │ Low Cost  │  │ Local Only   │   │   │
│  │  │ Profile   │  │ Profile   │  │ Profile      │   │   │
│  │  └───────────┘  └───────────┘  └──────────────┘   │   │
│  │  ┌───────────┐  ┌───────────┐                      │   │
│  │  │ Custom    │  │ Custom    │  ... (user-created) │   │
│  │  │ Profile   │  │ Profile   │                      │   │
│  │  └───────────┘  └───────────┘                      │   │
│  └─────────────────────────────────────────────────────┘   │
│                             │                              │
│        ┌────────────────────┼────────────────────┐         │
│        ▼                    ▼                    ▼         │
│  Preference Engine      Intent Engine      TUI             │
│  (reads active)         (can switch)       (displays)      │
└─────────────────────────────────────────────────────────────┘
```

---

## 3. Profile Structure

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Unique identifier
    pub id: String,

    /// Display name
    pub name: String,

    /// Human-readable description
    pub description: String,

    /// Icon/emoji identifier for TUI display
    pub icon: String,

    /// Whether this is a built-in profile or user-created
    pub source: ProfileSource,

    /// The preferences this profile sets
    pub preferences: ProfilePreferences,

    /// Model overrides specific to this profile
    pub model_overrides: HashMap<String, String>,

    /// When the profile was last switched to
    pub last_used: Option<String>,

    /// How many times this profile has been used
    pub usage_count: u32,

    /// Created timestamp
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilePreferences {
    /// Coding style preference
    pub coding_style: Option<String>,
    pub include_tests: Option<bool>,
    pub add_comments: Option<bool>,
    pub error_handling: Option<String>,

    /// Cost settings
    pub cost_tier: Option<String>,
    pub daily_limit_usd: Option<f64>,
    pub prefer_local: Option<bool>,

    /// Language preferences
    pub primary_language: Option<String>,
    pub additional_languages: Vec<String>,

    /// Workflow preferences
    pub ask_before_install: Option<bool>,
    pub show_diff_before_edit: Option<bool>,
    pub run_tests_before_commit: Option<bool>,
    pub review_style: Option<String>,

    /// Provider preferences
    pub default_provider: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProfileSource {
    BuiltIn,
    UserCreated,
}
```

---

## 4. Built-In Profiles

### 4.1 Coding Profile

Optimized for active development:

```rust
Profile {
    name: "Coding",
    icon: "⚡",
    preferences: ProfilePreferences {
        coding_style: Some("idiomatic".to_string()),
        include_tests: Some(true),
        add_comments: Some(false),
        error_handling: Some("result".to_string()),
        cost_tier: Some("balanced".to_string()),
        ask_before_install: Some(false),
        show_diff_before_edit: Some(true),
        run_tests_before_commit: Some(true),
        review_style: Some("safety-focused".to_string()),
    },
    model_overrides: HashMap::new(),
    source: ProfileSource::BuiltIn,
}
```

### 4.2 Review Profile

Optimized for code review:

```rust
Profile {
    name: "Review",
    icon: "🔍",
    preferences: ProfilePreferences {
        coding_style: Some("explicit".to_string()),
        include_tests: Some(false),
        add_comments: Some(true),
        error_handling: Some("defensive".to_string()),
        cost_tier: Some("quality".to_string()),
        ask_before_install: Some(true),
        show_diff_before_edit: Some(true),
        run_tests_before_commit: Some(true),
        review_style: Some("strict".to_string()),
    },
    model_overrides: {
        "reviewer".to_string() => "claude-sonnet-4".to_string(),
    },
    source: ProfileSource::BuiltIn,
}
```

### 4.3 Research Profile

Optimized for exploration and analysis:

```rust
Profile {
    name: "Research",
    icon: "🔬",
    preferences: ProfilePreferences {
        coding_style: Some("minimal".to_string()),
        include_tests: Some(false),
        add_comments: Some(true),
        error_handling: Some("explicit".to_string()),
        cost_tier: Some("balanced".to_string()),
        ask_before_install: Some(true),
        show_diff_before_edit: Some(true),
        run_tests_before_commit: Some(false),
        review_style: Some("light".to_string()),
    },
    model_overrides: {
        "researcher".to_string() => "claude-opus-4".to_string(),
    },
    source: ProfileSource::BuiltIn,
}
```

### 4.4 Planning Profile

Optimized for architecture and design:

```rust
Profile {
    name: "Planning",
    icon: "📋",
    preferences: ProfilePreferences {
        coding_style: Some("idiomatic".to_string()),
        include_tests: Some(false),
        add_comments: Some(true),
        error_handling: Some("result".to_string()),
        cost_tier: Some("quality".to_string()),
        ask_before_install: Some(true),
        show_diff_before_edit: Some(false),
        run_tests_before_commit: Some(false),
        review_style: Some("light".to_string()),
    },
    model_overrides: {
        "planner".to_string() => "claude-opus-4".to_string(),
    },
    source: ProfileSource::BuiltIn,
}
```

### 4.5 Low Cost Profile

Optimized for minimal spending:

```rust
Profile {
    name: "Low Cost",
    icon: "💰",
    preferences: ProfilePreferences {
        coding_style: Some("minimal".to_string()),
        include_tests: Some(true),
        add_comments: Some(false),
        error_handling: Some("minimal".to_string()),
        cost_tier: Some("minimal".to_string()),
        daily_limit_usd: Some(1.0),
        prefer_local: Some(true),
        ask_before_install: Some(true),
        show_diff_before_edit: Some(true),
        run_tests_before_commit: Some(false),
        review_style: Some("light".to_string()),
    },
    model_overrides: HashMap::new(),
    source: ProfileSource::BuiltIn,
}
```

### 4.6 Local Only Profile

Optimized for offline/local model usage:

```rust
Profile {
    name: "Local Only",
    icon: "🏠",
    preferences: ProfilePreferences {
        coding_style: Some("idiomatic".to_string()),
        include_tests: Some(true),
        add_comments: Some(false),
        error_handling: Some("result".to_string()),
        cost_tier: Some("minimal".to_string()),
        prefer_local: Some(true),
        ask_before_install: Some(true),
        show_diff_before_edit: Some(true),
        run_tests_before_commit: Some(true),
        review_style: Some("safety-focused".to_string()),
    },
    model_overrides: HashMap::new(),
    source: ProfileSource::BuiltIn,
}
```

---

## 5. Profile Switching

### 5.1 Switching Mechanism

When a profile is switched:
1. All preferences in the new profile override the current preferences
2. Model overrides are applied to the orchestrator
3. An `AdaptiveEvent::ProfileSwitched` is emitted
4. The old profile's `last_used` is updated
5. The new profile's `usage_count` is incremented

### 5.2 Merge Semantics

Profile preferences use **merge semantics**: only keys present in the profile are overridden. Keys not present retain their current value.

```
Current preferences:  { coding.style = "idiomatic", cost.tier = "balanced", language.primary = "rust" }
Coding profile:       { coding.style = "idiomatic", cost.tier = "balanced", include_tests = true }
Result:               { coding.style = "idiomatic", cost.tier = "balanced", language.primary = "rust", include_tests = true }
```

### 5.3 Reverting to Default

Switching to a built-in profile replaces all preferences from that profile. Switching to "Default" (no profile) restores the base preferences with no overrides.

---

## 6. Trait Contract

```rust
pub trait ProfileEngineTrait: Send + Sync {
    /// Get all available profiles
    fn get_profiles(&self) -> Vec<&Profile>;

    /// Get the currently active profile
    fn get_active_profile(&self) -> Option<&Profile>;

    /// Switch to a profile by ID
    fn switch_profile(&mut self, profile_id: &str) -> Result<ProfileSwitchEvent>;

    /// Create a new custom profile from current preferences
    fn create_profile_from_current(&mut self, name: &str, description: &str) -> Result<String>;

    /// Create a new custom profile with explicit preferences
    fn create_profile(&mut self, profile: Profile) -> Result<String>;

    /// Update an existing profile
    fn update_profile(&mut self, profile_id: &str, updates: ProfileUpdates) -> Result<()>;

    /// Delete a profile
    fn delete_profile(&mut self, profile_id: &str) -> Result<()>;

    /// Get the preference overrides that a profile would apply
    fn get_profile_overrides(&self, profile_id: &str) -> HashMap<String, String>;

    /// Suggest a profile based on current context
    fn suggest_profile(&self, context: &ProfileContext) -> Option<&Profile>;
}

pub struct ProfileSwitchEvent {
    pub from: Option<String>,
    pub to: String,
    pub preferences_changed: Vec<String>,
    pub model_overrides_changed: Vec<String>,
}

pub struct ProfileUpdates {
    pub name: Option<String>,
    pub description: Option<String>,
    pub preferences: Option<ProfilePreferences>,
    pub model_overrides: Option<HashMap<String, String>>,
}

pub struct ProfileContext {
    pub current_task: Option<String>,
    pub workspace_language: Option<String>,
    pub recent_cost: f64,
    pub daily_limit: Option<f64>,
}
```

---

## 7. TUI Integration

### 7.1 View: `/profile`

```
┌─────────────────────────────────────────────┐
│  PROFILES                                   │
├─────────────────────────────────────────────┤
│  Active: ⚡ Coding                          │
│                                             │
│  Built-in Profiles:                         │
│  ─────────────────────────────────          │
│  ⚡ Coding        - Active for development  │
│  🔍 Review        - Strict review focus     │
│  🔬 Research      - Exploration mode        │
│  📋 Planning      - Architecture mode       │
│  💰 Low Cost      - Minimize spending       │
│  🏠 Local Only    - Offline capability      │
│                                             │
│  Custom Profiles (2):                       │
│  ─────────────────────────────────          │
│  🎨 My Style      - Used 5 times           │
│  🚀 Fast Iteration - Used 12 times         │
│                                             │
│  [Switch]  [Edit]  [New]  [Delete]  [Close] │
└─────────────────────────────────────────────┘
```

### 7.2 Quick Switch

Users can quickly switch profiles using the command palette:
- `/profile coding` — switch to Coding profile
- `/profile review` — switch to Review profile
- `/profile list` — show all profiles

---

## 8. Profile Persistence

Profiles are stored in `~/.codebro/adaptive/profiles.json`:

```json
{
  "version": 1,
  "active_profile_id": "coding",
  "profiles": [
    {
      "id": "coding",
      "name": "Coding",
      "icon": "⚡",
      "source": "BuiltIn",
      ...
    },
    {
      "id": "custom-abc123",
      "name": "My Style",
      "icon": "🎨",
      "source": "UserCreated",
      ...
    }
  ]
}
```

---

## 9. Anti-Patterns

```rust
// NEVER: Allow a profile to set an invalid preference value
// ALWAYS: Validate profile preferences before saving

// NEVER: Delete a built-in profile
// ALWAYS: Built-in profiles are immutable; users can only delete custom profiles

// NEVER: Silently switch profiles based on context
// ALWAYS: Profile switches require explicit user action (or approval via recommendation)
```

---

## 10. References

- [ADAPTIVE_PLATFORM_SPEC.md](./ADAPTIVE_PLATFORM_SPEC.md)
- [USER_PREFERENCE_MODEL.md](./USER_PREFERENCE_MODEL.md)
- [SUBAGENT_ORCHESTRATION_SPEC.md](./SUBAGENT_ORCHESTRATION_SPEC.md)

---

## 11. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
