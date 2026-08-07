#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub max_agents: usize,
    pub token_budget: usize,
    pub max_execution_time_ms: u64,
    pub max_retries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub tokens_used: usize,
    pub time_elapsed_ms: u64,
    pub retries_used: u32,
    pub active_agents: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPriority {
    pub task: String,
    pub priority: PriorityLevel,
    pub assigned_agents: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PriorityLevel {
    Low,
    Normal,
    High,
    Critical,
}

impl PriorityLevel {
    pub fn order(&self) -> u32 {
        match self {
            PriorityLevel::Low => 0,
            PriorityLevel::Normal => 1,
            PriorityLevel::High => 2,
            PriorityLevel::Critical => 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceProfile {
    pub agent: String,
    pub domain: String,
    pub success_rate: f32,
    pub avg_duration_ms: u64,
    pub retry_count: u32,
    pub preferred_tasks: Vec<String>,
}

pub struct ResourceManager {
    pub limits: ResourceLimits,
    pub usage: ResourceUsage,
    pub task_queue: Vec<TaskPriority>,
    pub performance_profiles: HashMap<String, PerformanceProfile>,
}

impl ResourceManager {
    pub fn new() -> Self {
        ResourceManager {
            limits: ResourceLimits {
                max_agents: 10,
                token_budget: 200000,
                max_execution_time_ms: 300000,
                max_retries: 3,
            },
            usage: ResourceUsage {
                tokens_used: 0,
                time_elapsed_ms: 0,
                retries_used: 0,
                active_agents: Vec::new(),
            },
            task_queue: Vec::new(),
            performance_profiles: HashMap::new(),
        }
    }

    pub fn can_add_agent(&self, agent: &str) -> bool {
        self.usage.active_agents.len() < self.limits.max_agents
            && !self.usage.active_agents.contains(&agent.to_string())
    }

    pub fn add_agent(&mut self, agent: &str) {
        if !self.usage.active_agents.contains(&agent.to_string()) {
            self.usage.active_agents.push(agent.to_string());
        }
    }

    pub fn remove_agent(&mut self, agent: &str) {
        self.usage.active_agents.retain(|a| a != agent);
    }

    pub fn record_tokens(&mut self, tokens: usize) {
        self.usage.tokens_used += tokens;
    }

    pub fn record_time(&mut self, ms: u64) {
        self.usage.time_elapsed_ms += ms;
    }

    pub fn record_retry(&mut self) {
        self.usage.retries_used += 1;
    }

    pub fn should_schedule(&self) -> bool {
        self.usage.tokens_used < self.limits.token_budget
            && self.usage.time_elapsed_ms < self.limits.max_execution_time_ms
            && self.usage.retries_used < self.limits.max_retries
    }

    pub fn get_token_budget_remaining(&self) -> usize {
        self.limits
            .token_budget
            .saturating_sub(self.usage.tokens_used)
    }

    pub fn get_time_remaining_ms(&self) -> u64 {
        self.limits
            .max_execution_time_ms
            .saturating_sub(self.usage.time_elapsed_ms)
    }

    pub fn add_task(&mut self, task: String, priority: PriorityLevel, agents: Vec<String>) {
        self.task_queue.push(TaskPriority {
            task,
            priority,
            assigned_agents: agents,
        });
        self.task_queue
            .sort_by(|a, b| b.priority.order().cmp(&a.priority.order()));
    }

    pub fn get_next_task(&mut self) -> Option<TaskPriority> {
        if self.task_queue.is_empty() {
            return None;
        }
        Some(self.task_queue.remove(0))
    }

    pub fn update_performance(
        &mut self,
        agent: &str,
        domain: &str,
        success: bool,
        duration_ms: u64,
    ) {
        let entry = self
            .performance_profiles
            .entry(agent.to_string())
            .or_insert(PerformanceProfile {
                agent: agent.to_string(),
                domain: domain.to_string(),
                success_rate: 0.0,
                avg_duration_ms: 0,
                retry_count: 0,
                preferred_tasks: Vec::new(),
            });

        entry.domain = domain.to_string();
        if success {
            entry.success_rate = ((entry.success_rate * entry.retry_count as f32 + 1.0)
                / (entry.retry_count as f32 + 1.0))
                .min(1.0);
        } else {
            entry.retry_count += 1;
            entry.success_rate = entry.success_rate.max(0.0);
        }
        entry.avg_duration_ms =
            (entry.avg_duration_ms as f64 * 0.9 + duration_ms as f64 * 0.1) as u64;
        if !entry.preferred_tasks.contains(&domain.to_string()) {
            entry.preferred_tasks.push(domain.to_string());
        }
    }

    pub fn get_agent_performance(&self, agent: &str) -> Option<&PerformanceProfile> {
        self.performance_profiles.get(agent)
    }

    pub fn select_agents_for_task(&self, task: &str, required_skills: &[&str]) -> Vec<String> {
        let mut scored: Vec<(&String, f32)> = self
            .performance_profiles
            .iter()
            .filter(|(_, p)| {
                p.preferred_tasks
                    .iter()
                    .any(|t| task.to_lowercase().contains(t.as_str()))
            })
            .map(|(name, p)| {
                let mut score = p.success_rate;
                for skill in required_skills {
                    if p.domain
                        .to_lowercase()
                        .contains(skill.to_lowercase().as_str())
                    {
                        score += 0.2;
                    }
                }
                (name, score)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored
            .into_iter()
            .map(|(name, _)| name.clone())
            .take(3)
            .collect()
    }

    pub fn get_utilization(&self) -> f32 {
        let token_util = self.usage.tokens_used as f32 / self.limits.token_budget as f32;
        let time_util =
            self.usage.time_elapsed_ms as f32 / self.limits.max_execution_time_ms as f32;
        let agent_util = self.usage.active_agents.len() as f32 / self.limits.max_agents as f32;

        (token_util + time_util + agent_util) / 3.0
    }
}

impl Default for ResourceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_limits() {
        let mut rm = ResourceManager::new();
        assert!(rm.can_add_agent("agent1"));
        rm.add_agent("agent1");
        assert!(!rm.can_add_agent("agent1"));

        rm.remove_agent("agent1");
        assert!(rm.can_add_agent("agent1"));
    }

    #[test]
    fn test_token_tracking() {
        let mut rm = ResourceManager::new();
        rm.record_tokens(50000);
        rm.record_tokens(50000);
        assert_eq!(rm.get_token_budget_remaining(), 100000);
        assert!(rm.should_schedule());
    }

    #[test]
    fn test_time_tracking() {
        let mut rm = ResourceManager::new();
        rm.record_time(100000);
        rm.record_time(100000);
        assert_eq!(rm.get_time_remaining_ms(), 100000);
    }

    #[test]
    fn test_task_priority_queue() {
        let mut rm = ResourceManager::new();
        rm.add_task("small task".to_string(), PriorityLevel::Low, vec![]);
        rm.add_task("critical task".to_string(), PriorityLevel::Critical, vec![]);
        rm.add_task("normal task".to_string(), PriorityLevel::Normal, vec![]);

        let next = rm.get_next_task().unwrap();
        assert_eq!(next.task, "critical task");
        assert_eq!(next.priority, PriorityLevel::Critical);
    }

    #[test]
    fn test_performance_tracking() {
        let mut rm = ResourceManager::new();
        rm.update_performance("coding", "rust", true, 1000);
        rm.update_performance("coding", "rust", true, 2000);
        rm.update_performance("coding", "python", false, 3000);

        let profile = rm.get_agent_performance("coding").unwrap();
        assert!(profile.success_rate > 0.0);
        assert_eq!(profile.preferred_tasks.len(), 2);
    }

    #[test]
    fn test_agent_selection() {
        let mut rm = ResourceManager::new();
        rm.update_performance("researcher", "rust", true, 500);
        rm.update_performance("coding", "rust", true, 1000);
        rm.update_performance("testing", "python", false, 2000);

        let agents = rm.select_agents_for_task("rust refactor", &["rust"]);
        assert!(!agents.is_empty());
    }

    #[test]
    fn test_utilization() {
        let rm = ResourceManager::new();
        assert_eq!(rm.get_utilization(), 0.0);
    }
}
