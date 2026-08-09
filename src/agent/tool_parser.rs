#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::Result;
use serde_json;

/// Parsed tool call extracted from LLM response.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Parse tool calls from LLM response text.
///
/// Supports multiple formats:
/// 1. XML-style: `<invoke name="tool">{"arg": "val"}</invoke>`
/// 2. Backtick-wrapped: `` `
/// 3. JSON-style: `{"name": "bash", "arguments": {"command": "ls"}}`
pub fn parse_tool_calls(response: &str) -> Result<Vec<ToolCall>> {
    let mut calls = Vec::new();

    // Try XML-style parsing first: <invoke name="tool">{"arg": "val"}</invoke>
    let xml_pattern = regex::Regex::new(r#"<invoke\s+name="([^"]+)"\s*>(.*?)</invoke>"#)?;
    for cap in xml_pattern.captures_iter(response) {
        let name = cap[1].to_string();
        let args_str = &cap[2];

        // Validate tool name (alphanumeric + underscore)
        if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }

        // Parse JSON arguments
        let args = args_str.trim().to_string();
        calls.push(ToolCall {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            arguments: args,
        });
    }

    // Try backtick-wrapped format: `
    if calls.is_empty() {
        let backtick_pattern = regex::Regex::new(r"`([^`]+)`")?;
        for cap in backtick_pattern.captures_iter(response) {
            let block = &cap[1];
            if block.trim().starts_with("<invoke") {
                if let Ok(name_match) = regex::Regex::new(r#"name="([^"]+)"#) {
                    if let Some(name_cap) = name_match.captures(block) {
                        let name = name_cap[1].to_string();
                        if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                            // Extract JSON arguments (everything after >)
                            if let Some(brace_pos) = block.find('{') {
                                let args = block[brace_pos..].trim().to_string();
                                calls.push(ToolCall {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    name,
                                    arguments: args,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // If no XML/backtick calls found, try to parse as JSON object
    if calls.is_empty() {
        // Find JSON objects by matching balanced braces
        let mut depth = 0;
        let mut start = None;
        let chars: Vec<char> = response.chars().collect();

        for (i, c) in chars.iter().enumerate() {
            match c {
                '{' => {
                    if depth == 0 {
                        start = Some(i);
                    }
                    depth += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(s) = start {
                            let json_str: String = chars[s..=i].iter().collect();
                            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                if let (Some(name), Some(args)) = (
                                    obj.get("name").and_then(|v| v.as_str()),
                                    obj.get("arguments"),
                                ) {
                                    if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                                        calls.push(ToolCall {
                                            id: uuid::Uuid::new_v4().to_string(),
                                            name: name.to_string(),
                                            arguments: serde_json::to_string(args)
                                                .unwrap_or_default(),
                                        });
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    Ok(calls)
}

/// Check if response contains tool calls.
pub fn has_tool_calls(response: &str) -> bool {
    !parse_tool_calls(response).unwrap_or_default().is_empty()
}

/// Extract pure text content (remove tool call tags).
pub fn extract_text(response: &str) -> String {
    // Remove XML-style tool calls
    let xml_pattern = regex::Regex::new(r"<invoke[^>]*>.*?</invoke>").expect("valid regex");
    let mut text = xml_pattern.replace_all(response, "").to_string();

    // Remove backtick-wrapped tool calls
    let backtick_pattern = regex::Regex::new(r"`[^`]+`").expect("valid regex");
    text = backtick_pattern.replace_all(&text, "").to_string();

    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_xml_tool_call() {
        let response = r#"Here's what I found:

<invoke name="bash">{"command": "ls -la"}</invoke>

The directory contains...
"#;
        let calls = parse_tool_calls(response).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert_eq!(calls[0].arguments, "{\"command\": \"ls -la\"}");
    }

    #[test]
    fn test_parse_backtick_tool_call() {
        let response = r#"Here's what I found:

<invoke name="read_file">{"path": "src/main.rs"}</invoke>

Let me read it.
"#;
        let calls = parse_tool_calls(response).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
    }

    #[test]
    fn test_parse_multiple_tool_calls() {
        let response = r#"
<invoke name="read_file">{"path": "src/main.rs"}</invoke>

And also:

<invoke name="run_command">{"command": "cat src/main.rs"}</invoke>
"#;
        let calls = parse_tool_calls(response).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[1].name, "run_command");
    }

    #[test]
    fn test_parse_json_tool_call() {
        let response = r#"I'll help you with that. Here is the call:
{"name": "bash", "arguments": {"command": "cargo test"}}
Let me run the tests.
"#;
        let calls = parse_tool_calls(response).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
    }

    #[test]
    fn test_no_tool_calls() {
        let response = "Just a regular response with no tools.";
        let calls = parse_tool_calls(response).unwrap();
        assert!(calls.is_empty());
    }

    #[test]
    fn test_extract_text_removes_tool_calls() {
        let response = r#"Before
<invoke name="bash">{"command": "ls"}</invoke>
After
"#;
        let text = extract_text(response);
        assert!(text.contains("Before"));
        assert!(text.contains("After"));
        assert!(!text.contains("<invoke"));
    }

    #[test]
    fn test_has_tool_calls() {
        assert!(has_tool_calls("<invoke name=\"bash\">test</invoke>"));
        assert!(!has_tool_calls("Just text"));
    }
}
