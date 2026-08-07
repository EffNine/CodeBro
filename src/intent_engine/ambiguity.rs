//! Ambiguity Detection — identifies unclear or underspecified user input.
//!
/// The classifier must never guess. Always clarify.
use super::types::*;

/// Detects ambiguity in user input and generates clarification questions.
///
/// Returns an `AmbiguityResult` indicating whether the input is ambiguous
/// and what questions should be asked to resolve it.
#[derive(Debug, Clone, Default)]
pub struct AmbiguityDetector {
    _private: (),
}

impl AmbiguityDetector {
    pub fn new() -> Self {
        AmbiguityDetector { _private: () }
    }

    /// Check an intent plan for ambiguity.
    ///
    /// Returns an `AmbiguityResult` with clarification questions if needed.
    pub fn detect(&self, plan: &IntentPlan) -> AmbiguityResult {
        if plan.ambiguity {
            return AmbiguityResult::ambiguous(
                plan.ambiguity_reason
                    .as_deref()
                    .unwrap_or("Unknown ambiguity"),
                self.generate_clarification_questions(&plan.intent_type, &plan.detected_goal),
            );
        }

        if plan.confidence < 0.5 {
            return AmbiguityResult::ambiguous(
                "Low confidence classification",
                self.generate_clarification_questions(&plan.intent_type, &plan.detected_goal),
            );
        }

        AmbiguityResult::clear()
    }

    /// Check raw input for ambiguity before classification.
    pub fn detect_input(&self, input: &str) -> AmbiguityResult {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return AmbiguityResult::ambiguous(
                "Empty input",
                vec!["Please describe what you would like to do.".to_string()],
            );
        }

        let lower = trimmed.to_lowercase();

        // Detect vague model references
        if lower.contains("use")
            && lower.contains("claude")
            && !lower.contains("claude-")
            && !lower.contains("claude ")
        {
            return AmbiguityResult::ambiguous(
                "Vague model reference: 'Claude' without version specified",
                vec![
                    "Which Claude model? (e.g., claude-3-opus, claude-3-sonnet, claude-3-haiku)"
                        .to_string(),
                ],
            );
        }

        if lower.contains("use") && lower.contains("gpt") && !lower.contains("gpt-") {
            return AmbiguityResult::ambiguous(
                "Vague model reference: 'GPT' without version specified",
                vec!["Which GPT model? (e.g., gpt-4, gpt-4o, gpt-4o-mini)".to_string()],
            );
        }

        if lower.contains("use") && lower.contains("llm") {
            return AmbiguityResult::ambiguous(
                "Vague LLM reference",
                vec!["Which LLM provider and model would you like to use?".to_string()],
            );
        }

        // Detect vague change requests
        if lower.contains("change") && (lower.contains("something") || lower.contains("anything")) {
            return AmbiguityResult::ambiguous(
                "Vague change request",
                vec![
                    "What specifically would you like to change?".to_string(),
                    "Which preference or setting should be modified?".to_string(),
                ],
            );
        }

        // Detect context-dependent commands
        if lower == "do it"
            || lower == "go ahead"
            || lower == "proceed"
            || lower == "make it happen"
        {
            return AmbiguityResult::ambiguous(
                "Context-dependent command without prior context",
                vec![
                    "What would you like me to do?".to_string(),
                    "Please describe the task or request.".to_string(),
                ],
            );
        }

        // Detect missing object in preference changes
        if (lower.contains("change") || lower.contains("update") || lower.contains("set"))
            && !lower.contains("to")
            && !lower.contains("the")
            && !lower.contains("my")
        {
            return AmbiguityResult::ambiguous(
                "Incomplete preference change request",
                vec![
                    "What would you like to change?".to_string(),
                    "What value would you like to set?".to_string(),
                ],
            );
        }

        AmbiguityResult::clear()
    }

    fn generate_clarification_questions(
        &self,
        intent_type: &IntentType,
        goal: &str,
    ) -> Vec<String> {
        match intent_type {
            IntentType::Unknown => vec![
                format!("Could you clarify what you mean by '{}?'", goal),
                "What would you like to accomplish?".to_string(),
            ],
            IntentType::Preference => vec![
                "Which preference would you like to change?".to_string(),
                "What value would you like to set?".to_string(),
            ],
            IntentType::Execution => vec![
                "What specific action would you like to perform?".to_string(),
                "Which command or operation should be executed?".to_string(),
            ],
            IntentType::Workflow => vec![
                "Which workflow would you like to run?".to_string(),
                "What is the goal of this workflow?".to_string(),
            ],
            _ => vec![format!("Could you clarify your request: '{}'?", goal)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent_engine::classifier::IntentClassifier;

    #[test]
    fn test_detect_ambiguous_model_reference() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("Use Claude.");
        assert!(result.is_ambiguous);
        assert!(result.clarification_questions.len() >= 1);
        assert!(result.reason.is_some());
    }

    #[test]
    fn test_detect_ambiguous_gpt_reference() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("Use GPT.");
        assert!(result.is_ambiguous);
    }

    #[test]
    fn test_detect_clear_model_reference() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("Use Claude-3-Opus.");
        assert!(!result.is_ambiguous);
    }

    #[test]
    fn test_detect_vague_change() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("Change to something better");
        assert!(result.is_ambiguous);
    }

    #[test]
    fn test_detect_context_dependent() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("Do it");
        assert!(result.is_ambiguous);
    }

    #[test]
    fn test_detect_empty_input() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("   ");
        assert!(result.is_ambiguous);
    }

    #[test]
    fn test_detect_clear_preference() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("Change the model to gpt-4o");
        assert!(!result.is_ambiguous);
    }

    #[test]
    fn test_detect_clear_question() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("How do I configure CodeBro?");
        assert!(!result.is_ambiguous);
    }

    #[test]
    fn test_detect_clear_help() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("help");
        assert!(!result.is_ambiguous);
    }

    #[test]
    fn test_detect_vague_llm() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("Use LLM");
        assert!(result.is_ambiguous);
    }

    #[test]
    fn test_detect_input_with_plan() {
        let classifier = IntentClassifier::new();
        let detector = AmbiguityDetector::new();

        let plan = classifier.classify("Use Claude.");
        let result = detector.detect(&plan);
        assert!(result.is_ambiguous);
    }

    #[test]
    fn test_detect_input_low_confidence() {
        let classifier = IntentClassifier::new();
        let detector = AmbiguityDetector::new();

        let plan = classifier.classify("xyz123 random gibberish");
        let result = detector.detect(&plan);
        assert!(result.is_ambiguous);
    }

    #[test]
    fn test_detect_clear_plan() {
        let classifier = IntentClassifier::new();
        let detector = AmbiguityDetector::new();

        let plan = classifier.classify("Change the model to gpt-4o");
        let result = detector.detect(&plan);
        assert!(!result.is_ambiguous);
    }
}
