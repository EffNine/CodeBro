//! Intent Classifier — deterministic classification using rules and patterns.
//!
//! LLM fallback is architecture only; not implemented in this phase.
//! Every classification is explainable and auditable.

use super::types::*;
use regex::Regex;
use std::collections::HashMap;
use uuid::Uuid;

/// Pattern rules for deterministic classification.
///
/// Each rule has a regex pattern, the intent type it maps to,
/// a confidence boost, and optional evidence text.
#[derive(Debug, Clone)]
struct ClassificationRule {
    pattern: Regex,
    intent_type: IntentType,
    confidence_boost: f64,
    evidence_template: &'static str,
    requires_approval: bool,
}

impl ClassificationRule {
    fn new(
        pattern: &str,
        intent_type: IntentType,
        confidence_boost: f64,
        evidence_template: &'static str,
        requires_approval: bool,
    ) -> Self {
        ClassificationRule {
            pattern: Regex::new(pattern).unwrap(),
            intent_type,
            confidence_boost,
            evidence_template,
            requires_approval,
        }
    }
}

/// The deterministic intent classifier.
///
/// Pure function: same input always produces same output.
/// No state, no LLM calls, no side effects.
#[derive(Debug, Clone, Default)]
pub struct IntentClassifier {
    rules: Vec<ClassificationRule>,
}

impl IntentClassifier {
    pub fn new() -> Self {
        let mut classifier = IntentClassifier::default();
        classifier.load_rules();
        classifier
    }

    /// Classify a user input string into an IntentPlan.
    ///
    /// Returns a fully structured plan with confidence, evidence, and commands.
    pub fn classify(&self, input: &str) -> IntentPlan {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return self.unknown_plan("Empty input");
        }

        let mut best_match: Option<(f64, &ClassificationRule, Vec<String>)> = None;
        let mut all_evidence: Vec<String> = Vec::new();

        for rule in &self.rules {
            if rule.pattern.is_match(trimmed) {
                let evidence = vec![format!("Matched pattern: {}", rule.evidence_template)];
                all_evidence.extend(evidence.clone());

                match &best_match {
                    Some((current_score, _, _)) if *current_score >= rule.confidence_boost => {
                        // Keep current best
                    }
                    _ => {
                        best_match = Some((rule.confidence_boost, rule, evidence));
                    }
                }
            }
        }

