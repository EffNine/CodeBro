//! Recommendation Engine Types — core data model.
//!
//! All types are immutable, serializable, and auditable.
//! Recommendations are read-only observations — never mutations.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

// ─── Recommendation Type ────────────────────────────────────────────────────

/// The category of a recommendation.
///
/// Each type corresponds to a specific domain of settings or preferences.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RecommendationType {
    /// UI/UX layout recommendation.
    Layout,
    /// Visual appearance recommendation.
    Appearance,
    /// Keyboard interaction recommendation.
    Keyboard,
    /// Integration/plugin recommendation.
    Integration,
    /// Performance tuning recommendation.
    Performance,
    /// Workflow automation recommendation.
    Workflow,
    /// Language/locale recommendation.
    Language,
    /// Editor/IDE preference recommendation.
    Editor,
    /// Notification/alert recommendation.
    Notification,
    /// General suggestion.
    General,
}

impl fmt::Display for RecommendationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecommendationType::Layout => write!(f, "layout"),
            RecommendationType::Appearance => write!(f, "appearance"),
            RecommendationType::Keyboard => write!(f, "keyboard"),
            RecommendationType::Integration => write!(f, "integration"),
            RecommendationType::Performance => write!(f, "performance"),
            RecommendationType::Workflow => write!(f, "workflow"),
            RecommendationType::Language => write!(f, "language"),
            RecommendationType::Editor => write!(f, "editor"),
            RecommendationType::Notification => write!(f, "notification"),
            RecommendationType::General => write!(f, "general"),
        }
    }
}

// ─── Recommendation Reason ──────────────────────────────────────────────────

/// Why a recommendation was made.
///
/// Tied to a specific rule that matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecommendationReason {
    /// Based on a detected intent pattern.
    IntentPattern {
        pattern: String,
        intent_type: String,
    },
    /// Based on a detected command pattern.
    CommandPattern {
        command_kind: String,
        command_detail: String,
    },
    /// Based on existing preference values.
    PreferenceValue { key: String, value: String },
    /// Based on user context.
    Context { context: String },
    /// General heuristic recommendation.
    Heuristic { heuristic: String },
}

impl fmt::Display for RecommendationReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecommendationReason::IntentPattern {
                pattern,
                intent_type,
            } => {
                write!(f, "intent_pattern({intent_type}: {pattern})")
            }
            RecommendationReason::CommandPattern {
                command_kind,
                command_detail,
            } => {
                write!(f, "command_pattern({command_kind}: {command_detail})")
            }
            RecommendationReason::PreferenceValue { key, value } => {
                write!(f, "preference_value({key}={value})")
            }
            RecommendationReason::Context { context } => {
                write!(f, "context({context})")
            }
            RecommendationReason::Heuristic { heuristic } => {
                write!(f, "heuristic({heuristic})")
            }
        }
    }
}

// ─── Recommendation Confidence ───────────────────────────────────────────────

/// Confidence level for a recommendation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RecommendationConfidence {
    /// High confidence — rule matched with strong evidence.
    High(f64),
    /// Medium confidence — rule matched with moderate evidence.
    Medium(f64),
    /// Low confidence — heuristic or weak pattern match.
    Low(f64),
}

impl RecommendationConfidence {
    pub fn score(&self) -> f64 {
        match self {
            RecommendationConfidence::High(s) => *s,
            RecommendationConfidence::Medium(s) => *s,
            RecommendationConfidence::Low(s) => *s,
        }
    }

    pub fn is_high(&self) -> bool {
        matches!(self, RecommendationConfidence::High(_))
    }

    pub fn is_low(&self) -> bool {
        matches!(self, RecommendationConfidence::Low(_))
    }
}

impl fmt::Display for RecommendationConfidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecommendationConfidence::High(s) => write!(f, "high({:.2})", s),
            RecommendationConfidence::Medium(s) => write!(f, "medium({:.2})", s),
            RecommendationConfidence::Low(s) => write!(f, "low({:.2})", s),
        }
    }
}

// ─── Recommendation ─────────────────────────────────────────────────────────

