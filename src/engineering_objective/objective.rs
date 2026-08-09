//! The `EngineeringObjective` — a compact, first-class goal hierarchy.
//!
//! CodeBro reasons top-down:
//!
//! ```text
//! END GOAL
//!     ↓
//! PROJECT VISION
//!     ↓
//! CURRENT OBJECTIVE
//!     ↓
//! CURRENT MILESTONE / SPRINT
//!     ↓
//! CURRENT TASK
//!     ↓
//! CURRENT ACTION
//! ```
//!
//! The objective model is intentionally small. It stores compact,
//! authoritative strings — never full documents. Full documents live in
//! `docs/`; this model stores the distilled, structured reference plus an
//! optional pointer to the authoritative source document.

use serde::{Deserialize, Serialize};

/// Current schema version for engineering objectives.
pub const CURRENT_SCHEMA_VERSION: &str = "1.0.0";

/// The compact engineering objective hierarchy for a project.
///
/// ## Authority
///
/// Values must come from the repository's project documentation
/// (`docs/vision/`, `docs/architecture/`, `docs/ADR/`, roadmap, and the
/// current sprint definition). `source` records which document is
/// authoritative so conflicts resolve to the documented precedence:
///
/// ```text
/// Product Vision
///     > Architecture / ADR
///     > Current Objective
///     > Sprint / Milestone
///     > Task
///     > Temporary Memory
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineeringObjective {
    /// The terminal outcome of the project.
    pub end_goal: String,
    /// The product vision — why the project exists.
    pub project_vision: String,
    /// The objective being pursued right now.
    pub current_objective: String,
    /// The current milestone / sprint that scopes the objective.
    pub current_milestone: String,
    /// Compact success criteria for the current objective.
    pub success_criteria: Vec<String>,
    /// Non-goals — boundaries that must never be crossed.
    pub non_goals: Vec<String>,
    /// Pointer to the authoritative source document (compact reference).
    pub source: Option<String>,
    /// Schema version of this objective.
    pub schema_version: String,
}

impl EngineeringObjective {
    /// Create an objective with the given hierarchy.
    pub fn new(
        end_goal: impl Into<String>,
        project_vision: impl Into<String>,
        current_objective: impl Into<String>,
        current_milestone: impl Into<String>,
    ) -> Self {
        EngineeringObjective {
            end_goal: end_goal.into(),
            project_vision: project_vision.into(),
            current_objective: current_objective.into(),
            current_milestone: current_milestone.into(),
            success_criteria: Vec::new(),
            non_goals: Vec::new(),
            source: None,
            schema_version: CURRENT_SCHEMA_VERSION.to_string(),
        }
    }

    pub fn with_success_criteria(mut self, criteria: Vec<String>) -> Self {
        self.success_criteria = criteria;
        self.success_criteria.sort();
        self.success_criteria.dedup();
        self
    }

