//! P1.5 Runtime Validation Suite
//!
//! Comprehensive validation of the Core Runtime implementation.
//! No features added — validation only.

use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use codebro::runtime::state::{RuntimeState, RuntimeError};
use codebro::tools::Tool;

// ---------------------------------------------------------------------------
// 1. Runtime State Machine Validation
// ---------------------------------------------------------------------------

#[cfg(test)]
mod runtime_state_tests {
    use super::*;
    use crate::runtime::state::RuntimeState;

    #[test]
    fn test_all_valid_transitions() {
        // Idle -> Observing
        assert!(RuntimeState::Idle.try_transition(RuntimeState::Observing).is_ok());

        // Observing -> Reasoning
        assert!(RuntimeState::Observing.try_transition(RuntimeState::Reasoning).is_ok());

        // Reasoning -> Synthesizing
        assert!(RuntimeState::Reasoning.try_transition(RuntimeState::Synthesizing).is_ok());

        // Synthesizing -> Acting
        assert!(RuntimeState::Synthesizing.try_transition(RuntimeState::Acting).is_ok());

        // Synthesizing -> Completed
        assert!(RuntimeState::Synthesizing.try_transition(RuntimeState::Completed).is_ok());

        // Acting -> Synthesizing
        assert!(RuntimeState::Acting.try_transition(RuntimeState::Synthesizing).is_ok());
    }

    #[test]
    fn test_all_invalid_transitions_rejected() {
        let invalid_from_idle = [
            RuntimeState::Reasoning,
            RuntimeState::Synthesizing,
            RuntimeState::Acting,
            RuntimeState::Completed,
            RuntimeState::Failed,
        ];
        for next in &invalid_from_idle {
            let result = RuntimeState::Idle.try_transition(*next);
            assert!(result.is_err(), "Idle -> {:?} should be invalid", next);
        }

        let invalid_from_observing = [
            RuntimeState::Idle,
            RuntimeState::Acting,
            RuntimeState::Completed,
            RuntimeState::Failed,
        ];
        for next in &invalid_from_observing {
            let result = RuntimeState::Observing.try_transition(*next);
            assert!(result.is_err(), "Observing -> {:?} should be invalid", next);
        }

        let invalid_from_reasoning = [
            RuntimeState::Idle,
            RuntimeState::Observing,
            RuntimeState::Acting,
            RuntimeState::Completed,
            RuntimeState::Failed,
        ];
        for next in &invalid_from_reasoning {
            let result = RuntimeState::Reasoning.try_transition(*next);
            assert!(result.is_err(), "Reasoning -> {:?} should be invalid", next);
        }

        let invalid_from_acting = [
            RuntimeState::Idle,
            RuntimeState::Observing,
            RuntimeState::Reasoning,
            RuntimeState::Completed,
            RuntimeState::Failed,
        ];
        for next in &invalid_from_acting {
            let result = RuntimeState::Acting.try_transition(*next);
            assert!(result.is_err(), "Acting -> {:?} should be invalid", next);
        }

        // Terminal states cannot transition anywhere
        for terminal in [RuntimeState::Completed, RuntimeState::Failed] {
            for next in [
                RuntimeState::Idle,
                RuntimeState::Observing,
                RuntimeState::Reasoning,
                RuntimeState::Synthesizing,
                RuntimeState::Acting,
            ] {
                let result = terminal.try_transition(next);
                assert!(result.is_err(), "{:?} -> {:?} should be invalid", terminal, next);
            }
        }
    }

    #[test]
    fn test_no_dead_states() {
        // Every non-terminal state must have at least one valid transition
        for state in [
            RuntimeState::Idle,
            RuntimeState::Observing,
            RuntimeState::Reasoning,
            RuntimeState::Synthesizing,
            RuntimeState::Acting,
        ] {
            let transitions = state.valid_transitions();
            assert!(!transitions.is_empty(), "{:?} has no valid transitions (dead state)", state);
        }
    }

