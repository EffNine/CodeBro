//! Adaptive Validation Types — core data model.
//!
//! All types are immutable, serializable, and deterministic.

use serde::{Deserialize, Serialize};
use std::fmt;

// ─── Validation Result ───────────────────────────────────────────────────────

/// Overall validation result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationResult {
    /// All validations passed.
    Pass,
    /// Passed but with warnings.
    PassWithWarnings,
    /// Requires clarification before proceeding.
    RequiresClarification,
    /// Validation failed — cannot proceed.
    Reject,
}

impl fmt::Display for ValidationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationResult::Pass => write!(f, "PASS"),
            ValidationResult::PassWithWarnings => write!(f, "PASS_WITH_WARNINGS"),
            ValidationResult::RequiresClarification => write!(f, "REQUIRES_CLARIFICATION"),
            ValidationResult::Reject => write!(f, "REJECT"),
        }
    }
}

// ─── Risk Level ──────────────────────────────────────────────────────────────

/// Risk level for validation issues.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskLevel {
    /// Informational — no action needed.
    Info,
    /// Low risk — monitor but proceed.
    Low,
    /// Medium risk — review recommended.
    Medium,
    /// High risk — review required.
    High,
    /// Critical risk — block execution.
    Critical,
}

impl fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RiskLevel::Info => write!(f, "INFO"),
            RiskLevel::Low => write!(f, "LOW"),
            RiskLevel::Medium => write!(f, "MEDIUM"),
            RiskLevel::High => write!(f, "HIGH"),
            RiskLevel::Critical => write!(f, "CRITICAL"),
        }
    }
}

impl Default for RiskLevel {
    fn default() -> Self {
        RiskLevel::Info
    }
}

impl RiskLevel {
    /// Return numeric risk score (0-100).
    pub fn score(&self) -> u32 {
        match self {
            RiskLevel::Info => 0,
            RiskLevel::Low => 25,
            RiskLevel::Medium => 50,
            RiskLevel::High => 75,
            RiskLevel::Critical => 100,
        }
    }

    /// Check if this risk level blocks execution.
    pub fn is_blocking(&self) -> bool {
        matches!(self, RiskLevel::High | RiskLevel::Critical)
    }
}

// ─── Validation Category ─────────────────────────────────────────────────────

/// Category of validation check.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ValidationCategory {
    /// Workflow integrity validation.
    Workflow,
    /// Intent consistency validation.
    Intent,
    /// Recommendation consistency validation.
    Recommendation,
    /// Dependency integrity validation.
    Dependencies,
    /// Policy compliance validation.
    Policy,
    /// Preference consistency validation.
    Preference,
    /// Conflict detection validation.
    Conflict,
    /// Risk assessment validation.
    Risk,
    /// Confidence threshold validation.
    Confidence,
    /// Approval readiness validation.
    ApprovalReadiness,
}

impl fmt::Display for ValidationCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationCategory::Workflow => write!(f, "workflow"),
            ValidationCategory::Intent => write!(f, "intent"),
            ValidationCategory::Recommendation => write!(f, "recommendation"),
            ValidationCategory::Dependencies => write!(f, "dependencies"),
            ValidationCategory::Policy => write!(f, "policy"),
            ValidationCategory::Preference => write!(f, "preference"),
            ValidationCategory::Conflict => write!(f, "conflict"),
            ValidationCategory::Risk => write!(f, "risk"),
            ValidationCategory::Confidence => write!(f, "confidence"),
            ValidationCategory::ApprovalReadiness => write!(f, "approval_readiness"),
        }
    }
}

// ─── Validation Issue ────────────────────────────────────────────────────────

/// A validation issue found during evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// Unique issue identifier.
    pub issue_id: String,
    /// Category of the issue.
    pub category: ValidationCategory,
    /// Severity level.
    pub severity: RiskLevel,
    /// Human-readable message.
    pub message: String,
    /// Evidence supporting this issue.
    pub evidence: Vec<String>,
    /// Recommended action.
    pub recommended_action: String,
    /// Whether this issue blocks approval.
    pub blocks_approval: bool,
}

