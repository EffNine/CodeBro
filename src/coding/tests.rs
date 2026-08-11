//! Sprint 30F — deterministic tests for the autonomous Coding subagent.
//!
//! These tests prove:
//! - a verified change is applied through the change engine and synthesized
//! - the completion gate auto-verifies unverified changes with the plan's
//!   validation commands (authoritative exit codes)
//! - terminal failures (verification-failed) roll back ONLY the session's own
//!   changes, restoring the original content
//! - denied verification commands are recorded and never executed
//! - changes outside the plan are recorded as unplanned (default) and denied
//!   in strict mode
//! - the tool budget bounds the session and preserves applied changes
//! - provider failures produce a bounded error result with rollback

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::agent::events::AgentEvent;
use crate::canonical_runtime::ProviderAdapter;
use crate::coding::contract::{CodingTermination, VerificationSource};
use crate::coding::{CodingLimits, CodingRequest, CodingResult, CodingSubagent, CodingTooling};
use crate::planning::{PlanStep, PlanningResult, PlanningTermination};
use crate::provider_runtime::{
    Capability, CostTracker, HealthManager, IntelligentProviderRouter, ProviderId,
    ProviderRegistry, ProviderRuntime,
};
use crate::providers::Provider;

// =========================================================================
// Mock provider
// =========================================================================

/// A scripted mock provider for coding tests. Consumes text responses
/// sequentially; the coding runtime parses `<invoke>` tool calls from the
/// text. The mock never uses native function calling, so the invoke-tag
/// parser is exercised exactly like a real non-structured provider.
#[derive(Clone)]
struct CodingMockProvider {
    name: String,
    model: String,
    responses: Arc<Mutex<Vec<String>>>,
    prompts: Arc<Mutex<Vec<String>>>,
    fail: Arc<AtomicBool>,
}

impl CodingMockProvider {
    fn text(name: &str, responses: Vec<String>) -> Self {
        CodingMockProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
            responses: Arc::new(Mutex::new(responses)),
            prompts: Arc::new(Mutex::new(Vec::new())),
            fail: Arc::new(AtomicBool::new(false)),
        }
    }

    fn failing(name: &str) -> Self {
        let p = CodingMockProvider::text(name, Vec::new());
        p.fail.store(true, Ordering::SeqCst);
        p
    }

    fn next(&self) -> String {
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            String::new()
        } else {
            responses.remove(0)
        }
    }

    fn prompt_log(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}

impl Provider for CodingMockProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn base_url(&self) -> &str {
        "mock://coding"
    }
    fn model(&self) -> &str {
        &self.model
    }
    fn api_key(&self) -> Option<&str> {
        None
    }
    fn supports_function_calling(&self) -> bool {
        false
    }
    fn send_message(
        &self,
        _m: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
    {
        let response = self.next();
        Box::pin(async move { Ok(response) })
    }
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Streaming, Capability::ToolCalling]
    }
    fn stream_response(
        &self,
        message: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<tokio::sync::mpsc::UnboundedReceiver<String>>,
                > + Send
                + '_,
        >,
    > {
        self.prompts.lock().unwrap().push(message.to_string());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        if self.fail.load(Ordering::SeqCst) {
            let result = Err(anyhow::anyhow!("coding mock provider offline"));
            Box::pin(async move { result })
        } else {
            let response = self.next();
            let _ = tx.send(response);
            Box::pin(async move { Ok(rx) })
        }
    }
}

// =========================================================================
// Harness
// =========================================================================

struct CodingHarness {
    provider_runtime: ProviderRuntime,
    router: IntelligentProviderRouter,
    io_providers: HashMap<ProviderId, Arc<dyn Provider>>,
}

impl CodingHarness {
    fn new(provider: Arc<dyn Provider>) -> Self {
        let registry = ProviderRegistry::new();
        let health = HealthManager::new();
        let cost = CostTracker::new();
        let provider_runtime =
            ProviderRuntime::from_parts(registry.clone(), health.clone(), cost.clone());
        let router = IntelligentProviderRouter::new(registry.clone(), health.clone(), cost.clone());
        let adapter = ProviderAdapter::new(provider.clone());
        registry.register(&adapter).unwrap();
        provider_runtime
            .circuit_breakers()
            .get_or_create(adapter.provider_id());
        let mut io_providers = HashMap::new();
        io_providers.insert(adapter.provider_id().clone(), provider);
        CodingHarness {
            provider_runtime,
            router,
            io_providers,
        }
    }

