#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::agent::experience::ExperienceReplay;
use crate::agent::skill::SkillManager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionContext {
    pub question: String,
    pub context: String,
    pub options: Vec<String>,
    pub from_agent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub question: String,
    pub chosen_option: String,
    pub reasoning: String,
    pub confidence: f32,
    pub timestamp: String,
}

pub struct DecisionEngine {
    experiences: ExperienceReplay,
    skills: Option<SkillManager>,
    history: Vec<Decision>,
}

impl DecisionEngine {
    pub fn new() -> Self {
        DecisionEngine {
            experiences: ExperienceReplay::new().unwrap_or_default(),
            skills: None,
            history: Vec::new(),
        }
    }

    pub fn with_skills(mut self, skills: SkillManager) -> Self {
        self.skills = Some(skills);
        self
    }

    pub async fn make_decision(&mut self, context: DecisionContext) -> Option<Decision> {
        let mut best_option = context.options.first()?.clone();
        let mut best_score = 0.0;
        let reasoning;

        for option in &context.options {
            let mut score = 0.0;

            let exp_score = self.experience_score(option, &context);
            score += exp_score;

            let skill_score = self.skill_score(option, &context);
            score += skill_score;

            if score > best_score {
                best_score = score;
                best_option = option.clone();
            }
        }

        reasoning = format!(
            "Chose '{}' over {} options (confidence: {:.2})",
            best_option,
            context.options.len(),
            best_score
        );

        let decision = Decision {
            id: uuid::Uuid::new_v4().to_string(),
            question: context.question.clone(),
            chosen_option: best_option.clone(),
            reasoning,
            confidence: best_score,
            timestamp: chrono::Local::now().to_rfc3339(),
        };

        self.history.push(decision.clone());
        Some(decision)
    }

    fn experience_score(&self, option: &str, context: &DecisionContext) -> f32 {
        let similar = self.experiences.find_similar(option, 3);
        similar
            .iter()
            .map(|e| if e.success { 1.0 } else { 0.0 })
            .sum()
    }

    fn skill_score(&self, option: &str, _context: &DecisionContext) -> f32 {
        if let Some(ref skills) = self.skills {
            let skills_list = skills.list_skills();
            let relevant: Vec<_> = skills_list
                .iter()
                .filter(|s| {
                    s.name
                        .to_lowercase()
                        .contains(option.to_lowercase().as_str())
                })
                .collect();
            if !relevant.is_empty() {
                return relevant.iter().map(|s| s.confidence).sum::<f32>() / relevant.len() as f32;
            }
        }
        0.0
    }

    pub fn get_history(&self) -> &[Decision] {
        &self.history
    }

    pub fn get_last_decision(&self) -> Option<&Decision> {
        self.history.last()
    }

    pub fn decision_count(&self) -> usize {
        self.history.len()
    }
}

impl Default for DecisionEngine {
    fn default() -> Self {
        Self::new()
    }
}
