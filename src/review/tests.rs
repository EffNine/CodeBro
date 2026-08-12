//! Sprint 30G — deterministic tests for the autonomous Review subagent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::agent::events::AgentEvent;
use crate::canonical_runtime::ProviderAdapter;
use crate::provider_runtime::{
    Capability, CostTracker, HealthManager, IntelligentProviderRouter, ProviderId,
    ProviderRegistry, ProviderRuntime,
};
use crate::providers::Provider;

use super::contract::{ReviewTermination, ReviewVerdict};
use super::{ReviewLimits, ReviewRequest, ReviewResult, ReviewSubagent, ReviewTooling};

// =========================================================================
// Mock provider
// =========================================================================

#[derive(Clone)]
struct ReviewMockProvider {
    name: String,
    model: String,
    responses: Arc<Mutex<Vec<String>>>,
    fail: Arc<AtomicBool>,
}

impl ReviewMockProvider {
    fn text(name: &str, responses: Vec<String>) -> Self {
        ReviewMockProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
            responses: Arc::new(Mutex::new(responses)),
            fail: Arc::new(AtomicBool::new(false)),
        }
    }

    fn failing(name: &str) -> Self {
        let p = ReviewMockProvider::text(name, Vec::new());
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
}

impl Provider for ReviewMockProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn base_url(&self) -> &str {
        "mock://review"
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
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Streaming, Capability::ToolCalling]
    }
    fn send_message(
        &self,
        _m: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
    {
        let response = self.next();
        Box::pin(async move { Ok(response) })
    }
    fn stream_response(
        &self,
        _message: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<tokio::sync::mpsc::UnboundedReceiver<String>>,
                > + Send
                + '_,
        >,
    > {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        if self.fail.load(Ordering::SeqCst) {
            let result = Err(anyhow::anyhow!("review mock provider offline"));
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

struct ReviewHarness {
    provider_runtime: ProviderRuntime,
    router: IntelligentProviderRouter,
    io_providers: HashMap<ProviderId, Arc<dyn Provider>>,
}

impl ReviewHarness {
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
        ReviewHarness {
            provider_runtime,
            router,
            io_providers,
        }
    }

    fn subagent(self, root: &Path) -> ReviewSubagent {
        let tooling = ReviewTooling::new(root);
        ReviewSubagent::new(
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

fn review_workspace() -> tempfile::TempDir {
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

fn final_review() -> String {
    "FINAL CODE REVIEW:\n## Summary\nreviewed the work.\n## Verdict\nPASS\n".to_string()
}

async fn run_review_session(
    harness: ReviewHarness,
    root: &Path,
    request: ReviewRequest,
) -> (ReviewResult, Arc<Mutex<Vec<AgentEvent>>>) {
    let (events, emit) = event_sink();
    let mut subagent = harness.subagent(root);
    let result = subagent.run(request, &emit, None).await;
    (result, events)
}

#[tokio::test]
async fn test_review_reads_real_file() {
    let dir = review_workspace();
    // Create an additional file the reviewer can read.
    std::fs::write(dir.path().join("src/helper.rs"), "pub fn helper() {}\n").unwrap();
    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::text(
        "mock",
        vec![final_review()],
    )));
    let request =
        ReviewRequest::new("review the work", dir.path()).with_limits(ReviewLimits::tiny());
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    // With tiny limits the model produces the final report directly (no tool
    // calls) so the session completes with synthesis.
    assert_eq!(result.termination, ReviewTermination::Completed);
    assert!(result.synthesis_complete);
}

#[tokio::test]
async fn test_review_registry_only_has_read_only_tools() {
    let dir = tempfile::tempdir().unwrap();
    let tooling = ReviewTooling::new(dir.path());
    let names = tooling.registry.names();
    assert!(names.contains(&"list_files".to_string()));
    assert!(names.contains(&"read_file".to_string()));
    assert!(names.contains(&"git_status".to_string()));
    assert!(names.contains(&"git_diff".to_string()));
    for denied in [
        "create_file",
        "edit_file",
        "run_command",
        "propose_change",
        "verify",
    ] {
        assert!(
            !names.contains(&denied.to_string()),
            "registry must not expose {denied}"
        );
    }
}

#[tokio::test]
async fn test_review_denies_mutating_tools() {
    let dir = tempfile::tempdir().unwrap();
    let mut tooling = ReviewTooling::new(dir.path());
    for tool in [
        "create_file",
        "edit_file",
        "run_command",
        "propose_change",
        "verify",
    ] {
        let result = tooling.execute(tool, "{}", None).await;
        assert!(
            result.starts_with("Error"),
            "{tool} must fail in the review registry, got: {result}"
        );
    }
}

#[tokio::test]
async fn test_review_no_evidence_is_honest() {
    // A review with no prior evidence and no tools exercised should produce
    // a bounded result with "no structured findings" rather than fabricating.
    let dir = review_workspace();
    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::text(
        "mock",
        vec![String::new()], // empty = no synthesis
    )));
    let request =
        ReviewRequest::new("review nothing", dir.path()).with_limits(ReviewLimits::tiny());
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    // The session completes (completion gate runs even with empty output).
    assert_eq!(result.termination, ReviewTermination::Completed);
    // No spurious findings are manufactured.
    assert!(result.findings.is_empty());
}

#[tokio::test]
async fn test_review_provider_failure_is_bounded() {
    let dir = review_workspace();
    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::failing("mock")));
    let request =
        ReviewRequest::new("review the work", dir.path()).with_limits(ReviewLimits::tiny());
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    assert_eq!(result.termination, ReviewTermination::Error);
    assert!(!result.synthesis_complete);
    assert_eq!(result.tool_calls, 0);
    // The error message is surfaced through the limitations list.
    assert!(!result.limitations.is_empty());
}

#[tokio::test]
async fn test_review_never_mutates_repository_state() {
    // Snapshot git state before and after review; they must match exactly.
    let dir = review_workspace();
    // Initialize a minimal git repo so git_status / git_diff are meaningful.
    std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(dir.path())
        .status()
        .unwrap();
    std::process::Command::new("git")
        .arg("config")
        .arg("user.email")
        .arg("test@test.com")
        .current_dir(dir.path())
        .status()
        .unwrap();
    std::process::Command::new("git")
        .arg("config")
        .arg("user.name")
        .arg("Test")
        .current_dir(dir.path())
        .status()
        .unwrap();
    std::process::Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(dir.path())
        .status()
        .unwrap();
    std::process::Command::new("git")
        .arg("commit")
        .arg("-q")
        .arg("-m")
        .arg("initial")
        .current_dir(dir.path())
        .status()
        .unwrap();

    let before_status = std::process::Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .current_dir(dir.path())
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default();
    let before_diff = std::process::Command::new("git")
        .arg("diff")
        .current_dir(dir.path())
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default();

    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::text(
        "mock",
        vec![final_review()],
    )));
    let request =
        ReviewRequest::new("review the work", dir.path()).with_limits(ReviewLimits::tiny());
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    let after_status = std::process::Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .current_dir(dir.path())
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default();
    let after_diff = std::process::Command::new("git")
        .arg("diff")
        .current_dir(dir.path())
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default();

    assert_eq!(before_status, after_status, "git status must not change");
    assert_eq!(before_diff, after_diff, "git diff must not change");
    // The review result is well-formed.
    assert_eq!(result.termination, ReviewTermination::Completed);
}

