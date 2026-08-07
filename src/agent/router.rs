#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use crate::agent::subagent::{
    CodingAgent, PlanningAgent, ResearchAgent, ReviewAgent, SubAgent, TestingAgent,
};

#[derive(Debug, Clone, PartialEq)]
pub enum TaskComplexity {
    Simple,
    Moderate,
    Complex,
}

#[derive(Debug, Clone)]
pub struct TaskAnalysis {
    pub complexity: TaskComplexity,
    pub requires_research: bool,
    pub requires_planning: bool,
    pub requires_coding: bool,
    pub requires_testing: bool,
    pub requires_review: bool,
    pub suggested_agents: Vec<String>,
    pub estimated_duration_ms: u64,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
pub struct TaskRouter {
    simple_threshold: f32,
    moderate_threshold: f32,
}

impl TaskRouter {
    pub fn new() -> Self {
        TaskRouter {
            simple_threshold: 0.3,
            moderate_threshold: 0.6,
        }
    }

    pub fn analyze(&self, task: &str) -> TaskAnalysis {
        let task_lower = task.to_lowercase();
        let mut requires_research = false;
        let mut requires_planning = false;
        let mut requires_coding = false;
        let mut requires_testing = false;
        let mut requires_review = false;
        let confidence = 0.7;
        let estimated_duration_ms;

        let complexity = if task_lower.contains("refactor")
            || task_lower.contains("implement")
            || task_lower.contains("redesign")
            || task_lower.contains("migrate")
        {
            requires_research = true;
            requires_planning = true;
            requires_coding = true;
            requires_testing = true;
            requires_review = true;
            estimated_duration_ms = 10000;
            TaskComplexity::Complex
        } else if task_lower.contains("explain")
            || task_lower.contains("what is")
            || task_lower.contains("show")
            || task_lower.contains("where is")
        {
            estimated_duration_ms = 500;
            TaskComplexity::Simple
        } else if task_lower.contains("add")
            || task_lower.contains("create")
            || task_lower.contains("build")
            || task_lower.contains("fix")
        {
            if task_lower.contains("test") || task_lower.contains("validate") {
                requires_research = true;
                requires_coding = true;
                requires_testing = true;
            } else {
                requires_research = true;
                requires_planning = true;
                requires_coding = true;
                requires_testing = true;
            }
            estimated_duration_ms = 5000;
            TaskComplexity::Moderate
        } else {
            requires_research = true;
            requires_planning = true;
            estimated_duration_ms = 3000;
            TaskComplexity::Moderate
        };

        if task_lower.contains("test") {
            requires_testing = true;
        }

        if task_lower.contains("review") || task_lower.contains("check") {
            requires_review = true;
        }

        let mut suggested_agents = Vec::new();
        if requires_research {
            suggested_agents.push("research".to_string());
        }
        if requires_planning {
            suggested_agents.push("planning".to_string());
        }
        if requires_coding {
            suggested_agents.push("coding".to_string());
        }
        if requires_testing {
            suggested_agents.push("testing".to_string());
        }
        if requires_review {
            suggested_agents.push("review".to_string());
        }

        if suggested_agents.is_empty() {
            suggested_agents.push("research".to_string());
        }

        TaskAnalysis {
            complexity,
            requires_research,
            requires_planning,
            requires_coding,
            requires_testing,
            requires_review,
            suggested_agents,
            estimated_duration_ms,
            confidence,
        }
    }

    pub fn route(&self, task: &str) -> TaskRouting {
        let analysis = self.analyze(task);

        match analysis.complexity {
            TaskComplexity::Simple => TaskRouting::DirectMainAgent,
            TaskComplexity::Moderate => TaskRouting::SequentialSubAgents(analysis.suggested_agents),
            TaskComplexity::Complex => TaskRouting::ParallelSubAgents(analysis.suggested_agents),
        }
    }

    pub fn get_agent(&self, name: &str) -> Option<Box<dyn SubAgent>> {
        match name {
            "research" => Some(Box::new(ResearchAgent::new())),
            "planning" => Some(Box::new(PlanningAgent::new())),
            "coding" => Some(Box::new(CodingAgent::new())),
            "testing" => Some(Box::new(TestingAgent::new())),
            "review" => Some(Box::new(ReviewAgent::new())),
            _ => None,
        }
    }

    pub fn get_default_agents(&self) -> Vec<Box<dyn SubAgent>> {
        vec![
            Box::new(ResearchAgent::new()),
            Box::new(PlanningAgent::new()),
            Box::new(CodingAgent::new()),
            Box::new(TestingAgent::new()),
            Box::new(ReviewAgent::new()),
        ]
    }
}

#[derive(Debug, Clone)]
pub enum TaskRouting {
    DirectMainAgent,
    SequentialSubAgents(Vec<String>),
    ParallelSubAgents(Vec<String>),
}

impl Default for TaskRouter {
    fn default() -> Self {
        Self::new()
    }
}
