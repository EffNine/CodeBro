#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use serde::{Deserialize, Serialize};
use std::sync::mpsc;

use crate::agent::status::AgentStatus;
use crate::agent::task_graph::TaskGraph;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AgentEvent {
    AgentStarted {
        agent: String,
        task: String,
    },
    AgentProgress {
        agent: String,
        progress: f32,
        action: String,
    },
    AgentStatusChanged {
        agent: String,
        status: AgentStatus,
    },
    ToolStarted {
        tool: String,
        args: String,
    },
    ToolCompleted {
        tool: String,
        result: String,
        success: bool,
    },
    TaskUpdated {
        task_id: String,
        status: String,
        description: String,
    },
    MemoryUpdated {
        summary: String,
    },
    SkillUpdated {
        skill: String,
        confidence_before: f32,
        confidence_after: f32,
    },
    AgentCompleted {
        agent: String,
        duration_ms: u64,
    },
    AgentFailed {
        agent: String,
        error: String,
    },
    /// Emitted when a task is cancelled by the user (Ctrl+C).
    AgentCancelled {
        agent: String,
    },
    TaskGraphUpdated {
        graph: TaskGraph,
    },
    StreamChunk {
        content: String,
    },
    /// Live output from a PTY-backed process belonging to the task. This is
    /// authoritative streamed output, not a synthetic status message.
    PtyOutput {
        /// Opaque identifier of the console the chunk belongs to.
        console: String,
        content: String,
    },
    /// A PTY-backed process exited (or was terminated). Carries the real exit
    /// state so the UI never fabricates completion.
    PtyExited {
        console: String,
        exit_code: i32,
        status: String,
    },
    Log {
        level: String,
        message: String,
    },
}

impl AgentEvent {
    pub fn summary(&self) -> String {
        match self {
            AgentEvent::AgentStarted { agent, task } => {
                format!("Agent {} started: {}", agent, task)
            }
            AgentEvent::AgentProgress { agent, action, .. } => {
                format!("{}: {}", agent, action)
            }
            AgentEvent::AgentStatusChanged { agent, status } => {
                format!("{} -> {}", agent, status)
            }
            AgentEvent::ToolStarted { tool, .. } => format!("Tool started: {}", tool),
            AgentEvent::ToolCompleted { tool, success, .. } => {
                format!(
                    "Tool {} {}",
                    tool,
                    if *success { "completed" } else { "failed" }
                )
            }
            AgentEvent::TaskUpdated { description, .. } => {
                format!("Task updated: {}", description)
            }
            AgentEvent::MemoryUpdated { summary } => format!("Memory: {}", summary),
            AgentEvent::SkillUpdated { skill, .. } => format!("Skill updated: {}", skill),
            AgentEvent::AgentCompleted { agent, .. } => format!("Agent {} completed", agent),
            AgentEvent::AgentFailed { agent, .. } => format!("Agent {} failed", agent),
            AgentEvent::AgentCancelled { agent } => format!("Agent {} cancelled", agent),
            AgentEvent::TaskGraphUpdated { .. } => "Task graph updated".to_string(),
            AgentEvent::StreamChunk { .. } => "streaming".to_string(),
            AgentEvent::PtyOutput { .. } => "console output".to_string(),
            AgentEvent::PtyExited { status, .. } => format!("console {}", status),
            AgentEvent::Log { level, message } => format!("[{}] {}", level, message),
        }
    }
}

pub struct AgentEventBus {
    tx: mpsc::Sender<AgentEvent>,
    rx: mpsc::Receiver<AgentEvent>,
}

impl AgentEventBus {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        AgentEventBus { tx, rx }
    }

    pub fn sender(&self) -> mpsc::Sender<AgentEvent> {
        self.tx.clone()
    }

    pub fn publish(&self, event: AgentEvent) -> bool {
        self.tx.send(event).is_ok()
    }

    pub fn try_recv(&self) -> Option<AgentEvent> {
        self.rx.try_recv().ok()
    }

    pub fn recv(&self) -> Option<AgentEvent> {
        self.rx.recv().ok()
    }

    pub fn recv_timeout(&self, duration: std::time::Duration) -> Option<AgentEvent> {
        self.rx.recv_timeout(duration).ok()
    }

    pub fn drain(&self) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            events.push(event);
        }
        events
    }
}

impl Default for AgentEventBus {
    fn default() -> Self {
        Self::new()
    }
}

pub struct EventSubscriber {
    pub tx: mpsc::Sender<AgentEvent>,
}

impl EventSubscriber {
    pub fn subscribe(&self, event: AgentEvent) -> bool {
        self.tx.send(event).is_ok()
    }
}

pub struct EventHistory {
    pub events: Vec<AgentEvent>,
    max_events: usize,
}

impl EventHistory {
    pub fn new(max_events: usize) -> Self {
        EventHistory {
            events: Vec::new(),
            max_events,
        }
    }

    pub fn record(&mut self, event: AgentEvent) {
        self.events.push(event);
        if self.events.len() > self.max_events {
            let overflow = self.events.len() - self.max_events;
            self.events.drain(0..overflow);
        }
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&AgentEvent> {
        self.events.get(index)
    }

    pub fn last(&self) -> Option<&AgentEvent> {
        self.events.last()
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn events_of_agent(&self, agent: &str) -> Vec<&AgentEvent> {
        self.events
            .iter()
            .filter(|e| match e {
                AgentEvent::AgentStarted { agent: a, .. }
                | AgentEvent::AgentProgress { agent: a, .. }
                | AgentEvent::AgentStatusChanged { agent: a, .. }
                | AgentEvent::AgentCompleted { agent: a, .. }
                | AgentEvent::AgentFailed { agent: a, .. } => a == agent,
                _ => false,
            })
            .collect()
    }
}
