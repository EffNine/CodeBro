# Workflow Engine — P6 Design Specification

**Document:** `docs/design/WORKFLOW_ENGINE_SPEC.md`
**Version:** 1.0.0
**Phase:** P6 — Adaptive Intelligence
**Status:** Proposed — Design Summit
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Purpose

The Workflow Engine observes repeated developer actions, detects patterns, and generates suggestions for reusable workflow automations. It never activates automatically — it only suggests.

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Workflow Engine                        │
│                                                             │
│  ┌───────────────────┐    ┌───────────────────┐            │
│  │  Action Logger    │    │  Pattern          │            │
│  │  (tracks all      │    │  Detector         │            │
│  │   tool executions│    │   (sliding        │            │
│  │   and their       │    │   window)         │            │
│  │   outcomes)       │    │                   │            │
│  └────────┬──────────┘    └────────┬──────────┘            │
│           │                        │                        │
│           └──────────┬─────────────┘                        │
│                      ▼                                       │
│           ┌─────────────────────┐                           │
│           │  Pattern            │                           │
│           │  Matcher            │                           │
│           │  (fuzzy match       │                           │
│           │   against known     │                           │
│           │   patterns)         │                           │
│           └──────────┬──────────┘                           │
│                      ▼                                       │
│           ┌─────────────────────┐                           │
│           │  Workflow           │                           │
│           │  Suggestion         │                           │
│           │  (packaged for      │                           │
│           │   Recommendation   │                           │
│           │   Engine)           │                           │
│           └─────────────────────┘                           │
└─────────────────────────────────────────────────────────────┘
```

---

## 3. Action Logging

The Workflow Engine listens to all `AgentEvent::ToolCompleted` events and records:

```rust
pub struct ActionRecord {
    pub timestamp: String,
    pub tool_name: String,
    pub tool_args: String,
    pub success: bool,
    pub duration_ms: u64,
    pub task_context: Option<String>,
}
```

### 3.1 Sliding Window

Actions are evaluated in sliding windows of configurable size (default: 10 actions). A new pattern is detected when the same sequence of tools appears 3+ times within overlapping windows.

### 3.2 Pattern Representation

```rust
pub struct ActionPattern {
    pub sequence: Vec<String>,       // Tool names in order
    pub occurrences: u32,            // How many times observed
    pub success_rate: f32,           // Fraction of successful executions
    pub avg_duration_ms: u64,        // Average total duration
    pub first_seen: String,          // ISO timestamp
    pub last_seen: String,           // ISO timestamp
    pub context_tags: Vec<String>,   // Project language, framework, etc.
}
```

---

## 4. Pattern Detection Algorithm

```
For each new ActionRecord:
  1. Append to sliding window
  2. Check if the last N actions match any existing pattern
  3. If match found:
     - Increment occurrence count
     - Update success rate
     - Update last_seen timestamp
  4. If no match:
     - Check if the new sequence (last 2-5 actions) forms a new pattern
     - If the sequence has appeared 2+ times in the last 20 actions:
       - Create new ActionPattern with occurrences=2
  5. If any pattern reaches 3+ occurrences:
     - Generate WorkflowSuggestion via RecommendationEngine
```

### 4.1 Example

Developer workflow:
```
1. git_status
2. list_files ("src/")
3. read_file ("src/main.rs")
4. edit_file ("src/main.rs")
5. run_command ("cargo test")
```

After this sequence repeats 3 times:
```
Pattern: [git_status, list_files, read_file, edit_file, run_command]
Occurrences: 3
Success rate: 1.0
Suggestion: "Save this as a workflow? Named: 'Edit and test main.rs'"
```

---

## 5. Workflow Suggestion Structure

```rust
pub struct WorkflowSuggestion {
    pub id: String,
    pub name: String,
    pub description: String,
    pub pattern: Vec<String>,
    pub occurrence_count: u32,
    pub success_rate: f32,
    pub avg_duration_ms: u64,
    pub estimated_time_saved_ms: u64,
    pub confidence: f32,
    pub required_approval: bool,
}
```

---

## 6. Trait Contract

```rust
pub trait WorkflowEngineTrait: Send + Sync {
    /// Record a completed action for pattern detection
    fn record_action(&mut self, record: ActionRecord);

    /// Get detected patterns
    fn get_patterns(&self) -> Vec<&ActionPattern>;

    /// Get workflow suggestions (patterns with 3+ occurrences)
    fn get_suggestions(&self) -> Vec<&WorkflowSuggestion>;

    /// Save a workflow as a reusable template
    fn save_workflow(&mut self, suggestion_id: &str, name: &str) -> Result<String>;

    /// Get saved workflows
    fn get_saved_workflows(&self) -> Vec<&SavedWorkflow>;

    /// Delete a saved workflow
    fn delete_workflow(&mut self, workflow_id: &str) -> Result<()>;

    /// Clear old action records (older than threshold)
    fn prune_old_records(&mut self, max_age_days: u64) -> usize;
}

pub struct SavedWorkflow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub steps: Vec<String>,
    pub created_at: String,
    pub usage_count: u32,
    pub last_used: Option<String>,
}
```

---

## 7. Thresholds and Tuning

| Parameter | Default | Description |
|-----------|---------|-------------|
| `min_occurrences_for_suggestion` | 3 | Pattern must appear 3+ times before suggestion |
| `min_occurrences_for_detection` | 2 | New patterns appear after 2 occurrences |
| `sliding_window_size` | 10 | Number of recent actions to consider |
| `pattern_cooldown_minutes` | 60 | Same pattern won't trigger another suggestion within 60 min |
| `max_patterns_stored` | 50 | Maximum patterns to keep in memory |
| `action_record_retention_days` | 30 | Action records are pruned after 30 days |

---

## 8. TUI Integration

### 8.1 View: `/workflows`

```
┌─────────────────────────────────────────────┐
│  WORKFLOWS                                  │
├─────────────────────────────────────────────┤
│  Detected Patterns (2)                      │
│  ─────────────────────────────────          │
│  1. git_status → list_files → edit_file    │
│     Occurrences: 5  Success: 100%          │
│     Suggested name: "Quick edit pattern"   │
│     [Save] [Dismiss]                        │
│                                             │
│  2. read_file → edit_file → run_command    │
│     Occurrences: 3  Success: 67%           │
│     Suggested name: "Edit and run"         │
│     [Save] [Dismiss]                        │
│                                             │
│  Saved Workflows (1)                        │
│  ─────────────────────────────────          │
│  - Quick edit pattern (used 3 times)       │
│                                             │
│  [New]  [Edit]  [Delete]  [Close]           │
└─────────────────────────────────────────────┘
```

### 8.2 Integration with Skill System

Saved workflows can be converted into skills (stored in `skills/` directory) with a single command. This bridges the Workflow Engine with the existing `SkillManager`.

---

## 9. Anti-Patterns

```rust
// NEVER: Automatically execute a detected workflow
// ALWAYS: Present as a suggestion for user approval

// NEVER: Suggest workflows for sequences with < 3 occurrences
// ALWAYS: Wait for sufficient evidence

// NEVER: Suggest workflows that include destructive operations
// (e.g., git push --force, rm -rf) without explicit warning
```

---

## 10. References

- [ADAPTIVE_PLATFORM_SPEC.md](./ADAPTIVE_PLATFORM_SPEC.md)
- [RECOMMENDATION_ENGINE_SPEC.md](./RECOMMENDATION_ENGINE_SPEC.md)
- [SKILL_LIFECYCLE.md](./SKILL_LIFECYCLE.md)

---

## 11. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
