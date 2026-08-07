#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::Result;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::agent::memory::MemoryEntry;

pub struct MemoryConsolidationEngine {
    config_dir: PathBuf,
}

impl MemoryConsolidationEngine {
    pub fn new(config_dir: PathBuf) -> Self {
        MemoryConsolidationEngine { config_dir }
    }

    pub fn consolidate(&self, memory: &mut crate::agent::memory::Memory) -> Vec<String> {
        self.remove_duplicates(memory);
        self.merge_similar(memory);
        self.remove_outdated(memory);
        self.remove_low_value(memory);

        memory
            .short_term
            .iter()
            .filter(|e| e.is_low_value(0.2))
            .map(|e| e.user_input.clone())
            .collect()
    }

    fn remove_duplicates(&self, memory: &mut crate::agent::memory::Memory) {
        let mut seen = HashSet::new();
        memory.short_term.retain(|entry| {
            let normalized = entry.user_input.trim().to_lowercase();
            seen.insert(normalized)
        });
    }

    pub fn merge_similar(&self, memory: &mut crate::agent::memory::Memory) {
        let mut merged: Vec<MemoryEntry> = Vec::new();
        let mut used_indices = HashSet::new();

        for (i, entry) in memory.short_term.iter().enumerate() {
            if used_indices.contains(&i) {
                continue;
            }

            let mut best_match: Option<(usize, f32)> = None;

            for (j, other) in memory.short_term.iter().enumerate() {
                if i == j || used_indices.contains(&j) {
                    continue;
                }

                let similarity = self.compute_similarity(&entry.user_input, &other.user_input);
                if similarity > 0.7 {
                    if best_match.as_ref().map_or(true, |(_, s)| similarity > *s) {
                        best_match = Some((j, similarity));
                    }
                }
            }

            if let Some((match_idx, _)) = best_match {
                used_indices.insert(match_idx);
                let combined = MemoryEntry {
                    user_input: format!(
                        "Project: {}",
                        self.merge_texts(
                            &entry.user_input,
                            &memory.short_term[match_idx].user_input
                        )
                    ),
                    response: format!(
                        "{} {}",
                        entry.response, memory.short_term[match_idx].response
                    ),
                    timestamp: entry.timestamp.clone(),
                    session_id: entry.session_id.clone(),
                    importance: entry
                        .importance
                        .max(memory.short_term[match_idx].importance),
                    confidence: (entry.confidence + memory.short_term[match_idx].confidence) / 2.0,
                    usage_count: entry.usage_count + memory.short_term[match_idx].usage_count,
                    last_used: entry
                        .last_used
                        .clone()
                        .or_else(|| memory.short_term[match_idx].last_used.clone()),
                };
                merged.push(combined);
            } else {
                merged.push(entry.clone());
            }
        }

        memory.short_term = merged;
    }

    pub fn remove_outdated(&self, memory: &mut crate::agent::memory::Memory) {
        memory.short_term.retain(|entry| !entry.is_outdated(90));
    }

    pub fn remove_low_value(&self, memory: &mut crate::agent::memory::Memory) {
        memory.short_term.retain(|entry| !entry.is_low_value(0.2));
    }

    fn compute_similarity(&self, a: &str, b: &str) -> f32 {
        let a_lower = a.to_lowercase();
        let b_lower = b.to_lowercase();

        if a_lower == b_lower {
            return 1.0;
        }

        let a_words: HashSet<&str> = a_lower.split_whitespace().collect();
        let b_words: HashSet<&str> = b_lower.split_whitespace().collect();

        if a_words.is_empty() || b_words.is_empty() {
            return 0.0;
        }

        let intersection = a_words.intersection(&b_words).count();
        let union = a_words.union(&b_words).count();

        if union == 0 {
            return 0.0;
        }

        intersection as f32 / union as f32
    }

    fn merge_texts(&self, a: &str, b: &str) -> String {
        let a_lower = a.to_lowercase();
        let b_lower = b.to_lowercase();

        if a_lower.contains(&b_lower) {
            return a.to_string();
        }
        if b_lower.contains(&a_lower) {
            return b.to_string();
        }

        let a_words: Vec<&str> = a.split_whitespace().collect();
        let b_words: Vec<&str> = b.split_whitespace().collect();

        let common: Vec<&str> = a_words
            .iter()
            .filter(|w| {
                b_words
                    .iter()
                    .any(|bw| bw.to_lowercase() == w.to_lowercase())
            })
            .copied()
            .collect();

        if !common.is_empty() {
            format!("{} ({})", common.join(" "), a)
        } else {
            format!("{}; {}", a, b)
        }
    }
}

impl MemoryEntry {
    pub fn score(&self) -> f32 {
        let recency = self.recency_score();
        let importance = self.importance;
        let confidence = self.confidence;
        let frequency = (self.usage_count as f32).min(10.0) / 10.0;

        importance * 0.3 + confidence * 0.25 + frequency * 0.2 + recency * 0.25
    }

