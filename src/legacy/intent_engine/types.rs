//! Intent Engine Types — core data model.
//!
//! Strongly typed intent categories, plans, and commands.
//! Every type is immutable, serializable, and replayable.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

// ─── Intent Type ────────────────────────────────────────────────────────────

/// The category of user intent.
///
/// The classifier must never force an unknown intent into another category.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IntentType {
    /// User wants to change a preference.
    Preference,
    /// User wants to configure system settings.
    Configuration,
    /// User wants to execute a workflow or sequence of steps.
    Workflow,
    /// User wants to run a command or operation.
    Execution,
    /// User is asking a question.
    Question,
    /// User needs help or guidance.
    Help,
    /// Intent could not be classified deterministically.
    Unknown,
}

impl fmt::Display for IntentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntentType::Preference => write!(f, "preference"),
            IntentType::Configuration => write!(f, "configuration"),
            IntentType::Workflow => write!(f, "workflow"),
            IntentType::Execution => write!(f, "execution"),
            IntentType::Question => write!(f, "question"),
            IntentType::Help => write!(f, "help"),
            IntentType::Unknown => write!(f, "unknown"),
        }
    }
}

// ─── Intent Plan ────────────────────────────────────────────────────────────

/// A structured, explainable, serializable intent plan.
///
/// Every plan must contain enough information for auditing and replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentPlan {
    pub id: String,
    pub detected_goal: String,
    pub intent_type: IntentType,
    pub affected_subsystem: String,
    pub required_approval: bool,
    pub estimated_cost_impact: f64,
    pub confidence: f64,
    pub ambiguity: bool,
    pub ambiguity_reason: Option<String>,
    pub reasoning: String,
    pub evidence: Vec<String>,
    pub required_commands: Vec<IntentCommand>,
    pub created_at: String,
}

impl IntentPlan {
    pub fn new(
        id: String,
        detected_goal: &str,
        intent_type: IntentType,
        affected_subsystem: &str,
        required_approval: bool,
        estimated_cost_impact: f64,
        confidence: f64,
        ambiguity: bool,
        ambiguity_reason: Option<String>,
        reasoning: &str,
        evidence: Vec<String>,
        required_commands: Vec<IntentCommand>,
    ) -> Self {
        IntentPlan {
            id,
            detected_goal: detected_goal.to_string(),
            intent_type,
            affected_subsystem: affected_subsystem.to_string(),
            required_approval,
            estimated_cost_impact,
            confidence,
            ambiguity,
            ambiguity_reason,
            reasoning: reasoning.to_string(),
            evidence,
            required_commands,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Return whether this plan can proceed without clarification.
    pub fn is_actionable(&self) -> bool {
        !self.ambiguity && self.confidence >= 0.5
    }
}

// ─── Intent Command ─────────────────────────────────────────────────────────

/// An immutable command produced by the intent resolver.
///
/// Commands never modify state directly; they request approval first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IntentCommand {
    /// Request to update a model-related preference.
    UpdateModelPreference {
        key: String,
        new_value: String,
        reason: String,
    },
    /// Request to update a language preference.
    UpdateLanguagePreference {
        key: String,
        new_value: String,
        reason: String,
    },
    /// Request to update a cost-related preference.
    UpdateCostPreference {
        key: String,
        new_value: f64,
        reason: String,
    },
    /// Request to update an approval-related preference.
    UpdateApprovalPreference {
        key: String,
        new_value: bool,
        reason: String,
    },
    /// Request to execute a workflow by ID.
    ExecuteWorkflow { workflow_id: String, reason: String },
    /// Request to execute a shell/command operation.
    ExecuteCommand { command: String, reason: String },
    /// Request to answer a user question.
    AnswerQuestion { question: String, answer: String },
    /// Request to provide help text.
    ProvideHelp { topic: String, help_text: String },
}

impl IntentCommand {
    pub fn kind(&self) -> &str {
        match self {
            IntentCommand::UpdateModelPreference { .. } => "update_model_preference",
            IntentCommand::UpdateLanguagePreference { .. } => "update_language_preference",
            IntentCommand::UpdateCostPreference { .. } => "update_cost_preference",
            IntentCommand::UpdateApprovalPreference { .. } => "update_approval_preference",
            IntentCommand::ExecuteWorkflow { .. } => "execute_workflow",
            IntentCommand::ExecuteCommand { .. } => "execute_command",
            IntentCommand::AnswerQuestion { .. } => "answer_question",
            IntentCommand::ProvideHelp { .. } => "provide_help",
        }
    }

