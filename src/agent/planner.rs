#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use std::collections::HashMap;

use crate::agent::memory::Memory;
use crate::agent::plan_memory::PlanMemoryStore;
use crate::agent::skill::SkillManager;
use crate::intelligence::reasoning::AgentReasoningEngine;
use crate::intelligence::search::SearchResult;

pub struct Plan {
    pub summary: String,
    pub tools: Vec<String>,
    pub args: HashMap<String, String>,
    pub reused: bool,
    pub source: Option<String>,
    pub memory_influence: Vec<String>,
    pub skill_used: Option<String>,
    pub reasoning: String,
    pub code_intelligence: Option<CodeIntelligenceInsight>,
}

#[derive(Debug, Clone)]
pub struct CodeIntelligenceInsight {
    pub relevant_symbols: Vec<SearchResult>,
    pub related_files: Vec<String>,
    pub dependencies: Vec<String>,
    pub plan_steps: Vec<String>,
    pub confidence: f32,
}

pub struct Planner {
    pub plan_memory: Option<PlanMemoryStore>,
    pub skill_manager: Option<SkillManager>,
    pub memory: Option<Memory>,
    pub reasoning_engine: Option<AgentReasoningEngine>,
}

impl Planner {
    pub fn new() -> Self {
        Planner {
            plan_memory: None,
            skill_manager: None,
            memory: None,
            reasoning_engine: None,
        }
    }

    pub fn with_plan_memory(mut self, plan_memory: PlanMemoryStore) -> Self {
        self.plan_memory = Some(plan_memory);
        self
    }

    pub fn with_skill_manager(mut self, skill_manager: SkillManager) -> Self {
        self.skill_manager = Some(skill_manager);
        self
    }

    pub fn with_memory(mut self, memory: Memory) -> Self {
        self.memory = Some(memory);
        self
    }

    pub fn with_reasoning_engine(mut self, reasoning_engine: AgentReasoningEngine) -> Self {
        self.reasoning_engine = Some(reasoning_engine);
        self
    }

