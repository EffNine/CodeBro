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

use crate::provider_runtime::routing::ProviderRoutingDecision;
use crate::provider_runtime::{ProviderId, ProviderRuntime, RetryController, TokenUsage};
use crate::providers::{Provider, StructuredToolCall, ToolDefinition};

use super::TaskOptions;

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
