#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::agent::events::AgentEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created_at: String,
    pub task: String,
    pub agents: Vec<String>,
    pub timeline: Vec<TimelineEntry>,
    pub tools_used: Vec<String>,
    pub files_changed: Vec<String>,
    pub result: Option<String>,
    pub lessons: Vec<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub timestamp: String,
    pub agent: String,
    pub event: String,
    pub details: Option<String>,
    pub success: Option<bool>,
}

impl Session {
    pub fn new(task: impl Into<String>) -> Self {
        Session {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Local::now().to_rfc3339(),
            task: task.into(),
            agents: Vec::new(),
            timeline: Vec::new(),
            tools_used: Vec::new(),
            files_changed: Vec::new(),
            result: None,
            lessons: Vec::new(),
            duration_ms: 0,
        }
    }

    pub fn record_event(&mut self, event: &AgentEvent) {
        let (agent, event_name, details, success) = match event {
            AgentEvent::AgentStarted { agent, task } => {
                self.agents.push(agent.clone());
                self.agents.dedup();
                (
                    agent.clone(),
                    "agent_started".to_string(),
                    Some(task.clone()),
                    None,
                )
            }
            AgentEvent::AgentProgress { agent, action, .. } => (
                agent.clone(),
                "agent_progress".to_string(),
                Some(action.clone()),
                None,
            ),
            AgentEvent::AgentStatusChanged { agent, status } => {
                (agent.clone(), format!("status_{}", status), None, None)
            }
            AgentEvent::ToolStarted { tool, args } => {
                self.tools_used.push(tool.clone());
                self.tools_used.dedup();
                (
                    agent_for_tool(tool),
                    format!("tool_started_{}", tool),
                    Some(args.clone()),
                    None,
                )
            }
            AgentEvent::ToolCompleted {
                tool,
                result,
                success,
            } => (
                agent_for_tool(tool),
                format!("tool_completed_{}", tool),
                Some(result.clone()),
                Some(*success),
            ),
            AgentEvent::TaskUpdated {
                task_id,
                status,
                description,
            } => (
                "main".to_string(),
                format!("task_{}", status),
                Some(format!("{}: {}", task_id, description)),
                None,
            ),
            AgentEvent::MemoryUpdated { summary } => (
                "memory".to_string(),
                "memory_updated".to_string(),
                Some(summary.clone()),
                None,
            ),
            AgentEvent::SkillUpdated { skill, .. } => (
                "skill".to_string(),
                "skill_updated".to_string(),
                Some(skill.clone()),
                None,
            ),
            AgentEvent::TaskGraphUpdated { .. } => (
                "main".to_string(),
                "task_graph_updated".to_string(),
                None,
                None,
            ),
            AgentEvent::AgentCompleted { agent, duration_ms } => {
                self.duration_ms = self.duration_ms.saturating_add(*duration_ms);
                (
                    agent.clone(),
                    "agent_completed".to_string(),
                    Some(format!("{}ms", duration_ms)),
                    Some(true),
                )
            }
            AgentEvent::AgentFailed { agent, error } => (
                agent.clone(),
                "agent_failed".to_string(),
                Some(error.clone()),
                Some(false),
            ),
            AgentEvent::AgentCancelled { agent } => (
                agent.clone(),
                "agent_cancelled".to_string(),
                None,
                Some(false),
            ),
            AgentEvent::StreamChunk { .. } => return,
            AgentEvent::PtyOutput { console, .. } => (
                "console".to_string(),
                "pty_output".to_string(),
                Some(console.clone()),
                None,
            ),
            AgentEvent::PtyExited {
                console,
                exit_code,
                status,
            } => (
                "console".to_string(),
                format!("pty_{}", status),
                Some(format!("{} (exit {})", console, exit_code)),
                Some(*exit_code == 0),
            ),
            AgentEvent::Log { level, message } => (
                "log".to_string(),
                format!("log_{}", level),
                Some(message.clone()),
                None,
            ),
        };

        // Defense-in-depth: session files are a persistence boundary. Even
        // though secrets are redacted at the emission points, redact the
        // recorded detail so a secret can never reach a session file through
        // tool args, errors, or log lines. Uses the single redaction authority
        // from the tool platform — no separate redaction implementation.
        let details = details.map(|d| crate::tools::shell::redact_secrets_public(&d));

        self.timeline.push(TimelineEntry {
            timestamp: chrono::Local::now().to_rfc3339(),
            agent,
            event: event_name,
            details,
            success,
        });
    }