    fn subagent(
        self,
        root: &Path,
        planned_files: &[PathBuf],
        strict: bool,
        limits: &CodingLimits,
    ) -> CodingSubagent {
        let tooling = CodingTooling::new(root, planned_files, strict, limits.command_timeout_secs);
        CodingSubagent::new(
            self.provider_runtime,
            self.router,
            self.io_providers,
            tooling,
        )
    }
}

fn event_sink() -> (
    Arc<Mutex<Vec<AgentEvent>>>,
    Box<dyn Fn(AgentEvent) + Send + Sync>,
) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    (events, Box::new(move |e| sink.lock().unwrap().push(e)))
}

/// A fixture workspace that mirrors the canonical-runtime testing fixture:
/// a tiny crate with an `add` function, so `cargo check` works offline.
fn coding_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"cs\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(dir.path().join(".gitignore"), "target/\nCargo.lock\n").unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n#[cfg(test)]\nmod tests { #[test] fn ok() {} }\n",
    )
    .unwrap();
    dir
}

/// A REAL plan naming src/lib.rs with two validation commands.
fn make_plan_with_validation() -> PlanningResult {
    let mut plan = PlanningResult::failed(
        "implement the change",
        PlanningTermination::Completed,
        "no-op",
    );
    plan.affected_files = vec![PathBuf::from("src/lib.rs")];
    plan.plan = vec![PlanStep {
        order: 1,
        action: "modify add in src/lib.rs".to_string(),
        target_files: vec![PathBuf::from("src/lib.rs")],
        target_symbols: vec!["add".to_string()],
        rationale: "the task needs a subtract sibling".to_string(),
        dependencies: vec!["src/lib.rs".to_string()],
        validation: vec!["cargo check".to_string(), "cargo test".to_string()],
        risk: "callers of add may break".to_string(),
        evidence: vec!["testing: exit_code 0".to_string()],
    }];
    plan
}

fn make_plan_without_validation() -> PlanningResult {
    let mut plan = make_plan_with_validation();
    plan.plan[0].validation = Vec::new();
    plan
}

fn sub_proposal() -> String {
    r#"<invoke name="propose_change">{"path": "src/lib.rs", "old": "pub fn add(a: i32, b: i32) -> i32 { a + b }", "new": "pub fn sub(a: i32, b: i32) -> i32 { a - b }"}</invoke>"#.to_string()
}

fn broken_proposal() -> String {
    r#"<invoke name="propose_change">{"path": "src/lib.rs", "old": "pub fn add(a: i32, b: i32) -> i32 { a + b }", "new": "pub fn sub(a: i32, b: i32) -> i32 { a - b"}</invoke>"#.to_string()
}

fn verify_command(command: &str) -> String {
    format!(
        r#"<invoke name="verify">{}</invoke>"#,
        serde_json::json!({ "command": command })
    )
}

fn final_answer() -> String {
    "implemented the subtract change in src/lib.rs.".to_string()
}

async fn run_session(
    harness: CodingHarness,
    tooling_root: &Path,
    planned_files: &[PathBuf],
    strict: bool,
    request: CodingRequest,
) -> (CodingResult, Arc<Mutex<Vec<AgentEvent>>>) {
    let (events, emit) = event_sink();
    let mut subagent = harness.subagent(tooling_root, planned_files, strict, &request.limits);
    let result = subagent.run(request, &emit, None).await;
    (result, events)
}

// =========================================================================
// Tests
// =========================================================================

#[tokio::test]
async fn test_coding_applies_verified_change_and_completes() {
    let dir = coding_workspace();
    let harness = CodingHarness::new(Arc::new(CodingMockProvider::text(
        "mock",
        vec![
            sub_proposal(),
            verify_command("cargo check"),
            final_answer(),
        ],
    )));

    let request = CodingRequest::new("add a subtract function", dir.path())
        .with_planning(Some(make_plan_with_validation()))
        .with_limits(CodingLimits::default());
    let (result, _) = run_session(
        harness,
        dir.path(),
        &[PathBuf::from("src/lib.rs")],
        false,
        request,
    )
    .await;

    assert_eq!(result.termination, CodingTermination::Completed);
    assert!(result.synthesis_complete);
    assert_eq!(result.changes.len(), 1, "one change applied");
    let change = &result.changes[0];
    assert!(change.verified);
    assert!(!change.rolled_back);
    assert!(!change.unplanned);
    assert_eq!(result.verification.len(), 1);
    assert_eq!(result.verification[0].exit_code, 0);
    assert_eq!(result.verification[0].source, VerificationSource::Explicit);
    assert!(result.all_verified());

    // The repository really changed: add replaced by sub.
    let lib = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert!(lib.contains("pub fn sub(a: i32, b: i32) -> i32 { a - b }"));
    assert!(!lib.contains("pub fn add"));
}