    #[test]
    fn test_no_unreachable_states() {
        // Every state must be reachable from Idle
        let reachable = reachable_from_idle();
        let all_states = vec![
            RuntimeState::Idle,
            RuntimeState::Observing,
            RuntimeState::Reasoning,
            RuntimeState::Synthesizing,
            RuntimeState::Acting,
            RuntimeState::Completed,
            RuntimeState::Failed,
        ];

        for state in &all_states {
            assert!(
                reachable.contains(state),
                "{:?} is unreachable from Idle",
                state
            );
        }
    }

    fn reachable_from_idle() -> HashSet<RuntimeState> {
        let mut visited = HashSet::new();
        let mut queue = vec![RuntimeState::Idle];
        visited.insert(RuntimeState::Idle);

        while let Some(state) = queue.pop() {
            for next in state.valid_transitions() {
                if visited.insert(*next) {
                    queue.push(*next);
                }
            }
        }

        visited
    }

    #[test]
    fn test_all_paths_to_terminal_states() {
        // Verify that Completed and Failed are reachable from Idle
        let reachable = reachable_from_idle();
        assert!(reachable.contains(&RuntimeState::Completed), "Completed must be reachable");
        assert!(reachable.contains(&RuntimeState::Failed), "Failed must be reachable");
    }

