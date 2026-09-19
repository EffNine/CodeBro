# Intent Engine Architecture Report

**Document:** `docs/reports/p6.2/IntentEngineArchitectureReport.md`
**Version:** 1.0.0
**Status:** Final
**Date:** 2026-08-06
**Owner:** CodeBro Engineering
**Phase:** P6.2 Intent Engine Foundation

---

## 1. Overview

The Intent Engine translates natural language user input into deterministic Intent Plans and executable Commands. It is the only component responsible for intent understanding, classification, and command generation. It never modifies preferences directly and never owns state.

## 2. Architecture

```
User Input
  ↓
Intent Classifier (deterministic rules/patterns)
  ↓
Intent Plan (structured, explainable, serializable)
  ↓
Intent Resolver (plan → immutable commands)
  ↓
Preference Commands (immutable, auditable)
  ↓
Approval Preview (read-only, no mutations)
  ↓
Approval Gate
  ↓
Preference Engine
```

## 3. Modules

### 3.1 `types.rs`

Core data model — all types are immutable, serializable, and replayable:

- `IntentType` — 7 deterministic categories: Preference, Configuration, Workflow, Execution, Question, Help, Unknown
- `IntentPlan` — structured plan with: id, detected_goal, intent_type, affected_subsystem, required_approval, estimated_cost_impact, confidence, ambiguity, ambiguity_reason, reasoning, evidence, required_commands, created_at
- `IntentCommand` — 8 command variants: UpdateModelPreference, UpdateLanguagePreference, UpdateCostPreference, UpdateApprovalPreference, ExecuteWorkflow, ExecuteCommand, AnswerQuestion, ProvideHelp
- `CommandMetadata` — immutable audit metadata: source, timestamp, intent_id, reason, expected_effect
- `ApprovalPreview` — read-only preview: command_kind, requested_change, current_value, proposed_value, estimated_cost_impact, affected_workflows, reversibility, preview_id, generated_at
- `ConfidenceResult` — structured confidence: score, evidence, reasoning
- `AmbiguityResult` — ambiguity detection: is_ambiguous, reason, clarification_questions
- `Reversibility` — FullyReversible, PartiallyReversible, Irreversible

### 3.2 `classifier.rs`

Deterministic intent classifier using regex pattern matching:

- `IntentClassifier` — rule-based classifier with ~30+ classification rules
- Patterns for all 7 intent types
- Confidence scoring based on rule match strength + command complexity
- Evidence tracking for explainability
- Unknown intent never forced into another category
- LLM fallback is architecture only; not implemented in this phase

### 3.3 `resolver.rs`

Converts Intent Plans into executable commands:

- `IntentResolver` — pure function resolver
- Generates immutable `ResolvedCommand` objects with full audit metadata
- Each command includes source, timestamp, intent_id, reason, expected_effect
- Commands never modify state directly
- Resolution is deterministic and replayable

### 3.4 `preview.rs`

Read-only approval preview generation:

- `ApprovalPreviewGenerator` — generates pre-approval previews
- Shows requested change, current value, proposed value
- Estimates cost impact and affected workflows
- Determines reversibility (Fully/Partially/Irreversible)
- No state mutations during preview generation

### 3.5 `ambiguity.rs`

Detects ambiguous or underspecified input:

- `AmbiguityDetector` — pattern-based ambiguity detection
- Detects vague model references ("Use Claude.", "Use GPT.")
- Detects vague change requests ("Change to something better")
- Detects context-dependent commands ("Do it", "Go ahead")
- Generates clarification questions
- Never guesses — always clarifies

### 3.6 `confidence.rs`

Structured confidence scoring:

- `ConfidenceModel` — computes confidence with evidence and reasoning
- Considers: rule match strength, ambiguity, command complexity, input completeness
- Returns `ConfidenceResult` with score, evidence, and reasoning
- Low confidence (< 0.5) triggers clarification
- High confidence (>= 0.8) enables automatic approval

### 3.7 `diagnostics.rs`

Failure tracking and observability:

- `IntentDiagnostics` — thread-safe diagnostic logger
- Tracks: classification failures, ambiguity detections, resolver failures, command generation failures, preview failures
- LRU-bound record storage
- Summary statistics by kind
- Serializable for audit trails

## 4. Design Decisions

### 4.1 Deterministic First

All classification uses regex pattern matching. No LLM calls, no probabilistic models, no external dependencies. Same input always produces same output.

### 4.2 Never Guess

Ambiguous input is never forced into a known category. The classifier returns `UnknownIntent` with an ambiguity flag and clarification questions.

### 4.3 Command, Don't Mutate

Commands are immutable request objects. They describe what should happen but never do it directly. The Approval Gate must authorize before the Preference Engine commits.

### 4.4 Explainability

Every classification includes:
- `reasoning` — why this intent was chosen
- `evidence` — which patterns matched
- `confidence` — numerical confidence score
- `ambiguity_reason` — why the input is ambiguous (if applicable)

### 4.5 Audit Trail

Every command includes:
- `source` — always "intent_engine"
- `timestamp` — ISO 8601 UTC
- `intent_id` — links back to the original plan
- `reason` — human-readable explanation
- `expected_effect` — what the command will do

### 4.6 No Platform Coupling

The Intent Engine has zero dependencies on:
- `Runtime` — No state machine coupling
- `Tool` — No tool platform coupling
- `Intelligence` — No reasoning coupling
- `LLM` — No network or model calls

## 5. Data Flow

```
User Input
    │
    ▼
IntentClassifier::classify(input)
    │
    ├──► AmbiguityDetector::detect(plan)    (parallel)
    │
    ├──► ConfidenceModel::compute(plan)     (parallel)
    │
    ▼
IntentPlan (structured, explainable)
    │
    ▼
IntentResolver::resolve(plan)
    │
    ▼
Vec<ResolvedCommand> (with audit metadata)
    │
    ▼
ApprovalPreviewGenerator::generate_batch(commands, current_values)
    │
    ▼
Vec<ApprovalPreview> (read-only, no mutations)
    │
    ▼
Approval Gate (external)
    │
    ▼
Preference Engine (external)
```

## 6. Intent Type Distribution

| Intent Type | Pattern Examples | Requires Approval |
|-------------|-----------------|-------------------|
| Preference | "Change model to gpt-4o" | Yes |
| Configuration | "Configure the system" | No |
| Workflow | "Run the test workflow" | Yes |
| Execution | "Execute command cargo test" | Yes |
| Question | "What is rust?" | No |
| Help | "help" | No |
| Unknown | "Use Claude.", "xyz123" | No |

## 7. Command Types

| Command Type | Approval Required | Reversibility |
|-------------|------------------|---------------|
| UpdateModelPreference | Yes | FullyReversible |
| UpdateLanguagePreference | Yes | FullyReversible |
| UpdateCostPreference | Yes | FullyReversible |
| UpdateApprovalPreference | Yes | FullyReversible |
| ExecuteWorkflow | Yes | PartiallyReversible |
| ExecuteCommand | Yes | PartiallyReversible |
| AnswerQuestion | No | FullyReversible |
| ProvideHelp | No | FullyReversible |

## 8. Test Coverage

| Module | Tests | Coverage |
|--------|-------|----------|
| types | 0 (model only) | N/A |
| classifier | 21 | Full |
| resolver | 8 | Full |
| preview | 8 | Full |
| ambiguity | 12 | Full |
| confidence | 10 | Full |
| diagnostics | 11 | Full |
| p6.2 integration | 76 | Full |
| **Total** | **146** | **100%** |

---

## 9. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