    pub fn requires_approval(&self) -> bool {
        matches!(
            self,
            IntentCommand::UpdateModelPreference { .. }
                | IntentCommand::UpdateLanguagePreference { .. }
                | IntentCommand::UpdateCostPreference { .. }
                | IntentCommand::UpdateApprovalPreference { .. }
                | IntentCommand::ExecuteWorkflow { .. }
                | IntentCommand::ExecuteCommand { .. }
        )
    }
}

// ─── Command Metadata (audit) ───────────────────────────────────────────────

/// Immutable audit metadata attached to every command execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandMetadata {
    pub source: String,
    pub timestamp: String,
    pub intent_id: String,
    pub reason: String,
    pub expected_effect: String,
}

impl CommandMetadata {
    pub fn new(source: &str, intent_id: &str, reason: &str, expected_effect: &str) -> Self {
        CommandMetadata {
            source: source.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            intent_id: intent_id.to_string(),
            reason: reason.to_string(),
            expected_effect: expected_effect.to_string(),
        }
    }
}

// ─── Approval Preview ───────────────────────────────────────────────────────

/// Read-only preview of a command's proposed change.
///
/// No state is modified during preview generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalPreview {
    pub command_kind: String,
    pub requested_change: String,
    pub current_value: Option<String>,
    pub proposed_value: String,
    pub estimated_cost_impact: f64,
    pub affected_workflows: Vec<String>,
    pub reversibility: Reversibility,
    pub preview_id: String,
    pub generated_at: String,
}

impl ApprovalPreview {
    pub fn new(
        command_kind: &str,
        requested_change: &str,
        current_value: Option<String>,
        proposed_value: &str,
        estimated_cost_impact: f64,
        affected_workflows: Vec<String>,
        reversibility: Reversibility,
    ) -> Self {
        let preview_id = Uuid::new_v4().to_string();
        ApprovalPreview {
            command_kind: command_kind.to_string(),
            requested_change: requested_change.to_string(),
            current_value,
            proposed_value: proposed_value.to_string(),
            estimated_cost_impact,
            affected_workflows,
            reversibility: reversibility.clone(),
            preview_id,
            generated_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Describes whether a proposed change can be safely undone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reversibility {
    FullyReversible,
    PartiallyReversible,
    Irreversible,
}

impl fmt::Display for Reversibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reversibility::FullyReversible => write!(f, "fully_reversible"),
            Reversibility::PartiallyReversible => write!(f, "partially_reversible"),
            Reversibility::Irreversible => write!(f, "irreversible"),
        }
    }
}

// ─── Confidence & Evidence ──────────────────────────────────────────────────

/// Structured confidence result from classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceResult {
    pub score: f64,
    pub evidence: Vec<String>,
    pub reasoning: String,
}

impl ConfidenceResult {
    pub fn new(score: f64, evidence: Vec<String>, reasoning: &str) -> Self {
        ConfidenceResult {
            score,
            evidence,
            reasoning: reasoning.to_string(),
        }
    }

    pub fn is_confident(&self) -> bool {
        self.score >= 0.5
    }

    pub fn is_high_confidence(&self) -> bool {
        self.score >= 0.8
    }
}

// ─── Ambiguity ──────────────────────────────────────────────────────────────

/// Captures ambiguous user input that requires clarification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmbiguityResult {
    pub is_ambiguous: bool,
    pub reason: Option<String>,
    pub clarification_questions: Vec<String>,
}

impl AmbiguityResult {
    pub fn new(
        is_ambiguous: bool,
        reason: Option<String>,
        clarification_questions: Vec<String>,
    ) -> Self {
        AmbiguityResult {
            is_ambiguous,
            reason,
            clarification_questions,
        }
    }

    pub fn ambiguous(reason: &str, questions: Vec<String>) -> Self {
        AmbiguityResult {
            is_ambiguous: true,
            reason: Some(reason.to_string()),
            clarification_questions: questions,
        }
    }

    pub fn clear() -> Self {
        AmbiguityResult {
            is_ambiguous: false,
            reason: None,
            clarification_questions: Vec::new(),
        }
    }
}