#[tokio::test]
async fn test_completion_gate_auto_verifies_with_plan_commands() {
    let dir = coding_workspace();
    let harness = CodingHarness::new(Arc::new(CodingMockProvider::text(
        "mock",
        vec![sub_proposal(), final_answer()],
    )));

    let request = CodingRequest::new("add a subtract function", dir.path())
        .with_planning(Some(make_plan_with_validation()))
        .with_limits(CodingLimits::default());
    let (result, _) = run_session(
        harness,
        dir.path(),
        &[PathBuf::from("src/lib.rs")],
        false,
        request,
    )
    .await;

    // No explicit verify was scripted: the completion gate ran the plan's
    // validation commands through the authoritative Testing surface (one per
    // plan validation command).
    assert_eq!(result.termination, CodingTermination::Completed);
    assert_eq!(result.verification.len(), 2, "gate ran both plan commands");
    assert!(
        result.verification.iter().all(|r| r.exit_code == 0),
        "gate commands must be authoritative exits"
    );
    assert!(
        result
            .verification
            .iter()
            .all(|r| r.source == VerificationSource::CompletionGate),
        "gate records must carry their provenance"
    );
    assert!(result.all_verified());
    assert_eq!(result.changes.len(), 1);
    assert!(result.changes[0].verified);
}

#[tokio::test]
async fn test_verification_failure_rolls_back_session_changes() {
    let dir = coding_workspace();
    let harness = CodingHarness::new(Arc::new(CodingMockProvider::text(
        "mock",
        vec![
            broken_proposal(),
            verify_command("cargo check"),
            verify_command("cargo check"),
        ],
    )));

    let request = CodingRequest::new("break the crate", dir.path())
        .with_planning(Some(make_plan_with_validation()))
        .with_limits(CodingLimits::default());
    let (result, _) = run_session(
        harness,
        dir.path(),
        &[PathBuf::from("src/lib.rs")],
        false,
        request,
    )
    .await;

    assert_eq!(result.termination, CodingTermination::VerificationFailed);
    assert_eq!(result.revisions, 2, "bounded revision budget exhausted");
    assert!(result.changes[0].rolled_back, "session change rolled back");
    assert_eq!(result.verification.len(), 2);

    // The repository is back to the pre-session content.
    let lib = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert!(lib.contains("pub fn add(a: i32, b: i32) -> i32 { a + b }"));
    assert!(!lib.contains("pub fn sub"));
}

/// The full preservation proof: a PRE-EXISTING user change A in the tree,
/// a Coding change B applied on top, verification failure, rollback. The
/// rollback must restore the pre-existing content (A preserved) and remove
/// only the session's own change (B reverted). This is a file-level
/// restore — never git reset/checkout/clean.
#[tokio::test]
async fn test_rollback_preserves_pre_existing_user_changes() {
    let dir = coding_workspace();
    // Pre-existing user change A: a function the user added BEFORE coding ran.
    let lib_path = dir.path().join("src/lib.rs");
    let original = std::fs::read_to_string(&lib_path).unwrap();
    let pre_existing = format!("pub fn user_fn() -> i32 {{ 42 }}\n{original}");
    std::fs::write(&lib_path, &pre_existing).unwrap();

    let harness = CodingHarness::new(Arc::new(CodingMockProvider::text(
        "mock",
        vec![
            broken_proposal(),
            verify_command("cargo check"),
            verify_command("cargo check"),
        ],
    )));
    let request = CodingRequest::new("break the crate", dir.path())
        .with_planning(Some(make_plan_with_validation()))
        .with_limits(CodingLimits::default());
    let (result, _) = run_session(
        harness,
        dir.path(),
        &[PathBuf::from("src/lib.rs")],
        false,
        request,
    )
    .await;

    assert_eq!(result.termination, CodingTermination::VerificationFailed);
    assert!(
        result.changes[0].rolled_back,
        "session change must be rolled back"
    );
    assert!(
        result
            .limitations
            .iter()
            .any(|l| l.contains("restored original content")),
        "the rollback log must be observable: {:?}",
        result.limitations
    );
    // A preserved, B reverted: exact byte-for-byte restore of the
    // pre-existing content.
    let lib = std::fs::read_to_string(&lib_path).unwrap();
    assert_eq!(lib, pre_existing, "pre-existing user change preserved");
    assert!(lib.contains("pub fn user_fn() -> i32 { 42 }"));
    assert!(!lib.contains("pub fn sub"));
}

