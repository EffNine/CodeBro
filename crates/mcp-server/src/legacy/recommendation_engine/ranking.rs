//! Recommendation Ranking — priority ordering and duplicate removal.
//!
/// Rankings are deterministic and stable.
use super::types::*;

/// Rank recommendations by confidence (highest first), then by type priority.
///
/// Returns a new sorted Vec — the input is not modified.
pub fn rank(recommendations: Vec<Recommendation>) -> Vec<Recommendation> {
    let mut ranked = recommendations;
    ranked.sort_by(|a, b| {
        // Primary: confidence score (descending)
        let conf_cmp = b
            .confidence
            .score()
            .partial_cmp(&a.confidence.score())
            .unwrap_or(std::cmp::Ordering::Equal);
        if conf_cmp != std::cmp::Ordering::Equal {
            return conf_cmp;
        }
        // Secondary: type priority (alphabetical for stability)
        let type_cmp = a.rec_type.to_string().cmp(&b.rec_type.to_string());
        if type_cmp != std::cmp::Ordering::Equal {
            return type_cmp;
        }
        // Tertiary: title (alphabetical for stability)
        a.title.cmp(&b.title)
    });
    ranked
}

/// Remove duplicate recommendations based on title and target key.
///
/// Keeps the highest-confidence version of each duplicate.
pub fn deduplicate(recommendations: Vec<Recommendation>) -> Vec<Recommendation> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut result: Vec<Recommendation> = Vec::new();

    for rec in recommendations {
        let key = dedup_key(&rec);
        match seen.get(&key) {
            Some(&idx) => {
                // Keep the higher-confidence version
                if rec.confidence.score() > result[idx].confidence.score() {
                    result[idx] = rec;
                }
            }
            None => {
                seen.insert(key, result.len());
                result.push(rec);
            }
        }
    }

    result
}

/// Remove recommendations that conflict with each other.
///
/// Two recommendations conflict if they target the same key with different values.
pub fn remove_conflicts(recommendations: Vec<Recommendation>) -> (Vec<Recommendation>, usize) {
    let mut kept: Vec<Recommendation> = Vec::new();
    let mut conflicts_removed = 0;
    let mut target_keys: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    for rec in recommendations {
        if let Some(ref key) = rec.target_key {
            if let Some(&idx) = target_keys.get(key) {
                // Conflict detected — keep the higher-confidence one
                if rec.confidence.score() > kept[idx].confidence.score() {
                    let key_clone = key.clone();
                    kept[idx] = rec;
                    conflicts_removed += 1;
                    target_keys.insert(key_clone, idx);
                } else {
                    conflicts_removed += 1;
                    continue;
                }
            } else {
                target_keys.insert(key.clone(), kept.len());
                kept.push(rec);
            }
        } else {
            kept.push(rec);
        }
    }

    (kept, conflicts_removed)
}

/// Apply all ranking operations: sort, deduplicate, remove conflicts.
pub fn full_rank(recommendations: Vec<Recommendation>) -> (Vec<Recommendation>, usize, usize) {
    let ranked = rank(recommendations);
    let (deduped, dup_count) = deduplicate_with_count(ranked);
    let (final_recs, conflict_count) = remove_conflicts(deduped);
    (final_recs, dup_count, conflict_count)
}

fn dedup_key(rec: &Recommendation) -> String {
    format!(
        "{}:{}:{}",
        rec.title,
        rec.target_key.as_deref().unwrap_or(""),
        rec.rec_type.to_string()
    )
}

fn deduplicate_with_count(recommendations: Vec<Recommendation>) -> (Vec<Recommendation>, usize) {
    let original_len = recommendations.len();
    let deduped = deduplicate(recommendations);
    let count = original_len - deduped.len();
    (deduped, count)
}