    pub fn with_non_goals(mut self, non_goals: Vec<String>) -> Self {
        self.non_goals = non_goals;
        self.non_goals.sort();
        self.non_goals.dedup();
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Returns `true` when no meaningful goal content is present.
    pub fn is_empty(&self) -> bool {
        self.end_goal.trim().is_empty()
            && self.project_vision.trim().is_empty()
            && self.current_objective.trim().is_empty()
            && self.current_milestone.trim().is_empty()
    }

    /// Estimated token count of the compact render (approximate).
    pub fn estimated_tokens(&self) -> usize {
        self.render_compact("", "", None).len() / 4
    }

    /// Render the objective hierarchy as the compact always-on block.
    ///
    /// This is the only representation that reaches the model. It targets
    /// roughly 100–300 tokens and never dumps source documents.
    pub fn render_compact(
        &self,
        project_name: &str,
        current_task: &str,
        alignment: Option<GoalAlignment>,
    ) -> String {
        let mut lines: Vec<String> = Vec::new();

        if !project_name.trim().is_empty() {
            lines.push("PROJECT".to_string());
            lines.push(project_name.trim().to_string());
            lines.push(String::new());
        }

        if !self.end_goal.trim().is_empty() {
            lines.push("END GOAL".to_string());
            lines.push(self.end_goal.trim().to_string());
            lines.push(String::new());
        }

        if !self.current_objective.trim().is_empty() {
            lines.push("CURRENT OBJECTIVE".to_string());
            lines.push(self.current_objective.trim().to_string());
            lines.push(String::new());
        }

        if !self.current_milestone.trim().is_empty() {
            lines.push("CURRENT MILESTONE".to_string());
            lines.push(self.current_milestone.trim().to_string());
            lines.push(String::new());
        }

        if !current_task.trim().is_empty() {
            lines.push("CURRENT TASK".to_string());
            lines.push(current_task.trim().to_string());
            lines.push(String::new());
        }

        if let Some(alignment) = alignment {
            lines.push("TASK ALIGNMENT".to_string());
            match alignment {
                GoalAlignment::Unclear => {
                    lines.push(format!("{} ⚠ Task alignment unclear", alignment.as_str()));
                }
                other => lines.push(other.as_str().to_string()),
            }
        }

        while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
            lines.pop();
        }

        lines.join("\n")
    }

    /// Deterministic goal-alignment of a task against this objective.
    ///
    /// This is not an ML score — it is a deterministic token-overlap
    /// heuristic. It informs awareness; it never blocks execution and
    /// never overrides user intent.
    pub fn align_task(&self, task_keywords: &[String]) -> GoalAlignment {
        if self.is_empty() {
            return GoalAlignment::Unclear;
        }
        let haystack = format!(
            "{} {} {}",
            self.current_objective.to_lowercase(),
            self.current_milestone.to_lowercase(),
            self.end_goal.to_lowercase()
        );
        let overlap = task_keywords
            .iter()
            .filter(|kw| {
                let kw = kw.to_lowercase();
                kw.len() > 2 && haystack.contains(&kw)
            })
            .count();

        match overlap {
            0 => GoalAlignment::WeaklyRelated,
            1 => GoalAlignment::Supporting,
            _ => GoalAlignment::Direct,
        }
    }
}

impl Default for EngineeringObjective {
    fn default() -> Self {
        EngineeringObjective {
            end_goal: String::new(),
            project_vision: String::new(),
            current_objective: String::new(),
            current_milestone: String::new(),
            success_criteria: Vec::new(),
            non_goals: Vec::new(),
            source: None,
            schema_version: CURRENT_SCHEMA_VERSION.to_string(),
        }
    }
}

/// Deterministic goal-alignment metadata for a task.
///
/// Possible values: `Direct`, `Supporting`, `WeaklyRelated`, `Unclear`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GoalAlignment {
    /// The task directly advances the current objective.
    Direct,
    /// The task supports the current objective without directly advancing it.
    Supporting,
    /// The task is only weakly related to the current objective.
    WeaklyRelated,
    /// Alignment could not be determined.
    Unclear,
}

impl GoalAlignment {
    pub fn as_str(self) -> &'static str {
        match self {
            GoalAlignment::Direct => "Direct",
            GoalAlignment::Supporting => "Supporting",
            GoalAlignment::WeaklyRelated => "Weakly Related",
            GoalAlignment::Unclear => "Unclear",
        }
    }
}