/// Authoritative verification: exit_code != 0 + model prose claiming success
/// ⇒ Coding is NOT successful. The model applies a broken change and then
/// asserts "all green" in its final report; the completion gate's failing
/// cargo check is terminal and the change is rolled back.
#[tokio::test]
async fn test_machine_failure_overrides_model_success_claim() {
    let dir = coding_workspace();
    let harness = CodingHarness::new(Arc::new(CodingMockProvider::text(
        "mock",
        vec![
            broken_proposal(),
            "## Changed files\nsrc/lib.rs: done.\n## Verification\ncargo check: all green, success.\n".to_string(),
        ],
    )));
    let request = CodingRequest::new("break the crate", dir.path())
        .with_planning(Some(make_plan_with_validation()))
        .with_limits(CodingLimits::default());
    let (result, _) = run_session(
        harness,
        dir.path(),
        &[PathBuf::from("src/lib.rs")],
        false,
        request,
    )
    .await;

    // The prose says done; the machine says broken. The machine wins.
    assert!(
        result.synthesis_complete,
        "the model produced its final report"
    );
    assert_eq!(result.termination, CodingTermination::VerificationFailed);
    assert!(!result.termination.is_completed());
    assert!(result.changes[0].rolled_back);
    assert!(
        result
            .verification
            .iter()
            .any(|r| r.source == VerificationSource::CompletionGate && !r.success),
        "the gate's authoritative failure must be recorded"
    );
    let lib = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert!(lib.contains("pub fn add(a: i32, b: i32) -> i32 { a + b }"));
    assert!(!lib.contains("pub fn sub"));
}

/// Authoritative verification, reverse direction: exit_code == 0 + model
/// prose falsely claiming failure ⇒ the machine fact stays the source of
/// truth: the session completes successfully with the change verified.
#[tokio::test]
async fn test_machine_success_is_authoritative_over_failure_prose() {
    let dir = coding_workspace();
    let harness = CodingHarness::new(Arc::new(CodingMockProvider::text(
        "mock",
        vec![
            sub_proposal(),
            verify_command("cargo check"),
            "## Changed files\nsrc/lib.rs: changed.\n## Verification\ncargo check: FAILED (wrong output text).\n".to_string(),
        ],
    )));
    let request = CodingRequest::new("add a subtract function", dir.path())
        .with_planning(Some(make_plan_with_validation()))
        .with_limits(CodingLimits::default());
    let (result, _) = run_session(
        harness,
        dir.path(),
        &[PathBuf::from("src/lib.rs")],
        false,
        request,
    )
    .await;

    assert_eq!(result.termination, CodingTermination::Completed);
    assert_eq!(result.verification.len(), 1);
    let record = &result.verification[0];
    assert_eq!(record.exit_code, 0);
    assert!(
        record.success,
        "the authoritative exit code decides success"
    );
    assert!(result.changes[0].verified);
    assert!(!result.changes[0].rolled_back);
    assert!(result.all_verified());
    let lib = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert!(lib.contains("pub fn sub(a: i32, b: i32) -> i32 { a - b }"));
}

#[tokio::test]
async fn test_denied_verification_is_recorded_never_executed() {
    let dir = coding_workspace();
    let harness = CodingHarness::new(Arc::new(CodingMockProvider::text(
        "mock",
        vec![verify_command("rm -rf /"), final_answer()],
    )));

    let request = CodingRequest::new("clean up", dir.path())
        .with_planning(Some(make_plan_with_validation()))
        .with_limits(CodingLimits::default());
    let (result, _) = run_session(harness, dir.path(), &[], false, request).await;

    assert_eq!(result.termination, CodingTermination::Completed);
    assert_eq!(result.verification.len(), 1);
    let denied = &result.verification[0];
    assert!(denied.denied, "rm must be denied by policy");
    assert_eq!(denied.exit_code, -1);
    assert!(denied.denied_reason.is_some());
    // No change was applied, so there was nothing left for the gate to
    // verify: the session completed with the policy verdict authoritative.
    assert!(result.all_verified());
}

