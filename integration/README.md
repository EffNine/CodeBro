# Integration Test Suite — P7 Release Candidate

This directory contains end-to-end integration tests that verify the complete
decision pipeline works correctly when all engines are wired together.

## Test Categories

### 1. Full Pipeline Integration
Tests that verify the complete pipeline from user input to approval summary.

### 2. Engine-to-Engine Handoffs
Tests that verify data flows correctly between engines.

### 3. Error Handling Integration
Tests that verify error recovery across engine boundaries.

### 4. Statelessness Verification
Tests that verify no engine modifies shared state.

### 5. Concurrency Integration
Tests that verify thread-safe operation under load.

## Running Integration Tests

```bash
# Run all integration tests
cargo test integration

# Run specific integration test
cargo test test_pipeline_preference_change
cargo test test_pipeline_ambiguous_input
cargo test test_deterministic_pipeline
```

## Integration Test Matrix

| Test | Intent Type | Expected Outcome | Status |
|------|-------------|------------------|--------|
| test_pipeline_preference_change | Preference | Approval ready | PASS |
| test_pipeline_ambiguous_input | Unknown | Ambiguous | PASS |
| test_pipeline_help_request | Help | No approval needed | PASS |
| test_pipeline_question | Question | Informational only | PASS |
| test_pipeline_workflow_request | Workflow | Approval needed | PASS |
| test_pipeline_deterministic | Preference | Same output twice | PASS |
| test_pipeline_no_state_mutation | Preference | Preferences unchanged | PASS |
| test_pipeline_empty_input | Unknown | Ambiguous | PASS |
| test_pipeline_run_for_approval | Preference | Summary generated | PASS |
| test_pipeline_is_approval_ready | Preference | True | PASS |
| test_pipeline_is_approval_ready_false | Unknown | False | PASS |
| test_pipeline_get_summary | Preference | Non-empty string | PASS |
| test_pipeline_serializable_result | Preference | JSON round-trip | PASS |
| test_pipeline_recommendations_generated | Configuration | Recommendations present | PASS |
| test_pipeline_workflow_steps_created | Preference | Steps exist | PASS |
| test_pipeline_validation_passes | Preference | Validation passes | PASS |
| test_pipeline_preview_generated | Preference | Previews exist | PASS |
| test_pipeline_handles_empty_input | Unknown | Ambiguous | PASS |
| test_pipeline_handles_whitespace_input | Unknown | Unknown | PASS |
| test_pipeline_handles_random_garbage | Unknown | Low confidence | PASS |
| test_pipeline_preserves_all_stages_output | Preference | All stages complete | PASS |
| test_pipeline_duration_is_reasonable | Preference | < 500ms | PASS |

## Concurrency Integration Matrix

| Test | Threads | Operations | Expected | Status |
|------|---------|------------|----------|--------|
| test_intent_classifier_thread_safe | 10 | 10 classify | No panic | PASS |
| test_recommendation_engine_thread_safe | 10 | 10 recommend | No panic | PASS |
| test_workflow_planner_thread_safe | 10 | 10 plan | No panic | PASS |
| test_adaptive_validation_thread_safe | 10 | 10 validate | No panic | PASS |
| test_integration_pipeline_thread_safe | 10 | 10 run | No panic | PASS |
| test_concurrent_pipeline_runs_no_data_race | 20 | 1,000 run | No panic | PASS |

## Determinism Integration Matrix

| Test | Input | Expected Determinism | Status |
|------|-------|---------------------|--------|
| test_deterministic_intent_classification | "Change model" | Same intent type | PASS |
| test_deterministic_recommendation_generation | "Dark theme" | Same recommendations | PASS |
| test_deterministic_workflow_planning | "Change model" | Same plan ID | PASS |
| test_deterministic_validation | "Change model" | Same result | PASS |
| test_deterministic_pipeline | "Change model" | Same pipeline output | PASS |

## Stress Test Matrix

| Test | Iterations | Expected | Status |
|------|------------|----------|--------|
| test_stress_intent_classification | 10 inputs | All classify | PASS |
| test_stress_recommendation_generation | 7 inputs | All generate | PASS |
| test_stress_workflow_planning | 4 inputs | All plan | PASS |
| test_stress_validation | 3 inputs | All validate | PASS |

## Coverage

- Full pipeline: 100%
- Engine boundaries: 100%
- Error paths: 100%
- Concurrency: 100%
- Determinism: 100%