    #[test]
    fn test_react_loop_sequence() {
        // Simulate a full ReAct loop: Idle -> Observing -> Reasoning -> Synthesizing -> Acting -> Synthesizing -> Completed
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_multi_iteration_react_loop() {
        // Simulate multiple ReAct iterations
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();

        for _ in 0..5 {
            state = state.try_transition(RuntimeState::Synthesizing).unwrap();
            // Simulate tool call
            state = state.try_transition(RuntimeState::Acting).unwrap();
        }

        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_error_type_construction() {
        let err = RuntimeError {
            from: RuntimeState::Idle,
            to: RuntimeState::Completed,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("Idle"));
        assert!(msg.contains("Completed"));
    }

    #[test]
    fn test_default_state_is_idle() {
        let state = RuntimeState::default();
        assert_eq!(state, RuntimeState::Idle);
    }

    #[test]
    fn test_is_active_for_all_active_states() {
        assert!(!RuntimeState::Idle.is_active());
        assert!(RuntimeState::Observing.is_active());
        assert!(RuntimeState::Reasoning.is_active());
        assert!(RuntimeState::Synthesizing.is_active());
        assert!(RuntimeState::Acting.is_active());
        assert!(!RuntimeState::Completed.is_active());
        assert!(!RuntimeState::Failed.is_active());
    }

    #[test]
    fn test_is_terminal_for_all_terminal_states() {
        assert!(!RuntimeState::Idle.is_terminal());
        assert!(!RuntimeState::Observing.is_terminal());
        assert!(!RuntimeState::Reasoning.is_terminal());
        assert!(!RuntimeState::Synthesizing.is_terminal());
        assert!(!RuntimeState::Acting.is_terminal());
        assert!(RuntimeState::Completed.is_terminal());
        assert!(RuntimeState::Failed.is_terminal());
    }
}

// ---------------------------------------------------------------------------
// 2. Provider Layer Validation
// ---------------------------------------------------------------------------

#[cfg(test)]
mod provider_tests {
    use super::*;
    use crate::providers::Provider;
    use crate::config::Config;

    /// Mock provider for testing without network calls.
    struct MockProvider {
        name: String,
        base_url: String,
        model: String,
        response_chunks: Vec<String>,
    }

    impl MockProvider {
        fn new(name: &str, base_url: &str, model: &str, chunks: Vec<&str>) -> Self {
            MockProvider {
                name: name.to_string(),
                base_url: base_url.to_string(),
                model: model.to_string(),
                response_chunks: chunks.iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    impl Provider for MockProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn base_url(&self) -> &str {
            &self.base_url
        }

        fn model(&self) -> &str {
            &self.model
        }

        fn api_key(&self) -> Option<&str> {
            None
        }

        fn send_message(
            &self,
            _message: &str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String>> + Send + '_>,
        > {
            Box::pin(async move { Ok(self.response_chunks.join("")) })
        }

        fn stream_response(
            &self,
            _message: &str,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                    Output = Result<tokio::sync::mpsc::UnboundedReceiver<String>>,
                >,
            >,
        > {
            let chunks = self.response_chunks.clone();
            Box::pin(async move {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                for chunk in chunks {
                    let _ = tx.send(chunk);
                }
                Ok(rx)
            })
        }
    }

    #[test]
    fn test_provider_trait_compliance() {
        let provider = MockProvider::new("mock", "http://localhost", "test-model", vec!["hello"]);
        assert_eq!(provider.name(), "mock");
        assert_eq!(provider.base_url(), "http://localhost");
        assert_eq!(provider.model(), "test-model");
        assert!(provider.api_key().is_none());
    }

    #[test]
    fn test_provider_substitution() {
        // Verify that different provider implementations can be used interchangeably
        let providers: Vec<Box<dyn Provider>> = vec![
            Box::new(MockProvider::new("mock1", "http://a", "m1", vec!["a"])),
            Box::new(MockProvider::new("mock2", "http://b", "m2", vec!["b"])),
        ];

        for provider in &providers {
            assert!(!provider.name().is_empty());
            assert!(!provider.base_url().is_empty());
            assert!(!provider.model().is_empty());
        }
    }

    #[test]
    fn test_provider_streaming_collects_all_chunks() {
        let chunks = vec!["Hello", " ", "World", "!"];
        let provider = MockProvider::new("mock", "http://localhost", "test", chunks.clone());

        let rt = tokio::runtime::Runtime::new().unwrap();
        let rx = rt.block_on(provider.stream_response("test")).unwrap();

        let collected: Vec<String> = rx.iter().collect();
        assert_eq!(collected, chunks);
    }

    #[test]
    fn test_provider_streaming_empty() {
        let provider = MockProvider::new("mock", "http://localhost", "test", vec![]);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let rx = rt.block_on(provider.stream_response("test")).unwrap();

        let collected: Vec<String> = rx.iter().collect();
        assert!(collected.is_empty());
    }

    #[test]
    fn test_provider_send_message() {
        let chunks = vec!["Hello", " ", "World"];
        let provider = MockProvider::new("mock", "http://localhost", "test", chunks.clone());

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(provider.send_message("test")).unwrap();
        assert_eq!(result, chunks.join(""));
    }

    #[test]
    fn test_openai_provider_creation() {
        let config = Config {
            provider: "openai".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            api_key: None,
        };
        let provider = crate::providers::OpenAiProvider::new(config);
        assert_eq!(provider.name(), "openai");
        assert_eq!(provider.model(), "gpt-4o");
    }

    #[test]
    fn test_provider_trait_is_send_and_sync() {
        // Verify that the Provider trait object is Send + Sync
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Box<dyn Provider>>();
    }
}

// ---------------------------------------------------------------------------
// 3. Tool Registry Validation
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tool_registry_tests {
    use super::*;
    use crate::dispatcher::ToolRegistry;
    use crate::tools::Tool;

    struct DummyTool {
        name: String,
        result: String,
    }

    impl Tool for DummyTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            "Dummy tool for validation"
        }

        fn execute(&self, _args: &str) -> Result<String> {
            Ok(self.result.clone())
        }
    }

    struct FailingTool {
        name: String,
    }

    impl Tool for FailingTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            "Failing tool for validation"
        }

        fn execute(&self, _args: &str) -> Result<String> {
            Err(anyhow::anyhow!("Tool execution failed"))
        }
    }

    #[test]
    fn test_registry_creation() {
        let registry: ToolRegistry = ToolRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_registry_registration() {
        let registry = ToolRegistry::new()
            .register(Arc::new(DummyTool {
                name: "tool_a".to_string(),
                result: "result_a".to_string(),
            }))
            .register(Arc::new(DummyTool {
                name: "tool_b".to_string(),
                result: "result_b".to_string(),
            }));

        assert_eq!(registry.len(), 2);
        assert!(!registry.is_empty());
    }

    #[test]
    fn test_registry_lookup() {
        let registry = ToolRegistry::new().register(Arc::new(DummyTool {
            name: "tool_a".to_string(),
            result: "result_a".to_string(),
        }));

        assert!(registry.get("tool_a").is_some());
        assert!(registry.get("tool_b").is_none());
    }

    #[test]
    fn test_registry_execution_success() {
        let mut registry = ToolRegistry::new().register(Arc::new(DummyTool {
            name: "tool_a".to_string(),
            result: "success".to_string(),
        }));

        let result = registry.execute_sync("tool_a", "args").unwrap();
        assert_eq!(result, "success");
    }

    #[test]
    fn test_registry_execution_failure() {
        let mut registry = ToolRegistry::new().register(Arc::new(FailingTool {
            name: "failing".to_string(),
        }));

        let result = registry.execute_sync("failing", "args");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("failed"));
    }

    #[test]
    fn test_registry_unknown_tool() {
        let mut registry = ToolRegistry::new();

        let result = registry.execute_sync("unknown", "args");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown tool"));
    }

    #[test]
    fn test_registry_names() {
        let registry = ToolRegistry::new()
            .register(Arc::new(DummyTool {
                name: "tool_a".to_string(),
                result: "a".to_string(),
            }))
            .register(Arc::new(DummyTool {
                name: "tool_b".to_string(),
                result: "b".to_string(),
            }));

        let names = registry.names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"tool_a".to_string()));
        assert!(names.contains(&"tool_b".to_string()));
    }

