#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Shared prompt builder for ChatGPT consultant providers.
//!
//! Both the extension-based and Playwright-based ChatGPT providers use the
//! same prompt format so that behavior is consistent regardless of transport.

use super::types::ConsultantRequest;

/// Build the prompt text sent to ChatGPT.
///
/// The prompt includes mode, optional project context, file contents, the
/// question, and formatting constraints. Prompt contents are **not** logged.
pub fn build_prompt(request: &ConsultantRequest) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push(format!("Mode: {}", request.mode));

    if request.include_project_context {
        if let Some(ctx) = &request.context {
            parts.push(ctx.clone());
        }
    }

    for file in &request.files {
        parts.push(format!("\n--- {} ---\n{}", file.path, file.content));
    }

    parts.push(format!("\nQuestion:\n{}", request.question));

    if request.max_answer_length > 0 {
        parts.push(format!(
            "\nMax answer length: {} characters.",
            request.max_answer_length
        ));
    }

    parts.push(
        "\n\nProvide a clear, structured answer. Do not modify the repository.\n".to_string(),
    );

    parts.join("\n")
}

/// Truncate an answer to the maximum length, cutting at a sentence boundary
/// when possible.
pub fn truncate_answer(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_len).collect();
    if let Some(pos) = truncated.rfind(". ") {
        let cut = pos + 2;
        if cut < truncated.len() {
            return truncated[..cut].to_string();
        }
    }
    format!("{truncated}…[truncated]")
}

#[cfg(test)]
mod tests {
    use super::super::types::{ConsultantFileContext, ConsultantMode, ConsultantRequest};
    use super::*;

    #[test]
    fn build_prompt_includes_question_and_mode() {
        let req = ConsultantRequest {
            question: "Should we refactor?".to_string(),
            mode: ConsultantMode::Architecture,
            ..Default::default()
        };
        let prompt = build_prompt(&req);
        assert!(prompt.contains("Should we refactor?"));
        assert!(prompt.contains("Mode: architecture"));
    }

    #[test]
    fn build_prompt_includes_files() {
        let req = ConsultantRequest {
            question: "Review this code".to_string(),
            files: vec![ConsultantFileContext {
                path: "src/main.rs".to_string(),
                content: "fn main() {}".to_string(),
            }],
            ..Default::default()
        };
        let prompt = build_prompt(&req);
        assert!(prompt.contains("src/main.rs"));
        assert!(prompt.contains("fn main()"));
    }

    #[test]
    fn build_prompt_includes_context() {
        let req = ConsultantRequest {
            question: "What do you think?".to_string(),
            context: Some("Project uses Rust.".to_string()),
            include_project_context: true,
            ..Default::default()
        };
        let prompt = build_prompt(&req);
        assert!(prompt.contains("Project uses Rust."));
    }

    #[test]
    fn build_prompt_excludes_context_when_disabled() {
        let req = ConsultantRequest {
            question: "Hello".to_string(),
            context: Some("Secret context".to_string()),
            include_project_context: false,
            ..Default::default()
        };
        let prompt = build_prompt(&req);
        assert!(!prompt.contains("Secret context"));
    }

    #[test]
    fn build_prompt_includes_max_length() {
        let req = ConsultantRequest {
            question: "Hello".to_string(),
            max_answer_length: 500,
            ..Default::default()
        };
        let prompt = build_prompt(&req);
        assert!(prompt.contains("500 characters"));
    }

    #[test]
    fn build_prompt_omits_max_length_when_zero() {
        let req = ConsultantRequest {
            question: "Hello".to_string(),
            max_answer_length: 0,
            ..Default::default()
        };
        let prompt = build_prompt(&req);
        assert!(!prompt.contains("Max answer length"));
    }
}
