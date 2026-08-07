#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::agent::experience::ExperienceReplay;
use crate::agent::skill::SkillManager;
use crate::agent::status::AgentStatus;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPerformance {
    pub name: String,
    pub total_tasks: u32,
    pub completed_tasks: u32,
    pub failed_tasks: u32,
    pub avg_duration_ms: u64,
    pub total_duration_ms: u64,
    pub domains: HashMap<String, DomainPerformance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainPerformance {
    pub task_type: String,
    pub count: u32,
    pub success_count: u32,
    pub success_rate: f32,
    pub avg_duration_ms: u64,
    pub confidence: f32,
}

pub struct PerformanceLogger {
    pub agents: HashMap<String, AgentPerformance>,
    pub experiences: ExperienceReplay,
}

impl PerformanceLogger {
    pub fn new() -> Self {
        PerformanceLogger {
            agents: HashMap::new(),
            experiences: ExperienceReplay::new().unwrap_or_default(),
        }
    }

    pub fn record_task_start(&mut self, agent: &str, task: &str) {
        self.ensure_agent(agent);
        if let Some(perf) = self.agents.get_mut(agent) {
            perf.total_tasks += 1;
        }
    }

    pub fn record_task_complete(&mut self, agent: &str, task: &str, duration_ms: u64) {
        let domain = self.extract_domain(task);
        if let Some(perf) = self.agents.get_mut(agent) {
            perf.completed_tasks += 1;
            perf.total_duration_ms += duration_ms;
            perf.avg_duration_ms = perf.total_duration_ms / perf.total_tasks as u64;
            let domain_perf = perf.domains.entry(domain).or_insert(DomainPerformance {
                task_type: String::new(),
                count: 0,
                success_count: 0,
                success_rate: 0.0,
                avg_duration_ms: 0,
                confidence: 0.0,
            });
            domain_perf.count += 1;
            domain_perf.success_count += 1;
            domain_perf.success_rate = domain_perf.success_count as f32 / domain_perf.count as f32;
            domain_perf.avg_duration_ms =
                (domain_perf.avg_duration_ms as f64 * 0.9 + duration_ms as f64 * 0.1) as u64;
            domain_perf.confidence = domain_perf.success_rate;
        }
    }

    pub fn record_task_fail(&mut self, agent: &str, task: &str, duration_ms: u64) {
        let domain = self.extract_domain(task);
        if let Some(perf) = self.agents.get_mut(agent) {
            perf.failed_tasks += 1;
            perf.total_duration_ms += duration_ms;
            perf.avg_duration_ms = perf.total_duration_ms / perf.total_tasks as u64;
            let domain_perf = perf.domains.entry(domain).or_insert(DomainPerformance {
                task_type: String::new(),
                count: 0,
                success_count: 0,
                success_rate: 0.0,
                avg_duration_ms: 0,
                confidence: 0.0,
            });
            domain_perf.count += 1;
            domain_perf.avg_duration_ms =
                (domain_perf.avg_duration_ms as f64 * 0.9 + duration_ms as f64 * 0.1) as u64;
        }
    }

    fn extract_domain(&self, task: &str) -> String {
        let t = task.to_lowercase();
        if t.contains("rust") || t.contains("compile") || t.contains("refactor") {
            "rust".to_string()
        } else if t.contains("python") || t.contains("migration") {
            "python".to_string()
        } else if t.contains("test") || t.contains("validate") {
            "testing".to_string()
        } else if t.contains("auth") || t.contains("security") {
            "security".to_string()
        } else if t.contains("api") || t.contains("endpoint") {
            "api".to_string()
        } else {
            "general".to_string()
        }
    }

    fn ensure_agent(&mut self, agent: &str) {
        self.agents
            .entry(agent.to_string())
            .or_insert(AgentPerformance {
                name: agent.to_string(),
                total_tasks: 0,
                completed_tasks: 0,
                failed_tasks: 0,
                avg_duration_ms: 0,
                total_duration_ms: 0,
                domains: HashMap::new(),
            });
    }

    pub fn get_agent_stats(&self, agent: &str) -> Option<&AgentPerformance> {
        self.agents.get(agent)
    }

    pub fn get_best_agent_for_task(&self, task: &str) -> Option<String> {
        let domain = self.extract_domain(task);
        let mut candidates: Vec<(&String, &DomainPerformance)> = self
            .agents
            .iter()
            .flat_map(|(name, perf)| {
                perf.domains
                    .iter()
                    .filter(|(d, _)| d.to_lowercase() == domain.to_lowercase())
                    .map(move |(d, perf)| (name, perf))
            })
            .collect();

        candidates.sort_by(|a, b| {
            b.1.success_rate
                .partial_cmp(&a.1.success_rate)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if let Some((name, _)) = candidates.first() {
            Some(name.to_string())
        } else {
            None
        }
    }

    pub fn get_all_agents(&self) -> Vec<&AgentPerformance> {
        self.agents.values().collect()
    }

    pub fn get_agent_status_summary(&self, agent: &str) -> Option<String> {
        let perf = self.agents.get(agent)?;
        let success_rate = if perf.total_tasks > 0 {
            perf.completed_tasks as f32 / perf.total_tasks as f32
        } else {
            0.0
        };
        Some(format!(
            "{}: {} tasks, {:.0}% success, avg {}ms",
            agent,
            perf.total_tasks,
            success_rate * 100.0,
            perf.avg_duration_ms
        ))
    }
}

impl Default for PerformanceLogger {
    fn default() -> Self {
        Self::new()
    }
}
