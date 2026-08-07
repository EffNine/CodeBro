//! Confidence Scoring — structured confidence results with evidence.
//!
/// Every classification returns a confidence score, evidence, and reasoning.
/// Low confidence triggers clarification.
use super::types::*;

/// Computes and returns a structured confidence result for an intent plan.
///
/// The confidence model considers:
/// - Rule match strength
/// - Command complexity
/// - Ambiguity detection
/// - Input length and completeness
#[derive(Debug, Clone, Default)]
pub struct ConfidenceModel {
    _private: (),
}

impl ConfidenceModel {
    pub fn new() -> Self {
        ConfidenceModel { _private: () }
    }

    /// Compute confidence for a classified intent plan.
    pub fn compute(&self, plan: &IntentPlan) -> ConfidenceResult {
        let mut evidence: Vec<String> = Vec::new();
        let mut score = plan.confidence;

        // Boost for non-ambiguous input
        if !plan.ambiguity {
            score = (score + 0.05).min(1.0);
            evidence.push("Input is unambiguous".to_string());
        } else {
            score = (score - 0.2).max(0.0);
            evidence.push("Input is ambiguous".to_string());
        }

        // Boost for commands present
        if !plan.required_commands.is_empty() {
            score = (score + 0.05).min(1.0);
            evidence.push(format!(
                "Generated {} commands",
                plan.required_commands.len()
            ));
        }

        // Penalty for unknown intent
        if matches!(plan.intent_type, IntentType::Unknown) {
            score = score.min(0.3);
            evidence.push("Intent type is unknown".to_string());
        }

        // Boost for clear goal
        if plan.detected_goal.trim().len() > 3 {
            score = (score + 0.02).min(1.0);
            evidence.push("Goal is clearly stated".to_string());
        }

        let reasoning = format!(
            "Confidence: {:.2} | Type: {} | Ambiguous: {} | Commands: {}",
            score,
            plan.intent_type,
            plan.ambiguity,
            plan.required_commands.len()
        );

        ConfidenceResult::new(score, evidence, &reasoning)
    }

    /// Compute confidence directly from input without a full plan.
    pub fn compute_from_input(&self, input: &str, intent_type: &IntentType) -> ConfidenceResult {
        let trimmed = input.trim();
        let mut score = match intent_type {
            IntentType::Help => 0.95,
            IntentType::Question => 0.85,
            IntentType::Preference => 0.8,
            IntentType::Execution => 0.75,
            IntentType::Workflow => 0.7,
            IntentType::Configuration => 0.65,
            IntentType::Unknown => 0.1,
        };

        let mut evidence = Vec::new();

        if trimmed.len() > 5 {
            score = (score + 0.03_f64).min(1.0_f64);
            evidence.push("Input length sufficient".to_string());
        }

        if trimmed.contains('?') {
            score = (score + 0.05).min(1.0);
            evidence.push("Question marker detected".to_string());
        }

        if trimmed.to_lowercase().starts_with("help") {
            evidence.push("Explicit help request".to_string());
        }

        let reasoning = format!(
            "Direct confidence: {:.2} | Type: {} | Length: {}",
            score,
            intent_type,
            trimmed.len()
        );

        ConfidenceResult::new(score, evidence, &reasoning)
    }

    /// Check if confidence is sufficient to proceed without clarification.
    pub fn is_sufficient(&self, result: &ConfidenceResult) -> bool {
        result.score >= 0.5
    }

    /// Check if confidence is high enough for automatic execution (no approval).
    pub fn is_high(&self, result: &ConfidenceResult) -> bool {
        result.score >= 0.8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent_engine::classifier::IntentClassifier;

    #[test]
    fn test_compute_high_confidence_preference() {
        let model = ConfidenceModel::new();
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Change the model to gpt-4o");
        let result = model.compute(&plan);

        assert!(result.is_confident());
        assert!(result.score >= 0.8);
        assert!(!result.evidence.is_empty());
    }

    #[test]
    fn test_compute_low_confidence_unknown() {
        let model = ConfidenceModel::new();
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("xyz random gibberish");
        let result = model.compute(&plan);

        assert!(!result.is_confident());
        assert!(result.score < 0.5);
    }

    #[test]
    fn test_compute_ambiguous_plan() {
        let model = ConfidenceModel::new();
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Use Claude.");
        let result = model.compute(&plan);

        assert!(!result.is_confident());
        assert!(result.evidence.iter().any(|e| e.contains("ambiguous")));
    }

    #[test]
    fn test_compute_help_high_confidence() {
        let model = ConfidenceModel::new();
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("help");
        let result = model.compute(&plan);

        assert!(result.is_confident());
        assert!(result.score >= 0.9);
    }

    #[test]
    fn test_compute_from_input() {
        let model = ConfidenceModel::new();
        let result = model.compute_from_input("Change model to gpt-4", &IntentType::Preference);

        assert!(result.is_confident());
        assert!(!result.evidence.is_empty());
    }

    #[test]
    fn test_compute_from_input_question() {
        let model = ConfidenceModel::new();
        let result = model.compute_from_input("What is rust?", &IntentType::Question);

        assert!(result.is_confident());
        assert!(result
            .evidence
            .iter()
            .any(|e| e.to_lowercase().contains("question")));
    }

    #[test]
    fn test_compute_from_input_unknown() {
        let model = ConfidenceModel::new();
        let result = model.compute_from_input("xyz", &IntentType::Unknown);

        assert!(!result.is_confident());
        assert!(result.score < 0.5);
    }

    #[test]
    fn test_is_sufficient_threshold() {
        let model = ConfidenceModel::new();
        let high = ConfidenceResult::new(0.9, vec![], "high");
        let low = ConfidenceResult::new(0.3, vec![], "low");

        assert!(model.is_sufficient(&high));
        assert!(!model.is_sufficient(&low));
    }

    #[test]
    fn test_is_high_threshold() {
        let model = ConfidenceModel::new();
        let high = ConfidenceResult::new(0.9, vec![], "high");
        let medium = ConfidenceResult::new(0.7, vec![], "medium");

        assert!(model.is_high(&high));
        assert!(!model.is_high(&medium));
    }

    #[test]
    fn test_compute_preserves_evidence() {
        let model = ConfidenceModel::new();
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Run the test workflow");
        let result = model.compute(&plan);

        assert!(!result.evidence.is_empty());
        assert!(result.evidence.iter().any(|e| e.contains("command")));
        assert!(result.reasoning.len() > 0);
    }
}
