#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::agent::communication::MessageType;
use crate::agent::events::AgentEvent;
use crate::agent::recovery::RecoveryEngine;
use crate::agent::router::TaskRouter;
use crate::agent::status::AgentStatus;
use crate::agent::subagent::{SubAgent, SubAgentContext, SubAgentResult};
use crate::agent::task_graph::{TaskGraph, TaskStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorState {
    pub active_agents: Vec<String>,
    pub agent_tasks: HashMap<String, String>,
    pub agent_status: HashMap<String, String>,
    pub shared_workspace: HashMap<String, String>,
    pub decision_log: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AgentRole {
    Main,
    Research,
    Planning,
    Coding,
    Testing,
    Review,
}

pub struct AgentCoordinator {
    pub state: Arc<Mutex<CoordinatorState>>,
    pub message_bus: crate::agent::communication::AgentMessageBus,
    pub router: TaskRouter,
    pub max_agents: usize,
}

impl Clone for AgentCoordinator {
    fn clone(&self) -> Self {
        AgentCoordinator {
            state: self.state.clone(),
            message_bus: self.message_bus.clone_bus(),
            router: self.router.clone(),
            max_agents: self.max_agents,
        }
    }
}

impl AgentCoordinator {
    pub fn new(max_agents: usize) -> Self {
        AgentCoordinator {
            state: Arc::new(Mutex::new(CoordinatorState {
                active_agents: Vec::new(),
                agent_tasks: HashMap::new(),
                agent_status: HashMap::new(),
                shared_workspace: HashMap::new(),
                decision_log: Vec::new(),
            })),
            message_bus: crate::agent::communication::AgentMessageBus::new(),
            router: TaskRouter::new(),
            max_agents,
        }
    }

    pub async fn spawn_agent(
        &mut self,
        name: &str,
        _role: crate::agent::coordinator::AgentRole,
    ) -> bool {
        let mut state = self.state.lock().await;
        if state.active_agents.len() >= self.max_agents {
            return false;
        }
        if !state.active_agents.contains(&name.to_string()) {
            state.active_agents.push(name.to_string());
            state
                .agent_status
                .insert(name.to_string(), "initialized".to_string());
        }
        true
    }

    pub async fn assign_task(
        &mut self,
        agent: &str,
        task: &str,
        workspace_key: Option<&str>,
    ) -> bool {
        let mut state = self.state.lock().await;
        if !state.active_agents.contains(&agent.to_string()) {
            return false;
        }
        state
            .agent_tasks
            .insert(agent.to_string(), task.to_string());
        state
            .agent_status
            .insert(agent.to_string(), "working".to_string());

        if let Some(key) = workspace_key {
            state
                .shared_workspace
                .insert(format!("task_{}", key), task.to_string());
        }

        true
    }

    pub async fn complete_task(&mut self, agent: &str, result: &str) {
        let mut state = self.state.lock().await;
        state
            .agent_status
            .insert(agent.to_string(), "completed".to_string());
        if let Some(task) = state.agent_tasks.get(agent).cloned() {
            state
                .shared_workspace
                .insert(format!("result_{}", task), result.to_string());
        }
    }

    pub async fn fail_task(&mut self, agent: &str, error: &str) {
        let mut state = self.state.lock().await;
        state
            .agent_status
            .insert(agent.to_string(), "failed".to_string());
        if let Some(task) = state.agent_tasks.get(agent).cloned() {
            state
                .shared_workspace
                .insert(format!("error_{}", task), error.to_string());
        }
    }

    pub async fn send_message(
        &self,
        from: &str,
        to: &str,
        msg_type: MessageType,
        content: &str,
        priority: crate::agent::communication::MessagePriority,
    ) -> String {
        self.message_bus
            .send(
                from,
                to,
                msg_type,
                content,
                priority,
                crate::agent::communication::MessageChannel::Public,
                HashMap::new(),
            )
            .await
    }

    pub async fn broadcast(&self, from: &str, msg_type: MessageType, content: &str) {
        let agents: Vec<String> = self.state.lock().await.active_agents.clone();
        for agent in &agents {
            if agent != from {
                let _ = self
                    .send_message(
                        from,
                        agent,
                        msg_type.clone(),
                        content,
                        crate::agent::communication::MessagePriority::Normal,
                    )
                    .await;
            }
        }
    }

    pub async fn log_decision(&self, decision: &str) {
        let mut state = self.state.lock().await;
        state.decision_log.push(decision.to_string());
    }

    pub async fn get_agent_status(&self, agent: &str) -> Option<String> {
        let state = self.state.lock().await;
        state.agent_status.get(agent).cloned()
    }

    pub async fn get_all_status(&self) -> HashMap<String, String> {
        let state = self.state.lock().await;
        state.agent_status.clone()
    }

    pub async fn get_workspace(&self, key: &str) -> Option<String> {
        let state = self.state.lock().await;
        state.shared_workspace.get(key).cloned()
    }

    pub async fn get_decisions(&self, limit: usize) -> Vec<String> {
        let state = self.state.lock().await;
        state
            .decision_log
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    pub async fn select_agents_for_task(&self, task: &str) -> Vec<String> {
        let analysis = self.router.analyze(task);
        analysis.suggested_agents
    }

    /// Orchestrates a full task through the router and subagents, emitting
    /// lifecycle events, populating a TaskGraph, and routing failures through
    /// the RecoveryEngine. Returns the aggregated subagent report.
    pub async fn run_task(
        &mut self,
        task: &str,
        project_root: Option<&str>,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> String {
        let router = self.router.clone();
        let factory = move |name: &str| router.get_agent(name);
        self.run_task_with(task, project_root, emit, &factory).await
    }

    /// Like [`AgentCoordinator::run_task`] but lets the caller supply the agent
    /// factory (used by tests to inject failing agents).
    pub async fn run_task_with(
        &mut self,
        task: &str,
        project_root: Option<&str>,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
        agent_factory: &(dyn Fn(&str) -> Option<Box<dyn SubAgent>> + Send + Sync),
    ) -> String {
        let analysis = self.router.analyze(task);
        let agent_names = analysis.suggested_agents.clone();

        // Build a task graph with one node per planned agent, chained.
        let mut graph = TaskGraph::new(task);
        let mut prev_id: Option<String> = None;
        let mut task_ids: HashMap<String, String> = HashMap::new();
        for name in &agent_names {
            let deps = prev_id.clone().map(|d| vec![d]).unwrap_or_default();
            let id = graph.add_task(&format!("{}: {}", capitalize(name), task), name, deps);
            task_ids.insert(name.clone(), id.clone());
            prev_id = Some(id);
        }
        emit(AgentEvent::TaskGraphUpdated {
            graph: graph.clone(),
        });

        let mut previous_results: Vec<SubAgentResult> = Vec::new();
        let mut report = String::new();

        for name in &agent_names {
            let status = match name.as_str() {
                "research" => AgentStatus::Searching,
                "planning" => AgentStatus::Planning,
                "coding" => AgentStatus::Executing,
                "testing" => AgentStatus::Testing,
                "review" => AgentStatus::Reviewing,
                _ => AgentStatus::Thinking,
            };

            emit(AgentEvent::AgentStarted {
                agent: name.clone(),
                task: task.to_string(),
            });
            emit(AgentEvent::AgentStatusChanged {
                agent: name.clone(),
                status: status.clone(),
            });

            let _ = self.spawn_agent(name, AgentRole::Main).await;
            let _ = self.assign_task(name, task, None).await;

            let context = SubAgentContext {
                task_description: task.to_string(),
                project_root: project_root.unwrap_or(".").to_string(),
                relevant_files: Vec::new(),
                related_symbols: Vec::new(),
                dependencies: Vec::new(),
                previous_results: previous_results.clone(),
                memory_entries: Vec::new(),
            };

            emit(AgentEvent::AgentProgress {
                agent: name.clone(),
                progress: 0.1,
                action: format!("{} starting", name),
            });

            let node_id = task_ids.get(name).cloned();
            if let Some(agent) = agent_factory(name) {
                let start = std::time::Instant::now();
                let result = agent.execute(&context);
                let duration_ms = start.elapsed().as_millis() as u64;

                // NOTE: subagents are lightweight analysis/generation helpers
                // (plans, reports, test outlines). They do NOT execute the real
                // filesystem/shell git tools - that is the sole responsibility of
                // `crate::tools::run_tool_pipeline`, which runs first and emits the
                // authoritative ToolStarted/ToolCompleted events. We intentionally
                // do NOT emit fake tool events here.

                emit(AgentEvent::Log {
                    level: "coordination".to_string(),
                    message: format!("{}: {}", name, summarize_output(&result.output)),
                });

                if result.success {
                    emit(AgentEvent::AgentCompleted {
                        agent: name.clone(),
                        duration_ms,
                    });
                    self.complete_task(name, &result.output).await;
                    if let Some(id) = node_id {
                        graph.update_status(&id, TaskStatus::Completed);
                    }
                    report.push_str(&format!("## {}\n{}\n\n", capitalize(name), result.output));
                } else {
                    let err = if result.errors.is_empty() {
                        "subagent reported failure".to_string()
                    } else {
                        result.errors.join("; ")
                    };
                    emit(AgentEvent::AgentFailed {
                        agent: name.clone(),
                        error: err.clone(),
                    });
                    self.fail_task(name, &err).await;
                    if let Some(id) = node_id {
                        graph.update_status(&id, TaskStatus::Failed);
                    }

                    // Error flow: route through the RecoveryEngine and notify the TUI.
                    if let Ok(mut recovery) = RecoveryEngine::new() {
                        if let Ok(plan) = recovery.handle_failure(name, task, &err) {
                            emit(AgentEvent::Log {
                                level: "coordination".to_string(),
                                message: format!(
                                    "Recovery for {}: {:?} -> {}",
                                    name, plan.action, plan.suggested_agent
                                ),
                            });
                        }
                    }

                    report.push_str(&format!("## {}\nFAILED: {}\n\n", capitalize(name), err));
                }
                previous_results.push(result);
            } else {
                let err = format!("No agent registered for '{}'", name);
                emit(AgentEvent::AgentFailed {
                    agent: name.clone(),
                    error: err.clone(),
                });
                if let Some(id) = node_id {
                    graph.update_status(&id, TaskStatus::Failed);
                }
                report.push_str(&format!("## {}\nFAILED: {}\n\n", capitalize(name), err));
            }

            emit(AgentEvent::TaskGraphUpdated {
                graph: graph.clone(),
            });
        }

        report
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn summarize_output(output: &str) -> String {
    output
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("done")
        .to_string()
}
