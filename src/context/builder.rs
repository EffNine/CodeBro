#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use crate::indexer::RepositoryIndex;
use crate::scanner::ProjectInfo;
use crate::tools::Tool;
use std::collections::HashMap;

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct ContextConfig {
    pub token_budget: usize,
    pub max_files: usize,
    pub max_file_size: usize,
    pub include_conversation: bool,
    pub include_project_summary: bool,
    pub include_dependencies: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        ContextConfig {
            token_budget: 8000,
            max_files: 20,
            max_file_size: 2000,
            include_conversation: true,
            include_project_summary: true,
            include_dependencies: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Context {
    pub system_prompt: String,
    pub conversation: Vec<ContextMessage>,
    pub project_summary: Option<String>,
    pub relevant_files: Vec<ContextFile>,
    pub user_request: String,
    pub total_tokens: usize,
}

#[derive(Debug, Clone)]
pub struct ContextMessage {
    pub role: String,
    pub content: String,
    pub tokens: usize,
}

#[derive(Debug, Clone)]
pub struct ContextFile {
    pub path: String,
    pub language: String,
    pub content: String,
    pub relevance: f32,
    pub tokens: usize,
}

pub struct ContextBuilder {
    config: ContextConfig,
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ContextBuilder {
    pub fn new(config: ContextConfig) -> Self {
        ContextBuilder {
            config,
            tools: HashMap::new(),
        }
    }

    pub fn with_tool(mut self, name: String, tool: Box<dyn Tool>) -> Self {
        self.tools.insert(name, tool);
        self
    }

    pub fn build(
        &self,
        user_input: &str,
        conversation: &[ContextMessage],
        project_info: Option<&ProjectInfo>,
        index: &RepositoryIndex,
    ) -> Result<Context> {
        let relevant_files = self.select_relevant_files(user_input, index)?;
        let project_summary = if self.config.include_project_summary {
            project_info.map(|p| self.format_project_summary(p))
        } else {
            None
        };

        let conversation = if self.config.include_conversation {
            self.truncate_conversation(conversation)
        } else {
            Vec::new()
        };

        let mut total_tokens = self.estimate_tokens(user_input);
        total_tokens += project_summary
            .as_ref()
            .map(|s| self.estimate_tokens(s))
            .unwrap_or(0);
        total_tokens += conversation.iter().map(|m| m.tokens).sum::<usize>();

        let mut selected_files = Vec::new();
        for file in relevant_files {
            if total_tokens >= self.config.token_budget {
                break;
            }
            if selected_files.len() >= self.config.max_files {
                break;
            }

            let file_tokens = self.estimate_tokens(&file.content);
            if file_tokens > self.config.max_file_size {
                continue;
            }

            if total_tokens + file_tokens > self.config.token_budget {
                continue;
            }

            total_tokens += file_tokens;
            selected_files.push(file);
        }

        Ok(Context {
            system_prompt: self.default_system_prompt(),
            conversation,
            project_summary,
            relevant_files: selected_files,
            user_request: user_input.to_string(),
            total_tokens,
        })
    }

    fn select_relevant_files(
        &self,
        query: &str,
        index: &RepositoryIndex,
    ) -> Result<Vec<ContextFile>> {
        let query_lower = query.to_lowercase();
        let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(&crate::indexer::IndexEntry, f32)> = index
            .entries
            .iter()
            .filter(|e| !e.ignored)
            .map(|entry| {
                let path_lower = entry.path.to_lowercase();
                let content = std::fs::read_to_string(&entry.path)
                    .unwrap_or_default()
                    .to_lowercase();

                let mut score: f32 = 0.0;

                for term in &query_terms {
                    if path_lower.contains(term) {
                        score += 2.0;
                    }
                    if content.contains(term) {
                        score += 1.0;
                    }
                }

                let path_parts: Vec<&str> = entry.path.split('/').collect();
                for part in &path_parts {
                    for term in &query_terms {
                        if part.contains(term) {
                            score += 1.5;
                        }
                    }
                }

                let important_dirs = vec!["src", "lib", "app", "core", "main"];
                for dir in &important_dirs {
                    if path_parts.contains(dir) {
                        score += 0.5;
                    }
                }

                let important_exts = ["rs", "ts", "tsx", "js", "jsx", "py", "go"];
                if let Some(ext) = entry.path.rsplit('.').next() {
                    if important_exts.contains(&ext) {
                        score += 0.5;
                    }
                }

                let penalties = vec![
                    "test",
                    "spec",
                    ".test.",
                    ".spec.",
                    "node_modules",
                    "vendor",
                    "target",
                    "dist",
                    "build",
                ];
                for penalty in &penalties {
                    if path_lower.contains(penalty) {
                        score -= 2.0;
                    }
                }

                (entry, score)
            })
            .filter(|(_, score)| *score > 0.0)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let top_files: Vec<_> = scored
            .into_iter()
            .take(self.config.max_files)
            .filter_map(|(entry, _)| {
                let content = std::fs::read_to_string(&entry.path).ok()?;
                Some(ContextFile {
                    path: entry.path.clone(),
                    language: entry.language.clone(),
                    content,
                    relevance: 0.0,
                    tokens: 0,
                })
            })
            .collect();

        Ok(top_files)
    }

    fn truncate_conversation(&self, conversation: &[ContextMessage]) -> Vec<ContextMessage> {
        let mut truncated = Vec::new();
        let mut tokens = 0;
        let max_conv_tokens = self.config.token_budget / 3;

        for msg in conversation.iter().rev() {
            if tokens + msg.tokens > max_conv_tokens {
                break;
            }
            tokens += msg.tokens;
            truncated.insert(0, msg.clone());
        }

        truncated
    }

    fn format_project_summary(&self, project: &ProjectInfo) -> String {
        let mut summary = format!(
            "Project: {} (Language: {})\n",
            project.name, project.language
        );

        if let Some(ref framework) = project.framework {
            summary.push_str(&format!("Framework: {}\n", framework));
        }

        if let Some(ref build_system) = project.build_system {
            summary.push_str(&format!("Build System: {}\n", build_system));
        }

        if let Some(ref pkg_mgr) = project.package_manager {
            summary.push_str(&format!("Package Manager: {}\n", pkg_mgr));
        }

        if let Some(ref testing) = project.testing_framework {
            summary.push_str(&format!("Testing: {}\n", testing));
        }

        if !project.important_files.is_empty() {
            summary.push_str("Important Files:\n");
            for file in &project.important_files {
                summary.push_str(&format!("  - {}\n", file));
            }
        }

        summary
    }

    fn default_system_prompt(&self) -> String {
        r#"You are CodeBro, an AI coding assistant operating inside a developer's terminal.

Your capabilities:
- Read, create, and edit files in the repository
- Execute shell commands (with user awareness)
- Inspect git status and diffs
- Understand project structure and context

Your constraints:
- Never expose secrets, API keys, or credentials
- Never run destructive commands without explicit user confirmation
- Always explain what you are about to do before doing it
- Ask for clarification when requirements are ambiguous
- Prefer minimal, targeted changes over large rewrites

Your output format:
- Use clear, structured responses
- Show code blocks with proper language tags
- Explain trade-offs when multiple approaches exist
- Provide commands the user can run directly"#
            .to_string()
    }

    fn estimate_tokens(&self, text: &str) -> usize {
        text.len() / 4
    }
}
