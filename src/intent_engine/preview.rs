//! Approval Preview — read-only preview of proposed command changes.
//!
/// No state is modified during preview generation.
/// The preview is presented to the user before the Approval Gate.
use super::resolver::ResolvedCommand;
use super::types::*;
use std::collections::HashMap;

/// Generates a read-only approval preview for a resolved command.
///
/// The preview includes:
/// - requested_change: What the user asked for
/// - current_value: Current preference value (if available)
/// - proposed_value: The new value being requested
/// - estimated_cost_impact: Cost estimate
/// - affected_workflows: Workflows that may be impacted
/// - reversibility: Whether the change can be undone
#[derive(Debug, Clone, Default)]
pub struct ApprovalPreviewGenerator {
    _private: (),
}

impl ApprovalPreviewGenerator {
    pub fn new() -> Self {
        ApprovalPreviewGenerator { _private: () }
    }

    /// Generate a preview for a single resolved command.
    pub fn generate(
        &self,
        command: &ResolvedCommand,
        current_values: &HashMap<String, String>,
    ) -> ApprovalPreview {
        let requested_change = self.extract_requested_change(command);
        let (proposed_value, reversibility) = self.extract_proposed_value(command);
        let current_value = self.lookup_current_value(command, current_values);

        ApprovalPreview::new(
            command.kind(),
            &requested_change,
            current_value,
            &proposed_value,
            0.0,
            Vec::new(),
            reversibility,
        )
    }

    /// Generate previews for multiple resolved commands.
    pub fn generate_batch(
        &self,
        commands: &[ResolvedCommand],
        current_values: &HashMap<String, String>,
    ) -> Vec<ApprovalPreview> {
        commands
            .iter()
            .map(|cmd| self.generate(cmd, current_values))
            .collect()
    }

    fn extract_requested_change(&self, command: &ResolvedCommand) -> String {
        match &command.command {
            IntentCommand::UpdateModelPreference {
                key,
                new_value,
                reason,
            } => {
                format!(
                    "Update preference '{}': {} → {}",
                    key,
                    self.current_display(key),
                    new_value
                )
            }
            IntentCommand::UpdateLanguagePreference {
                key,
                new_value,
                reason,
            } => {
                format!(
                    "Update language preference '{}': {} → {}",
                    key,
                    self.current_display(key),
                    new_value
                )
            }
            IntentCommand::UpdateCostPreference {
                key,
                new_value,
                reason,
            } => {
                format!(
                    "Update cost preference '{}': {} → {}",
                    key,
                    self.current_display(key),
                    new_value
                )
            }
            IntentCommand::UpdateApprovalPreference {
                key,
                new_value,
                reason,
            } => {
                format!(
                    "Update approval preference '{}': {} → {}",
                    key,
                    self.current_display(key),
                    new_value
                )
            }
            IntentCommand::ExecuteWorkflow {
                workflow_id,
                reason,
            } => {
                format!("Execute workflow: {}", workflow_id)
            }
            IntentCommand::ExecuteCommand { command, reason } => {
                format!("Execute command: {}", command)
            }
            IntentCommand::AnswerQuestion { question, answer } => {
                format!("Answer: {}", question)
            }
            IntentCommand::ProvideHelp { topic, help_text } => {
                format!("Help: {}", topic)
            }
        }
    }

    fn extract_proposed_value(&self, command: &ResolvedCommand) -> (String, Reversibility) {
        match &command.command {
            IntentCommand::UpdateModelPreference { new_value, .. } => {
                (new_value.clone(), Reversibility::FullyReversible)
            }
            IntentCommand::UpdateLanguagePreference { new_value, .. } => {
                (new_value.clone(), Reversibility::FullyReversible)
            }
            IntentCommand::UpdateCostPreference { new_value, .. } => {
                (format!("{}", new_value), Reversibility::FullyReversible)
            }
            IntentCommand::UpdateApprovalPreference { new_value, .. } => {
                (format!("{}", new_value), Reversibility::FullyReversible)
            }
            IntentCommand::ExecuteWorkflow { .. } => (
                "workflow execution".to_string(),
                Reversibility::PartiallyReversible,
            ),
            IntentCommand::ExecuteCommand { .. } => (
                "command execution".to_string(),
                Reversibility::PartiallyReversible,
            ),
            IntentCommand::AnswerQuestion { .. } => (
                "informational response".to_string(),
                Reversibility::FullyReversible,
            ),
            IntentCommand::ProvideHelp { .. } => (
                "informational help".to_string(),
                Reversibility::FullyReversible,
            ),
        }
    }

    fn lookup_current_value(
        &self,
        command: &ResolvedCommand,
        current_values: &HashMap<String, String>,
    ) -> Option<String> {
        match &command.command {
            IntentCommand::UpdateModelPreference { key, .. }
            | IntentCommand::UpdateLanguagePreference { key, .. }
            | IntentCommand::UpdateCostPreference { key, .. }
            | IntentCommand::UpdateApprovalPreference { key, .. } => {
                current_values.get(key).cloned()
            }
            _ => None,
        }
    }