impl std::fmt::Display for GoalAlignment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_objective() -> EngineeringObjective {
        EngineeringObjective::new(
            "Build a terminal-native engineering intelligence runtime.",
            "CodeBro is the most trustworthy engineering intelligence runtime for developers.",
            "Make CodeBro capable of maintaining software projects.",
            "Engineering Objective & Lazy Execution.",
        )
        .with_success_criteria(vec![
            "All production tasks use the canonical runtime.".to_string(),
            "The model sees compact objective context.".to_string(),
        ])
        .with_non_goals(vec![
            "IDE replacement".to_string(),
            "General chatbot".to_string(),
        ])
    }

    #[test]
    fn test_new_objective_fields() {
        let o = sample_objective();
        assert_eq!(o.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(!o.is_empty());
        assert_eq!(o.success_criteria.len(), 2);
        assert_eq!(o.non_goals.len(), 2);
    }

    #[test]
    fn test_default_objective_is_empty() {
        let o = EngineeringObjective::default();
        assert!(o.is_empty());
        assert_eq!(o.align_task(&["test".to_string()]), GoalAlignment::Unclear);
    }

    #[test]
    fn test_criteria_sorted_and_deduped() {
        let o = EngineeringObjective::new("g", "v", "o", "m")
            .with_success_criteria(vec!["z".to_string(), "a".to_string(), "a".to_string()])
            .with_non_goals(vec!["b".to_string(), "a".to_string(), "b".to_string()]);
        assert_eq!(o.success_criteria, vec!["a", "z"]);
        assert_eq!(o.non_goals, vec!["a", "b"]);
    }

    #[test]
    fn test_render_compact_contains_hierarchy() {
        let o = sample_objective();
        let rendered = o.render_compact("CodeBro", "Implement indexed retrieval", None);
        assert!(rendered.contains("PROJECT"));
        assert!(rendered.contains("CodeBro"));
        assert!(rendered.contains("END GOAL"));
        assert!(rendered.contains("CURRENT OBJECTIVE"));
        assert!(rendered.contains("CURRENT MILESTONE"));
        assert!(rendered.contains("CURRENT TASK"));
        assert!(rendered.contains("Implement indexed retrieval"));
    }

    #[test]
    fn test_render_compact_includes_alignment() {
        let o = sample_objective();
        let rendered = o.render_compact(
            "CodeBro",
            "Implement indexed retrieval",
            Some(GoalAlignment::Supporting),
        );
        assert!(rendered.contains("TASK ALIGNMENT"));
        assert!(rendered.contains("Supporting"));
    }

    #[test]
    fn test_render_compact_unclear_alignment_warns() {
        let o = sample_objective();
        let rendered = o.render_compact("CodeBro", "Book a flight", Some(GoalAlignment::Unclear));
        assert!(rendered.contains("⚠ Task alignment unclear"));
    }

    #[test]
    fn test_render_compact_is_small() {
        let o = sample_objective();
        let rendered = o.render_compact("CodeBro", "Implement indexed retrieval", None);
        let tokens = rendered.len() / 4;
        assert!(
            tokens <= 300,
            "objective block should be compact, got ~{} tokens",
            tokens
        );
    }

    #[test]
    fn test_align_task_direct() {
        let o = sample_objective();
        // "maintain" + "software" both appear in the objective text.
        let kw = vec!["maintain".to_string(), "software".to_string()];
        assert_eq!(o.align_task(&kw), GoalAlignment::Direct);
    }

    #[test]
    fn test_align_task_supporting() {
        let o = sample_objective();
        let kw = vec!["maintain".to_string()];
        assert_eq!(o.align_task(&kw), GoalAlignment::Supporting);
    }

    #[test]
    fn test_align_task_weakly_related() {
        let o = sample_objective();
        let kw = vec!["calendar".to_string(), "emails".to_string()];
        assert_eq!(o.align_task(&kw), GoalAlignment::WeaklyRelated);
    }

    #[test]
    fn test_align_task_deterministic() {
        let o = sample_objective();
        let kw = vec!["maintain".to_string(), "software".to_string()];
        assert_eq!(o.align_task(&kw), o.align_task(&kw));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let o = sample_objective();
        let json = serde_json::to_string(&o).expect("serialize");
        let decoded: EngineeringObjective = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(o, decoded);
    }

    #[test]
    fn test_goal_alignment_display() {
        assert_eq!(GoalAlignment::Direct.as_str(), "Direct");
        assert_eq!(GoalAlignment::Supporting.as_str(), "Supporting");
        assert_eq!(GoalAlignment::WeaklyRelated.as_str(), "Weakly Related");
        assert_eq!(GoalAlignment::Unclear.as_str(), "Unclear");
    }
}
