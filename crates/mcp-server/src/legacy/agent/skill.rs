#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use crate::error::{CodeBroError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SkillStatus {
    Draft,
    Testing,
    Trusted,
    Deprecated,
}

impl Default for SkillStatus {
    fn default() -> Self {
        SkillStatus::Draft
    }
}

impl std::fmt::Display for SkillStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillStatus::Draft => write!(f, "draft"),
            SkillStatus::Testing => write!(f, "testing"),
            SkillStatus::Trusted => write!(f, "trusted"),
            SkillStatus::Deprecated => write!(f, "deprecated"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub trigger_conditions: Vec<String>,
    pub workflow: Vec<String>,
    pub examples: Vec<String>,
    pub tools_used: Vec<String>,
    pub files_changed: Vec<String>,
    pub confidence: f32,
    pub usage_count: u32,
    pub success_count: u32,
    pub failure_count: u32,
    pub status: SkillStatus,
    pub project_specific: bool,
    pub language: Option<String>,
    pub framework: Option<String>,
    pub created_at: String,
    pub last_used: Option<String>,
}

impl Skill {
    pub fn is_applicable(
        &self,
        project_language: Option<&str>,
        project_framework: Option<&str>,
    ) -> bool {
        if self.status == SkillStatus::Deprecated {
            return false;
        }

        if self.confidence < 0.3 {
            return false;
        }

        if let Some(lang) = &self.language {
            if let Some(project_lang) = project_language {
                if lang.to_lowercase() != project_lang.to_lowercase() {
                    return false;
                }
            }
        }

        if let Some(fw) = &self.framework {
            if let Some(project_fw) = project_framework {
                if fw.to_lowercase() != project_fw.to_lowercase() {
                    return false;
                }
            }
        }

        true
    }

    pub fn success_rate(&self) -> f32 {
        if self.usage_count == 0 {
            return 0.0;
        }
        self.success_count as f32 / self.usage_count as f32
    }

    pub fn advance_status(&mut self) {
        match self.status {
            SkillStatus::Draft => {
                if self.success_count >= 3 && self.success_rate() >= 0.7 {
                    self.status = SkillStatus::Testing;
                }
            }
            SkillStatus::Testing => {
                if self.success_rate() >= 0.8 && self.usage_count >= 5 {
                    self.status = SkillStatus::Trusted;
                }
            }
            SkillStatus::Trusted => {
                if self.failure_count >= 5 && self.success_rate() < 0.4 {
                    self.status = SkillStatus::Deprecated;
                }
            }
            SkillStatus::Deprecated => {}
        }
        self.update_confidence();
    }

    pub fn demote_status(&mut self) {
        match self.status {
            SkillStatus::Testing => {
                self.status = SkillStatus::Draft;
            }
            SkillStatus::Trusted => {
                self.status = SkillStatus::Testing;
            }
            SkillStatus::Deprecated => {}
            SkillStatus::Draft => {}
        }
        self.update_confidence();
    }

    pub fn update_confidence(&mut self) {
        if self.usage_count == 0 {
            self.confidence = 0.5;
            return;
        }

        let base_rate = self.success_count as f32 / self.usage_count as f32;
        let usage_factor = (self.usage_count as f32).min(50.0) / 50.0;
        let recency_bonus = if let Some(ref last) = self.last_used {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(last) {
                let age_days = (chrono::Local::now() - dt.with_timezone(&chrono::Local)).num_days();
                if age_days < 7 {
                    0.1
                } else if age_days < 30 {
                    0.05
                } else {
                    0.0
                }
            } else {
                0.0
            }
        } else {
            0.0
        };

        self.confidence = (base_rate * 0.7 + usage_factor * 0.2 + recency_bonus).clamp(0.0, 1.0);
    }
}