    fn current_display(&self, key: &str) -> String {
        // Placeholder — real implementation would query PreferenceStore
        format!("<{}>", key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preview_model_preference() {
        let generator = ApprovalPreviewGenerator::new();
        let current_values = HashMap::from([("model".to_string(), "gpt-4o".to_string())]);

        let resolved = ResolvedCommand {
            command: IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "claude-3".to_string(),
                reason: "User requested".to_string(),
            },
            metadata: CommandMetadata::new("test", "plan-1", "test", "update model"),
            resolution_order: 0,
        };

        let preview = generator.generate(&resolved, &current_values);
        assert!(preview.requested_change.contains("model"));
        assert_eq!(preview.proposed_value, "claude-3");
        assert_eq!(preview.current_value, Some("gpt-4o".to_string()));
        assert!(matches!(
            preview.reversibility,
            Reversibility::FullyReversible
        ));
    }

    #[test]
    fn test_preview_cost_preference() {
        let generator = ApprovalPreviewGenerator::new();
        let current_values =
            HashMap::from([("max_cost_per_session".to_string(), "5.0".to_string())]);

        let resolved = ResolvedCommand {
            command: IntentCommand::UpdateCostPreference {
                key: "max_cost_per_session".to_string(),
                new_value: 10.0,
                reason: "User requested".to_string(),
            },
            metadata: CommandMetadata::new("test", "plan-1", "test", "update cost"),
            resolution_order: 0,
        };

        let preview = generator.generate(&resolved, &current_values);
        assert_eq!(preview.proposed_value, "10");
        assert_eq!(preview.current_value, Some("5.0".to_string()));
        assert!(preview.command_kind.contains("cost"));
    }

    #[test]
    fn test_preview_workflow() {
        let generator = ApprovalPreviewGenerator::new();
        let current_values = HashMap::new();

        let resolved = ResolvedCommand {
            command: IntentCommand::ExecuteWorkflow {
                workflow_id: "test_workflow".to_string(),
                reason: "User requested".to_string(),
            },
            metadata: CommandMetadata::new("test", "plan-1", "test", "run workflow"),
            resolution_order: 0,
        };

        let preview = generator.generate(&resolved, &current_values);
        assert!(preview.requested_change.contains("test_workflow"));
        assert!(matches!(
            preview.reversibility,
            Reversibility::PartiallyReversible
        ));
    }

    #[test]
    fn test_preview_question_no_reversibility_issue() {
        let generator = ApprovalPreviewGenerator::new();
        let current_values = HashMap::new();

        let resolved = ResolvedCommand {
            command: IntentCommand::AnswerQuestion {
                question: "What is rust?".to_string(),
                answer: "A systems language".to_string(),
            },
            metadata: CommandMetadata::new("test", "plan-1", "test", "answer"),
            resolution_order: 0,
        };

        let preview = generator.generate(&resolved, &current_values);
        assert!(preview.requested_change.contains("What is rust?"));
        assert!(matches!(
            preview.reversibility,
            Reversibility::FullyReversible
        ));
    }

    #[test]
    fn test_preview_batch() {
        let generator = ApprovalPreviewGenerator::new();
        let current_values = HashMap::new();

        let commands = vec![
            ResolvedCommand {
                command: IntentCommand::UpdateModelPreference {
                    key: "model".to_string(),
                    new_value: "gpt-4".to_string(),
                    reason: "test".to_string(),
                },
                metadata: CommandMetadata::new("test", "p1", "test", "update"),
                resolution_order: 0,
            },
            ResolvedCommand {
                command: IntentCommand::AnswerQuestion {
                    question: "Q?".to_string(),
                    answer: "A".to_string(),
                },
                metadata: CommandMetadata::new("test", "p2", "test", "answer"),
                resolution_order: 1,
            },
        ];

        let previews = generator.generate_batch(&commands, &current_values);
        assert_eq!(previews.len(), 2);
        assert!(previews[0].command_kind.contains("model"));
        assert!(previews[1].command_kind.contains("question"));
    }

    #[test]
    fn test_preview_serializable() {
        let generator = ApprovalPreviewGenerator::new();
        let current_values = HashMap::new();

        let resolved = ResolvedCommand {
            command: IntentCommand::UpdateApprovalPreference {
                key: "auto_approve".to_string(),
                new_value: true,
                reason: "test".to_string(),
            },
            metadata: CommandMetadata::new("test", "p1", "test", "update"),
            resolution_order: 0,
        };

        let preview = generator.generate(&resolved, &current_values);
        let json = serde_json::to_string(&preview).expect("should serialize");
        let deserialized: ApprovalPreview =
            serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(preview.command_kind, deserialized.command_kind);
        assert_eq!(preview.proposed_value, deserialized.proposed_value);
    }

    #[test]
    fn test_preview_id_is_unique() {
        let generator = ApprovalPreviewGenerator::new();
        let current_values = HashMap::new();

        let resolved1 = ResolvedCommand {
            command: IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "a".to_string(),
                reason: "t".to_string(),
            },
            metadata: CommandMetadata::new("test", "p1", "t", "e"),
            resolution_order: 0,
        };

        let resolved2 = ResolvedCommand {
            command: IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "b".to_string(),
                reason: "t".to_string(),
            },
            metadata: CommandMetadata::new("test", "p2", "t", "e"),
            resolution_order: 0,
        };

        let p1 = generator.generate(&resolved1, &current_values);
        let p2 = generator.generate(&resolved2, &current_values);
        assert_ne!(p1.preview_id, p2.preview_id);
    }

    #[test]
    fn test_preview_timestamp_present() {
        let generator = ApprovalPreviewGenerator::new();
        let current_values = HashMap::new();

        let resolved = ResolvedCommand {
            command: IntentCommand::AnswerQuestion {
                question: "Q".to_string(),
                answer: "A".to_string(),
            },
            metadata: CommandMetadata::new("test", "p1", "t", "e"),
            resolution_order: 0,
        };

        let preview = generator.generate(&resolved, &current_values);
        assert!(!preview.generated_at.is_empty());
        assert!(preview.generated_at.contains("T"));
    }
}