        match best_match {
            Some((score, rule, evidence)) => {
                let requires_approval = rule.requires_approval;
                let commands = self.generate_commands(&rule.intent_type, trimmed);
                let confidence = Self::compute_confidence(score, &commands);

                IntentPlan::new(
                    Uuid::new_v4().to_string(),
                    trimmed,
                    rule.intent_type.clone(),
                    &self.affected_subsystem(&rule.intent_type),
                    requires_approval,
                    self.estimate_cost(&rule.intent_type, trimmed),
                    confidence,
                    false,
                    None,
                    &format!("Classified as {} via rule matching", rule.intent_type),
                    evidence,
                    commands,
                )
            }
            None => self.unknown_plan(trimmed),
        }
    }

    /// Classify with explicit intent type override (used by resolver for known intents).
    pub fn classify_with_type(&self, input: &str, intent_type: IntentType) -> IntentPlan {
        let commands = self.generate_commands(&intent_type, input);
        let confidence = Self::compute_confidence(0.9, &commands);

        IntentPlan::new(
            Uuid::new_v4().to_string(),
            input,
            intent_type,
            &self.affected_subsystem(&IntentType::Preference),
            true,
            self.estimate_cost(&IntentType::Preference, input),
            confidence,
            false,
            None,
            "Explicit intent classification",
            vec!["Intent type provided explicitly".to_string()],
            commands,
        )
    }

    // ─── Rule Loading ─────────────────────────────────────────────────────

    fn load_rules(&mut self) {
        // Preference intent patterns
        self.rules.push(ClassificationRule::new(
            r"(?i)(change|update|set|switch|use|prefer|make|switch\s+to)\s+(the\s+)?(?:model|provider|LLM)\s*(?:to|as)?\s*(.+)",
            IntentType::Preference,
            0.9,
            "Preference change: model/provider switching",
            true,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(change|update|set|switch|use)\s+(my\s+)?(?:language|lang)\s*(?:to|as)?\s*(.+)",
            IntentType::Preference,
            0.85,
            "Preference change: language",
            true,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(change|update|set)\s+(my\s+)?(?:cost|budget|spending|limit)\s*(?:to|at|of|:)?\s*(.+)",
            IntentType::Preference,
            0.85,
            "Preference change: cost/budget",
            true,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(enable|disable|turn\s+(on|off)|set\s+(on|off))\s+(auto\s+)?(?:approve|approval)\s*(?:mode)?",
            IntentType::Preference,
            0.85,
            "Preference change: approval settings",
            true,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(update|set|change)\s+(my\s+)?(?:preference|preferences?)\s*(?:of)?\s*(.+)",
            IntentType::Preference,
            0.8,
            "Preference change: generic",
            true,
        ));

        // Configuration intent patterns
        self.rules.push(ClassificationRule::new(
            r"(?i)(configure|setup|config(?:uration)?|settings?)\s+(the\s+)?(?:system|app|project|tool)",
            IntentType::Configuration,
            0.8,
            "System configuration",
            false,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(set\s+up|configure|init(?:ialize)?)\s+(?:the\s+)?(?:workspace|project)",
            IntentType::Configuration,
            0.75,
            "Workspace/project configuration",
            false,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(add|install|register)\s+(a\s+)?(?:provider|model|tool|plugin)",
            IntentType::Configuration,
            0.8,
            "Adding a configuration item",
            false,
        ));

        // Workflow intent patterns
        self.rules.push(ClassificationRule::new(
            r"(?i)(run|execute|start|launch|begin|trigger)\s+(the\s+)?(?:workflow|pipeline|process)",
            IntentType::Workflow,
            0.85,
            "Workflow execution",
            true,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(create|generate|build|produce)\s+(a\s+)?(?:workflow|pipeline|process)\s+for\s*(.+)",
            IntentType::Workflow,
            0.8,
            "Workflow creation",
            true,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(run|execute)\s+(the\s+)?(?:test|build|deploy)\s*(workflow)?",
            IntentType::Workflow,
            0.85,
            "Standard workflow (test/build/deploy)",
            true,
        ));

        // Execution intent patterns
        self.rules.push(ClassificationRule::new(
            r"(?i)(run|execute|run\s+the|run\s+a|perform|do)\s+(?:the\s+)?(?:command|operation|task)\s*(?:of|:)?\s*(.+)",
            IntentType::Execution,
            0.9,
            "Command execution",
            true,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(edit|modify|change|update)\s+(the\s+)?(?:file|code|source)\s*(?:at|in)?\s*(.+)",
            IntentType::Execution,
            0.85,
            "File/code modification",
            true,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(read|show|display|print|output)\s+(the\s+)?(?:file|content|document|code)",
            IntentType::Execution,
            0.9,
            "Read/display operation",
            false,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(create|write)\s+(a\s+)?(?:file|document|new\s+file)",
            IntentType::Execution,
            0.85,
            "File creation",
            true,
        ));

        // Question intent patterns
        self.rules.push(ClassificationRule::new(
            r"(?i)^(how|what|why|when|where|who|can|should|will)\s+(.+)\??$",
            IntentType::Question,
            0.85,
            "Question detected by interrogative pattern",
            false,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(explain|describe|clarify|tell\s+me\s+about)\s+(the\s+)?(.+)",
            IntentType::Question,
            0.8,
            "Explanation request",
            false,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(what\s+is|what\s+are|what\s+does)\s+(the\s+)?(.+)",
            IntentType::Question,
            0.8,
            "Definition request",
            false,
        ));

        // Help intent patterns
        self.rules.push(ClassificationRule::new(
            r"(?i)^help(?:\s+(.+))?$",
            IntentType::Help,
            0.95,
            "Help request",
            false,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(show\s+me\s+)?(?:help|commands?|options?|usage|manual|guide)",
            IntentType::Help,
            0.9,
            "Help/commands request",
            false,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)(how\s+do\s+I?|how\s+can\s+I?|what\s+can\s+I?)\s+(do|use)\s*(.+)?",
            IntentType::Help,
            0.85,
            "How-to help request",
            false,
        ));

        // Ambiguity patterns — these increase ambiguity detection
        self.rules.push(ClassificationRule::new(
            r"(?i)^use\s+(claude|gpt|gemini|llama|llm|model|provider)\s*$",
            IntentType::Unknown,
            0.2,
            "Ambiguous model reference without specifics",
            false,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)^(change|update|switch)\s+(to)\s+(something|anything|better|faster)\s*$",
            IntentType::Unknown,
            0.15,
            "Vague change request",
            false,
        ));
        self.rules.push(ClassificationRule::new(
            r"(?i)^(do\s+it|go\s+ahead|proceed|continue|make\s+it\s+happen)\s*$",
            IntentType::Unknown,
            0.1,
            "Context-dependent ambiguous command",
            false,
        ));
    }

    // ─── Command Generation ─────────────────────────────────────────────────

    fn generate_commands(&self, intent_type: &IntentType, input: &str) -> Vec<IntentCommand> {
        match intent_type {
            IntentType::Preference => self.generate_preference_commands(input),
            IntentType::Configuration => self.generate_configuration_commands(input),
            IntentType::Workflow => self.generate_workflow_commands(input),
            IntentType::Execution => self.generate_execution_commands(input),
            IntentType::Question => self.generate_question_commands(input),
            IntentType::Help => self.generate_help_commands(input),
            IntentType::Unknown => vec![],
        }
    }

    fn generate_preference_commands(&self, input: &str) -> Vec<IntentCommand> {
        let lower = input.to_lowercase();

        if lower.contains("model") || lower.contains("provider") || lower.contains("llm") {
            let value = self.extract_value(&lower, &["model", "provider", "to", "as"]);
            return vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: value,
                reason: format!("User requested model/provider change: {}", input),
            }];
        }

        if lower.contains("language") || lower.contains("lang") {
            let value = self.extract_value(&lower, &["language", "lang", "to", "as"]);
            return vec![IntentCommand::UpdateLanguagePreference {
                key: "language".to_string(),
                new_value: value,
                reason: format!("User requested language change: {}", input),
            }];
        }

        if lower.contains("cost") || lower.contains("budget") || lower.contains("spending") {
            let value = self.extract_numeric_value(&lower);
            return vec![IntentCommand::UpdateCostPreference {
                key: "max_cost_per_session".to_string(),
                new_value: value,
                reason: format!("User requested cost change: {}", input),
            }];
        }

        if lower.contains("approve") || lower.contains("approval") {
            let value =
                lower.contains("enable") || lower.contains("turn on") || lower.contains("set on");
            return vec![IntentCommand::UpdateApprovalPreference {
                key: "auto_approve_safe_ops".to_string(),
                new_value: value,
                reason: format!("User requested approval change: {}", input),
            }];
        }

        vec![IntentCommand::UpdateModelPreference {
            key: "preference".to_string(),
            new_value: input.to_string(),
            reason: format!("Generic preference update: {}", input),
        }]
    }

    fn generate_configuration_commands(&self, input: &str) -> Vec<IntentCommand> {
        vec![IntentCommand::ExecuteCommand {
            command: format!("configure: {}", input),
            reason: format!("User requested configuration: {}", input),
        }]
    }

    fn generate_workflow_commands(&self, input: &str) -> Vec<IntentCommand> {
        let workflow_id = self.extract_workflow_id(input);
        vec![IntentCommand::ExecuteWorkflow {
            workflow_id,
            reason: format!("User requested workflow: {}", input),
        }]
    }

    fn generate_execution_commands(&self, input: &str) -> Vec<IntentCommand> {
        vec![IntentCommand::ExecuteCommand {
            command: input.to_string(),
            reason: format!("User requested execution: {}", input),
        }]
    }

    fn generate_question_commands(&self, input: &str) -> Vec<IntentCommand> {
        vec![IntentCommand::AnswerQuestion {
            question: input.trim_end_matches('?').to_string(),
            answer: format!("Question received: {}", input),
        }]
    }

    fn generate_help_commands(&self, input: &str) -> Vec<IntentCommand> {
        let topic = if input.trim().to_lowercase() == "help" {
            "general".to_string()
        } else {
            input.trim().to_lowercase()
        };
        vec![IntentCommand::ProvideHelp {
            topic,
            help_text: "CodeBro help: Use natural language to manage preferences, execute commands, and run workflows. All changes require approval.".to_string(),
        }]
    }

    // ─── Helpers ────────────────────────────────────────────────────────────

    fn extract_value(&self, lower: &str, skip_words: &[&str]) -> String {
        // Build a regex that matches any of the skip words as whole words
        let mut patterns: Vec<String> = skip_words
            .iter()
            .map(|w| format!(r"\b{}\b", regex::escape(w)))
            .collect();
        // Also add common stop words
        let stop_words = [
            "the", "a", "an", "to", "for", "of", "and", "or", "but", "in", "on", "at", "by",
            "with", "from", "is", "are", "was", "were", "be", "been", "being", "have", "has",
            "had", "do", "does", "did", "will", "would", "shall", "should", "may", "might", "must",
            "can", "could", "not", "no", "nor", "so", "if", "then", "than", "too", "very", "just",
            "about", "up", "out", "into", "over", "after", "before", "between", "through",
            "during", "above", "below", "set", "change", "update", "make", "use", "switch",
        ];
        for word in stop_words {
            patterns.push(format!(r"\b{}\b", regex::escape(word)));
        }
        let pattern = patterns.join("|");
        match Regex::new(&format!(r"(?i){}", pattern)) {
            Ok(re) => re.replace_all(lower, "").into_owned().trim().to_string(),
            Err(_) => lower.trim().to_string(),
        }
    }

    fn extract_numeric_value(&self, lower: &str) -> f64 {
        let digits: String = lower
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        digits.parse::<f64>().unwrap_or(5.0)
    }

    fn extract_workflow_id(&self, input: &str) -> String {
        let lower = input.to_lowercase();
        if lower.contains("test") {
            "test_workflow".to_string()
        } else if lower.contains("build") {
            "build_workflow".to_string()
        } else if lower.contains("deploy") {
            "deploy_workflow".to_string()
        } else {
            Uuid::new_v4().to_string()
        }
    }

    fn affected_subsystem(&self, intent_type: &IntentType) -> String {
        match intent_type {
            IntentType::Preference => "preference_engine".to_string(),
            IntentType::Configuration => "configuration".to_string(),
            IntentType::Workflow => "workflow_engine".to_string(),
            IntentType::Execution => "execution".to_string(),
            IntentType::Question => "question_engine".to_string(),
            IntentType::Help => "help_system".to_string(),
            IntentType::Unknown => "unknown".to_string(),
        }
    }

    fn estimate_cost(&self, intent_type: &IntentType, _input: &str) -> f64 {
        match intent_type {
            IntentType::Preference => 0.0,
            IntentType::Configuration => 0.0,
            IntentType::Workflow => 0.5,
            IntentType::Execution => 1.0,
            IntentType::Question => 0.0,
            IntentType::Help => 0.0,
            IntentType::Unknown => 0.0,
        }
    }

    fn compute_confidence(base_score: f64, commands: &[IntentCommand]) -> f64 {
        let command_bonus = (commands.len() as f64 * 0.02).min(0.1);
        (base_score + command_bonus).min(1.0)
    }

    fn unknown_plan(&self, input: &str) -> IntentPlan {
        IntentPlan::new(
            Uuid::new_v4().to_string(),
            input,
            IntentType::Unknown,
            "unknown",
            false,
            0.0,
            0.1,
            true,
            Some("No deterministic rule matched this input".to_string()),
            "No matching classification rule found",
            vec!["Input did not match any known pattern".to_string()],
            vec![],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classifier_preference_model() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Change the model to gpt-4o");
        assert_eq!(plan.intent_type, IntentType::Preference);
        assert!(plan.confidence >= 0.5);
        assert!(!plan.ambiguity);
        assert_eq!(plan.required_commands.len(), 1);
        assert!(matches!(
            &plan.required_commands[0],
            IntentCommand::UpdateModelPreference { .. }
        ));
    }

    #[test]
    fn test_classifier_preference_language() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Set language to french");
        assert_eq!(plan.intent_type, IntentType::Preference);
        assert!(matches!(
            &plan.required_commands[0],
            IntentCommand::UpdateLanguagePreference { .. }
        ));
    }

    #[test]
    fn test_classifier_preference_cost() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Update cost limit to 10.50");
        assert_eq!(plan.intent_type, IntentType::Preference);
        assert!(matches!(
            &plan.required_commands[0],
            IntentCommand::UpdateCostPreference { new_value, .. } if *new_value == 10.5
        ));
    }

    #[test]
    fn test_classifier_preference_approval() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Enable auto approve");
        assert_eq!(plan.intent_type, IntentType::Preference);
        assert!(matches!(
            &plan.required_commands[0],
            IntentCommand::UpdateApprovalPreference {
                new_value: true,
                ..
            }
        ));
    }

    #[test]
    fn test_classifier_configuration() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Configure the system settings");
        assert_eq!(plan.intent_type, IntentType::Configuration);
        assert!(!plan.required_approval);
    }

    #[test]
    fn test_classifier_workflow() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Run the test workflow");
        assert_eq!(plan.intent_type, IntentType::Workflow);
        assert!(plan.required_approval);
        if let IntentCommand::ExecuteWorkflow { workflow_id, .. } = &plan.required_commands[0] {
            assert_eq!(workflow_id, "test_workflow");
        } else {
            panic!("Expected ExecuteWorkflow command");
        }
    }

    #[test]
    fn test_classifier_execution() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Execute the command cargo test");
        assert_eq!(plan.intent_type, IntentType::Execution);
        assert!(plan.required_approval);
        if let IntentCommand::ExecuteCommand { command, .. } = &plan.required_commands[0] {
            assert_eq!(command, "Execute the command cargo test");
        } else {
            panic!("Expected ExecuteCommand command");
        }
    }

    #[test]
    fn test_classifier_question() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("How does the approval gate work?");
        assert_eq!(plan.intent_type, IntentType::Question);
        assert!(!plan.required_approval);
        assert!(matches!(
            &plan.required_commands[0],
            IntentCommand::AnswerQuestion { .. }
        ));
    }

    #[test]
    fn test_classifier_help() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("help");
        assert_eq!(plan.intent_type, IntentType::Help);
        assert!(!plan.required_approval);
        assert!(matches!(
            &plan.required_commands[0],
            IntentCommand::ProvideHelp { .. }
        ));
    }

    #[test]
    fn test_classifier_ambiguous_model_reference() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Use Claude.");
        assert_eq!(plan.intent_type, IntentType::Unknown);
        assert!(plan.ambiguity);
        assert!(plan.confidence < 0.5);
    }

    #[test]
    fn test_classifier_ambiguous_vague_change() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Change to something better");
        assert_eq!(plan.intent_type, IntentType::Unknown);
        assert!(plan.ambiguity);
    }

    #[test]
    fn test_classifier_empty_input() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("   ");
        assert_eq!(plan.intent_type, IntentType::Unknown);
        assert!(plan.ambiguity);
    }

    #[test]
    fn test_classifier_unrecognized_input() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("xyz123 random gibberish");
        assert_eq!(plan.intent_type, IntentType::Unknown);
        assert!(plan.confidence < 0.5);
    }

    #[test]
    fn test_classifier_is_actionable() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Change the model to gpt-4o");
        assert!(plan.is_actionable());

        let ambiguous = classifier.classify("Use Claude.");
        assert!(!ambiguous.is_actionable());
    }

    #[test]
    fn test_classifier_case_insensitive() {
        let classifier = IntentClassifier::new();
        let plan1 = classifier.classify("change the model to gpt-4o");
        let plan2 = classifier.classify("CHANGE THE MODEL TO GPT-4O");
        assert_eq!(plan1.intent_type, plan2.intent_type);
        assert_eq!(plan1.required_commands.len(), plan2.required_commands.len());
    }

    #[test]
    fn test_classifier_command_requires_approval() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Change the model to gpt-4o");
        assert!(plan.required_commands[0].requires_approval());
    }

    #[test]
    fn test_classifier_help_no_approval() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("help");
        assert!(!plan.required_commands[0].requires_approval());
    }

    #[test]
    fn test_classifier_question_no_approval() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("What is rust?");
        assert!(!plan.required_commands[0].requires_approval());
    }

    #[test]
    fn test_classifier_detects_multiple_preference_types() {
        let classifier = IntentClassifier::new();

        let model_plan = classifier.classify("Switch provider to openai");
        assert!(matches!(
            &model_plan.required_commands[0],
            IntentCommand::UpdateModelPreference { .. }
        ));

        let lang_plan = classifier.classify("Change language to spanish");
        assert!(matches!(
            &lang_plan.required_commands[0],
            IntentCommand::UpdateLanguagePreference { .. }
        ));

        let cost_plan = classifier.classify("Set cost budget to 3.0");
        assert!(matches!(
            &cost_plan.required_commands[0],
            IntentCommand::UpdateCostPreference { .. }
        ));

        let approval_plan = classifier.classify("Disable auto approve");
        assert!(matches!(
            &approval_plan.required_commands[0],
            IntentCommand::UpdateApprovalPreference {
                new_value: false,
                ..
            }
        ));
    }

    #[test]
    fn test_classifier_reasoning_is_explained() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Run the build workflow");
        assert!(!plan.reasoning.is_empty());
        assert!(!plan.evidence.is_empty());
    }

    #[test]
    fn test_classifier_id_is_unique() {
        let classifier = IntentClassifier::new();
        let plan1 = classifier.classify("Test one");
        let plan2 = classifier.classify("Test two");
        assert_ne!(plan1.id, plan2.id);
    }

    #[test]
    fn test_classifier_timestamp_present() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("help");
        assert!(!plan.created_at.is_empty());
        assert!(plan.created_at.contains("T"));
    }
}
