#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use crate::error::{CodeBroError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRecord {
    pub id: String,
    pub summary: String,
    pub user_input: String,
    pub tools: Vec<String>,
    pub args: HashMap<String, String>,
    pub success: bool,
    pub usage_count: u32,
    pub success_count: u32,
    pub confidence: f32,
    pub created_at: String,
    pub last_used: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlanMemory {
    pub plans: Vec<PlanRecord>,
}

impl PlanMemory {
    pub fn new() -> Self {
        PlanMemory::default()
    }

    pub fn add_plan(&mut self, plan: PlanRecord) {
        self.plans.push(plan);
    }

    pub fn find_similar(&self, query: &str, available_tools: &[String]) -> Vec<&PlanRecord> {
        let query_lower = query.to_lowercase();
        let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(&PlanRecord, f32)> = self
            .plans
            .iter()
            .filter(|plan| plan.success)
            .map(|plan| {
                let mut score = 0.0f32;
                let input_lower = plan.user_input.to_lowercase();
                let summary_lower = plan.summary.to_lowercase();

                for term in &query_terms {
                    if input_lower.contains(term) {
                        score += 2.0;
                    }
                    if summary_lower.contains(term) {
                        score += 1.5;
                    }
                }

                for tool in &plan.tools {
                    if available_tools.contains(tool) {
                        score += 0.5;
                    }
                }

                score += plan.confidence * 2.0;
                score += (plan.success_count as f32) / ((plan.usage_count as f32).max(1.0)) * 1.0;

                (plan, score)
            })
            .filter(|(_, score)| *score > 0.0)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().map(|(plan, _)| plan).collect()
    }

    pub fn record_usage(&mut self, plan_id: &str, success: bool) {
        if let Some(plan) = self.plans.iter_mut().find(|p| p.id == plan_id) {
            plan.usage_count += 1;
            if success {
                plan.success_count += 1;
            }
            plan.confidence = plan.success_count as f32 / plan.usage_count as f32;
            plan.last_used = Some(chrono::Local::now().to_rfc3339());
        }
    }

    pub fn best_plan(&self, query: &str, available_tools: &[String]) -> Option<&PlanRecord> {
        self.find_similar(query, available_tools).first().copied()
    }
}

pub struct PlanMemoryStore {
    path: PathBuf,
    memory: PlanMemory,
}

impl PlanMemoryStore {
    pub fn new(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| CodeBroError::Config(e.to_string()))?;
        }

        let memory = if path.exists() {
            let content =
                fs::read_to_string(&path).map_err(|e| CodeBroError::Config(e.to_string()))?;
            serde_json::from_str(&content).map_err(|e| CodeBroError::Config(e.to_string()))?
        } else {
            PlanMemory::new()
        };

        Ok(PlanMemoryStore { path, memory })
    }

    pub fn save(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.memory)
            .map_err(|e| CodeBroError::Config(e.to_string()))?;
        fs::write(&self.path, content).map_err(|e| CodeBroError::Config(e.to_string()))?;
        Ok(())
    }

    pub fn add_plan(&mut self, plan: PlanRecord) -> Result<()> {
        self.memory.add_plan(plan);
        self.save()
    }

    pub fn find_similar(&self, query: &str, available_tools: &[String]) -> Vec<&PlanRecord> {
        self.memory.find_similar(query, available_tools)
    }

    pub fn best_plan(&self, query: &str, available_tools: &[String]) -> Option<&PlanRecord> {
        self.memory.best_plan(query, available_tools)
    }

    pub fn record_usage(&mut self, plan_id: &str, success: bool) -> Result<()> {
        self.memory.record_usage(plan_id, success);
        self.save()
    }
}