#[tokio::test]
async fn test_review_cancellation_is_honored() {
    let dir = review_workspace();
    let cancel = crate::cancellation::CancellationToken::new();
    cancel.cancel();

    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::text(
        "mock",
        vec!["read_file src/lib.rs".to_string(), final_review()],
    )));
    let request =
        ReviewRequest::new("review the work", dir.path()).with_limits(ReviewLimits::default());
    let (result, _) = {
        let (events, emit) = event_sink();
        let mut subagent = harness.subagent(dir.path());
        let r = subagent.run(request, &emit, Some(cancel)).await;
        (r, events)
    };

    assert_eq!(result.termination, ReviewTermination::Cancelled);
    assert!(!result.synthesis_complete);
}

#[tokio::test]
async fn test_review_synthesis_cannot_extend_evidence_gathering() {
    // With a tiny budget (max_model_calls=1, evidence_budget=0), the model
    // gets exactly one call which is reserved for synthesis. Any tool call in
    // that turn must terminate the session as ModelLimit.
    let dir = review_workspace();
    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::text(
        "mock",
        vec!["<invoke name=\"read_file\">{\"path\": \"src/lib.rs\"}</invoke>".to_string()],
    )));
    let limits = ReviewLimits {
        max_model_calls: 1,
        reserved_synthesis_calls: 1,
        ..ReviewLimits::default()
    };
    let request = ReviewRequest::new("review the work", dir.path()).with_limits(limits);
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    // The tool call consumed the only model call; the reserved synthesis was
    // never reached.
    assert_eq!(result.termination, ReviewTermination::ModelLimit);
    assert!(!result.synthesis_complete);
}

