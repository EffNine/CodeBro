#!/usr/bin/env bash
# Phase-9 validation gate: single command that checks every required lock.
#
#   bash scripts/jev_validate_phase9.sh
#
# If ANY required gate fails: ADVISORY MUST REMAIN OFF (exit nonzero).
# Advisory-only phase: this script never enables flags, never mutates policy,
# never prints TYPESAFE_API_KEY (secret checks report counts only).
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PASS=0
FAIL=0
check() { # check <name> <command...>
  local name="$1"; shift
  if "$@" >/tmp/jev_phase9_gate_out.log 2>&1; then
    echo "GATE PASS: $name"
    PASS=$((PASS+1))
  else
    echo "GATE FAIL: $name (see /tmp/jev_phase9_gate_out.log)"
    tail -n 5 /tmp/jev_phase9_gate_out.log
    FAIL=$((FAIL+1))
  fi
}

echo "=== Phase-9 Jev validation gate ==="
echo "root: $ROOT"
echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"

# 1. Model version pinned to jev-1.13.0 (no jev-latest).
check "model-pin-constants" \
  bash -c 'grep -q "jev-1.13.0" crates/jev-shadow/src/config.rs && grep -q "LOCKED_MODEL: &str = \"jev-1.13.0\"" crates/jev-shadow/src/change_control.rs && ! grep -Eq "DEFAULT_MODEL.*latest|LOCKED_MODEL.*latest" crates/jev-shadow/src/*.rs'

# 2. Question-set hash lock (v3 frozen).
check "question-lock-test" \
  cargo test -q -p codebro-jev-shadow --test change_control question_lock_hash_matches_expected

# 3. State-schema version + shape validation.
check "state-schema-tests" \
  cargo test -q -p codebro-jev-shadow --test change_control state_

# 4. Confidence threshold >= 0.80 (constant + boundary behavior).
check "confidence-threshold" \
  cargo test -q -p codebro-jev-shadow --test change_control confidence_threshold_is_at_least_0_80_and_enforced

# 5. Flags default OFF (both shadow and advisory).
check "flags-default-off" \
  cargo test -q -p codebro-jev-shadow --test change_control flags_default_off_no_silent_enable

# 6. No authority surfaces (change_control + advisory + hook scan).
check "no-authority-surfaces" \
  cargo test -q -p codebro-jev-shadow --test change_control change_control_and_advisory_have_no_authority_surface

# 7. Regression fixtures (all 7 boundaries) + non-escalation silence.
check "regression-fixtures" \
  cargo test -q -p codebro-jev-shadow --test change_control regression_fixtures_match_frozen_oracles
check "non-escalation-silence" \
  cargo test -q -p codebro-jev-shadow --test change_control non_escalation_fixtures_stay_shadow_only

# 8. Secret redaction (builders + fixtures; counts only, never prints key).
check "secret-redaction" \
  cargo test -q -p codebro-jev-shadow --test change_control fixture_states_and_builders_redact_secrets
check "no-secret-in-phase9-artifacts" \
  bash -c 'test -z "${TYPESAFE_API_KEY:-}" || ! grep -rF -- "${TYPESAFE_API_KEY}" /tmp/opencode/jev-phase9/ crates/jev-shadow/ 2>/dev/null'

# 9. Jev failure isolation (timeout/network/http/malformed -> no event).
check "failure-isolation" \
  cargo test -q -p codebro-jev-shadow --test change_control jev_failure_isolation_no_event_without_success

# 10. Lock enforcement (model/question drift -> OFF) + state-gate + rollback.
check "lock-enforcement" \
  cargo test -q -p codebro-jev-shadow --test change_control advisory_enforces_model_question_locks
check "state-gate-live-path" \
  cargo test -q -p codebro-jev-shadow --test change_control incompatible_state_shape_disables_advisory_but_keeps_shadow_record
check "rollback" \
  cargo test -q -p codebro-jev-shadow --test change_control rollback_to_shadow_only_removes_advisory_output

# 11. Observability envelope on every advisory event.
check "observability-envelope" \
  cargo test -q -p codebro-jev-shadow --test change_control advisory_events_carry_full_observability_envelope

# 16. Baseline lock (Phase-10 audit: record parses, every tripwire trips,
#     no moving alias, validated operation advises under the gate).
check "baseline-lock" \
  cargo test -q -p codebro-jev-shadow --test baseline_lock

# 17. Full jev-shadow suite (shadow/advisory/change-control/baseline: no regressions).
check "jev-shadow-full" \
  cargo test -q -p codebro-jev-shadow

# 13. Dependency direction (workspace architecture rule).
check "workspace-deps" \
  bash scripts/check_workspace_deps.sh

echo "=== gate summary: PASS=$PASS FAIL=$FAIL ==="
if [ "$FAIL" -ne 0 ]; then
  echo "ADVISORY MUST REMAIN OFF."
  exit 1
fi
echo "All Phase-9 gates passed. (Advisory still requires explicit flags + revalidation policy; this script never enables it.)"