impl ValidationIssue {
    pub fn new(
        category: &ValidationCategory,
        severity: RiskLevel,
        message: &str,
        evidence: Vec<String>,
        recommended_action: &str,
        blocks_approval: bool,
    ) -> Self {
        ValidationIssue {
            issue_id: Self::generate_id(category, message),
            category: category.clone(),
            severity,
            message: message.to_string(),
            evidence,
            recommended_action: recommended_action.to_string(),
            blocks_approval,
        }
    }

    fn generate_id(category: &ValidationCategory, message: &str) -> String {
        let mut hash: u64 = 14695981039346656037;
        for byte in message.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(1099511628211);
        }
        format!("issue_{}_{:x}", category, hash)
    }
}

// ─── Validation Warning ──────────────────────────────────────────────────────

/// A non-fatal validation warning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationWarning {
    pub warning_id: String,
    pub category: ValidationCategory,
    pub message: String,
    pub risk_level: RiskLevel,
}

impl ValidationWarning {
    pub fn new(category: ValidationCategory, message: &str, risk_level: RiskLevel) -> Self {
        ValidationWarning {
            warning_id: format!("warn_{}_{}", category, message.len()),
            category,
            message: message.to_string(),
            risk_level,
        }
    }
}

// ─── Validation Evidence ─────────────────────────────────────────────────────

/// Evidence collected during validation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationEvidence {
    pub checks_performed: usize,
    pub checks_passed: usize,
    pub checks_failed: usize,
    pub issues_found: usize,
    pub warnings_found: usize,
    pub policy_evaluations: usize,
    pub risk_assessments: usize,
    pub confidence_calculations: usize,
}

impl ValidationEvidence {
    pub fn record_check(&mut self, passed: bool) {
        self.checks_performed += 1;
        if passed {
            self.checks_passed += 1;
        } else {
            self.checks_failed += 1;
        }
    }

    pub fn record_issue(&mut self) {
        self.issues_found += 1;
    }

    pub fn record_warning(&mut self) {
        self.warnings_found += 1;
    }

    pub fn record_policy_evaluation(&mut self) {
        self.policy_evaluations += 1;
    }

    pub fn record_risk_assessment(&mut self) {
        self.risk_assessments += 1;
    }

    pub fn record_confidence_calculation(&mut self) {
        self.confidence_calculations += 1;
    }
}

// ─── Validation Report ───────────────────────────────────────────────────────

/// Complete validation report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    /// Deterministic report identifier.
    pub report_id: String,
    /// Overall result.
    pub result: ValidationResult,
    /// All issues found.
    pub issues: Vec<ValidationIssue>,
    /// All warnings found.
    pub warnings: Vec<ValidationWarning>,
    /// Evidence collected.
    pub evidence: ValidationEvidence,
    /// Maximum risk level encountered.
    pub max_risk_level: RiskLevel,
    /// Average confidence score.
    pub avg_confidence: f64,
    /// Validation timestamp.
    pub validated_at: String,
    /// Summary message.
    pub summary: String,
}

impl ValidationReport {
    pub fn new(report_id: String, result: ValidationResult) -> Self {
        ValidationReport {
            report_id,
            result: result.clone(),
            issues: Vec::new(),
            warnings: Vec::new(),
            evidence: ValidationEvidence::default(),
            max_risk_level: RiskLevel::Info,
            avg_confidence: 1.0,
            validated_at: chrono::Utc::now().to_rfc3339(),
            summary: format!("Validation complete: {}", result),
        }
    }

    /// Add an issue to the report.
    pub fn add_issue(&mut self, issue: ValidationIssue) {
        let severity = issue.severity.clone();
        self.issues.push(issue);
        self.evidence.record_issue();
        if severity.score() > self.max_risk_level.score() {
            self.max_risk_level = severity;
        }
    }

    /// Add a warning to the report.
    pub fn add_warning(&mut self, warning: ValidationWarning) {
        let risk = warning.risk_level.clone();
        self.warnings.push(warning);
        self.evidence.record_warning();
        if risk.score() > self.max_risk_level.score() {
            self.max_risk_level = risk;
        }
    }

    /// Update the summary.
    pub fn update_summary(&mut self) {
        self.summary = format!(
            "Validation {}: {} issues, {} warnings, max risk={}",
            self.result,
            self.issues.len(),
            self.warnings.len(),
            self.max_risk_level
        );
    }

