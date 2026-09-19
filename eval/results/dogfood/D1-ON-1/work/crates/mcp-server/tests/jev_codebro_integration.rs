//! Jev–CodeBro live-seam integration (jev-baseline-1).
//!
//! Pins the single live advisory seam: skill `request_approval` calls
//! `observe_escalation_advisory` post-decision, detached, discarded.
//! With the default flags OFF this is a zero-cost no-op: the deterministic
//! `needs_input` response is unchanged, zero Jev calls happen, and no
//! advisory file appears.
//!
//! Heavier behavior (thresholds, mismatch, failures, wording, telemetry) is
//! covered in `codebro-jev-shadow/tests/codebro_integration.rs` through the
//! same `evaluate_advisory` gate this seam flows through. This file pins the
//! CodeBro side: the seam exists, stays detached/display-only, and stays
//! silent when OFF.

use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn hook_source_has_no_authority_surface() {
    let src = include_str!("../src/jev_shadow_hook.rs");
    // No execution/policy/task handles may ever appear in the hook.
    // (Prose mentions of handler names in safety comments are allowed;
    // what is forbidden is an actual handle, call, or definition.)
    // Note: `&crate::sandbox::VerificationResult` as a borrowed read-only
    // input is allowed by design (already-computed deterministic outcome);
    // what is forbidden is any runtime/handle that can act.
    for banned in [
        "ChangeEngine",
        "SandboxRuntime",
        "SandboxCommand",
        "SandboxPolicy",
        "call_tool",
        "tool_router",
        "std::process",
        "tokio::process",
        "Command::new",
        "crate::change",
        "crate::context_runtime",
        "fn approve",
        "fn deny",
        "fn execute",
        "fn retry",
    ] {
        assert!(!src.contains(banned), "jev_shadow_hook.rs must not contain {banned}");
    }
    // The only crate::sandbox reference must be the read-only borrowed input.
    let sandbox_refs = src.matches("crate::sandbox").count();
    assert_eq!(sandbox_refs, 1, "only the read-only VerificationResult borrow is allowed");
    assert!(src.contains("&crate::sandbox::VerificationResult"));
    // The live seam must be documented: skill request_approval is the wired site.
    assert!(
        src.contains("request_approval"),
        "hook must document its live CodeBro seam"
    );
    // The hook must stay detached/discarded by contract.
    assert!(src.contains("tokio::spawn"), "hook must stay detached");
    assert!(src.contains("outcome is discarded"), "hook must discard its outcome");
}

#[tokio::test]
async fn escalation_hook_with_flags_off_is_zero_cost_noop() {
    let _guard = env_lock();
    // Default environment: both flags OFF, no key.
    std::env::remove_var("JEV_SHADOW_ENABLED");
    std::env::remove_var("JEV_ADVISORY_ENABLED");
    std::env::remove_var("TYPESAFE_API_KEY");
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("JEV_SHADOW_LOG_PATH", dir.path().join("shadow.jsonl"));
    std::env::set_var("JEV_ADVISORY_LOG_PATH", dir.path().join("advisory.jsonl"));
    // Must return synchronously without spawning work or touching disk.
    codebro_mcp_server::jev_shadow_hook::observe_escalation_advisory(
        dir.path(),
        "skill",
        "skill approve candidate=test-candidate request=test-request",
        true,
        None,
        "codebro:skill:request_approval",
    );
    // Give any (unexpected) detached task a moment; OFF must spawn nothing.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        !dir.path().join("shadow.jsonl").exists(),
        "flags OFF must write no shadow record"
    );
    assert!(
        !dir.path().join("advisory.jsonl").exists(),
        "flags OFF must write no advisory event"
    );
    std::env::remove_var("JEV_SHADOW_LOG_PATH");
    std::env::remove_var("JEV_ADVISORY_LOG_PATH");
}

#[tokio::test]
async fn advisory_without_shadow_flag_stays_silent() {
    let _guard = env_lock();
    // Advisory ON without shadow MUST NOT activate (zero calls, zero output).
    std::env::set_var("JEV_SHADOW_ENABLED", "false");
    std::env::set_var("JEV_ADVISORY_ENABLED", "true");
    std::env::set_var("TYPESAFE_API_KEY", "test-key-never-a-real-secret");
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("JEV_SHADOW_LOG_PATH", dir.path().join("shadow.jsonl"));
    std::env::set_var("JEV_ADVISORY_LOG_PATH", dir.path().join("advisory.jsonl"));
    codebro_mcp_server::jev_shadow_hook::observe_escalation_advisory(
        dir.path(),
        "skill",
        "skill approve candidate=test-candidate request=test-request",
        true,
        Some("task-123".to_string()),
        "codebro:skill:request_approval",
    );
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        !dir.path().join("shadow.jsonl").exists(),
        "advisory-without-shadow must make zero calls"
    );
    assert!(
        !dir.path().join("advisory.jsonl").exists(),
        "advisory-without-shadow must produce zero output"
    );
    std::env::remove_var("JEV_SHADOW_ENABLED");
    std::env::remove_var("JEV_ADVISORY_ENABLED");
    std::env::remove_var("TYPESAFE_API_KEY");
    std::env::remove_var("JEV_SHADOW_LOG_PATH");
    std::env::remove_var("JEV_ADVISORY_LOG_PATH");
}
