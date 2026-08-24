#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageChannel {
    Public,
    Direct(String),
    Group(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: String,
    pub timestamp: String,
    pub from: String,
    pub to: String,
    pub channel: MessageChannel,
    pub message_type: MessageType,
    pub content: String,
    pub priority: MessagePriority,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageType {
    ResearchResult(ResearchResult),
    PlanningUpdate(PlanningUpdate),
    CodeChangeProposal(CodeChangeProposal),
    ReviewFeedback(ReviewFeedback),
    TestResult(TestResult),
    RecoveryRequest(RecoveryRequest),
    DecisionRequest(DecisionRequest),
    StatusUpdate(StatusUpdate),
    Information(Information),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResearchResult {
    pub findings: Vec<String>,
    pub files: Vec<String>,
    pub symbols: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanningUpdate {
    pub plan_id: String,
    pub changes: Vec<String>,
    pub estimated_duration_ms: u64,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeChangeProposal {
    pub file: String,
    pub changes: Vec<String>,
    pub reason: String,
    pub risks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewFeedback {
    pub file: String,
    pub issues: Vec<String>,
    pub severity: ReviewSeverity,
    pub suggestions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReviewSeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestResult {
    pub test_name: String,
    pub passed: bool,
    pub output: String,
    pub duration_ms: u64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecoveryRequest {
    pub agent: String,
    pub task: String,
    pub error: String,
    pub failure_type: String,
    pub proposed_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionRequest {
    pub question: String,
    pub context: String,
    pub options: Vec<String>,
    pub from_agent: String,
    pub deadline_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusUpdate {
    pub status: String,
    pub progress: f32,
    pub action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Information {
    pub info_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessagePriority {
    Low,
    Normal,
    High,
    Critical,
}

impl MessagePriority {
    #[allow(dead_code)]
    pub fn order(&self) -> u32 {
        match self {
            MessagePriority::Low => 0,
            MessagePriority::Normal => 1,
            MessagePriority::High => 2,
            MessagePriority::Critical => 3,
        }
    }
}

pub struct AgentMessageBus {
    messages: Arc<Mutex<Vec<AgentMessage>>>,
    history: Arc<Mutex<Vec<AgentMessage>>>,
    by_agent: Arc<Mutex<HashMap<String, Vec<usize>>>>,
    by_channel: Arc<Mutex<HashMap<String, Vec<usize>>>>,
}

impl AgentMessageBus {
    pub fn new() -> Self {
        AgentMessageBus {
            messages: Arc::new(Mutex::new(Vec::new())),
            history: Arc::new(Mutex::new(Vec::new())),
            by_agent: Arc::new(Mutex::new(HashMap::new())),
            by_channel: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn clone_bus(&self) -> Self {
        AgentMessageBus {
            messages: self.messages.clone(),
            history: self.history.clone(),
            by_agent: self.by_agent.clone(),
            by_channel: self.by_channel.clone(),
        }
    }

    pub async fn send(
        &self,
        from: &str,
        to: &str,
        message_type: MessageType,
        content: &str,
        priority: MessagePriority,
        channel: MessageChannel,
        metadata: HashMap<String, String>,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let message = AgentMessage {
            id: id.clone(),
            timestamp: chrono::Local::now().to_rfc3339(),
            from: from.to_string(),
            to: to.to_string(),
            channel: channel.clone(),
            message_type,
            content: content.to_string(),
            priority,
            metadata,
        };

        let mut msgs = self.messages.lock().await;
        let mut hist = self.history.lock().await;
        let mut by_agent = self.by_agent.lock().await;
        let mut by_channel = self.by_channel.lock().await;

        let idx = msgs.len();
        msgs.push(message.clone());
        hist.push(message);

        by_agent
            .entry(from.to_string())
            .or_insert_with(Vec::new)
            .push(idx);
        by_agent
            .entry(to.to_string())
            .or_insert_with(Vec::new)
            .push(idx);

        let channel_key = match &channel {
            MessageChannel::Public => "public".to_string(),
            MessageChannel::Direct(target) => format!("direct:{}", target),
            MessageChannel::Group(group) => format!("group:{}", group),
        };
        by_channel
            .entry(channel_key)
            .or_insert_with(Vec::new)
            .push(idx);

        id
    }

    pub async fn receive(&self, agent: &str) -> Vec<AgentMessage> {
        let by_agent = self.by_agent.lock().await;
        let msgs = self.messages.lock().await;

        if let Some(indices) = by_agent.get(agent) {
            indices
                .iter()
                .filter_map(|&i| msgs.get(i).cloned())
                .collect()
        } else {
            Vec::new()
        }
    }

    pub async fn get_history(&self, limit: usize) -> Vec<AgentMessage> {
        let history = self.history.lock().await;
        history.iter().rev().take(limit).cloned().collect()
    }

    pub async fn get_by_agent(&self, agent: &str, limit: usize) -> Vec<AgentMessage> {
        let by_agent = self.by_agent.lock().await;
        let msgs = self.messages.lock().await;

        if let Some(indices) = by_agent.get(agent) {
            indices
                .iter()
                .rev()
                .filter_map(|&i| msgs.get(i).cloned())
                .take(limit)
                .collect()
        } else {
            Vec::new()
        }
    }

    pub async fn count(&self) -> usize {
        let msgs = self.messages.lock().await;
        msgs.len()
    }

    pub async fn clear_history(&self) {
        let mut hist = self.history.lock().await;
        hist.clear();
    }
}

impl Default for AgentMessageBus {
    fn default() -> Self {
        Self::new()
    }
}
