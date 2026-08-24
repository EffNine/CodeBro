//! Adaptive Validation Confidence — structured confidence evaluation.
//!
/// Computes and validates confidence scores across the pipeline.
use super::types::*;

/// Computes confidence assessment for validation.
#[derive(Debug, Clone, Default)]
pub struct ConfidenceEvaluator {
    _private: (),
}

impl ConfidenceEvaluator {
    pub fn new() -> Self {
        ConfidenceEvaluator { _private: () }
    }

    /// Evaluate confidence for a validation input.
    pub fn evaluate(&self, input: &str, config: &ValidationConfig) -> f64 {
        let mut score = 1.0;

        // Penalize low confidence indicators
        if input.contains("low_confidence") {
            score -= 0.3;
        }

        // Penalize ambiguity
        if input.contains("ambiguous") || input.contains("unclear") {
            score -= 0.2;
        }

        // Penalize missing information
        if input.trim().len() < 5 {
            score -= 0.1;
        }

        // Ensure score doesn't go below 0
        if score < 0.0_f64 {
            0.0_f64
        } else {
            score
        }
    }

    /// Check if confidence meets threshold.
    pub fn is_above_threshold(&self, score: f64, config: &ValidationConfig) -> bool {
        score >= config.min_confidence
    }

    /// Get confidence risk level.
    pub fn risk_level_for_confidence(&self, score: f64) -> RiskLevel {
        if score >= 0.8 {
            RiskLevel::Info
        } else if score >= 0.6 {
            RiskLevel::Low
        } else if score >= 0.4 {
            RiskLevel::Medium
        } else if score >= 0.2 {
            RiskLevel::High
        } else {
            RiskLevel::Critical
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_normal_input() {
        let evaluator = ConfidenceEvaluator::new();
        let config = ValidationConfig::new();
        let score = evaluator.evaluate("normal input", &config);
        assert!(score >= 0.9);
    }

    #[test]
    fn test_evaluate_low_confidence() {
        let evaluator = ConfidenceEvaluator::new();
        let config = ValidationConfig::new();
        let score = evaluator.evaluate("low_confidence input", &config);
        assert!(score < 0.9);
    }

    #[test]
    fn test_evaluate_ambiguous() {
        let evaluator = ConfidenceEvaluator::new();
        let config = ValidationConfig::new();
        let score = evaluator.evaluate("ambiguous input", &config);
        assert!(score < 0.9);
    }

    #[test]
    fn test_is_above_threshold() {
        let evaluator = ConfidenceEvaluator::new();
        let config = ValidationConfig::new().with_min_confidence(0.5);
        assert!(evaluator.is_above_threshold(0.8, &config));
        assert!(!evaluator.is_above_threshold(0.3, &config));
    }

    #[test]
    fn test_risk_level_for_confidence() {
        let evaluator = ConfidenceEvaluator::new();
        assert_eq!(evaluator.risk_level_for_confidence(0.9), RiskLevel::Info);
        assert_eq!(evaluator.risk_level_for_confidence(0.7), RiskLevel::Low);
        assert_eq!(evaluator.risk_level_for_confidence(0.5), RiskLevel::Medium);
        assert_eq!(evaluator.risk_level_for_confidence(0.3), RiskLevel::High);
        assert_eq!(
            evaluator.risk_level_for_confidence(0.1),
            RiskLevel::Critical
        );
    }
}