impl Default for Skill {
    fn default() -> Self {
        Skill {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            trigger_conditions: Vec::new(),
            workflow: Vec::new(),
            examples: Vec::new(),
            tools_used: Vec::new(),
            files_changed: Vec::new(),
            confidence: 0.5,
            usage_count: 0,
            success_count: 0,
            failure_count: 0,
            status: SkillStatus::Draft,
            project_specific: false,
            language: None,
            framework: None,
            created_at: String::new(),
            last_used: None,
        }
    }
}

pub struct SkillManager {
    skills_dir: PathBuf,
    skills: HashMap<String, Skill>,
}

impl SkillManager {
    pub fn new(skills_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&skills_dir).map_err(|e| CodeBroError::Config(e.to_string()))?;

        let mut manager = SkillManager {
            skills_dir,
            skills: HashMap::new(),
        };

        manager.load_skills()?;
        Ok(manager)
    }

    pub fn create_skill(
        &mut self,
        name: String,
        description: String,
        trigger_conditions: Vec<String>,
        workflow: Vec<String>,
        examples: Vec<String>,
        tools_used: Vec<String>,
        files_changed: Vec<String>,
    ) -> Result<Skill> {
        let id = uuid::Uuid::new_v4().to_string();
        let skill = Skill {
            id: id.clone(),
            name,
            description,
            trigger_conditions,
            workflow,
            examples,
            tools_used,
            files_changed,
            confidence: 0.5,
            usage_count: 0,
            success_count: 0,
            failure_count: 0,
            status: SkillStatus::Draft,
            project_specific: false,
            language: None,
            framework: None,
            created_at: chrono::Local::now().to_rfc3339(),
            last_used: None,
        };

        self.save_skill(&skill)?;
        self.skills.insert(id.clone(), skill.clone());
        Ok(skill)
    }

    pub fn get_skill(&self, id: &str) -> Option<&Skill> {
        self.skills.get(id)
    }

    pub fn get_skill_mut(&mut self, id: &str) -> Option<&mut Skill> {
        self.skills.get_mut(id)
    }

    pub fn find_skills_by_trigger(&self, query: &str) -> Vec<&Skill> {
        let query_lower = query.to_lowercase();
        let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(&Skill, f32)> = self
            .skills
            .values()
            .filter(|skill| skill.is_applicable(None, None))
            .map(|skill| {
                let mut score = 0.0f32;
                let name_lower = skill.name.to_lowercase();
                let desc_lower = skill.description.to_lowercase();

                for term in &query_terms {
                    if name_lower.contains(term) {
                        score += 3.0;
                    }
                    if desc_lower.contains(term) {
                        score += 2.0;
                    }
                    for trigger in &skill.trigger_conditions {
                        if trigger.to_lowercase().contains(term) {
                            score += 2.5;
                        }
                    }
                }

                score += skill.confidence * 2.0;
                score += skill.success_rate() * 1.0;

                (skill, score)
            })
            .filter(|(_, score)| *score > 0.0)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().map(|(skill, _)| skill).collect()
    }

    pub fn rank_skills(&self, query: &str) -> Vec<&Skill> {
        self.find_skills_by_trigger(query)
    }

    pub fn find_best_skill(
        &self,
        _query: &str,
        project_language: Option<&str>,
        project_framework: Option<&str>,
    ) -> Option<&Skill> {
        let mut candidates: Vec<&Skill> = self
            .skills
            .values()
            .filter(|skill| {
                skill.is_applicable(project_language, project_framework)
                    && skill.status != SkillStatus::Deprecated
            })
            .collect();

        candidates.sort_by(|a, b| {
            b.project_specific
                .cmp(&a.project_specific)
                .then_with(|| {
                    let b_last = b
                        .last_used
                        .as_ref()
                        .and_then(|l| chrono::DateTime::parse_from_rfc3339(l).ok())
                        .unwrap_or_else(|| {
                            chrono::DateTime::from_timestamp(i64::MIN, 0)
                                .unwrap()
                                .into()
                        });
                    let a_last = a
                        .last_used
                        .as_ref()
                        .and_then(|l| chrono::DateTime::parse_from_rfc3339(l).ok())
                        .unwrap_or_else(|| {
                            chrono::DateTime::from_timestamp(i64::MIN, 0)
                                .unwrap()
                                .into()
                        });
                    b_last.cmp(&a_last)
                })
                .then_with(|| {
                    b.success_rate()
                        .partial_cmp(&a.success_rate())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| {
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        candidates.first().copied()
    }

    pub fn record_usage(&mut self, skill_id: &str, success: bool) -> Result<()> {
        let skill_id = skill_id.to_string();
        if let Some(skill) = self.skills.get_mut(&skill_id) {
            skill.usage_count += 1;
            if success {
                skill.success_count += 1;
            } else {
                skill.failure_count += 1;
            }
            skill.last_used = Some(chrono::Local::now().to_rfc3339());
            skill.update_confidence();
            skill.advance_status();
            let skill_clone = skill.clone();
            let _ = skill;
            self.save_skill(&skill_clone)?;
        }
        Ok(())
    }

    pub fn list_skills(&self) -> Vec<&Skill> {
        self.skills.values().collect()
    }

    pub fn remove_skill(&mut self, id: &str) -> Result<bool> {
        if self.skills.remove(id).is_some() {
            let path = self.skills_dir.join(format!("{}.json", id));
            if path.exists() {
                fs::remove_file(path).map_err(|e| CodeBroError::Config(e.to_string()))?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn validate_skill_compatibility(
        &self,
        skill_id: &str,
        project_language: Option<&str>,
        project_framework: Option<&str>,
    ) -> bool {
        if let Some(skill) = self.skills.get(skill_id) {
            skill.is_applicable(project_language, project_framework)
        } else {
            false
        }
    }

    fn load_skills(&mut self) -> Result<()> {
        if !self.skills_dir.exists() {
            return Ok(());
        }

        for entry in
            fs::read_dir(&self.skills_dir).map_err(|e| CodeBroError::Config(e.to_string()))?
        {
            let entry = entry.map_err(|e| CodeBroError::Config(e.to_string()))?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(skill) = serde_json::from_str::<Skill>(&content) {
                        self.skills.insert(skill.id.clone(), skill);
                    }
                }
            }
        }

        Ok(())
    }

    fn save_skill(&self, skill: &Skill) -> Result<()> {
        let path = self.skills_dir.join(format!("{}.json", skill.id));
        let content =
            serde_json::to_string_pretty(skill).map_err(|e| CodeBroError::Config(e.to_string()))?;
        fs::write(path, content).map_err(|e| CodeBroError::Config(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_lifecycle_draft_to_testing() {
        let mut skill = Skill::default();
        skill.name = "test_skill".to_string();
        skill.status = SkillStatus::Draft;

        for _ in 0..3 {
            skill.success_count += 1;
            skill.usage_count += 1;
        }
        skill.update_confidence();
        skill.advance_status();

        assert_eq!(skill.status, SkillStatus::Testing);
    }

    #[test]
    fn test_skill_lifecycle_testing_to_trusted() {
        let mut skill = Skill::default();
        skill.name = "test_skill".to_string();
        skill.status = SkillStatus::Testing;

        for _ in 0..5 {
            skill.success_count += 1;
            skill.usage_count += 1;
        }
        skill.update_confidence();
        skill.advance_status();

        assert_eq!(skill.status, SkillStatus::Trusted);
    }

    #[test]
    fn test_skill_confidence_update_success() {
        let mut skill = Skill::default();
        skill.name = "test_skill".to_string();
        skill.confidence = 0.5;

        for _ in 0..10 {
            skill.success_count += 1;
            skill.usage_count += 1;
        }
        skill.update_confidence();

        assert!(skill.confidence > 0.5);
    }

    #[test]
    fn test_skill_confidence_update_failure() {
        let mut skill = Skill::default();
        skill.name = "test_skill".to_string();
        skill.confidence = 0.5;

        for _ in 0..10 {
            skill.failure_count += 1;
            skill.usage_count += 1;
        }
        skill.update_confidence();

        assert!(skill.confidence < 0.5);
    }

    #[test]
    fn test_skill_not_applicable_deprecated() {
        let mut skill = Skill::default();
        skill.name = "old_skill".to_string();
        skill.status = SkillStatus::Deprecated;

        assert!(!skill.is_applicable(Some("rust"), Some("cargo")));
    }

    #[test]
    fn test_skill_not_applicable_low_confidence() {
        let mut skill = Skill::default();
        skill.name = "low_conf_skill".to_string();
        skill.confidence = 0.2;

        assert!(!skill.is_applicable(Some("rust"), Some("cargo")));
    }

    #[test]
    fn test_skill_language_mismatch() {
        let mut skill = Skill::default();
        skill.name = "rust_skill".to_string();
        skill.language = Some("rust".to_string());

        assert!(!skill.is_applicable(Some("python"), None));
    }

    #[test]
    fn test_skill_language_match() {
        let mut skill = Skill::default();
        skill.name = "rust_skill".to_string();
        skill.language = Some("rust".to_string());

        assert!(skill.is_applicable(Some("rust"), None));
    }

    #[test]
    fn test_skill_conflict_resolution() {
        let dir = std::env::temp_dir();
        let mut manager = SkillManager::new(dir.join("skills_test_conflict")).unwrap();

        let skill1 = manager
            .create_skill(
                "rust_test".to_string(),
                "Rust test skill".to_string(),
                vec!["test".to_string()],
                vec!["run_command".to_string()],
                vec![],
                vec!["run_command".to_string()],
                vec![],
            )
            .unwrap();

        let skill2 = manager
            .create_skill(
                "general_test".to_string(),
                "General test skill".to_string(),
                vec!["test".to_string()],
                vec!["run_command".to_string()],
                vec![],
                vec!["run_command".to_string()],
                vec![],
            )
            .unwrap();

        manager.record_usage(&skill1.id, true).unwrap();
        manager.record_usage(&skill1.id, true).unwrap();
        manager.record_usage(&skill1.id, true).unwrap();

        manager.record_usage(&skill2.id, true).unwrap();

        let best = manager.find_best_skill("test", Some("rust"), None);
        assert!(best.is_some());
    }

    #[test]
    fn test_skill_validation() {
        let dir = std::env::temp_dir();
        let mut manager = SkillManager::new(dir.join("skills_test_validation")).unwrap();

        let skill = manager
            .create_skill(
                "rust_skill".to_string(),
                "Rust specific skill".to_string(),
                vec!["build".to_string()],
                vec!["run_command".to_string()],
                vec![],
                vec!["run_command".to_string()],
                vec![],
            )
            .unwrap();

        assert!(manager.validate_skill_compatibility(&skill.id, Some("rust"), None));
        assert!(manager.validate_skill_compatibility(&skill.id, Some("python"), None));
    }

    #[test]
    fn test_skill_manager_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skills");
        let mut manager = SkillManager::new(path.clone()).unwrap();
        let _skill = manager
            .create_skill(
                "rust".to_string(),
                "Rust API".to_string(),
                vec!["test".to_string()],
                vec!["run_command".to_string()],
                vec![],
                vec!["run_command".to_string()],
                vec![],
            )
            .unwrap();
        drop(manager);

        let loaded = SkillManager::new(path).unwrap();
        let skills = loaded.list_skills();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "rust");
    }

    #[test]
    fn test_skill_manager_load_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skills");
        // Should not fail when directory doesn't exist yet
        let manager = SkillManager::new(path);
        assert!(manager.is_ok());
    }
}
