#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::{CodeBroError, Result};
use crate::scanner::ProjectInfo;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Reflection {
    pub task: String,
    pub plan_used: Option<String>,
    pub tools_used: Vec<String>,
    pub files_changed: Vec<String>,
    pub what_worked: Vec<String>,
    pub what_failed: Vec<String>,
    pub lessons_learned: Vec<String>,
    pub success: bool,
    pub confidence: f32,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReflectionStore {
    pub reflections: Vec<Reflection>,
}

impl ReflectionStore {
    pub fn new() -> Self {
        ReflectionStore::default()
    }

    pub fn add_reflection(&mut self, reflection: Reflection) {
        self.reflections.push(reflection);
    }

    pub fn recent_reflections(&self, count: usize) -> Vec<&Reflection> {
        self.reflections.iter().rev().take(count).collect()
    }

    pub fn successful_patterns(&self) -> Vec<String> {
        let mut patterns = Vec::new();
        for reflection in &self.reflections {
            if reflection.success {
                for lesson in &reflection.lessons_learned {
                    if !patterns.contains(lesson) {
                        patterns.push(lesson.clone());
                    }
                }
            }
        }
        patterns
    }

    pub fn failure_patterns(&self) -> Vec<String> {
        let mut patterns = Vec::new();
        for reflection in &self.reflections {
            if !reflection.success {
                for lesson in &reflection.lessons_learned {
                    if !patterns.contains(lesson) {
                        patterns.push(lesson.clone());
                    }
                }
            }
        }
        patterns
    }
}

pub struct ReflectionEngine;

impl ReflectionEngine {
    pub fn reflect(
        task: &str,
        plan: Option<&str>,
        tools_used: &[String],
        files_changed: &[String],
        success: bool,
        errors: &[String],
    ) -> Reflection {
        let what_worked = if success {
            vec![format!("Completed task: {}", task)]
        } else {
            Vec::new()
        };

        let what_failed = if success { Vec::new() } else { errors.to_vec() };

        let mut lessons_learned = Vec::new();
        if success {
            for tool in tools_used {
                lessons_learned.push(format!("Tool {} was effective", tool));
            }
            if !files_changed.is_empty() {
                lessons_learned.push(format!("Modified {} files", files_changed.len()));
            }
        } else {
            for error in errors {
                lessons_learned.push(format!("Avoid: {}", error));
            }
        }

        Reflection {
            task: task.to_string(),
            plan_used: plan.map(|s| s.to_string()),
            tools_used: tools_used.to_vec(),
            files_changed: files_changed.to_vec(),
            what_worked,
            what_failed,
            lessons_learned,
            success,
            confidence: if success { 0.8 } else { 0.3 },
            timestamp: chrono::Local::now().to_rfc3339(),
        }
    }
}