/// A single recommendation produced by the engine.
///
/// Immutable, serializable, fully explainable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recommendation {
    pub id: String,
    pub rec_type: RecommendationType,
    pub title: String,
    pub explanation: String,
    pub evidence: Vec<String>,
    pub confidence: RecommendationConfidence,
    pub source_rule: String,
    pub target_key: Option<String>,
    pub target_value: Option<String>,
    pub related_intent_id: String,
    pub created_at: String,
}

impl Recommendation {
    pub fn new(
        rec_type: RecommendationType,
        title: &str,
        explanation: &str,
        evidence: Vec<String>,
        confidence: RecommendationConfidence,
        source_rule: &str,
        target_key: Option<String>,
        target_value: Option<String>,
        related_intent_id: &str,
    ) -> Self {
        Recommendation {
            id: Uuid::new_v4().to_string(),
            rec_type,
            title: title.to_string(),
            explanation: explanation.to_string(),
            evidence,
            confidence,
            source_rule: source_rule.to_string(),
            target_key,
            target_value,
            related_intent_id: related_intent_id.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Check if this recommendation has high confidence.
    pub fn is_strong(&self) -> bool {
        self.confidence.is_high() && self.confidence.score() >= 0.8
    }

    /// Check if this recommendation is worth showing to the user.
    pub fn is_actionable(&self) -> bool {
        self.confidence.score() >= 0.5
    }
}

// ─── Recommendation Set ─────────────────────────────────────────────────────

/// A collection of recommendations for a single intent plan.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecommendationSet {
    pub intent_id: String,
    pub recommendations: Vec<Recommendation>,
    pub filtered_count: usize,
    pub duplicates_removed: usize,
    pub conflicts_removed: usize,
    pub generated_at: String,
}

impl RecommendationSet {
    pub fn new(intent_id: &str) -> Self {
        RecommendationSet {
            intent_id: intent_id.to_string(),
            recommendations: Vec::new(),
            filtered_count: 0,
            duplicates_removed: 0,
            conflicts_removed: 0,
            generated_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn add(&mut self, rec: Recommendation) {
        self.recommendations.push(rec);
    }

    pub fn is_empty(&self) -> bool {
        self.recommendations.is_empty()
    }

    pub fn len(&self) -> usize {
        self.recommendations.len()
    }

    /// Get recommendations sorted by confidence (highest first).
    pub fn sorted_by_confidence(&self) -> Vec<&Recommendation> {
        let mut indexed: Vec<_> = self.recommendations.iter().enumerate().collect();
        indexed.sort_by(|a, b| {
            b.1.confidence
                .score()
                .partial_cmp(&a.1.confidence.score())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        indexed.iter().map(|(_, r)| *r).collect()
    }

    /// Get recommendations of a specific type.
    pub fn by_type(&self, rec_type: &RecommendationType) -> Vec<&Recommendation> {
        self.recommendations
            .iter()
            .filter(|r| &r.rec_type == rec_type)
            .collect()
    }
}

// ─── Recommendation Context ─────────────────────────────────────────────────

/// Context passed to the recommendation engine for rule evaluation.
#[derive(Debug, Clone)]
pub struct RecommendationContext {
    /// Current user preferences (read-only view).
    pub preferences: std::collections::HashMap<String, String>,
    /// Maximum number of recommendations to produce.
    pub max_recommendations: usize,
    /// Minimum confidence threshold for inclusion.
    pub min_confidence: f64,
    /// Whether to include low-confidence recommendations.
    pub include_low_confidence: bool,
}

impl Default for RecommendationContext {
    fn default() -> Self {
        RecommendationContext {
            preferences: std::collections::HashMap::new(),
            max_recommendations: 10,
            min_confidence: 0.0,
            include_low_confidence: false,
        }
    }
}

impl RecommendationContext {
    pub fn new() -> Self {
        RecommendationContext::default()
    }

    pub fn with_max_recommendations(mut self, max: usize) -> Self {
        self.max_recommendations = max;
        self
    }

    pub fn with_min_confidence(mut self, min: f64) -> Self {
        self.min_confidence = min;
        self
    }

    pub fn with_preferences(mut self, prefs: std::collections::HashMap<String, String>) -> Self {
        self.preferences = prefs;
        self
    }
}
