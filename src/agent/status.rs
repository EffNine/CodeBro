#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AgentStatus {
    Idle,
    Thinking,
    Searching,
    Analysing,
    Planning,
    Executing,
    Testing,
    Reviewing,
    Completed,
    Failed,
    Cancelled,
}

impl AgentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentStatus::Idle => "idle",
            AgentStatus::Thinking => "thinking",
            AgentStatus::Searching => "searching",
            AgentStatus::Analysing => "analysing",
            AgentStatus::Planning => "planning",
            AgentStatus::Executing => "executing",
            AgentStatus::Testing => "testing",
            AgentStatus::Reviewing => "reviewing",
            AgentStatus::Completed => "completed",
            AgentStatus::Failed => "failed",
            AgentStatus::Cancelled => "cancelled",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self,
            AgentStatus::Thinking
                | AgentStatus::Searching
                | AgentStatus::Analysing
                | AgentStatus::Planning
                | AgentStatus::Executing
                | AgentStatus::Testing
                | AgentStatus::Reviewing
        )
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            AgentStatus::Completed | AgentStatus::Failed | AgentStatus::Cancelled
        )
    }
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub name: String,
    pub status: AgentStatus,
    pub current_task: Option<String>,
    pub progress: f32,
    pub latest_action: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

impl AgentState {
    pub fn new(name: impl Into<String>) -> Self {
        AgentState {
            name: name.into(),
            status: AgentStatus::Idle,
            current_task: None,
            progress: 0.0,
            latest_action: None,
            started_at: None,
            completed_at: None,
        }
    }

    pub fn set_status(&mut self, status: AgentStatus) {
        match status {
            AgentStatus::Completed | AgentStatus::Failed | AgentStatus::Cancelled => {
                self.completed_at = Some(chrono::Local::now().to_rfc3339());
            }
            _ => {
                if self.started_at.is_none() && status.is_active() {
                    self.started_at = Some(chrono::Local::now().to_rfc3339());
                }
            }
        }
        self.status = status;
    }

    pub fn set_task(&mut self, task: impl Into<String>) {
        self.current_task = Some(task.into());
    }

    pub fn set_progress(&mut self, progress: f32) {
        self.progress = progress.clamp(0.0, 1.0);
    }

    pub fn set_action(&mut self, action: impl Into<String>) {
        self.latest_action = Some(action.into());
    }

    pub fn reset(&mut self) {
        self.status = AgentStatus::Idle;
        self.current_task = None;
        self.progress = 0.0;
        self.latest_action = None;
        self.started_at = None;
        self.completed_at = None;
    }
}

#[derive(Debug, Clone)]
pub struct AgentStatusMonitor {
    pub agents: std::collections::HashMap<String, AgentState>,
    order: Vec<String>,
}

impl AgentStatusMonitor {
    pub fn new() -> Self {
        AgentStatusMonitor {
            agents: std::collections::HashMap::new(),
            order: Vec::new(),
        }
    }

    pub fn register_agent(&mut self, name: impl Into<String>) {
        let name = name.into();
        if !self.agents.contains_key(&name) {
            self.agents
                .insert(name.clone(), AgentState::new(name.clone()));
            self.order.push(name);
        }
    }

    pub fn get(&self, name: &str) -> Option<&AgentState> {
        self.agents.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut AgentState> {
        self.agents.get_mut(name)
    }

    pub fn update_status(&mut self, name: &str, status: AgentStatus) {
        if let Some(state) = self.agents.get_mut(name) {
            state.set_status(status);
        }
    }

    pub fn update_progress(&mut self, name: &str, progress: f32) {
        if let Some(state) = self.agents.get_mut(name) {
            state.set_progress(progress);
        }
    }

    pub fn update_action(&mut self, name: &str, action: impl Into<String>) {
        if let Some(state) = self.agents.get_mut(name) {
            state.set_action(action);
        }
    }

    pub fn update_task(&mut self, name: &str, task: impl Into<String>) {
        if let Some(state) = self.agents.get_mut(name) {
            state.set_task(task);
        }
    }

    pub fn list(&self) -> Vec<&AgentState> {
        self.order
            .iter()
            .filter_map(|name| self.agents.get(name))
            .collect()
    }

    pub fn count(&self) -> usize {
        self.agents.len()
    }

    pub fn active_count(&self) -> usize {
        self.agents
            .values()
            .filter(|a| a.status.is_active())
            .count()
    }

    pub fn completed_count(&self) -> usize {
        self.agents
            .values()
            .filter(|a| a.status == AgentStatus::Completed)
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.agents
            .values()
            .filter(|a| a.status == AgentStatus::Failed)
            .count()
    }

    pub fn cancelled_count(&self) -> usize {
        self.agents
            .values()
            .filter(|a| a.status == AgentStatus::Cancelled)
            .count()
    }

    pub fn get_all_status(&self) -> HashMap<String, String> {
        self.agents
            .iter()
            .map(|(name, state)| (name.clone(), state.status.to_string()))
            .collect()
    }
}

impl Default for AgentStatusMonitor {
    fn default() -> Self {
        Self::new()
    }
}