    fn recency_score(&self) -> f32 {
        let now = chrono::Local::now();
        let parsed = chrono::DateTime::parse_from_rfc3339(&self.timestamp).ok();
        match parsed {
            Some(dt) => {
                let age_hours = (now - dt.with_timezone(&chrono::Local)).num_hours();
                if age_hours < 1 {
                    1.0
                } else if age_hours < 24 {
                    0.8
                } else if age_hours < 168 {
                    0.5
                } else if age_hours < 720 {
                    0.3
                } else {
                    0.1
                }
            }
            None => 0.5,
        }
    }

    pub fn is_outdated(&self, max_age_days: u64) -> bool {
        let now = chrono::Local::now();
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&self.timestamp) {
            let age_days = (now - dt.with_timezone(&chrono::Local)).num_days();
            age_days > max_age_days as i64
        } else {
            false
        }
    }

    pub fn is_low_value(&self, min_score: f32) -> bool {
        self.score() < min_score && self.usage_count < 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_entry_score() {
        let entry = MemoryEntry {
            user_input: "test".to_string(),
            response: "response".to_string(),
            timestamp: chrono::Local::now().to_rfc3339(),
            session_id: None,
            importance: 0.8,
            confidence: 0.9,
            usage_count: 5,
            last_used: Some(chrono::Local::now().to_rfc3339()),
        };
        assert!(entry.score() > 0.5);
    }

    #[test]
    fn test_memory_entry_recency_score() {
        let entry = MemoryEntry {
            user_input: "test".to_string(),
            response: "response".to_string(),
            timestamp: chrono::Local::now().to_rfc3339(),
            session_id: None,
            importance: 0.5,
            confidence: 0.5,
            usage_count: 0,
            last_used: None,
        };
        assert_eq!(entry.recency_score(), 1.0);
    }

    #[test]
    fn test_memory_entry_outdated() {
        let entry = MemoryEntry {
            user_input: "old".to_string(),
            response: "response".to_string(),
            timestamp: "2020-01-01T00:00:00Z".to_string(),
            session_id: None,
            importance: 0.5,
            confidence: 0.5,
            usage_count: 0,
            last_used: None,
        };
        assert!(entry.is_outdated(30));
    }

    #[test]
    fn test_memory_entry_not_outdated() {
        let entry = MemoryEntry {
            user_input: "recent".to_string(),
            response: "response".to_string(),
            timestamp: chrono::Local::now().to_rfc3339(),
            session_id: None,
            importance: 0.5,
            confidence: 0.5,
            usage_count: 0,
            last_used: None,
        };
        assert!(!entry.is_outdated(90));
    }

    #[test]
    fn test_compute_similarity_identical() {
        let engine = MemoryConsolidationEngine {
            config_dir: PathBuf::from("/tmp"),
        };
        assert_eq!(engine.compute_similarity("hello world", "hello world"), 1.0);
    }

    #[test]
    fn test_compute_similarity_no_overlap() {
        let engine = MemoryConsolidationEngine {
            config_dir: PathBuf::from("/tmp"),
        };
        assert_eq!(engine.compute_similarity("abc", "xyz"), 0.0);
    }

    #[test]
    fn test_compute_similarity_partial() {
        let engine = MemoryConsolidationEngine {
            config_dir: PathBuf::from("/tmp"),
        };
        let score = engine.compute_similarity("cargo test", "cargo test passed");
        assert!(score > 0.5);
    }

    #[test]
    fn test_remove_duplicates() {
        let mut memory = crate::agent::memory::Memory::default();
        memory.add_entry("cargo test".to_string(), "passed".to_string());
        memory.add_entry("cargo test".to_string(), "passed".to_string());
        memory.add_entry("rust build".to_string(), "success".to_string());

        let engine = MemoryConsolidationEngine {
            config_dir: PathBuf::from("/tmp"),
        };
        engine.remove_duplicates(&mut memory);

        assert!(memory.short_term.len() <= 2);
    }

    #[test]
    fn test_remove_outdated() {
        let mut memory = crate::agent::memory::Memory::default();
        memory.add_entry("old task".to_string(), "done".to_string());
        memory.short_term[0].timestamp = "2020-01-01T00:00:00Z".to_string();

        let engine = MemoryConsolidationEngine {
            config_dir: PathBuf::from("/tmp"),
        };
        engine.remove_outdated(&mut memory);

        assert_eq!(memory.short_term.len(), 0);
    }

    #[test]
    fn test_merge_texts() {
        let engine = MemoryConsolidationEngine {
            config_dir: PathBuf::from("/tmp"),
        };
        let merged = engine.merge_texts("cargo test", "cargo test passed");
        assert!(merged.contains("cargo test"));
    }

    #[test]
    fn test_consolidate_reduces_duplicates() {
        let mut memory = crate::agent::memory::Memory::default();
        memory.add_entry("cargo test".to_string(), "passed".to_string());
        memory.add_entry("cargo test".to_string(), "passed".to_string());
        memory.add_entry("different task".to_string(), "done".to_string());

        let engine = MemoryConsolidationEngine {
            config_dir: PathBuf::from("/tmp"),
        };
        let reduced = engine.consolidate(&mut memory);
        // After consolidation, duplicates are removed and low-value entries
        // are filtered. Should have at most 2 entries.
        assert!(memory.short_term.len() <= 2);
        assert!(reduced.len() <= 2);
    }
}
