//! Shared provider-execution primitive for the canonical runtime.
//!
//! Both the main ReAct loop and the autonomous Research subagent drive the
//! provider through this single primitive. Reusing it guarantees that every
//! execution path shares the exact same circuit-breaker gate, health
//! reporting, retry policy, cancellation, deadline and structured-tool-calling
//! behaviour — the Research subagent never gets a weaker or different
//! execution model than the main agent.

use std::collections::HashMap;
use std::sync::Arc;

use crate::agent::tool_parser::ToolCall;
use crate::provider_runtime::routing::ProviderRoutingDecision;
use crate::provider_runtime::{ProviderId, ProviderRuntime, RetryController, TokenUsage};
use crate::providers::{Provider, StructuredToolCall, ToolDefinition};

use super::TaskOptions;

/// What the reasoning loop must do with a provider response.
///
/// This is the single three-state classification the ReAct loops use:
///
/// ```text
/// model response
///      │
///      ├── usable final text (no usable tool calls)   → Final(text)
///      ├── usable tool calls                          → Execute(calls)
///      └── neither                                    → Empty(reason)
/// ```
///
/// STATE 1 (final text) and STATE 3 (nothing usable) are deliberately
/// distinct: a valid text-only answer like `OK` terminates successfully with
/// zero tool calls, while a response with no usable text and no usable tool
/// calls terminates as a bounded error instead of consuming another
/// reasoning iteration.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ResponseDisposition {
    /// Usable final text and no usable tool calls → the task is complete.
    Final(String),
    /// Usable tool calls → execute them, feed the observations back, iterate.
    Execute(Vec<ToolCall>),
    /// Neither usable text nor usable tool calls → bounded error. Never
    /// silently continue.
    Empty(String),
}

/// Classify a provider response into the three loop states.
///
/// A tool call is *usable* when it carries a non-empty name. Structured
/// providers sometimes stream placeholder tool-call deltas (an index with an
/// empty name/arguments) even when the response finishes with prose; such
/// calls are malformed and must never be executed as "Unknown tool" nor
/// allowed to consume a reasoning iteration. When the response has usable
/// text and only malformed/empty calls, the text IS the final answer
/// (this implements `finish_reason == "stop"` semantics: the provider
/// finished, the answer is already in hand). When neither exists, the
/// response is empty/malformed and the loop must terminate honestly.
pub(crate) fn classify_response(
    full: &str,
    structured: Vec<StructuredToolCall>,
) -> ResponseDisposition {
    let mut parsed: Vec<ToolCall> = if !structured.is_empty() {
        structured
            .into_iter()
            .map(|c| ToolCall {
                id: c.id,
                name: c.name,
                arguments: c.arguments,
            })
            .collect()
    } else {
        crate::agent::tool_parser::parse_tool_calls(full).unwrap_or_default()
    };
    // Normalize structured and text-parsed tool calls into the same internal
    // representation (the `{"input": ...}` envelope unwrap is a no-op for raw
    // argument strings).
    for call in parsed.iter_mut() {
        call.arguments = crate::agent::tool_parser::unwrap_tool_arguments(&call.arguments);
    }
    let usable: Vec<ToolCall> = parsed
        .into_iter()
        .filter(|c| !c.name.trim().is_empty())
        .collect();
    if !usable.is_empty() {
        return ResponseDisposition::Execute(usable);
    }
    if !full.trim().is_empty() {
        return ResponseDisposition::Final(full.to_string());
    }
    ResponseDisposition::Empty(
        "Model returned no usable response (empty text and no usable tool calls).".to_string(),
    )
}

