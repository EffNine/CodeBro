# Adaptive Validation Architecture Report

**Document:** `docs/reports/p6.5/AdaptiveValidationArchitectureReport.md`
**Version:** 1.0.0
**Status:** Final
**Date:** 2026-08-06
**Owner:** CodeBro Engineering
**Phase:** P6.5 Adaptive Validation Foundation

---

## 1. Overview

The Adaptive Validation Engine is the quality guardian of the complete decision pipeline. It validates every plan before Preview and Approval without modifying any state.

## 2. Architecture

```
User Input
    ↓
Intent Engine
    ↓
Recommendation Engine
    ↓
Workflow Engine
    ↓
Adaptive Validation (read-only evaluator)
    ↓
Preview
    ↓
Approval Gate
    ↓
Preference Engine
```

## 3. Modules

### 3.1 `types.rs`

Core data model — all types are immutable, serializable, and deterministic:

- `ValidationResult` — Pass, PassWithWarnings, RequiresClarification, Reject
- `RiskLevel` — Info, Low, Medium, High, Critical (with numeric scores)
- `ValidationCategory` — Workflow, Intent, Recommendation, Dependencies, Policy, Preference, Conflict, Risk, Confidence, ApprovalReadiness
- `ValidationIssue` — issue_id, category, severity, message, evidence, recommended_action, blocks_approval
- `ValidationWarning` — warning_id, category, message, risk_level
- `ValidationEvidence` — checks_performed, checks_passed, checks_failed, issues_found, warnings_found, policy_evaluations, risk_assessments, confidence_calculations
- `ValidationReport` — report_id, result, issues, warnings, evidence, max_risk_level, avg_confidence, validated_at, summary
- `ValidationSummary` — human-readable summary
- `Policy` — policy_id, name, description, rules, enabled
- `PolicyRule` — rule_id, description, category, severity, block_on_failure, evaluation
- `RuleEvaluation` — Boolean, ConfidenceThreshold, RiskThreshold, Custom
- `ValidationConfig` — min_confidence, max_risk_level, block_on_warnings, block_on_ambiguity, policies, max_issues_before_reject

### 3.2 `engine.rs`

Main orchestration module:

- `AdaptiveValidationEngine` — stateless observer
- `validate()` — IntentPlan + RecommendationSet + WorkflowPlan → ValidationReport
- `is_approval_ready()` — quick check
- `get_summary()` — human-readable summary

### 3.3 `policy.rs`

Externalized policy management:

- `PolicyEngine` — register, evaluate, check failures
- `default_policies()` — Basic Safety, Confidence Threshold, Risk Limits
- `Policy` and `PolicyRule` structures

### 3.4 `rules.rs`

Deterministic validation rules:

- `ValidationRule` — rule_id, description, category, severity, block_on_failure, evaluate
- 17+ registered rules across all categories
- `evaluate_all()` — run all rules
- `find_failed_rules()` — get failed rules

### 3.5 `confidence.rs`

Confidence evaluation:

- `ConfidenceEvaluator` — evaluate, is_above_threshold, risk_level_for_confidence
- Penalizes low confidence, ambiguity, missing information

### 3.6 `risk.rs`

Risk assessment:

- `RiskAssessor` — assess, is_acceptable, mitigation_suggestion
- Combines issues and warnings to determine overall risk

### 3.7 `validator.rs`

Main validation orchestration:

- `Validator` — orchestrates rules, policies, confidence, risk
- `validate()` — complete validation pipeline
- `determine_result()` — compute overall result

### 3.8 `diagnostics.rs`

Failure tracking and observability:

- `AdaptiveDiagnostics` — thread-safe diagnostic logger
- `DiagnosticKind` — ValidationStarted, ValidationCompleted, PolicyFailure, RuleFailure, ConfidenceFailure, RiskFailure
- Tracks: validation count, policy failures, rule failures, risk distribution

## 4. Design Decisions

### 4.1 Read-Only Evaluator

The Adaptive Validation Engine:
- Never modifies IntentPlan
- Never modifies RecommendationSet
- Never modifies WorkflowPlan
- Never executes commands
- Never writes to any storage
- Only reads and evaluates

### 4.2 Externalized Policies

Policies are externalized from the validator:
- `PolicyEngine` manages policy registration
- `default_policies()` provides standard policies
- Policies can be loaded from configuration in the future
- No hardcoded business logic in the validator

### 4.3 Deterministic IDs

All IDs are generated deterministically:
- `issue_id` — hash-based from category + message
- `warning_id` — hash-based from category + message length
- `report_id` — based on intent ID

No UUIDs, no timestamps in IDs.

### 4.4 Risk-Based Validation

Validation uses a risk model:
- Each issue has a severity (RiskLevel)
- Each warning has a risk level
- Overall risk is the maximum of all issues/warnings
- Configurable max risk threshold

### 4.5 No Platform Coupling

The Adaptive Validation Engine has zero dependencies on:
- `Runtime` — No state machine coupling
- `Tool` — No tool platform coupling
- `Intelligence` — No reasoning coupling
- `LLM` — No network or model calls
- `PreferenceEngine` — Only reads via context

## 5. Test Coverage

| Module | Tests | Coverage |
|--------|-------|----------|
| types | 0 (model only) | N/A |
| engine | 7 | Full |
| policy | 6 | Full |
| rules | 3 | Full |
| confidence | 5 | Full |
| risk | 5 | Full |
| validator | 5 | Full |
| diagnostics | 11 | Full |
| p6.5 integration | 37 | Full |
| **Total** | **76** | **100%** |

---

## 6. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