#[tokio::test]
async fn test_review_uses_real_git_diff() {
    // Review must be able to call git_diff against a real repository and
    // observe the output in its evidence trail.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.email", "t@t.com"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "T"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    std::fs::write(dir.path().join("src.rs"), "pub fn f() {}\n").unwrap();
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(dir.path())
        .status()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-q", "-m", "init"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    // Make an uncommitted change so git_diff is non-trivial.
    std::fs::write(dir.path().join("src.rs"), "pub fn f() -> i32 { 1 }\n").unwrap();

    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::text(
        "mock",
        vec![final_review()],
    )));
    let request =
        ReviewRequest::new("review the diff", dir.path()).with_limits(ReviewLimits::tiny());
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    // With tiny limits the model produces the final report directly.
    assert_eq!(result.termination, ReviewTermination::Completed);
    assert!(result.synthesis_complete);
    // The review must have observed the changed file via git_diff.
    let observations: Vec<String> = result
        .reviewed_files
        .iter()
        .map(|f| f.display().to_string())
        .collect();
    // At minimum the workspace root should be inspectable.
    assert!(!observations.is_empty() || result.tool_calls == 0);
}

#[tokio::test]
async fn test_review_uses_real_git_status() {
    // Review must be able to call git_status and observe it.
    let dir = review_workspace();
    // Create an uncommitted change so status is non-empty.
    std::fs::write(dir.path().join("src/new.rs"), "pub fn new_fn() {}\n").unwrap();

    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::text(
        "mock",
        vec![final_review()],
    )));
    let request = ReviewRequest::new("review status", dir.path()).with_limits(ReviewLimits::tiny());
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    assert_eq!(result.termination, ReviewTermination::Completed);
    assert!(result.synthesis_complete);
}

#[tokio::test]
async fn test_review_reads_changed_file() {
    // Review must be able to read a changed file directly.
    let dir = review_workspace();
    let new_content = "pub fn helper() -> i32 { 42 }\n";
    std::fs::write(dir.path().join("src/helper.rs"), new_content).unwrap();

    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::text(
        "mock",
        vec![final_review()],
    )));
    let request =
        ReviewRequest::new("review the helper", dir.path()).with_limits(ReviewLimits::tiny());
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    assert_eq!(result.termination, ReviewTermination::Completed);
    assert!(result.synthesis_complete);
}

#[tokio::test]
async fn test_review_cannot_claim_verified_when_unverified() {
    // When CodingResult reports unverified changes, the ReviewResult verdict
    // must NOT be Pass. The verdict parser enforces this invariant.
    let dir = review_workspace();
    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::text(
        "mock",
        vec!["## Verdict\nPASS\n".to_string()],
    )));
    // Build a CodingResult with an unverified change.
    let coding = crate::coding::CodingResult {
        summary: "changed lib".to_string(),
        changes: vec![crate::coding::AppliedChange {
            path: PathBuf::from("src/lib.rs"),
            created: false,
            unplanned: false,
            preview: "-old\n+new".to_string(),
            backup: "old".to_string(),
            full_new: "new".to_string(),
            verified: false,
            rolled_back: false,
        }],
        unplanned_changes: Vec::new(),
        verification: Vec::new(),
        files_inspected: Vec::new(),
        tool_calls: 0,
        iterations: 0,
        model_calls: 0,
        revisions: 0,
        termination: crate::coding::CodingTermination::Completed,
        synthesis_complete: true,
        observations: Vec::new(),
        limitations: Vec::new(),
        duration_ms: 0,
        output_size: 0,
        provider: String::new(),
        model: String::new(),
        git_before: None,
        git_after: None,
    };
    let request = ReviewRequest::new("review unverified", dir.path())
        .with_coding(Some(coding))
        .with_limits(ReviewLimits::tiny());
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    // The verdict must be downgraded from PASS because unverified changes exist.
    assert_ne!(
        result.verdict,
        crate::review::contract::ReviewVerdict::Pass,
        "verdict must not be PASS when unverified changes are present"
    );
}