    pub fn set_result(&mut self, result: impl Into<String>) {
        self.result = Some(result.into());
        if let Some(duration) = self.started_duration() {
            self.duration_ms = duration;
        }
    }

    pub fn add_lesson(&mut self, lesson: impl Into<String>) {
        self.lessons.push(lesson.into());
    }

    pub fn add_file(&mut self, file: impl Into<String>) {
        self.files_changed.push(file.into());
        self.files_changed.dedup();
    }

    fn started_duration(&self) -> Option<u64> {
        let start = chrono::DateTime::parse_from_rfc3339(&self.created_at).ok()?;
        let now = chrono::Local::now();
        Some(
            (now - start.with_timezone(&chrono::Local))
                .num_milliseconds()
                .max(0) as u64,
        )
    }

    pub fn replay_timeline(&self) -> Vec<String> {
        self.timeline
            .iter()
            .map(|entry| {
                let time = entry
                    .timestamp
                    .parse::<chrono::DateTime<chrono::FixedOffset>>()
                    .map(|t| t.format("%H:%M:%S").to_string())
                    .unwrap_or_else(|_| entry.timestamp.clone());
                let status = match entry.success {
                    Some(true) => "✓",
                    Some(false) => "✗",
                    None => "→",
                };
                let details = entry.details.as_deref().unwrap_or("");
                format!(
                    "{} {} [{}] {} {}",
                    time, status, entry.agent, entry.event, details
                )
            })
            .collect()
    }
}

fn agent_for_tool(tool: &str) -> String {
    match tool {
        t if t.contains("test") || t == "cargo_test" || t == "run_command" => "testing".to_string(),
        t if t.contains("edit") || t.contains("patch") || t == "create_file" => {
            "coding".to_string()
        }
        t if t.contains("search") || t.contains("find") || t.contains("read") => {
            "research".to_string()
        }
        _ => "main".to_string(),
    }
}

pub struct SessionStore {
    sessions_dir: PathBuf,
}

impl SessionStore {
    pub fn new(base_dir: impl AsRef<Path>) -> Result<Self> {
        let sessions_dir = base_dir.as_ref().join("sessions");
        fs::create_dir_all(&sessions_dir)
            .with_context(|| format!("Failed to create sessions dir: {:?}", sessions_dir))?;
        Ok(SessionStore { sessions_dir })
    }

    pub fn save_session(&self, session: &Session) -> Result<()> {
        let path = self.sessions_dir.join(format!("{}.json", session.id));
        let content = serde_json::to_string_pretty(session)
            .with_context(|| format!("Failed to serialize session {}", session.id))?;
        fs::write(&path, content)
            .with_context(|| format!("Failed to write session file {:?}", path))?;
        Ok(())
    }

