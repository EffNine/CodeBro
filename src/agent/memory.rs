#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::config::Config;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Memory {
    pub short_term: Vec<MemoryEntry>,
    pub project: ProjectMemory,
    pub global: GlobalMemory,
    pub sessions: Vec<Session>,
    pub current_session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectMemory {
    pub project_summary: Option<ProjectSummary>,
    pub recent_files: Vec<String>,
    pub recent_commands: Vec<String>,
    pub recent_plans: Vec<String>,
    pub tasks: Vec<TaskRecord>,
    pub decisions: Vec<DecisionRecord>,
    pub preferences: CodingPreferences,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalMemory {
    pub skills: Vec<SkillRecord>,
    pub reflections: Vec<ReflectionRecord>,
    pub successful_solutions: Vec<SolutionRecord>,
    pub lessons: Vec<LessonRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub user_input: String,
    pub response: String,
    pub timestamp: String,
    pub session_id: Option<String>,
    pub importance: f32,
    pub confidence: f32,
    pub usage_count: u32,
    pub last_used: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectSummary {
    pub name: String,
    pub language: String,
    pub framework: String,
    pub files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub last_active: String,
    pub messages: Vec<MemoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub description: String,
    pub status: TaskStatus,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub tools_used: Vec<String>,
    pub files_changed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub id: String,
    pub context: String,
    pub decision: String,
    pub rationale: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodingPreferences {
    pub preferred_language: Option<String>,
    pub preferred_framework: Option<String>,
    pub preferred_test_framework: Option<String>,
    pub preferred_package_manager: Option<String>,
    pub style_preferences: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub confidence: f32,
    pub usage_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionRecord {
    pub task: String,
    pub success: bool,
    pub lessons_learned: Vec<String>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolutionRecord {
    pub problem: String,
    pub solution: String,
    pub context: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonRecord {
    pub lesson: String,
    pub context: String,
    pub source: String,
    pub timestamp: String,
}

impl Memory {
    pub fn load() -> Result<Self> {
        let path = Self::memory_path()?;
        if path.exists() {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read memory file: {:?}", path))?;
            let memory: Memory = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse memory file: {:?}", path))?;
            Ok(memory)
        } else {
            Ok(Memory::default())
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::memory_path()?;
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, content)?;
        Ok(())
    }

    pub fn add_entry(&mut self, user_input: String, response: String) {
        let session_id = self.current_session_id.clone();
        let timestamp = chrono::Local::now().to_rfc3339();

        let entry = MemoryEntry {
            user_input: user_input.clone(),
            response: response.clone(),
            timestamp: timestamp.clone(),
            session_id: session_id.clone(),
            importance: 0.5,
            confidence: 0.5,
            usage_count: 0,
            last_used: Some(timestamp),
        };

        self.short_term.push(entry.clone());

        if self.short_term.len() > 100 {
            self.short_term.remove(0);
        }

        if let Some(ref sid) = session_id {
            if let Some(session) = self.sessions.iter_mut().find(|s| s.id == *sid) {
                session.last_active = chrono::Local::now().to_rfc3339();
                session.messages.push(entry);
            }
        }
    }

    pub fn add_recent_file(&mut self, file: String) {
        self.project.recent_files.retain(|f| f != &file);
        self.project.recent_files.insert(0, file);
        self.project.recent_files.truncate(20);
    }

    pub fn add_recent_command(&mut self, command: String) {
        self.project.recent_commands.retain(|c| c != &command);
        self.project.recent_commands.insert(0, command);
        self.project.recent_commands.truncate(20);
    }

    pub fn add_recent_plan(&mut self, plan: String) {
        self.project.recent_plans.retain(|p| p != &plan);
        self.project.recent_plans.insert(0, plan);
        self.project.recent_plans.truncate(20);
    }

    pub fn start_session(&mut self, name: Option<String>) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let session = Session {
            id: id.clone(),
            name: name.unwrap_or_else(|| format!("Session {}", self.sessions.len() + 1)),
            created_at: chrono::Local::now().to_rfc3339(),
            last_active: chrono::Local::now().to_rfc3339(),
            messages: Vec::new(),
        };
        self.sessions.push(session);
        self.current_session_id = Some(id.clone());
        id
    }

    pub fn end_session(&mut self) -> Result<()> {
        self.current_session_id = None;
        self.save()
    }

    pub fn list_sessions(&self) -> Vec<&Session> {
        self.sessions.iter().collect()
    }

    pub fn resume_session(&mut self, session_id: &str) -> Result<()> {
        if self.sessions.iter().any(|s| s.id == session_id) {
            self.current_session_id = Some(session_id.to_string());
            Ok(())
        } else {
            Err(anyhow::anyhow!("Session not found: {}", session_id))
        }
    }

    pub fn clear(&mut self) -> Result<()> {
        self.short_term.clear();
        self.project = ProjectMemory::default();
        self.save()
    }

    pub fn add_task(
        &mut self,
        description: String,
        tools_used: Vec<String>,
        files_changed: Vec<String>,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let task = TaskRecord {
            id: id.clone(),
            description,
            status: TaskStatus::InProgress,
            created_at: chrono::Local::now().to_rfc3339(),
            completed_at: None,
            tools_used,
            files_changed,
        };
        self.project.tasks.push(task);
        id
    }

    pub fn complete_task(&mut self, task_id: &str) -> Result<()> {
        if let Some(task) = self.project.tasks.iter_mut().find(|t| t.id == task_id) {
            task.status = TaskStatus::Completed;
            task.completed_at = Some(chrono::Local::now().to_rfc3339());
        }
        Ok(())
    }

    pub fn add_decision(&mut self, context: String, decision: String, rationale: String) {
        let record = DecisionRecord {
            id: uuid::Uuid::new_v4().to_string(),
            context,
            decision,
            rationale,
            timestamp: chrono::Local::now().to_rfc3339(),
        };
        self.project.decisions.push(record);
    }

    pub fn add_reflection(&mut self, reflection: ReflectionRecord) {
        self.global.reflections.push(reflection);
    }

    pub fn add_lesson(&mut self, lesson: String, context: String, source: String) {
        let record = LessonRecord {
            lesson,
            context,
            source,
            timestamp: chrono::Local::now().to_rfc3339(),
        };
        self.global.lessons.push(record);
    }

    pub fn add_solution(&mut self, problem: String, solution: String, context: String) {
        let record = SolutionRecord {
            problem,
            solution,
            context,
            timestamp: chrono::Local::now().to_rfc3339(),
        };
        self.global.successful_solutions.push(record);
    }

    pub fn search_lessons(&self, query: &str) -> Vec<&LessonRecord> {
        let query_lower = query.to_lowercase();
        self.global
            .lessons
            .iter()
            .filter(|l| {
                l.lesson.to_lowercase().contains(&query_lower)
                    || l.context.to_lowercase().contains(&query_lower)
            })
            .collect()
    }

    pub fn search_solutions(&self, query: &str) -> Vec<&SolutionRecord> {
        let query_lower = query.to_lowercase();
        self.global
            .successful_solutions
            .iter()
            .filter(|s| {
                s.problem.to_lowercase().contains(&query_lower)
                    || s.solution.to_lowercase().contains(&query_lower)
            })
            .collect()
    }

    fn memory_path() -> Result<PathBuf> {
        let config_dir = Config::config_dir();
        Ok(config_dir.join("memory.json"))
    }
}