    /// Check if validation blocks approval.
    pub fn blocks_approval(&self) -> bool {
        matches!(
            self.result,
            ValidationResult::Reject | ValidationResult::RequiresClarification
        ) || self.issues.iter().any(|i| i.blocks_approval)
    }

    /// Check if clarification is needed.
    pub fn requires_clarification(&self) -> bool {
        matches!(self.result, ValidationResult::RequiresClarification)
    }
}

// ─── Validation Summary ──────────────────────────────────────────────────────

/// Human-readable validation summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationSummary {
    pub result: String,
    pub issue_count: usize,
    pub warning_count: usize,
    pub max_risk: String,
    pub avg_confidence: f64,
    pub checks_passed: usize,
    pub checks_failed: usize,
    pub approval_ready: bool,
}

impl ValidationSummary {
    pub fn from_report(report: &ValidationReport) -> Self {
        ValidationSummary {
            result: report.result.to_string(),
            issue_count: report.issues.len(),
            warning_count: report.warnings.len(),
            max_risk: report.max_risk_level.to_string(),
            avg_confidence: report.avg_confidence,
            checks_passed: report.evidence.checks_passed,
            checks_failed: report.evidence.checks_failed,
            approval_ready: !report.blocks_approval(),
        }
    }
}

// ─── Policy ──────────────────────────────────────────────────────────────────

/// Externalized policy for validation rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub policy_id: String,
    pub name: String,
    pub description: String,
    pub rules: Vec<PolicyRule>,
    pub enabled: bool,
}

impl Policy {
    pub fn new(policy_id: &str, name: &str, description: &str, rules: Vec<PolicyRule>) -> Self {
        Policy {
            policy_id: policy_id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            rules,
            enabled: true,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// A single rule within a policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub rule_id: String,
    pub description: String,
    pub category: ValidationCategory,
    pub severity: RiskLevel,
    pub block_on_failure: bool,
    pub evaluation: RuleEvaluation,
}

/// How a rule is evaluated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuleEvaluation {
    /// Simple boolean check.
    Boolean(bool),
    /// Confidence threshold check.
    ConfidenceThreshold { min: f64 },
    /// Risk level check.
    RiskThreshold { max: RiskLevel },
    /// Custom evaluation function.
    Custom(String),
}

impl PolicyRule {
    pub fn new(
        rule_id: &str,
        description: &str,
        category: ValidationCategory,
        severity: RiskLevel,
        block_on_failure: bool,
        evaluation: RuleEvaluation,
    ) -> Self {
        PolicyRule {
            rule_id: rule_id.to_string(),
            description: description.to_string(),
            category,
            severity,
            block_on_failure,
            evaluation,
        }
    }
}

// ─── Validation Config ───────────────────────────────────────────────────────

/// Configuration for the adaptive validation engine.
#[derive(Debug, Clone)]
pub struct ValidationConfig {
    /// Minimum confidence threshold.
    pub min_confidence: f64,
    /// Maximum allowed risk level.
    pub max_risk_level: RiskLevel,
    /// Whether to block on warnings.
    pub block_on_warnings: bool,
    /// Whether to block on ambiguities.
    pub block_on_ambiguity: bool,
    /// List of enabled policies.
    pub policies: Vec<Policy>,
    /// Maximum number of issues before reject.
    pub max_issues_before_reject: usize,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        ValidationConfig {
            min_confidence: 0.5,
            max_risk_level: RiskLevel::High,
            block_on_warnings: false,
            block_on_ambiguity: false,
            policies: Vec::new(),
            max_issues_before_reject: 10,
        }
    }
}

impl ValidationConfig {
    pub fn new() -> Self {
        ValidationConfig::default()
    }

    pub fn with_min_confidence(mut self, min: f64) -> Self {
        self.min_confidence = min;
        self
    }

    pub fn with_max_risk_level(mut self, level: RiskLevel) -> Self {
        self.max_risk_level = level;
        self
    }

    pub fn with_policy(mut self, policy: Policy) -> Self {
        self.policies.push(policy);
        self
    }
}
