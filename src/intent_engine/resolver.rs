//! Intent Resolver — converts Intent Plans into executable commands.
//!
/// Commands are immutable, deterministic, serializable, and auditable.
/// No command may modify state directly.
use super::types::*;
use serde::{Deserialize, Serialize};

/// Resolves an IntentPlan into a list of executable IntentCommands.
///
/// The resolver is a pure function: given the same plan, it always
/// produces the same commands. No state, no side effects.
#[derive(Debug, Clone, Default)]
pub struct IntentResolver {
    _private: (),
}

impl IntentResolver {
    pub fn new() -> Self {
        IntentResolver { _private: () }
    }

    /// Resolve an IntentPlan into a list of commands with metadata.
    ///
    /// Returns commands that can be presented to the Approval Gate.
    pub fn resolve(&self, plan: &IntentPlan) -> Vec<ResolvedCommand> {
        plan.required_commands
            .iter()
            .enumerate()
            .map(|(i, cmd)| self.resolve_command(cmd, plan, i))
            .collect()
    }

    /// Resolve a single command with full audit metadata.
    fn resolve_command(
        &self,
        command: &IntentCommand,
        plan: &IntentPlan,
        order: usize,
    ) -> ResolvedCommand {
        let metadata = CommandMetadata::new(
            "intent_engine",
            &plan.id,
            &plan.reasoning,
            &self.expected_effect(command),
        );

        ResolvedCommand {
            command: command.clone(),
            metadata,
            resolution_order: order,
        }
    }

    /// Describe the expected effect of a command for audit purposes.
    fn expected_effect(&self, command: &IntentCommand) -> String {
        match command {
            IntentCommand::UpdateModelPreference { key, new_value, .. } => {
                format!("Update preference '{}' to '{}'", key, new_value)
            }
            IntentCommand::UpdateLanguagePreference { key, new_value, .. } => {
                format!("Update preference '{}' to '{}'", key, new_value)
            }
            IntentCommand::UpdateCostPreference { key, new_value, .. } => {
                format!("Update preference '{}' to {}", key, new_value)
            }
            IntentCommand::UpdateApprovalPreference { key, new_value, .. } => {
                format!("Update preference '{}' to {}", key, new_value)
            }
            IntentCommand::ExecuteWorkflow { workflow_id, .. } => {
                format!("Execute workflow '{}'", workflow_id)
            }
            IntentCommand::ExecuteCommand { command, .. } => {
                format!("Execute command: {}", command)
            }
            IntentCommand::AnswerQuestion { question, .. } => {
                format!("Answer question: {}", question)
            }
            IntentCommand::ProvideHelp { topic, .. } => {
                format!("Provide help for topic: {}", topic)
            }
        }
    }
}

/// A resolved command with full audit metadata.
///
/// This is the final output of the resolver, ready for approval preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedCommand {
    pub command: IntentCommand,
    pub metadata: CommandMetadata,
    pub resolution_order: usize,
}

impl ResolvedCommand {
    /// Check if this command requires approval before execution.
    pub fn requires_approval(&self) -> bool {
        self.command.requires_approval()
    }

    /// Get the command kind for display/audit.
    pub fn kind(&self) -> &str {
        self.command.kind()
    }

    /// Get the expected effect for audit.
    pub fn expected_effect(&self) -> &str {
        &self.metadata.expected_effect
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_preference_command() {
        let resolver = IntentResolver::new();
        let plan = IntentPlan::new(
            "plan-1".to_string(),
            "Change model to gpt-4o",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model preference update",
            vec!["Rule match: model change".to_string()],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "User requested".to_string(),
            }],
        );