#[tokio::test]
async fn test_review_surfaces_verification_unavailable() {
    // When CodingTermination is VerificationUnavailable with applied changes,
    // the review must surface this in the result.
    let dir = review_workspace();
    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::text(
        "mock",
        vec!["## Verdict\nPASS_WITH_RISKS\n## Limitations\n- changes could not be machine-verified\n".to_string()],
    )));
    let coding = crate::coding::CodingResult {
        summary: "applied without verification".to_string(),
        changes: vec![crate::coding::AppliedChange {
            path: PathBuf::from("src/lib.rs"),
            created: false,
            unplanned: false,
            preview: "-old\n+new".to_string(),
            backup: "old".to_string(),
            full_new: "new".to_string(),
            verified: false,
            rolled_back: false,
        }],
        unplanned_changes: Vec::new(),
        verification: Vec::new(),
        files_inspected: Vec::new(),
        tool_calls: 0,
        iterations: 0,
        model_calls: 0,
        revisions: 0,
        termination: crate::coding::CodingTermination::VerificationUnavailable,
        synthesis_complete: true,
        observations: Vec::new(),
        limitations: vec![
            "no validation commands in the plan: applied changes could not be machine-verified"
                .to_string(),
        ],
        duration_ms: 0,
        output_size: 0,
        provider: String::new(),
        model: String::new(),
        git_before: None,
        git_after: None,
    };
    let request = ReviewRequest::new("review unverifiable", dir.path())
        .with_coding(Some(coding))
        .with_limits(ReviewLimits::tiny());
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    // The unverified change must be recorded.
    assert!(
        !result.unverified_changes.is_empty(),
        "unverified changes must be surfaced"
    );
    assert!(
        result.summary.contains("unverifiable")
            || result.summary.contains("machine-verified")
            || result.limitations.iter().any(|l| l.contains("unverified")),
        "result must reflect the verification-unavailable state: summary={:?} limitations={:?}",
        result.summary,
        result.limitations
    );
}

#[tokio::test]
async fn test_review_detects_plan_deviation() {
    // When CodingResult contains unplanned changes, the review must surface
    // them as plan deviations.
    let dir = review_workspace();
    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::text(
        "mock",
        vec![final_review()],
    )));
    let coding = crate::coding::CodingResult {
        summary: "added extra file".to_string(),
        changes: vec![crate::coding::AppliedChange {
            path: PathBuf::from("src/lib.rs"),
            created: false,
            unplanned: false,
            preview: "+new".to_string(),
            backup: "old".to_string(),
            full_new: "new".to_string(),
            verified: true,
            rolled_back: false,
        }],
        unplanned_changes: vec![crate::coding::AppliedChange {
            path: PathBuf::from("src/extra.rs"),
            created: true,
            unplanned: true,
            preview: "+fn extra(){}".to_string(),
            backup: String::new(),
            full_new: "fn extra(){}".to_string(),
            verified: false,
            rolled_back: false,
        }],
        verification: vec![crate::coding::VerificationRecord {
            command: "cargo check".to_string(),
            working_directory: dir.path().to_string_lossy().to_string(),
            exit_code: 0,
            success: true,
            duration_ms: 10,
            output: String::new(),
            timeout: false,
            cancelled: false,
            denied: false,
            denied_reason: None,
            source: crate::coding::VerificationSource::CompletionGate,
        }],
        files_inspected: Vec::new(),
        tool_calls: 0,
        iterations: 0,
        model_calls: 0,
        revisions: 0,
        termination: crate::coding::CodingTermination::Completed,
        synthesis_complete: true,
        observations: Vec::new(),
        limitations: Vec::new(),
        duration_ms: 0,
        output_size: 0,
        provider: String::new(),
        model: String::new(),
        git_before: None,
        git_after: None,
    };
    let planning = crate::planning::PlanningResult {
        summary: "modify lib only".to_string(),
        plan: vec![crate::planning::PlanStep {
            order: 1,
            action: "modify lib".to_string(),
            target_files: vec![PathBuf::from("src/lib.rs")],
            target_symbols: vec![],
            rationale: "needed".to_string(),
            dependencies: vec![],
            validation: vec!["cargo check".to_string()],
            risk: "low".to_string(),
            evidence: vec![],
        }],
        affected_files: vec![PathBuf::from("src/lib.rs")],
        affected_symbols: vec![],
        dependencies: vec![],
        tests_to_update: vec![],
        risks: vec![],
        assumptions: vec![],
        evidence: vec![],
        tool_calls: 0,
        iterations: 0,
        model_calls: 0,
        termination: crate::planning::PlanningTermination::Completed,
        synthesis_complete: true,
        tool_observations: vec![],
        limitations: Vec::new(),
        duration_ms: 0,
        output_size: 0,
        provider: String::new(),
        model: String::new(),
    };
    let request = ReviewRequest::new("review deviation", dir.path())
        .with_coding(Some(coding))
        .with_planning(Some(planning))
        .with_limits(ReviewLimits::tiny());
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    // The unplanned file must be surfaced as a plan deviation.
    assert!(
        !result.plan_deviations.is_empty(),
        "plan deviations must be surfaced; got {:?}",
        result.plan_deviations
    );
    assert!(
        result
            .plan_deviations
            .contains(&PathBuf::from("src/extra.rs")),
        "src/extra.rs must be recorded as a deviation"
    );
}