#[tokio::test]
async fn test_unplanned_change_is_recorded_by_default() {
    let dir = coding_workspace();
    let extra = r#"<invoke name="propose_change">{"path": "src/extra.rs", "old": "", "new": "pub fn extra() {}\n"}</invoke>"#;
    let harness = CodingHarness::new(Arc::new(CodingMockProvider::text(
        "mock",
        vec![extra.to_string(), final_answer()],
    )));

    let request = CodingRequest::new("add an extra file", dir.path())
        .with_planning(Some(make_plan_without_validation()))
        .with_limits(CodingLimits::default());
    let (result, _) = run_session(
        harness,
        dir.path(),
        &[PathBuf::from("src/lib.rs")],
        false,
        request,
    )
    .await;

    assert_eq!(result.termination, CodingTermination::Completed);
    assert_eq!(result.changes.len(), 1);
    assert!(
        result.changes[0].unplanned,
        "deviation must be recorded, not hidden"
    );
    assert_eq!(result.unplanned_changes.len(), 1);
    // The deviation was still applied (default mode records, does not deny).
    assert!(
        dir.path().join("src/extra.rs").exists(),
        "default mode applies the unplanned change"
    );
}

#[tokio::test]
async fn test_strict_mode_denies_out_of_plan_changes() {
    let dir = coding_workspace();
    let extra = r#"<invoke name="propose_change">{"path": "src/extra.rs", "old": "", "new": "pub fn extra() {}\n"}</invoke>"#;
    let harness = CodingHarness::new(Arc::new(CodingMockProvider::text(
        "mock",
        vec![extra.to_string(), final_answer()],
    )));

    let request = CodingRequest::new("add an extra file", dir.path())
        .with_planning(Some(make_plan_without_validation()))
        .with_limits(CodingLimits::default());
    let (result, _) = run_session(
        harness,
        dir.path(),
        &[PathBuf::from("src/lib.rs")],
        true,
        request,
    )
    .await;

    assert_eq!(result.termination, CodingTermination::Completed);
    assert!(
        result.changes.is_empty(),
        "strict mode must not apply out-of-plan changes"
    );
    assert!(
        !dir.path().join("src/extra.rs").exists(),
        "strict mode denied the file creation"
    );
    // The denial was observable to the model and preserved as an observation.
    assert!(
        result
            .observations
            .iter()
            .any(|o| o.name == "propose_change" && !o.success),
        "failed propose must be observable"
    );
}

#[tokio::test]
async fn test_tool_limit_bounds_session_and_preserves_changes() {
    let dir = coding_workspace();
    let harness = CodingHarness::new(Arc::new(CodingMockProvider::text(
        "mock",
        vec![sub_proposal(), sub_proposal()],
    )));

    let limits = CodingLimits {
        max_tool_calls: 1,
        ..CodingLimits::default()
    };
    let request = CodingRequest::new("add a subtract function", dir.path())
        .with_planning(Some(make_plan_without_validation()))
        .with_limits(limits.clone());
    let (result, _) = run_session(
        harness,
        dir.path(),
        &[PathBuf::from("src/lib.rs")],
        false,
        request,
    )
    .await;

    assert_eq!(result.termination, CodingTermination::ToolLimit);
    // Bound terminations leave applied changes in place for inspection.
    assert_eq!(result.changes.len(), 1);
    assert!(!result.changes[0].rolled_back);
    let lib = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert!(lib.contains("pub fn sub(a: i32, b: i32) -> i32 { a - b }"));
}

#[tokio::test]
async fn test_provider_failure_is_bounded_error_result_with_rollback() {
    let dir = coding_workspace();
    let harness = CodingHarness::new(Arc::new(CodingMockProvider::failing("mock")));

    let request = CodingRequest::new("implement", dir.path())
        .with_planning(Some(make_plan_without_validation()))
        .with_limits(CodingLimits::default());
    let (result, _) = run_session(harness, dir.path(), &[], false, request).await;

    assert_eq!(result.termination, CodingTermination::Error);
    assert!(!result.synthesis_complete);
    assert!(result.changes.is_empty());
    assert!(
        result
            .limitations
            .iter()
            .any(|l| l.contains("coding mock provider offline")),
        "the failure must be observable in the limitations: {:?}",
        result.limitations
    );
    let lib = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert!(lib.contains("pub fn add(a: i32, b: i32) -> i32 { a + b }"));
}