        let commands = resolver.resolve(&plan);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].kind(), "update_model_preference");
        assert!(commands[0].requires_approval());
        assert_eq!(commands[0].metadata.intent_id, "plan-1");
        assert!(!commands[0].metadata.timestamp.is_empty());
    }

    #[test]
    fn test_resolve_workflow_command() {
        let resolver = IntentResolver::new();
        let plan = IntentPlan::new(
            "plan-2".to_string(),
            "Run test workflow",
            IntentType::Workflow,
            "workflow_engine",
            true,
            0.5,
            0.85,
            false,
            None,
            "Workflow execution",
            vec!["Rule match: test workflow".to_string()],
            vec![IntentCommand::ExecuteWorkflow {
                workflow_id: "test_workflow".to_string(),
                reason: "User requested".to_string(),
            }],
        );

        let commands = resolver.resolve(&plan);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].kind(), "execute_workflow");
        assert!(commands[0].requires_approval());
    }

    #[test]
    fn test_resolve_question_command() {
        let resolver = IntentResolver::new();
        let plan = IntentPlan::new(
            "plan-3".to_string(),
            "What is rust?",
            IntentType::Question,
            "question_engine",
            false,
            0.0,
            0.9,
            false,
            None,
            "Question response",
            vec!["Rule match: question".to_string()],
            vec![IntentCommand::AnswerQuestion {
                question: "What is rust?".to_string(),
                answer: "Rust is a systems language.".to_string(),
            }],
        );

        let commands = resolver.resolve(&plan);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].kind(), "answer_question");
        assert!(!commands[0].requires_approval());
    }

    #[test]
    fn test_resolve_empty_commands() {
        let resolver = IntentResolver::new();
        let plan = IntentPlan::new(
            "plan-4".to_string(),
            "Unknown input",
            IntentType::Unknown,
            "unknown",
            false,
            0.0,
            0.1,
            true,
            Some("No match".to_string()),
            "No classification",
            vec!["No rule matched".to_string()],
            vec![],
        );

        let commands = resolver.resolve(&plan);
        assert!(commands.is_empty());
    }

    #[test]
    fn test_resolve_multiple_commands() {
        let resolver = IntentResolver::new();
        let plan = IntentPlan::new(
            "plan-5".to_string(),
            "Multi-step plan",
            IntentType::Execution,
            "execution",
            true,
            1.0,
            0.8,
            false,
            None,
            "Multiple operations",
            vec!["Rule match".to_string()],
            vec![
                IntentCommand::ExecuteCommand {
                    command: "echo step1".to_string(),
                    reason: "First step".to_string(),
                },
                IntentCommand::ExecuteCommand {
                    command: "echo step2".to_string(),
                    reason: "Second step".to_string(),
                },
            ],
        );

        let commands = resolver.resolve(&plan);
        assert_eq!(commands.len(), 2);
    }

    #[test]
    fn test_resolve_produces_immutable_commands() {
        let resolver = IntentResolver::new();
        let plan = IntentPlan::new(
            "plan-6".to_string(),
            "Test immutability",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Immutable test",
            vec!["Rule match".to_string()],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "claude".to_string(),
                reason: "User request".to_string(),
            }],
        );

        let commands1 = resolver.resolve(&plan);
        let commands2 = resolver.resolve(&plan);

        assert_eq!(commands1.len(), commands2.len());
        for (c1, c2) in commands1.iter().zip(commands2.iter()) {
            assert_eq!(c1.command, c2.command);
            assert_eq!(c1.metadata.intent_id, c2.metadata.intent_id);
            assert_eq!(c1.resolution_order, c2.resolution_order);
        }
    }

    #[test]
    fn test_resolve_serializable() {
        let resolver = IntentResolver::new();
        let plan = IntentPlan::new(
            "plan-7".to_string(),
            "Serialize test",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Serialize test",
            vec!["Rule match".to_string()],
            vec![IntentCommand::UpdateCostPreference {
                key: "max_cost".to_string(),
                new_value: 10.0,
                reason: "User request".to_string(),
            }],
        );

        let commands = resolver.resolve(&plan);
        let json = serde_json::to_string(&commands).expect("should serialize");
        let deserialized: Vec<ResolvedCommand> =
            serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(commands.len(), deserialized.len());
        assert_eq!(commands[0].kind(), deserialized[0].kind());
    }

    #[test]
    fn test_resolve_updates_resolution_order() {
        let resolver = IntentResolver::new();
        let plan = IntentPlan::new(
            "plan-8".to_string(),
            "Order test",
            IntentType::Execution,
            "execution",
            true,
            1.0,
            0.8,
            false,
            None,
            "Order test",
            vec!["Rule match".to_string()],
            vec![
                IntentCommand::ExecuteCommand {
                    command: "first".to_string(),
                    reason: "1st".to_string(),
                },
                IntentCommand::ExecuteCommand {
                    command: "second".to_string(),
                    reason: "2nd".to_string(),
                },
                IntentCommand::ExecuteCommand {
                    command: "third".to_string(),
                    reason: "3rd".to_string(),
                },
            ],
        );

        let commands = resolver.resolve(&plan);
        assert_eq!(commands[0].resolution_order, 0);
        assert_eq!(commands[1].resolution_order, 1);
        assert_eq!(commands[2].resolution_order, 2);
    }
}