fn dropped_count<T>(_iter: impl Iterator<Item = T>) {
    // intentionally unused
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rec(title: &str, confidence: f64, target_key: Option<&str>) -> Recommendation {
        Recommendation::new(
            RecommendationType::General,
            title,
            "Test explanation",
            vec!["Test evidence".to_string()],
            RecommendationConfidence::High(confidence),
            "test-rule",
            target_key.map(|s| s.to_string()),
            None,
            "plan-1",
        )
    }

    #[test]
    fn test_rank_sorts_by_confidence() {
        let recs = vec![
            make_rec("Low", 0.5, None),
            make_rec("High", 0.9, None),
            make_rec("Medium", 0.7, None),
        ];
        let ranked = rank(recs);
        assert!((ranked[0].confidence.score() - 0.9).abs() < 0.001);
        assert!((ranked[1].confidence.score() - 0.7).abs() < 0.001);
        assert!((ranked[2].confidence.score() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_rank_stable_for_same_confidence() {
        let recs = vec![
            make_rec("B", 0.7, None),
            make_rec("A", 0.7, None),
            make_rec("C", 0.7, None),
        ];
        let ranked = rank(recs);
        // Same confidence → alphabetical by type (all General, so by title)
        assert_eq!(ranked[0].title, "A");
        assert_eq!(ranked[1].title, "B");
        assert_eq!(ranked[2].title, "C");
    }

    #[test]
    fn test_deduplicate_keeps_highest() {
        let recs = vec![
            make_rec("Same Title", 0.5, Some("key1")),
            make_rec("Same Title", 0.9, Some("key1")),
            make_rec("Same Title", 0.7, Some("key1")),
        ];
        let deduped = deduplicate(recs);
        assert_eq!(deduped.len(), 1);
        assert!((deduped[0].confidence.score() - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_deduplicate_keeps_different_titles() {
        let recs = vec![
            make_rec("Title A", 0.5, Some("key1")),
            make_rec("Title B", 0.6, Some("key2")),
        ];
        let deduped = deduplicate(recs);
        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn test_remove_conflicts_keeps_higher() {
        let recs = vec![
            make_rec("Option A", 0.9, Some("setting")),
            make_rec("Option B", 0.5, Some("setting")),
        ];
        let (kept, removed) = remove_conflicts(recs);
        assert_eq!(kept.len(), 1);
        assert_eq!(removed, 1);
        assert!((kept[0].confidence.score() - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_remove_conflicts_no_conflict() {
        let recs = vec![
            make_rec("Option A", 0.9, Some("setting1")),
            make_rec("Option B", 0.5, Some("setting2")),
        ];
        let (kept, removed) = remove_conflicts(recs);
        assert_eq!(kept.len(), 2);
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_full_rank_pipeline() {
        let recs = vec![
            make_rec("Low Priority", 0.3, Some("key1")),
            make_rec("High Priority", 0.9, Some("key2")),
            make_rec("Medium Priority", 0.6, Some("key3")),
            make_rec("High Priority", 0.95, Some("key2")), // duplicate of High Priority (same title+key)
            make_rec("Low Priority", 0.5, Some("key1")), // duplicate of Low Priority (same title+key)
        ];
        let (final_recs, dup_count, conflict_count) = full_rank(recs);
        assert_eq!(final_recs.len(), 3);
        assert!(dup_count >= 2);
        assert!((final_recs[0].confidence.score() - 0.95).abs() < 0.001);
    }

    #[test]
    fn test_rank_empty_input() {
        let recs: Vec<Recommendation> = vec![];
        let ranked = rank(recs);
        assert!(ranked.is_empty());
    }

    #[test]
    fn test_deduplicate_empty_input() {
        let recs: Vec<Recommendation> = vec![];
        let deduped = deduplicate(recs);
        assert!(deduped.is_empty());
    }

    #[test]
    fn test_remove_conflicts_empty_input() {
        let recs: Vec<Recommendation> = vec![];
        let (kept, removed) = remove_conflicts(recs);
        assert!(kept.is_empty());
        assert_eq!(removed, 0);
    }
}