/// Stream a response from the routed provider with circuit breaker gate,
/// health reporting and retry policy. Never bypasses the circuit breaker.
///
/// Both stages — provider invocation (`stream_response`) and chunk
/// consumption (`rx.recv`) — are guarded by the same
/// cancellation/deadline mechanism so that an in-flight provider can be
/// interrupted promptly. Cancellation and deadline errors terminate the
/// caller immediately and are never retried; only genuine provider failures
/// enter the existing retry / circuit-breaker path.
pub(crate) async fn stream_once(
    provider_runtime: &ProviderRuntime,
    io_providers: &HashMap<ProviderId, Arc<dyn Provider>>,
    decision: &ProviderRoutingDecision,
    prompt: &str,
    tools: &[ToolDefinition],
    on_chunk: &(dyn Fn(&str) + Send + Sync),
    opts: &TaskOptions,
) -> std::result::Result<(String, Vec<StructuredToolCall>), String> {
    let provider_id = decision.provider_id().clone();

    let breaker = provider_runtime
        .circuit_breakers()
        .get_or_create(&provider_id);
    if !breaker.can_execute() {
        provider_runtime.report_failure(&provider_id);
        return Err(format!(
            "Circuit breaker open for {} ({:?})",
            provider_id,
            breaker.state()
        ));
    }

    let io = io_providers
        .get(&provider_id)
        .cloned()
        .ok_or_else(|| format!("No provider handler registered for {provider_id}"))?;

    // Decide the tool-calling mode: if the provider supports native
    // function calling and we have tool definitions, send them. Otherwise
    // fall back to the plain text protocol (structured calls stay empty
    // and the text parser is used downstream).
    let use_structured = io.supports_function_calling() && !tools.is_empty();

    let policy = provider_runtime.retry_policy().clone();
    let mut retry = RetryController::new(policy);
    let mut attempt = 0usize;

    loop {
        // 1. Cooperative cancellation / deadline check before each
        //    provider invocation. If either has already fired we exit
        //    immediately — no retry.
        if let Some(cancel) = &opts.cancel {
            if cancel.is_cancelled() {
                return Err("Task cancelled".to_string());
            }
        }
        if let Some(dl) = opts.deadline {
            if dl <= tokio::time::Instant::now() {
                return Err("Task timed out".to_string());
            }
        }

        // 2. Await the provider, concurrently monitoring cancellation
        //    and deadline. We use `tokio::select!` with three
        //    arms: (a) the provider call, (b) a cancellation
        //    waiter, (c) a deadline watcher. All three run
        //    concurrently; whichever fires first wins.
        let response = if use_structured {
            let structured_fut = io.stream_response_with_tools(prompt, tools);
            tokio::select! {
                result = structured_fut => result,
                _ = async {
                    if let Some(cancel) = &opts.cancel {
                        cancel.cancelled().await;
                    } else {
                        futures::future::pending::<()>().await;
                    }
                } => return Err("Task cancelled".to_string()),
                _ = async {
                    match opts.deadline {
                        Some(dl) => tokio::time::sleep_until(dl).await,
                        None => {
                            futures::future::pending::<()>().await;
                        }
                    }
                } => return Err("Task timed out".to_string()),
            }
        } else {
            let plain_fut = io.stream_response(prompt);
            let rx = tokio::select! {
                result = plain_fut => result,
                _ = async {
                    if let Some(cancel) = &opts.cancel {
                        cancel.cancelled().await;
                    } else {
                        futures::future::pending::<()>().await;
                    }
                } => return Err("Task cancelled".to_string()),
                _ = async {
                    match opts.deadline {
                        Some(dl) => tokio::time::sleep_until(dl).await,
                        None => {
                            futures::future::pending::<()>().await;
                        }
                    }
                } => return Err("Task timed out".to_string()),
            };
            match rx {
                Ok(rx) => {
                    // Receive chunks, concurrently monitoring cancellation
                    // and deadline.
                    let mut full = String::new();
                    let mut rx = rx;
                    loop {
                        tokio::select! {
                            chunk_opt = rx.recv() => {
                                match chunk_opt {
                                    Some(chunk) => {
                                        full.push_str(&chunk);
                                        on_chunk(&chunk);
                                    }
                                    None => break, // Channel closed normally.
                                }
                            }
                            _ = async {
                                if let Some(cancel) = &opts.cancel {
                                    cancel.cancelled().await;
                                } else {
                                    futures::future::pending::<()>().await;
                                }
                            } => {
                                return Err("Task cancelled".to_string());
                            }
                            _ = async {
                                match opts.deadline {
                                    Some(dl) => tokio::time::sleep_until(dl).await,
                                    None => {
                                        futures::future::pending::<()>().await;
                                    }
                                }
                            } => {
                                return Err("Task timed out".to_string());
                            }
                        }
                    }
                    Ok((full, Vec::new()))
                }
                Err(e) => {
                    // Only retry on genuine provider errors.
                    attempt += 1;
                    provider_runtime.report_failure(&provider_id);
                    match retry.next_attempt(std::time::Duration::ZERO, &provider_id) {
                        Ok(delay) => {
                            if !delay.is_zero() {
                                tokio::time::sleep(delay).await;
                            }
                            continue;
                        }
                        Err(_) => {
                            return Err(format!(
                                "Provider {} failed after {} attempt(s): {}",
                                provider_id, attempt, e
                            ));
                        }
                    }
                }
            }
        };

        // 3. Handle the structured response.
        let (full, structured) = match response {
            Ok(result) => result,
            Err(e) => {
                // Only retry on genuine provider errors.
                attempt += 1;
                provider_runtime.report_failure(&provider_id);
                match retry.next_attempt(std::time::Duration::ZERO, &provider_id) {
                    Ok(delay) => {
                        if !delay.is_zero() {
                            tokio::time::sleep(delay).await;
                        }
                        continue;
                    }
                    Err(_) => {
                        return Err(format!(
                            "Provider {} failed after {} attempt(s): {}",
                            provider_id, attempt, e
                        ));
                    }
                }
            }
        };

        let tokens = TokenUsage::new(prompt.len() / 4, full.len() / 4);
        provider_runtime.report_success(
            &provider_id,
            tokens,
            crate::provider_runtime::ProviderCost::default(),
        );
        return Ok((full, structured));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::StructuredToolCall;

    fn call(name: &str) -> StructuredToolCall {
        StructuredToolCall {
            id: "c1".to_string(),
            name: name.to_string(),
            arguments: r#"{"input": "x"}"#.to_string(),
        }
    }

    #[test]
    fn test_text_only_is_final() {
        // STATE 1: usable text, no tool calls → Final.
        let d = classify_response("OK", vec![]);
        assert_eq!(d, ResponseDisposition::Final("OK".to_string()));
    }

    #[test]
    fn test_whitespace_only_text_is_not_final() {
        // Whitespace is not an answer.
        let d = classify_response("\n\n  \n", vec![]);
        assert_eq!(
            d,
            ResponseDisposition::Empty(
                "Model returned no usable response (empty text and no usable tool calls)."
                    .to_string()
            )
        );
    }

    #[test]
    fn test_empty_response_is_bounded_error_not_continue() {
        // STATE 3: no text, no calls → Empty (never silently continue).
        let d = classify_response("", vec![]);
        assert!(matches!(d, ResponseDisposition::Empty(_)));
    }

    #[test]
    fn test_structured_call_is_execute() {
        // STATE 2: usable structured call → Execute.
        let d = classify_response("", vec![call("list_files")]);
        match d {
            ResponseDisposition::Execute(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "list_files");
                // The `{"input": ...}` envelope is unwrapped.
                assert_eq!(calls[0].arguments, "x");
            }
            other => panic!("expected Execute, got {:?}", other),
        }
    }

    #[test]
    fn test_text_with_tool_call_is_execute() {
        // ReAct: text + usable tool calls → execute and continue.
        let d = classify_response("Let me check.", vec![call("list_files")]);
        assert!(matches!(d, ResponseDisposition::Execute(_)));
    }

    #[test]
    fn test_text_parsed_tool_call_is_execute() {
        // Text-protocol tool call.
        let d = classify_response(
            r#"<invoke name="list_files">{"path": "."}</invoke>"#,
            vec![],
        );
        match d {
            ResponseDisposition::Execute(calls) => {
                assert_eq!(calls[0].name, "list_files");
            }
            other => panic!("expected Execute, got {:?}", other),
        }
    }

    #[test]
    fn test_placeholder_call_with_text_is_final() {
        // A provider that streams a placeholder tool-call delta (empty name)
        // but finishes with prose: the text IS the final answer. This is the
        // `finish_reason == "stop"` over placeholder calls case.
        let d = classify_response(
            "The answer is OK.",
            vec![StructuredToolCall {
                id: "p1".to_string(),
                name: String::new(),
                arguments: String::new(),
            }],
        );
        assert_eq!(
            d,
            ResponseDisposition::Final("The answer is OK.".to_string())
        );
    }

    #[test]
    fn test_placeholder_call_without_text_is_bounded_error() {
        // `finish_reason == "tool_calls"` without a usable call: no silent
        // loop, no "Unknown tool" execution — bounded error.
        let d = classify_response(
            "",
            vec![StructuredToolCall {
                id: "p1".to_string(),
                name: String::new(),
                arguments: String::new(),
            }],
        );
        assert!(matches!(d, ResponseDisposition::Empty(_)));
    }

    #[test]
    fn test_malformed_call_never_executed() {
        // Empty-name calls must never reach the tool registry.
        let d = classify_response("", vec![call("")]);
        assert!(matches!(d, ResponseDisposition::Empty(_)));
    }
}