    pub fn load_session(&self, id: &str) -> Result<Session> {
        let path = self.sessions_dir.join(format!("{}.json", id));
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read session file {:?}", path))?;
        let session: Session = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse session file {:?}", path))?;
        Ok(session)
    }

    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.sessions_dir)
            .with_context(|| format!("Failed to read sessions dir {:?}", self.sessions_dir))?
        {
            let entry = entry?;
            if entry
                .path()
                .extension()
                .map(|e| e == "json")
                .unwrap_or(false)
            {
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    if let Ok(session) = serde_json::from_str::<Session>(&content) {
                        sessions.push(session);
                    }
                }
            }
        }
        sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(sessions)
    }

    pub fn delete_session(&self, id: &str) -> Result<bool> {
        let path = self.sessions_dir.join(format!("{}.json", id));
        if path.exists() {
            fs::remove_file(&path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn session_count(&self) -> Result<usize> {
        Ok(self.list_sessions()?.len())
    }
}

pub struct SessionTracker {
    store: SessionStore,
    current: Option<Session>,
}

impl SessionTracker {
    pub fn new(base_dir: impl AsRef<Path>) -> Result<Self> {
        Ok(SessionTracker {
            store: SessionStore::new(base_dir)?,
            current: None,
        })
    }

    pub fn start_session(&mut self, task: impl Into<String>) -> Result<String> {
        let session = Session::new(task);
        let id = session.id.clone();
        self.store.save_session(&session)?;
        self.current = Some(session);
        Ok(id)
    }

    pub fn current_session(&self) -> Option<&Session> {
        self.current.as_ref()
    }

    pub fn current_session_mut(&mut self) -> Option<&mut Session> {
        self.current.as_mut()
    }

    pub fn record_event(&mut self, event: &AgentEvent) -> Result<()> {
        if let Some(session) = self.current.as_mut() {
            session.record_event(event);
            self.store.save_session(session)?;
        }
        Ok(())
    }

    pub fn end_session(&mut self) -> Result<Option<Session>> {
        if let Some(mut session) = self.current.take() {
            if session.result.is_none() {
                session.set_result("completed");
            }
            self.store.save_session(&session)?;
            Ok(Some(session))
        } else {
            Ok(None)
        }
    }

    pub fn store(&self) -> &SessionStore {
        &self.store
    }
}

pub fn format_duration_ms(ms: u64) -> String {
    let secs = ms / 1000;
    let mins = secs / 60;
    let secs = secs % 60;
    format!("{:02}:{:02}", mins, secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_creation() {
        let session = Session::new("Add caching");
        assert!(!session.id.is_empty());
        assert_eq!(session.task, "Add caching");
        assert!(session.timeline.is_empty());
    }

    #[test]
    fn test_session_record_event() {
        let mut session = Session::new("Test task");
        session.record_event(&AgentEvent::AgentStarted {
            agent: "research".to_string(),
            task: "Find auth".to_string(),
        });
        assert_eq!(session.timeline.len(), 1);
        assert!(session.agents.contains(&"research".to_string()));
    }

    #[test]
    fn test_session_tool_tracking() {
        let mut session = Session::new("Test task");
        session.record_event(&AgentEvent::ToolStarted {
            tool: "cargo_test".to_string(),
            args: "cargo test".to_string(),
        });
        assert!(session.tools_used.contains(&"cargo_test".to_string()));
    }

    #[test]
    fn test_session_store_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        let mut session = Session::new("Test task");
        session.record_event(&AgentEvent::AgentStarted {
            agent: "coding".to_string(),
            task: "Implement".to_string(),
        });
        store.save_session(&session).unwrap();
        let loaded = store.load_session(&session.id).unwrap();
        assert_eq!(loaded.task, "Test task");
        assert_eq!(loaded.timeline.len(), 1);
    }

    #[test]
    fn test_session_store_list() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        store.save_session(&Session::new("Task 1")).unwrap();
        store.save_session(&Session::new("Task 2")).unwrap();
        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn test_session_replay() {
        let mut session = Session::new("Test task");
        session.record_event(&AgentEvent::AgentStarted {
            agent: "research".to_string(),
            task: "Find".to_string(),
        });
        session.record_event(&AgentEvent::AgentCompleted {
            agent: "research".to_string(),
            duration_ms: 1000,
        });
        let replay = session.replay_timeline();
        assert_eq!(replay.len(), 2);
    }

    #[test]
    fn test_duration_format() {
        assert_eq!(format_duration_ms(5000), "00:05");
        assert_eq!(format_duration_ms(150000), "02:30");
        assert_eq!(format_duration_ms(0), "00:00");
    }
}
