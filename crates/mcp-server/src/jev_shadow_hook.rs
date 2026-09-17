//! Phase-4 Jev shadow sidecar hook — the single post-decision observation seam.
//!
//! Called at the END of the `sandbox_test` / `sandbox_build` MCP handlers,
//! AFTER the deterministic verification payload is fully built. The hook:
//!
//! - takes `&VerificationResult` (already computed) + workspace root,
//! - returns `()` synchronously; nothing flows back into the response,
//! - early-returns when the sidecar is not live (`JEV_SHADOW_ENABLED` off
//!   or no key: zero network calls, zero impact),
//! - otherwise spawns a DETACHED task whose outcome is discarded.
//!
//! This hook cannot approve, deny, execute, retry, delete, escalate, alter
//! task state, retry behavior, or sandbox boundaries. It observes.
//!
//! Phase-8 addition: [`observe_escalation_advisory`] provides the LIMITED
//! ADVISORY projection for escalation states (frozen v3, confidence-gated,
//! informational-only). It is wired to exactly ONE live site — the skill
//! `request_approval` handler — post-decision, detached, and display-only:
//! the deterministic `needs_input` verdict is computed first, the response is
//! built from it alone, and the observer's outcome is discarded. Advisory
//! validation also runs through this function explicitly (offline collectors
//! / tests). Any additional future live escalation site must follow the same
//! pattern: post-decision, detached, display-only — never as authority input.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

/// Post-decision observation for a completed test/build verification.
///
/// Fire-and-forget by contract: the spawned future's result is discarded.
pub fn observe_test_classification(
    workspace_root: &std::path::Path,
    verification: &crate::sandbox::VerificationResult,
) {
    let observer = codebro_jev_shadow::ShadowObserver::from_env(workspace_root);
    if !observer.is_live() {
        return;
    }
    // Minimal sanitized state, built from the deterministic outcome only:
    // coarse classification + exit code + bounded redacted diagnostics.
    // No raw output, no env, no secrets.
    let diags: Vec<codebro_jev_shadow::questions::DiagInput> = verification
        .diagnostics
        .iter()
        .map(|d| codebro_jev_shadow::questions::DiagInput {
            severity: d.severity.clone(),
            message: d.message.clone(),
            test: d.test.clone(),
            file_hash: d.file.as_deref().map(codebro_jev_shadow::questions::hash_id),
        })
        .collect();
    let state = codebro_jev_shadow::questions::state_test_classification(
        verification.classification.as_deref().unwrap_or("unknown"),
        verification.execution.exit_code,
        Some(&verification.execution.command),
        &diags,
    );
    let det_label =
        codebro_jev_shadow::questions::deterministic_test_label(verification.classification.as_deref())
            .to_string();
    tokio::spawn(async move {
        let _ = observer
            .observe(
                "test_classification",
                state,
                codebro_jev_shadow::shadow::DeterministicVerdict::Label(det_label),
            )
            .await;
    });
}

/// Phase-8 LIMITED ADVISORY observation for one escalation state.
///
/// Post-decision, detached, display-only:
/// - returns `()` synchronously; nothing flows back into any response;
/// - early-returns unless BOTH `JEV_SHADOW_ENABLED` and
///   `JEV_ADVISORY_ENABLED` are on plus a key (advisory ON without shadow
///   MUST NOT activate);
/// - otherwise spawns a DETACHED task whose outcome is discarded. The task
///   appends the shadow record + (when the advisory rule holds) the
///   structured advisory event, and emits the informational advisory string
///   via tracing only for `ADVISORY_ESCALATION`.
///
/// This function cannot approve, deny, execute, retry, delete, escalate,
/// alter task state, retry behavior, sandbox boundaries, or policy. It only
/// adds information. The human (or the pre-existing deterministic policy)
/// remains responsible for deciding what happens next.
///
/// NOT wired into any MCP handler by default except skill `request_approval`
/// (live escalation seam: deterministic `needs_input` first, observer
/// detached/discarded after). The `sandbox_test`/`sandbox_build` handlers stay
/// shadow-only test-classification. Call explicitly from offline
/// collectors/tests or a future post-decision escalation site.
pub fn observe_escalation_advisory(
    workspace_root: &std::path::Path,
    action_kind: &str,
    summary: &str,
    needs_approval: bool,
    session_task: Option<String>,
    provenance: &str,
) {
    let observer = codebro_jev_shadow::ShadowObserver::from_env(workspace_root);
    if !observer.is_live() {
        return;
    }
    // Advisory needs both flags; shadow-only stays silent here (the shadow
    // record is still appended by the versioned call below, but no advisory
    // event/message is produced unless advisory is live — enforced inside
    // `observe_escalation_with_advisory` via `evaluate_advisory`).
    let state = codebro_jev_shadow::questions::state_escalation(action_kind, summary);
    let det = codebro_jev_shadow::shadow::DeterministicVerdict::Bool(needs_approval);
    let provenance = provenance.to_string();
    tokio::spawn(async move {
        let _ = observer
            .observe_escalation_with_advisory(
                codebro_jev_shadow::advisory::ADVISORY_QUESTION_SET_VERSION,
                state,
                det,
                session_task,
                &provenance,
            )
            .await;
    });
}
