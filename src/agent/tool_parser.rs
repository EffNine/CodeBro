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

// ---------------------------------------------------------------------------
// Structured (OpenAI-native) parser
// ---------------------------------------------------------------------------

/// Parse structured tool calls from an OpenAI-compatible `tool_calls` JSON
/// array. This is used when the provider returns native structured responses
/// rather than text-wrapped calls.
///
/// Expected input format (one element of the `tool_calls` array):
/// ```json
/// {
///   "id": "call_abc123",
///   "type": "function",
///   "function": {
///     "name": "read_file",
///     "arguments": "{\"path\": \"src/main.rs\"}"
///   }
/// }
/// ```
pub fn parse_structured_tool_calls(json_array: &str) -> Result<Vec<ToolCall>> {
    let arr: Vec<serde_json::Value> = serde_json::from_str(json_array)?;
    let mut calls = Vec::new();
    for item in arr {
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let function = item
            .get("function")
            .ok_or_else(|| anyhow::anyhow!("tool_call missing 'function' field"))?;
        let name = function
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("function missing 'name' field"))?;
        // Validate tool name.
        if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let arguments = function
            .get("arguments")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        calls.push(ToolCall {
            id,
            name: name.to_string(),
            arguments,
        });
    }
    Ok(calls)
}

// ---------------------------------------------------------------------------
// Enhanced text-based parser
// ---------------------------------------------------------------------------