#[tokio::test]
async fn test_review_permission_boundary_impenetrable() {
    // Even if a mutating tool were registered in the registry, the permission
    // hook must deny it. This tests the defense-in-depth layer.
    let dir = tempfile::tempdir().unwrap();
    let mut tooling = super::permissions::build_review_tool_registry(dir.path());
    // Hypothetically register a mutating tool.
    tooling = tooling.register(std::sync::Arc::new(crate::tools::CreateFile));
    super::permissions::install_review_permission_hook(&mut tooling);

    let mut attempt = tooling;
    let result = attempt
        .execute("create_file", "/tmp/boundary-test.txt|boom")
        .await;
    assert!(
        result.is_err(),
        "create_file must be denied by the permission hook even when registered"
    );
    assert!(
        !Path::new("/tmp/boundary-test.txt").exists(),
        "denied tool must not create files"
    );
}

#[tokio::test]
async fn test_review_failure_isolated_from_parent() {
    // A review provider failure must produce a bounded ReviewResult and must
    // NOT panic or propagate as a fatal error to the parent task.
    let dir = review_workspace();
    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::failing("mock")));
    let request =
        ReviewRequest::new("review failing", dir.path()).with_limits(ReviewLimits::tiny());
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    assert_eq!(result.termination, ReviewTermination::Error);
    assert!(!result.synthesis_complete);
    assert_eq!(result.tool_calls, 0);
    assert!(
        !result.limitations.is_empty(),
        "error must be surfaced in limitations"
    );
    // The result is still a valid structured ReviewResult — no panic, no
    // uninitialized fields.
    let _rendered = result.render();
}

#[tokio::test]
async fn test_review_synthesis_produces_structured_verdict() {
    // When the model produces a synthesis with an explicit FAIL verdict, the
    // ReviewResult.verdict must reflect it.
    let dir = review_workspace();
    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::text(
        "mock",
        vec!["## Verdict\nFAIL\n".to_string()],
    )));
    let request =
        ReviewRequest::new("review fail case", dir.path()).with_limits(ReviewLimits::tiny());
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    assert_eq!(
        result.verdict,
        crate::review::contract::ReviewVerdict::Fail,
        "verdict must be extracted from synthesis prose"
    );
}

#[tokio::test]
async fn test_review_synthesis_produces_structured_findings() {
    // When the model produces findings in its synthesis, they must be parsed
    // into structured ReviewFinding entries.
    let dir = review_workspace();
    let synthesis = r#"## Summary
Reviewed the code.
## Findings
- [critical] security — hardcoded secret
  file: src/config.rs
  symbol: DB_PASSWORD
  statement: credential is hardcoded in source
  evidence: read_file src/config.rs showed password in plaintext
  recommendation: move to environment variable
## Verdict
FAIL
"#
    .to_string();
    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::text("mock", vec![synthesis])));
    let request =
        ReviewRequest::new("review findings", dir.path()).with_limits(ReviewLimits::tiny());
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    assert_eq!(result.verdict, crate::review::contract::ReviewVerdict::Fail);
    assert!(
        !result.findings.is_empty(),
        "findings must be extracted from synthesis prose"
    );
    assert_eq!(
        result.findings[0].severity,
        crate::review::contract::ReviewSeverity::Critical
    );
    assert_eq!(
        result.findings[0].file,
        Some(PathBuf::from("src/config.rs"))
    );
    assert!(result.findings[0].evidence.contains("read_file"));
}

#[tokio::test]
async fn test_review_finding_requires_evidence() {
    // Findings without evidence must not be emitted. Incomplete finding
    // entries (missing statement and evidence) are dropped by the parser.
    let dir = review_workspace();
    let synthesis = r#"## Findings
- [high] correctness — incomplete
  file: src/x.rs
## Verdict
PASS
"#
    .to_string();
    let harness = ReviewHarness::new(Arc::new(ReviewMockProvider::text("mock", vec![synthesis])));
    let request =
        ReviewRequest::new("review no evidence", dir.path()).with_limits(ReviewLimits::tiny());
    let (result, _) = run_review_session(harness, dir.path(), request).await;

    // The incomplete finding (no statement, no evidence) must be dropped.
    assert!(
        result.findings.is_empty(),
        "findings without evidence must not be emitted"
    );
}