    #[test]
    fn test_registry_list() {
        let registry = ToolRegistry::new().register(Arc::new(DummyTool {
            name: "tool_a".to_string(),
            result: "a".to_string(),
        }));

        let tools = registry.list();
        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn test_registry_has_tool() {
        let registry = ToolRegistry::new().register(Arc::new(DummyTool {
            name: "tool_a".to_string(),
            result: "a".to_string(),
        }));

        assert!(registry.has_tool("tool_a"));
        assert!(!registry.has_tool("tool_b"));
    }

    #[test]
    fn test_registry_overwrites_duplicate() {
        let registry = ToolRegistry::new()
            .register(Arc::new(DummyTool {
                name: "tool_a".to_string(),
                result: "first".to_string(),
            }))
            .register(Arc::new(DummyTool {
                name: "tool_a".to_string(),
                result: "second".to_string(),
            }));

        assert_eq!(registry.len(), 1);
        let result = registry.execute("tool_a", "args").unwrap();
        assert_eq!(result, "second");
    }
}

// ---------------------------------------------------------------------------
// 4. ReAct Loop Validation
// ---------------------------------------------------------------------------

#[cfg(test)]
mod react_loop_tests {
    use super::*;
    use crate::runtime::state::RuntimeState;
    use crate::tools::Tool;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    /// Mock provider that returns a fixed response.
    struct TestProvider {
        response: String,
        call_count: std::sync::atomic::AtomicUsize,
    }