    pub fn create_plan(
        &self,
        user_input: &str,
        available_tools: &[&str],
        project_root: Option<&str>,
    ) -> Plan {
        let available: Vec<String> = available_tools.iter().map(|s| s.to_string()).collect();

        let mut memory_influence = Vec::new();
        let mut skill_used = None;
        let mut reasoning = String::new();

        let relevant_memories = self.retrieve_relevant_memories(user_input);
        if !relevant_memories.is_empty() {
            memory_influence = relevant_memories
                .iter()
                .take(3)
                .map(|m| m.to_string())
                .collect();
            reasoning.push_str(&format!(
                "Plan influenced by {} relevant memory entries. ",
                memory_influence.len()
            ));
        }

        if let Some(ref sm) = self.skill_manager {
            if let Some(skill) = sm.find_best_skill(user_input, None, None) {
                if skill.confidence >= 0.5 && skill.usage_count > 0 {
                    skill_used = Some(skill.name.clone());
                    reasoning.push_str(&format!(
                        "Selected skill '{}' (confidence: {:.2}, success rate: {:.2}). ",
                        skill.name,
                        skill.confidence,
                        skill.success_rate()
                    ));
                }
            }
        }

        if let Some(ref pm) = self.plan_memory {
            if let Some(best) = pm.best_plan(user_input, &available) {
                let mut args = best.args.clone();
                args.insert("input".to_string(), user_input.to_string());

                reasoning.push_str(&format!(
                    "Reused previous plan '{}' (confidence: {:.2}).",
                    best.summary, best.confidence
                ));

                return Plan {
                    summary: format!("Reused plan: {}", best.summary),
                    tools: best.tools.clone(),
                    args,
                    reused: true,
                    source: Some(best.id.clone()),
                    memory_influence,
                    skill_used,
                    reasoning,
                    code_intelligence: None,
                };
            }
        }

        let mut tools = Vec::new();
        let mut args = HashMap::new();
        let input_lower = user_input.to_lowercase();

        if input_lower.contains("file")
            || input_lower.contains("read")
            || input_lower.contains("create")
            || input_lower.contains("edit")
        {
            if available.contains(&"read_file".to_string()) {
                tools.push("read_file".to_string());
            }
            if available.contains(&"list_files".to_string()) {
                tools.push("list_files".to_string());
            }
        }

        if input_lower.contains("run")
            || input_lower.contains("command")
            || input_lower.contains("execute")
            || input_lower.contains("build")
            || input_lower.contains("test")
        {
            if available.contains(&"run_command".to_string()) {
                tools.push("run_command".to_string());
            }
        }

        if input_lower.contains("git")
            || input_lower.contains("status")
            || input_lower.contains("diff")
            || input_lower.contains("commit")
        {
            if available.contains(&"git_status".to_string()) {
                tools.push("git_status".to_string());
            }
            if available.contains(&"git_diff".to_string()) {
                tools.push("git_diff".to_string());
            }
        }

        if tools.is_empty() {
            if available.contains(&"read_file".to_string()) {
                tools.push("read_file".to_string());
            }
        }

        args.insert("input".to_string(), user_input.to_string());
        if let Some(root) = project_root {
            args.insert("project_root".to_string(), root.to_string());
        }

        if reasoning.is_empty() {
            reasoning.push_str("Created new plan based on user request and available tools.");
        }

        let code_intelligence = if let Some(ref engine) = self.reasoning_engine {
            if let Ok(result) = engine.analyze_before_modification(user_input) {
                Some(CodeIntelligenceInsight {
                    relevant_symbols: result.relevant_context.relevant_symbols,
                    related_files: result.relevant_context.related_files,
                    dependencies: result.relevant_context.dependencies,
                    plan_steps: result.plan,
                    confidence: result.confidence,
                })
            } else {
                None
            }
        } else {
            None
        };

        if let Some(ref insight) = code_intelligence {
            reasoning.push_str(&format!(
                " Code intelligence found {} relevant symbols and {} related files.",
                insight.relevant_symbols.len(),
                insight.related_files.len()
            ));
        }

        Plan {
            summary: format!("Plan for: {}", user_input),
            tools,
            args,
            reused: false,
            source: None,
            memory_influence,
            skill_used,
            reasoning,
            code_intelligence,
        }
    }

    fn retrieve_relevant_memories(&self, query: &str) -> Vec<String> {
        let mut memories = Vec::new();

        if let Some(ref mem) = self.memory {
            for entry in &mem.short_term {
                let query_lower = query.to_lowercase();
                let input_lower = entry.user_input.to_lowercase();
                if input_lower.contains(&query_lower) || query_lower.contains(&input_lower) {
                    memories.push(format!("Memory: {}", entry.user_input));
                }
            }

            for lesson in &mem.global.lessons {
                let query_lower = query.to_lowercase();
                let lesson_lower = lesson.lesson.to_lowercase();
                if lesson_lower.contains(&query_lower) || query_lower.contains(&lesson_lower) {
                    memories.push(format!("Lesson: {}", lesson.lesson));
                }
            }

            for solution in &mem.global.successful_solutions {
                let query_lower = query.to_lowercase();
                let problem_lower = solution.problem.to_lowercase();
                if problem_lower.contains(&query_lower) || query_lower.contains(&problem_lower) {
                    memories.push(format!("Solution: {}", solution.solution));
                }
            }
        }

        memories
    }
}

impl std::fmt::Display for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "PLAN:")?;
        writeln!(f, "  {}", self.summary)?;
        if self.reused {
            writeln!(f, "  (reused from memory)")?;
        }
        if let Some(ref skill) = self.skill_used {
            writeln!(f, "  Skill: {}", skill)?;
        }
        if !self.memory_influence.is_empty() {
            writeln!(f, "  Memory influence:")?;
            for mem in &self.memory_influence {
                writeln!(f, "    - {}", mem)?;
            }
        }
        writeln!(f, "  Reasoning: {}", self.reasoning)?;
        writeln!(f, "Tools:")?;
        for tool in &self.tools {
            writeln!(f, "  - {}", tool)?;
        }
        Ok(())
    }
}