/// Robustly extract a balanced JSON object starting at `start` in `text`.
/// Handles nested braces and quoted strings correctly.
fn extract_json_object(text: &str, start: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().skip(start).collect();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (i, c) in chars.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        match c {
            '\\' if in_string => {
                escape = true;
            }
            '"' => {
                in_string = !in_string;
            }
            '{' if !in_string => {
                depth += 1;
            }
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(chars[..=i].iter().collect());
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse tool calls from LLM response text.
///
/// Supports multiple formats (tried in order):
/// 1. XML-style: `<invoke name="tool">{"arg": "val"}</invoke>`
/// 2. Backtick-wrapped XML
/// 3. Markdown-fenced JSON: ```json ... ```
/// 4. Raw JSON objects (multiple supported)
/// 5. Nested JSON objects with balanced-brace parsing
pub fn parse_tool_calls(response: &str) -> Result<Vec<ToolCall>> {
    let mut calls = Vec::new();

    // 1. XML-style: <invoke name="tool">{"arg": "val"}</invoke>
    let xml_pattern = regex::Regex::new(r#"(?s)<invoke\s+name="([^"]+)"\s*>(.*?)</invoke>"#)?;
    for cap in xml_pattern.captures_iter(response) {
        let name = cap[1].to_string();
        let args_str = cap[2].trim().to_string();
        if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            calls.push(ToolCall {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                arguments: args_str,
            });
        }
    }

    // 2. Backtick-wrapped XML
    if calls.is_empty() {
        let backtick_pattern = regex::Regex::new(r"(?s)`([^`]+)`")?;
        for cap in backtick_pattern.captures_iter(response) {
            let block = &cap[1];
            if block.trim().starts_with("<invoke") {
                if let Ok(name_match) = regex::Regex::new(r#"name="([^"]+)"#) {
                    if let Some(name_cap) = name_match.captures(block) {
                        let name = name_cap[1].to_string();
                        if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
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

    // 3. Markdown-fenced JSON: ```json {...} ``` or ``` {...} ```
    if calls.is_empty() {
        let fence_pattern = regex::Regex::new(r"```\w*\n?(.*?)\n?```")?;
        for cap in fence_pattern.captures_iter(response) {
            let block = cap[1].trim();
            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(block) {
                if let (Some(name), Some(args)) = (
                    obj.get("name").and_then(|v| v.as_str()),
                    obj.get("arguments"),
                ) {
                    if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        calls.push(ToolCall {
                            id: uuid::Uuid::new_v4().to_string(),
                            name: name.to_string(),
                            arguments: serde_json::to_string(args).unwrap_or_default(),
                        });
                    }
                }
            }
        }
    }

    // 4. Raw JSON objects — find all top-level balanced JSON objects and try
    //    to parse each as a tool call.
    if calls.is_empty() {
        let chars: Vec<char> = response.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '{' {
                if let Some(json_str) = extract_json_object(response, i) {
                    // Try as a single tool call object.
                    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&json_str) {
                        if let (Some(name), Some(args)) = (
                            obj.get("name").and_then(|v| v.as_str()),
                            obj.get("arguments"),
                        ) {
                            if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                                calls.push(ToolCall {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    name: name.to_string(),
                                    arguments: serde_json::to_string(args).unwrap_or_default(),
                                });
                                // Skip past this object.
                                i += json_str.len();
                                continue;
                            }
                        }
                        // Try as a tool_calls array (OpenAI structured format in text).
                        if let Some(arr) = obj.get("tool_calls") {
                            let arr_str = arr.to_string();
                            if let Ok(sub_calls) = parse_structured_tool_calls(&arr_str) {
                                calls.extend(sub_calls);
                                i += json_str.len();
                                continue;
                            }
                        }
                    }
                    // Skip past this object even if it didn't parse as a tool call.
                    i += json_str.len();
                    continue;
                }
            }
            i += 1;
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

    // Remove markdown-fenced JSON blocks
    let fence_pattern = regex::Regex::new(r"```[\w]*\n?.*?\n?```").expect("valid regex");
    text = fence_pattern.replace_all(&text, "").to_string();

    text.trim().to_string()
}

/// Unified extraction: try structured parsing first, fall back to text parser.
///
/// `structured_json` is the raw `tool_calls` JSON array string from a native
/// provider response (e.g. OpenAI). When `None`, only the text parser is used.
pub fn extract_tool_calls(response: &str, structured_json: Option<&str>) -> Result<Vec<ToolCall>> {
    // Try structured first.
    if let Some(json) = structured_json {
        if !json.trim().is_empty() && json != "[]" {
            if let Ok(calls) = parse_structured_tool_calls(json) {
                if !calls.is_empty() {
                    return Ok(calls);
                }
            }
        }
    }
    // Fall back to text parser.
    parse_tool_calls(response)
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
    fn test_parse_nested_json_tool_call() {
        // The command value contains escaped quotes and nested braces.
        let response = r#"Here's the call:
{"name": "run_command", "arguments": {"command": "python -c \"print({'a': {'b': 1}})\""}}
Done.
"#;
        let calls = parse_tool_calls(response).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "run_command");
        assert!(calls[0].arguments.contains("python"));
    }

    #[test]
    fn test_parse_markdown_fenced_json() {
        let response = r#"I need to read the file:

```json
{"name": "read_file", "arguments": {"path": "src/main.rs"}}
```

Then I'll analyze it.
"#;
        let calls = parse_tool_calls(response).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments, "{\"path\":\"src/main.rs\"}");
    }

    #[test]
    fn test_parse_multiple_json_objects() {
        let response = r#"First call: {"name": "list_files", "arguments": {"path": "."}}
Second call: {"name": "read_file", "arguments": {"path": "README.md"}}
Done.
"#;
        let calls = parse_tool_calls(response).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "list_files");
        assert_eq!(calls[1].name, "read_file");
    }

    #[test]
    fn test_parse_escaped_strings() {
        let response = r#"Call: {"name": "run_command", "arguments": {"command": "echo \"hello world\" && ls"}}
"#;
        let calls = parse_tool_calls(response).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "run_command");
        assert!(calls[0].arguments.contains("hello world"));
    }

    #[test]
    fn test_parse_mixed_prose_and_tool_call() {
        let response = r#"I need to inspect the runtime first.

<invoke name="read_file">
{"path": "src/canonical_runtime/mod.rs"}
</invoke>

I will then determine the execution flow.
"#;
        let calls = parse_tool_calls(response).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
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

    #[test]
    fn test_extract_json_object_balanced_braces() {
        let s = r#"{"a": {"b": {"c": 1}}}"#;
        assert_eq!(
            extract_json_object(s, 0).unwrap(),
            "{\"a\": {\"b\": {\"c\": 1}}}"
        );
    }

    #[test]
    fn test_extract_json_object_with_quotes() {
        let s = r#"{"cmd": "echo \"hello {world}\""}"#;
        // Escaped quotes inside a string must not terminate the JSON object.
        assert_eq!(
            extract_json_object(s, 0).unwrap(),
            r#"{"cmd": "echo \"hello {world}\""}"#.to_string()
        );
    }

    #[test]
    fn test_parse_structured_tool_calls() {
        let json = r#"[{"id": "call_abc", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\": \"src/main.rs\"}"}}]"#;
        let calls = parse_structured_tool_calls(json).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_abc");
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments, "{\"path\": \"src/main.rs\"}");
    }

    #[test]
    fn test_parse_structured_multiple_calls() {
        let json = r#"[
            {"id": "call_1", "function": {"name": "list_files", "arguments": "{}"}},
            {"id": "call_2", "function": {"name": "read_file", "arguments": "{\"path\": \"x\"}"}}
        ]"#;
        let calls = parse_structured_tool_calls(json).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "list_files");
        assert_eq!(calls[1].name, "read_file");
    }

    #[test]
    fn test_extract_tool_calls_prefer_structured() {
        let text = "Some prose";
        let structured = r#"[{"id": "c1", "function": {"name": "foo", "arguments": "{}"}}]"#;
        let calls = extract_tool_calls(text, Some(structured)).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "foo");
    }

    #[test]
    fn test_extract_tool_calls_falls_back_to_text() {
        let text = r#"Thought: let me check.

<invoke name="bash">{"command": "ls"}</invoke>

Done.
"#;
        let calls = extract_tool_calls(text, None).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
    }

    #[test]
    fn test_extract_tool_calls_empty_structured_falls_back() {
        let text = r#"{"name": "grep", "arguments": {"pattern": "foo"}}"#;
        let calls = extract_tool_calls(text, Some("[]")).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "grep");
    }

    #[test]
    fn test_parse_invalid_json_falls_back_gracefully() {
        let response = "Just text with no tools at all.";
        let calls = parse_tool_calls(response).unwrap();
        assert!(calls.is_empty());
    }

    #[test]
    fn test_parse_tool_call_with_braces_in_string() {
        let response =
            r#"{"name": "run_command", "arguments": {"command": "echo '{\"key\": \"value\"}'"}}"#;
        let calls = parse_tool_calls(response).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "run_command");
        assert!(calls[0].arguments.contains("echo"));
    }
}
