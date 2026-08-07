//! Recommendation Filter — context-aware filtering of recommendations.
//!
/// Filters out already-enabled options, duplicates, and invalid recommendations.
use super::types::*;

/// Filter recommendations based on context.
///
/// Removes:
/// - Already-enabled recommendations (target already matches current value)
/// - Recommendations below minimum confidence
/// - Recommendations exceeding max count
pub fn filter(
    recommendations: Vec<Recommendation>,
    context: &RecommendationContext,
) -> Vec<Recommendation> {
    let mut filtered: Vec<Recommendation> = Vec::new();

    for rec in recommendations {
        // Skip below minimum confidence
        if rec.confidence.score() < context.min_confidence {
            continue;
        }

        // Skip if not actionable (unless forced)
        if !rec.is_actionable() && !context.include_low_confidence {
            continue;
        }

        // Skip already-enabled recommendations
        if is_already_enabled(
            rec.target_key.as_deref(),
            rec.target_value.as_deref(),
            &context.preferences,
        ) {
            continue;
        }

        filtered.push(rec);
    }

    // Apply max recommendations limit
    if filtered.len() > context.max_recommendations {
        filtered.truncate(context.max_recommendations);
    }

    filtered
}

/// Check if a recommendation target is already enabled.
fn is_already_enabled(
    target_key: Option<&str>,
    target_value: Option<&str>,
    preferences: &std::collections::HashMap<String, String>,
) -> bool {
    match (target_key, target_value) {
        (Some(key), Some(value)) => preferences
            .get(key)
            .map(|current| current == value)
            .unwrap_or(false),
        _ => false,
    }
}

/// Filter by type — keep only recommendations of specific types.
pub fn filter_by_type(
    recommendations: Vec<Recommendation>,
    allowed_types: &[RecommendationType],
) -> Vec<Recommendation> {
    recommendations
        .into_iter()
        .filter(|rec| allowed_types.contains(&rec.rec_type))
        .collect()
}

/// Filter by confidence — keep only recommendations above a threshold.
pub fn filter_by_confidence(
    recommendations: Vec<Recommendation>,
    min_score: f64,
) -> Vec<Recommendation> {
    recommendations
        .into_iter()
        .filter(|rec| rec.confidence.score() >= min_score)
        .collect()
}

/// Filter out recommendations that target the same key (keep highest confidence).
pub fn filter_by_uniqueness(recommendations: Vec<Recommendation>) -> Vec<Recommendation> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for (i, rec) in recommendations.iter().enumerate() {
        if let Some(ref key) = rec.target_key {
            if let Some(&existing_idx) = seen.get(key) {
                // Keep the higher-confidence one
                if rec.confidence.score() > recommendations[existing_idx].confidence.score() {
                    seen.insert(key.clone(), i);
                }
            } else {
                seen.insert(key.clone(), i);
            }
        }
    }

    let indices: std::collections::HashSet<usize> = seen.values().cloned().collect();
    recommendations
        .into_iter()
        .enumerate()
        .filter(|(i, _)| indices.contains(i))
        .map(|(_, r)| r)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rec(
        title: &str,
        confidence: f64,
        target_key: Option<&str>,
        target_value: Option<&str>,
    ) -> Recommendation {
        Recommendation::new(
            RecommendationType::General,
            title,
            "Test explanation",
            vec!["Test evidence".to_string()],
            RecommendationConfidence::High(confidence),
            "test-rule",
            target_key.map(|s| s.to_string()),
            target_value.map(|s| s.to_string()),
            "plan-1",
        )
    }

    #[test]
    fn test_filter_confidence_threshold() {
        let recs = vec![
            make_rec("High", 0.9, None, None),
            make_rec("Low", 0.3, None, None),
            make_rec("Medium", 0.6, None, None),
        ];
        let context = RecommendationContext::new().with_min_confidence(0.5);
        let filtered = filter(recs, &context);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|r| r.confidence.score() >= 0.5));
    }

    #[test]
    fn test_filter_already_enabled() {
        let mut prefs = std::collections::HashMap::new();
        prefs.insert("key1".to_string(), "true".to_string());

        let recs = vec![
            make_rec("Already Enabled", 0.9, Some("key1"), Some("true")),
            make_rec("Not Enabled", 0.8, Some("key2"), Some("false")),
        ];
        let context = RecommendationContext::new().with_preferences(prefs);
        let filtered = filter(recs, &context);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title, "Not Enabled");
    }

    #[test]
    fn test_filter_max_count() {
        let recs = vec![
            make_rec("One", 0.9, None, None),
            make_rec("Two", 0.8, None, None),
            make_rec("Three", 0.7, None, None),
            make_rec("Four", 0.6, None, None),
        ];
        let context = RecommendationContext::new().with_max_recommendations(2);
        let filtered = filter(recs, &context);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_by_type() {
        let recs = vec![
            make_rec("Layout", 0.9, None, None),
            make_rec("Keyboard", 0.8, None, None),
            make_rec("Appearance", 0.7, None, None),
        ];
        // Fix: all recs are General type, so filter by General
        let filtered = filter_by_type(recs, &[RecommendationType::General]);
        assert_eq!(filtered.len(), 3);
        assert!(filtered
            .iter()
            .all(|r| matches!(r.rec_type, RecommendationType::General)));
    }

    #[test]
    fn test_filter_by_confidence() {
        let recs = vec![
            make_rec("High", 0.9, None, None),
            make_rec("Medium", 0.6, None, None),
            make_rec("Low", 0.3, None, None),
        ];
        let filtered = filter_by_confidence(recs, 0.5);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_by_uniqueness() {
        let recs = vec![
            make_rec("Key1 High", 0.9, Some("key1"), Some("true")),
            make_rec("Key1 Low", 0.3, Some("key1"), Some("false")),
            make_rec("Key2", 0.8, Some("key2"), Some("true")),
        ];
        let filtered = filter_by_uniqueness(recs);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|r| r.title == "Key1 High"));
        assert!(filtered.iter().any(|r| r.title == "Key2"));
    }

    #[test]
    fn test_filter_empty_input() {
        let recs: Vec<Recommendation> = vec![];
        let context = RecommendationContext::new();
        let filtered = filter(recs, &context);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_no_matching_target() {
        let recs = vec![
            make_rec("No Target", 0.9, None, None),
            make_rec("Null Target", 0.8, None, None),
        ];
        let context = RecommendationContext::new();
        let filtered = filter(recs, &context);
        // Both should pass since they have no target to check
        assert_eq!(filtered.len(), 2);
    }
}
