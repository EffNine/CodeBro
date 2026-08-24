#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::config::Config;
use anyhow::{Context, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Experience {
    pub id: String,
    pub task_description: String,
    pub context: ExperienceContext,
    pub plan: Vec<String>,
    pub tools_used: Vec<String>,
    pub skills_used: Vec<String>,
    pub result: ExperienceResult,
    pub lessons_learned: Vec<String>,
    pub success: bool,
    pub duration_ms: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceContext {
    pub relevant_files: Vec<String>,
    pub related_symbols: Vec<String>,
    pub dependencies: Vec<String>,
    pub project_language: Option<String>,
    pub project_framework: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceResult {
    pub success: bool,
    pub output: String,
    pub files_modified: Vec<String>,
    pub errors: Vec<String>,
    pub recommendations: Vec<String>,
}

pub struct ExperienceReplay {
    experiences: HashMap<String, Experience>,
    experience_path: PathBuf,
}

impl ExperienceReplay {
    pub fn new() -> anyhow::Result<Self> {
        let config_dir = Config::config_dir();
        let experience_path = config_dir.join("experiences.json");

        let experiences = if experience_path.exists() {
            let content = fs::read_to_string(&experience_path).with_context(|| {
                format!("Failed to read experiences file: {:?}", experience_path)
            })?;
            let experiences: HashMap<String, Experience> = serde_json::from_str(&content)
                .with_context(|| {
                    format!("Failed to parse experiences file: {:?}", experience_path)
                })?;
            experiences
        } else {
            HashMap::new()
        };

        Ok(ExperienceReplay {
            experiences,
            experience_path,
        })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.experience_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        let content = serde_json::to_string_pretty(&self.experiences)?;
        fs::write(&self.experience_path, content)?;
        Ok(())
    }

    pub fn record_experience(&mut self, experience: Experience) {
        self.experiences.insert(experience.id.clone(), experience);
    }

    pub fn find_similar(&self, task: &str, limit: usize) -> Vec<&Experience> {
        let task_lower = task.to_lowercase();
        let task_terms: Vec<&str> = task_lower.split_whitespace().collect();

        let mut scored: Vec<(&Experience, f32)> = self
            .experiences
            .values()
            .map(|exp| {
                let mut score = 0.0;
                let exp_lower = exp.task_description.to_lowercase();

                for term in &task_terms {
                    if exp_lower.contains(term) {
                        score += 1.0;
                    }
                }

                if exp.success {
                    score += 2.0;
                }

                score *= exp.result.recommendations.len() as f32 + 1.0;

                (exp, score)
            })
            .filter(|(_, score)| *score > 0.0)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scored.into_iter().take(limit).map(|(exp, _)| exp).collect()
    }

    pub fn get_successful_patterns(&self, task_type: &str) -> Vec<&Experience> {
        self.experiences
            .values()
            .filter(|exp| {
                exp.success
                    && exp
                        .task_description
                        .to_lowercase()
                        .contains(&task_type.to_lowercase())
            })
            .collect()
    }

    pub fn get_lessons_learned(&self, task_type: &str) -> Vec<String> {
        let mut lessons = Vec::new();

        for exp in self.experiences.values() {
            if exp
                .task_description
                .to_lowercase()
                .contains(&task_type.to_lowercase())
            {
                for lesson in &exp.lessons_learned {
                    if !lessons.contains(lesson) {
                        lessons.push(lesson.clone());
                    }
                }
            }
        }

        lessons
    }

    pub fn get_recommended_tools(&self, task_type: &str) -> Vec<String> {
        let mut tool_counts: HashMap<String, u32> = HashMap::new();

        for exp in self.experiences.values() {
            if exp.success
                && exp
                    .task_description
                    .to_lowercase()
                    .contains(&task_type.to_lowercase())
            {
                for tool in &exp.tools_used {
                    *tool_counts.entry(tool.clone()).or_insert(0) += 1;
                }
            }
        }

        let mut tools: Vec<_> = tool_counts.into_iter().collect();
        tools.sort_by(|a, b| b.1.cmp(&a.1));
        tools.into_iter().map(|(tool, _)| tool).collect()
    }

    pub fn get_skill_usage_patterns(&self, task_type: &str) -> Vec<String> {
        let mut skill_counts: HashMap<String, u32> = HashMap::new();

        for exp in self.experiences.values() {
            if exp.success
                && exp
                    .task_description
                    .to_lowercase()
                    .contains(&task_type.to_lowercase())
            {
                for skill in &exp.skills_used {
                    *skill_counts.entry(skill.clone()).or_insert(0) += 1;
                }
            }
        }

        let mut skills: Vec<_> = skill_counts.into_iter().collect();
        skills.sort_by(|a, b| b.1.cmp(&a.1));
        skills.into_iter().map(|(skill, _)| skill).collect()
    }

    pub fn get_statistics(&self) -> ExperienceStatistics {
        let total = self.experiences.len();
        let successful = self.experiences.values().filter(|e| e.success).count();
        let failed = total - successful;

        let avg_duration = if total > 0 {
            self.experiences
                .values()
                .map(|e| e.duration_ms)
                .sum::<u64>()
                / total as u64
        } else {
            0
        };

        ExperienceStatistics {
            total_experiences: total,
            successful,
            failed,
            success_rate: if total > 0 {
                successful as f32 / total as f32
            } else {
                0.0
            },
            average_duration_ms: avg_duration,
        }
    }

    pub fn prune_old_experiences(&mut self, max_age_days: u64) {
        let cutoff = chrono::Local::now() - chrono::Duration::days(max_age_days as i64);
        let cutoff_str = cutoff.to_rfc3339();

        self.experiences
            .retain(|_, exp| exp.created_at > cutoff_str);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceStatistics {
    pub total_experiences: usize,
    pub successful: usize,
    pub failed: usize,
    pub success_rate: f32,
    pub average_duration_ms: u64,
}

impl Default for ExperienceReplay {
    fn default() -> Self {
        Self::new().expect("Failed to create ExperienceReplay")
    }
}