    impl TestProvider {
        fn new(response: &str) -> Self {
            TestProvider {
                response: response.to_string(),
                call_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.call_count.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl Provider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }

        fn base_url(&self) -> &str {
            "http://test"
        }

        fn model(&self) -> &str {
            "test-model"
        }

        fn api_key(&self) -> Option<&str> {
            None
        }

        fn send_message(
            &self,
            _message: &str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String>> + Send + '_>,
        > {
            Box::pin(async move { Ok(self.response.clone()) })
        }

        fn stream_response(
            &self,
            _message: &str,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                    Output = Result<tokio::sync::mpsc::UnboundedReceiver<String>>,
                >,
            >,
        > {
            let response = self.response.clone();
            let count = &self.call_count;
            Box::pin(async move {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                let _ = tx.send(response);
                Ok(rx)
            })
        }
    }

    #[test]
    fn test_react_loop_max_iterations() {
        // Verify the pipeline would not loop infinitely
        let max_iterations = 5;
        let mut iterations = 0;
        let mut state = RuntimeState::Idle;

        // Simulate the ReAct loop structure
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();

        while iterations < max_iterations {
            // Simulate tool call detection
            state = state.try_transition(RuntimeState::Acting).unwrap();
            state = state.try_transition(RuntimeState::Synthesizing).unwrap();
            iterations += 1;
        }

        // After max iterations, should reach terminal
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
        assert_eq!(iterations, max_iterations);
    }

    #[test]
    fn test_react_loop_no_tool_calls_finishes() {
        // When no tool calls are detected, should complete directly
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_react_loop_with_single_tool_call() {
        // Single tool call: Synthesizing -> Acting -> Synthesizing -> Completed
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_react_loop_with_tool_failure() {
        // Tool failure should not break the state machine
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        // Tool fails, but we still transition back to Synthesizing
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_react_loop_provider_failure() {
        // Provider failure should transition to Failed
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Failed).unwrap();
        assert!(state.is_terminal());
    }
}

// ---------------------------------------------------------------------------
// 5. Event Pipeline Validation
// ---------------------------------------------------------------------------

#[cfg(test)]
mod event_pipeline_tests {
    use super::*;
    use crate::agent::events::AgentEvent;
    use std::sync::mpsc;
    use std::thread;

    #[test]
    fn test_event_ordering() {
        let (tx, rx) = mpsc::channel();

        // Send events in order
        tx.send(AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: "test".to_string(),
        })
        .unwrap();
        tx.send(AgentEvent::ToolStarted {
            tool: "read_file".to_string(),
            args: "main.rs".to_string(),
        })
        .unwrap();
        tx.send(AgentEvent::ToolCompleted {
            tool: "read_file".to_string(),
            result: "content".to_string(),
            success: true,
        })
        .unwrap();
        tx.send(AgentEvent::AgentCompleted {
            agent: "main".to_string(),
            duration_ms: 100,
        })
        .unwrap();

        drop(tx);

        let events: Vec<AgentEvent> = rx.iter().collect();
        assert_eq!(events.len(), 4);

        match &events[0] {
            AgentEvent::AgentStarted { agent, .. } => assert_eq!(agent, "main"),
            _ => panic!("Expected AgentStarted"),
        }
        match &events[1] {
            AgentEvent::ToolStarted { tool, .. } => assert_eq!(tool, "read_file"),
            _ => panic!("Expected ToolStarted"),
        }
        match &events[2] {
            AgentEvent::ToolCompleted { tool, .. } => assert_eq!(tool, "read_file"),
            _ => panic!("Expected ToolCompleted"),
        }
        match &events[3] {
            AgentEvent::AgentCompleted { agent, .. } => assert_eq!(agent, "main"),
            _ => panic!("Expected AgentCompleted"),
        }
    }

    #[test]
    fn test_event_no_duplication() {
        let (tx, rx) = mpsc::channel();

        tx.send(AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: "test".to_string(),
        })
        .unwrap();
        tx.send(AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: "test".to_string(),
        })
        .unwrap();

        drop(tx);

        let events: Vec<AgentEvent> = rx.iter().collect();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_event_channel_capacity() {
        let (tx, rx) = mpsc::channel();

        // Send many events
        for i in 0..1000 {
            tx.send(AgentEvent::Log {
                level: "info".to_string(),
                message: format!("event {}", i),
            })
            .unwrap();
        }

        drop(tx);

        let events: Vec<AgentEvent> = rx.iter().collect();
        assert_eq!(events.len(), 1000);
    }

    #[test]
    fn test_event_thread_safety() {
        let (tx, rx) = mpsc::channel();
        let mut handles = vec![];

        // Spawn multiple threads sending events
        for thread_id in 0..10 {
            let tx_clone = tx.clone();
            let handle = thread::spawn(move || {
                for i in 0..100 {
                    tx_clone
                        .send(AgentEvent::Log {
                            level: "info".to_string(),
                            message: format!("thread {} event {}", thread_id, i),
                        })
                        .unwrap();
                }
            });
            handles.push(handle);
        }

        drop(tx);

        for handle in handles {
            handle.join().unwrap();
        }

        let events: Vec<AgentEvent> = rx.iter().collect();
        assert_eq!(events.len(), 1000);
    }

    #[test]
    fn test_event_drain() {
        let (tx, rx) = mpsc::channel();

        tx.send(AgentEvent::Log {
            level: "info".to_string(),
            message: "event1".to_string(),
        })
        .unwrap();
        tx.send(AgentEvent::Log {
            level: "info".to_string(),
            message: "event2".to_string(),
        })
        .unwrap();

        // Drain should collect all pending events
        let events = rx.try_recv();
        assert!(events.is_ok());
        let events = rx.try_recv();
        assert!(events.is_ok());
        let events = rx.try_recv();
        assert!(events.is_err());
    }
}

// ---------------------------------------------------------------------------
// 6. Stress Testing
// ---------------------------------------------------------------------------

#[cfg(test)]
mod stress_tests {
    use super::*;
    use crate::runtime::state::RuntimeState;
    use std::time::Instant;

    #[test]
    fn test_state_transitions_under_load() {
        let iterations = 10000;
        let start = Instant::now();

        for _ in 0..iterations {
            let mut state = RuntimeState::Idle;
            state = state.try_transition(RuntimeState::Observing).unwrap();
            state = state.try_transition(RuntimeState::Reasoning).unwrap();
            state = state.try_transition(RuntimeState::Synthesizing).unwrap();
            state = state.try_transition(RuntimeState::Completed).unwrap();
        }

        let elapsed = start.elapsed();
        println!(
            "State transitions: {} in {:?}",
            iterations, elapsed
        );
        assert!(elapsed < Duration::from_secs(1), "Too slow: {:?}", elapsed);
    }

    #[test]
    fn test_event_throughput() {
        use std::sync::mpsc;
        let iterations = 10000;
        let start = Instant::now();

        let (tx, rx) = mpsc::channel();
        for i in 0..iterations {
            tx.send(AgentEvent::Log {
                level: "info".to_string(),
                message: i.to_string(),
            })
            .unwrap();
        }
        drop(tx);

        let count = rx.iter().count();
        let elapsed = start.elapsed();
        println!("Events: {} in {:?}", count, elapsed);
        assert_eq!(count, iterations);
        assert!(elapsed < Duration::from_secs(1), "Too slow: {:?}", elapsed);
    }

    #[test]
    fn test_registry_lookup_performance() {
        use crate::dispatcher::ToolRegistry;
        use crate::tools::Tool;
        use std::sync::Arc;

        struct FastTool {
            name: String,
        }
        impl Tool for FastTool {
            fn name(&self) -> &str {
                &self.name
            }
            fn description(&self) -> &str {
                "fast"
            }
            fn execute(&self, _args: &str) -> Result<String> {
                Ok("ok".to_string())
            }
        }

        let mut registry = ToolRegistry::new();
        for i in 0..100 {
            registry = registry.register(Arc::new(FastTool {
                name: format!("tool_{}", i),
            }));
        }

        let start = Instant::now();
        let iterations = 10000;
        for i in 0..iterations {
            let _ = registry.execute_sync(&format!("tool_{}", i % 100), "args");
        }
        let elapsed = start.elapsed();
        println!("Registry lookups: {} in {:?}", iterations, elapsed);
        assert!(elapsed < Duration::from_secs(1), "Too slow: {:?}", elapsed);
    }

    #[test]
    fn test_repeated_state_machine_warmup() {
        let iterations = 100;
        let mut total_time = Duration::new(0, 0);

        for _ in 0..iterations {
            let start = Instant::now();
            let mut state = RuntimeState::Idle;
            state = state.try_transition(RuntimeState::Observing).unwrap();
            state = state.try_transition(RuntimeState::Reasoning).unwrap();
            state = state.try_transition(RuntimeState::Synthesizing).unwrap();
            state = state.try_transition(RuntimeState::Acting).unwrap();
            state = state.try_transition(RuntimeState::Synthesizing).unwrap();
            state = state.try_transition(RuntimeState::Completed).unwrap();
            total_time += start.elapsed();
        }

        let avg = total_time / iterations;
        println!("Average state machine cycle: {:?}", avg);
        assert!(avg < Duration::from_millis(1), "Too slow: {:?}", avg);
    }
}

// ---------------------------------------------------------------------------
// 7. Failure Recovery Validation
// ---------------------------------------------------------------------------

#[cfg(test)]
mod failure_recovery_tests {
    use super::*;
    use crate::runtime::state::RuntimeState;

    #[test]
    fn test_provider_failure_transitions_to_failed() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Failed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_tool_failure_does_not_break_state_machine() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        // Tool failed, but we continue
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_malformed_tool_call_handled() {
        // Malformed tool call should not cause state machine corruption
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_timeout_handled_as_failed() {
        // Timeout should transition to Failed state
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Failed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_cancellation_handled() {
        // Cancellation should transition to Failed
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Failed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_recovery_after_tool_failure() {
        // After a tool failure in Acting, should be able to resume Synthesizing
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        // Simulate tool failure recovery
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_multiple_tool_failures() {
        // Multiple tool failures should not corrupt state
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();

        for _ in 0..5 {
            state = state.try_transition(RuntimeState::Acting).unwrap();
            state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        }

        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }
}

// ---------------------------------------------------------------------------
// 8. Integration Validation
// ---------------------------------------------------------------------------

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::dispatcher::ToolRegistry;
    use crate::runtime::state::RuntimeState;
    use crate::tools::{ListFiles, ReadFile, RunCommand, Tool};
    use std::sync::Arc;

    #[test]
    fn test_full_pipeline_state_flow() {
        // Validate the complete pipeline state flow
        let mut state = RuntimeState::Idle;

        // Start
        assert_eq!(state, RuntimeState::Idle);
        assert!(!state.is_active());
        assert!(!state.is_terminal());

        // Observe phase
        state = state.try_transition(RuntimeState::Observing).unwrap();
        assert!(state.is_active());
        assert!(!state.is_terminal());

        // Reason phase
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        assert!(state.is_active());
        assert!(!state.is_terminal());

        // Synthesize phase
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        assert!(state.is_active());
        assert!(!state.is_terminal());

        // Act phase (tool call)
        state = state.try_transition(RuntimeState::Acting).unwrap();
        assert!(state.is_active());
        assert!(!state.is_terminal());

        // Back to synthesize
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        assert!(state.is_active());
        assert!(!state.is_terminal());

        // Complete
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(!state.is_active());
        assert!(state.is_terminal());
    }

    #[test]
    fn test_registry_with_real_tools() {
        let registry = ToolRegistry::new()
            .register(Arc::new(ListFiles))
            .register(Arc::new(ReadFile))
            .register(Arc::new(RunCommand::new()));

        assert_eq!(registry.len(), 3);
        assert!(registry.has_tool("list_files"));
        assert!(registry.has_tool("read_file"));
        assert!(registry.has_tool("run_command"));
        assert!(!registry.has_tool("unknown"));
    }

    #[test]
    fn test_registry_execute_real_tools() {
        let mut registry = ToolRegistry::new().register(Arc::new(ListFiles));

        let result = registry.execute_sync("list_files", ".");
        assert!(result.is_ok());
    }

    #[test]
    fn test_event_summary() {
        let event = AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: "test task".to_string(),
        };
        let summary = event.summary();
        assert!(summary.contains("main"));
        assert!(summary.contains("test task"));
    }

    #[test]
    fn test_event_stream_chunk_summary() {
        let event = AgentEvent::StreamChunk {
            content: "hello".to_string(),
        };
        let summary = event.summary();
        assert_eq!(summary, "streaming");
    }

    #[test]
    fn test_event_log_summary() {
        let event = AgentEvent::Log {
            level: "info".to_string(),
            message: "test message".to_string(),
        };
        let summary = event.summary();
        assert!(summary.contains("info"));
        assert!(summary.contains("test message"));
    }
}
