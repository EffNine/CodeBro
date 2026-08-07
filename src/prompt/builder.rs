#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use crate::context::{Context, ContextConfig};
use crate::scanner::ProjectInfo;

pub struct PromptBuilder {
    config: ContextConfig,
}

impl PromptBuilder {
    pub fn new(config: ContextConfig) -> Self {
        PromptBuilder { config }
    }

    pub fn build(&self, context: &Context, _project_info: Option<&ProjectInfo>) -> String {
        let mut prompt = String::new();

        if !context.system_prompt.is_empty() {
            prompt.push_str("=== SYSTEM PROMPT ===\n");
            prompt.push_str(&context.system_prompt);
            prompt.push_str("\n\n");
        }

        if let Some(ref summary) = context.project_summary {
            prompt.push_str("=== PROJECT SUMMARY ===\n");
            prompt.push_str(summary);
            prompt.push_str("\n\n");
        }

        if !context.conversation.is_empty() {
            prompt.push_str("=== CONVERSATION HISTORY ===\n");
            for msg in &context.conversation {
                let role = match msg.role.as_str() {
                    "user" => "USER",
                    "assistant" => "ASSISTANT",
                    "system" => "SYSTEM",
                    _ => "UNKNOWN",
                };
                prompt.push_str(&format!("[{}]: {}\n", role, msg.content));
            }
            prompt.push_str("\n");
        }

        if !context.relevant_files.is_empty() {
            prompt.push_str("=== RELEVANT FILES ===\n");
            for file in &context.relevant_files {
                prompt.push_str(&format!("--- {} ({}) ---\n", file.path, file.language));
                prompt.push_str(&file.content);
                prompt.push_str("\n\n");
            }
        }

        prompt.push_str("=== USER REQUEST ===\n");
        prompt.push_str(context.user_request.as_str());
        prompt.push_str("\n");

        prompt
    }

    pub fn build_with_recent_files(
        &self,
        context: &Context,
        recent_files: &[String],
        project_info: Option<&ProjectInfo>,
    ) -> String {
        let mut prompt = self.build(context, project_info);

        if !recent_files.is_empty() {
            prompt.push_str("\n=== RECENT FILES ===\n");
            for file in recent_files {
                prompt.push_str(&format!("- {}\n", file));
            }
        }

        prompt
    }
}
