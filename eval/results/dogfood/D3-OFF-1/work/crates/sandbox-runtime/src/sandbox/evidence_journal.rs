//! Execution Evidence Journal — durable, machine-generated validation memory.
//!
//! Records what CodeBro actually observed during validation runs so that
//! evidence survives process restarts and can be surfaced as HISTORICAL
//! context through the existing `sandbox_test` / `sandbox_build` responses.
//!
//! Trust model (invariant):
//! - The fact store is verified structural truth — the journal NEVER writes
//!   there and never promotes journal data into facts.
//! - Engineering memory is agent-recorded prose — the journal shares no
//!   code path, file, or schema with it.
//! - The journal is machine-generated EXECUTION EVIDENCE, always bound to
//!   the repository tree hash it was observed against. Historical evidence
//!   is never presented as current validation.
//!
//! Determinism: identical record sets serialize byte-identically; all
//! summaries are derived through fixed ordering rules. No probabilities,
//! no LLM, no embeddings. Bounds are hard constants below.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::sandbox::ParsedDiagnostic;

/// Schema version of the journal file and records.
pub const JOURNAL_SCHEMA_VERSION: &str = "1.0.0";

/// Hard bounds. Chosen to align with existing CodeBro conventions
/// (recent-edit ring 50, recommended-tests cap 32, evidence cap 8) while
/// giving the journal a useful multi-week horizon.
/// Maximum retained records (~200 x <=1KB => well under the byte cap).
pub const MAX_RECORDS: usize = 200;
/// Records older than this are dropped during retention (30 days).
pub const MAX_AGE_SECS: u64 = 30 * 24 * 3600;
/// Hard ceiling on the serialized journal file.
pub const MAX_BYTES: usize = 256 * 1024;
const MAX_COMMAND_LEN: usize = 512;
const MAX_FILTER_ENTRIES: usize = 32;
const MAX_FILTER_ENTRY_LEN: usize = 128;
const MAX_FAILED_TESTS: usize = 32;
const MAX_TEST_NAME_LEN: usize = 128;
const MAX_DIAGNOSTIC_SUMMARIES: usize = 8;
const MAX_SUMMARY_LEN: usize = 200;
const MAX_AFFECTED_MODULES: usize = 16;

/// One durably recorded execution observation. Smallest useful durable
/// representation — raw stdout/stderr are deliberately NOT fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionEvidenceRecord {
    pub schema_version: String,
    /// Sandbox execution identifier (from the evidence envelope).
    pub execution_id: String,
    /// Unix seconds when the run completed.
    pub recorded_at_unix: u64,
    /// Repository identity id at execution time (when available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Working-tree hash the run was observed against. REQUIRED for
    /// association; runs without a capturable tree state are not recorded.
    pub tree_hash: String,
    /// Runner family derived from the resolved command head
    /// (`cargo`, `go`, `python3`, `npm`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    /// Resolved command, secret-redacted and length-bounded.
    pub command: String,
    /// Explicit selection filter used (empty = full run).
    #[serde(default)]
    pub test_filter: Vec<String>,
    pub exit_code: i32,
    /// Coarse outcome classification copied from the verification layer.
    pub classification: String,
    pub success: bool,
    pub timed_out: bool,
    pub duration_ms: u64,
    /// Failing test identifiers reported by the runner (sorted, deduped).
    #[serde(default)]
    pub failed_tests: Vec<String>,
    /// Compact structured digests of leading diagnostics — NOT raw output.
    #[serde(default)]
    pub diagnostic_summary: Vec<String>,
    #[serde(default)]
    pub affected_modules: Vec<String>,
}

/// The journal file: schema version plus chronologically appended records
/// (oldest first). Retention trims from the front.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ExecutionEvidenceFile {
    pub schema_version: String,
    #[serde(default)]
    pub records: Vec<ExecutionEvidenceRecord>,
}

/// Everything needed to append one observation. Built by the caller (thin
/// MCP handler) from an existing `VerificationResult`; the journal never
/// re-derives execution facts.
pub struct JournalInput<'a> {
    pub execution_id: &'a str,
    pub project_id: Option<&'a str>,
    pub tree_hash: &'a str,
    pub command: &'a str,
    pub test_filter: &'a [String],
    pub exit_code: i32,
    pub classification: &'a str,
    pub success: bool,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub diagnostics: &'a [ParsedDiagnostic],
    pub affected_modules: &'a [String],
}

impl ExecutionEvidenceFile {
    pub fn empty() -> Self {
        ExecutionEvidenceFile {
            schema_version: JOURNAL_SCHEMA_VERSION.to_string(),
            records: Vec::new(),
        }
    }
}

fn journal_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join(".codebro")
        .join("execution_evidence.json")
}

fn clamp(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn bounded_filter(filter: &[String]) -> Vec<String> {
    filter
        .iter()
        .take(MAX_FILTER_ENTRIES)
        .map(|f| clamp(f.trim(), MAX_FILTER_ENTRY_LEN))
        .collect()
}

/// Derive a coarse runner family from the resolved command head.
fn runner_of(command: &str) -> Option<String> {
    let head = command.split_whitespace().next()?;
    if head.is_empty() {
        return None;
    }
    Some(clamp(head, 32))
}

/// Compact, redacted digest of one parsed diagnostic. Structured evidence
/// only — raw output is never persisted.
fn summarize_diagnostic(d: &ParsedDiagnostic) -> String {
    use codebro_core::tools::shell::redact_secrets_public;
    let mut s = String::new();
    s.push_str(&d.severity);
    if let Some(code) = &d.code {
        s.push('[');
        s.push_str(code);
        s.push(']');
    }
    s.push_str(": ");
    if let Some(file) = &d.file {
        s.push_str(file);
        if let Some(line) = d.line {
            s.push(':');
            s.push_str(&line.to_string());
        }
        s.push_str(": ");
    }
    s.push_str(&d.message);
    redact_secrets_public(&clamp(&s, MAX_SUMMARY_LEN))
}

/// Load the journal, tolerating absence. A corrupt file is quarantined
/// (never deleted) and treated as empty — mirroring the fact-store
/// fail-safe behavior.
pub fn load(workspace_root: &Path) -> ExecutionEvidenceFile {
    let path = journal_path(workspace_root);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return ExecutionEvidenceFile::empty(),
    };
    match serde_json::from_slice::<ExecutionEvidenceFile>(&bytes) {
        Ok(file) if file.schema_version == JOURNAL_SCHEMA_VERSION => file,
        Ok(_) | Err(_) => {
            // Wrong schema or unparseable: preserve the bytes aside, start clean.
            let _ = codebro_core::persistence::quarantine_file(&path);
            ExecutionEvidenceFile::empty()
        }
    }
}

/// Append one observation with retention enforcement. `now_unix` is
/// injected so tests stay deterministic. Returns the persisted record count.
pub fn record(workspace_root: &Path, input: &JournalInput<'_>, now_unix: u64) -> io::Result<usize> {
    use crate::tools::shell::redact_secrets_public;

    let mut file = load(workspace_root);
    // Age-based retention first.
    file.records
        .retain(|r| now_unix.saturating_sub(r.recorded_at_unix) <= MAX_AGE_SECS);

    let record = ExecutionEvidenceRecord {
        schema_version: JOURNAL_SCHEMA_VERSION.to_string(),
        execution_id: clamp(input.execution_id, 64),
        recorded_at_unix: now_unix,
        project_id: input.project_id.map(|p| clamp(p, 64)),
        tree_hash: clamp(input.tree_hash, 128),
        runner: runner_of(input.command),
        command: redact_secrets_public(&clamp(input.command.trim(), MAX_COMMAND_LEN)),
        test_filter: bounded_filter(input.test_filter),
        exit_code: input.exit_code,
        classification: clamp(input.classification, 32),
        success: input.success,
        timed_out: input.timed_out,
        duration_ms: input.duration_ms,
        failed_tests: {
            let mut v: Vec<String> = input
                .diagnostics
                .iter()
                .filter_map(|d| d.test.as_deref())
                .map(|t| redact_secrets_public(&clamp(t.trim(), MAX_TEST_NAME_LEN)))
                .filter(|t| !t.is_empty())
                .collect();
            v.sort();
            v.dedup();
            v.truncate(MAX_FAILED_TESTS);
            v
        },
        diagnostic_summary: input
            .diagnostics
            .iter()
            .take(MAX_DIAGNOSTIC_SUMMARIES)
            .map(summarize_diagnostic)
            .collect(),
        affected_modules: input
            .affected_modules
            .iter()
            .take(MAX_AFFECTED_MODULES)
            .map(|m| clamp(m, MAX_TEST_NAME_LEN))
            .collect(),
    };

    file.records.push(record);
    // Count-based retention.
    if file.records.len() > MAX_RECORDS {
        let overflow = file.records.len() - MAX_RECORDS;
        file.records.drain(0..overflow);
    }

    // Byte-cap retention: drop oldest until the serialized form fits.
    let mut bytes = serde_json::to_vec(&file)?;
    while bytes.len() > MAX_BYTES && !file.records.is_empty() {
        file.records.remove(0);
        bytes = serde_json::to_vec(&file)?;
    }

    let path = journal_path(workspace_root);
    codebro_core::persistence::write_atomic(&path, &bytes)?;
    Ok(file.records.len())
}

// ── Lookup semantics ─────────────────────────────────────────────────────

/// One failing test's durable pattern. Labels are honest and mutually
/// exclusive per test: `repeated_failure` (>=2 distinct tree hashes, no
/// recorded success under the identical invocation) or
/// `historical_variance` (both outcomes observed under the identical
/// invocation across >=2 tree hashes). Never "flaky" — that word implies a
/// calibrated model this journal deliberately does not claim.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestHistoryPattern {
    pub test: String,
    pub failures: usize,
    pub successes: usize,
    pub distinct_trees: usize,
    /// `repeated_failure` | `historical_variance`
    pub pattern: &'static str,
    pub last_seen_seconds_ago: u64,
}

/// Most recent prior run with the SAME tree hash and SAME invocation
/// (trimmed command + filter). This is the strongest reusable historical
/// evidence the journal can offer — and it remains historical: the current
/// run's outcome still governs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SameTreePriorRun {
    pub outcome: String,
    pub age_seconds: u64,
    pub exit_code: i32,
}

/// Compact summary of prior evidence for one (tree hash, command, filter)
/// context. Serialized into responses ONLY when at least one section is
/// populated; omitted entirely otherwise.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PriorEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_tree: Option<SameTreePriorRun>,
    /// Tests failing repeatedly across distinct tree states (cap 5).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub repeated_failures: Vec<TestHistoryPattern>,
    /// Tests showing both outcomes across tree states under the identical
    /// invocation (cap 5).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub historical_variance: Vec<TestHistoryPattern>,
    /// Total records considered — tiny honesty signal for consumers.
    pub history_size: usize,
}

impl PriorEvidence {
    pub fn is_empty(&self) -> bool {
        self.same_tree.is_none()
            && self.repeated_failures.is_empty()
            && self.historical_variance.is_empty()
    }
}

/// An invocation identity: trimmed command plus ordered filter.
fn invocation_matches(
    record_cmd: &str,
    record_filter: &[String],
    cmd: &str,
    filter: &[String],
) -> bool {
    record_cmd.trim() == cmd.trim() && record_filter == filter
}

/// Summarize prior evidence relevant to the given context, evaluated
/// against the journal state BEFORE the current run is recorded.
/// Deterministic: fixed iteration order, fixed sort keys, hard caps.
pub fn summarize_prior(
    workspace_root: &Path,
    tree_hash: &str,
    command: &str,
    test_filter: &[String],
    now_unix: u64,
) -> Option<PriorEvidence> {
    let file = load(workspace_root);
    if file.records.is_empty() {
        return None;
    }

    // Same-tree, same-invocation most recent run.
    let same_tree = file
        .records
        .iter()
        .rev()
        .find(|r| {
            r.tree_hash == tree_hash
                && invocation_matches(&r.command, &r.test_filter, command, test_filter)
        })
        .map(|r| SameTreePriorRun {
            outcome: r.classification.clone(),
            age_seconds: now_unix.saturating_sub(r.recorded_at_unix),
            exit_code: r.exit_code,
        });

    // Per-test history under the IDENTICAL invocation only — this keeps the
    // pass/fail attribution sound: a success record with this exact
    // invocation exercised every selected test, including this one.
    // Cross-invocation aggregation would silently mix incomparable
    // selections, so it is deliberately not done here.
    #[derive(Default)]
    struct TestStat {
        failures: usize,
        successes: usize,
        trees: std::collections::BTreeSet<String>,
        last_seen: u64,
    }
    let mut stats: std::collections::BTreeMap<String, TestStat> = Default::default();
    for r in &file.records {
        if !invocation_matches(&r.command, &r.test_filter, command, test_filter) {
            continue;
        }
        let failed_now: std::collections::HashSet<&str> =
            r.failed_tests.iter().map(|s| s.as_str()).collect();
        // Every recorded failing test is a failure observation.
        for t in &r.failed_tests {
            let e = stats.entry(t.clone()).or_default();
            e.failures += 1;
            e.trees.insert(r.tree_hash.clone());
            e.last_seen = e.last_seen.max(r.recorded_at_unix);
        }
        // A successful run of this invocation is a success observation for
        // every test the invocation could have exercised. Attribute it to
        // tests we are already tracking (from failures) rather than to the
        // universe of untracked tests, keeping the map bounded.
        if r.success {
            for (name, e) in stats.iter_mut() {
                if !failed_now.contains(name.as_str()) {
                    e.successes += 1;
                    e.trees.insert(r.tree_hash.clone());
                }
            }
        }
        let _ = now_unix;
    }

    let mut patterns: Vec<TestHistoryPattern> = stats
        .into_iter()
        .filter_map(|(test, s)| {
            let last_seen_seconds_ago = now_unix.saturating_sub(s.last_seen);
            if s.failures > 0 && s.successes > 0 && s.trees.len() >= 2 {
                Some(TestHistoryPattern {
                    test,
                    failures: s.failures,
                    successes: s.successes,
                    distinct_trees: s.trees.len(),
                    pattern: "historical_variance",
                    last_seen_seconds_ago,
                })
            } else if s.failures > 0 && s.trees.len() >= 2 {
                Some(TestHistoryPattern {
                    test,
                    failures: s.failures,
                    successes: s.successes,
                    distinct_trees: s.trees.len(),
                    pattern: "repeated_failure",
                    last_seen_seconds_ago,
                })
            } else {
                None
            }
        })
        .collect();

    // Deterministic ordering: pattern severity (variance first), then
    // failure count desc, then test name asc. Hard caps keep responses small.
    patterns.sort_by(|a, b| {
        a.pattern
            .cmp(b.pattern)
            .then(b.failures.cmp(&a.failures))
            .then(a.test.cmp(&b.test))
    });
    let repeated_failures: Vec<TestHistoryPattern> = patterns
        .iter()
        .filter(|p| p.pattern == "repeated_failure")
        .take(5)
        .cloned()
        .collect();
    let historical_variance: Vec<TestHistoryPattern> = patterns
        .iter()
        .filter(|p| p.pattern == "historical_variance")
        .take(5)
        .cloned()
        .collect();

    let prior = PriorEvidence {
        same_tree,
        repeated_failures,
        historical_variance,
        history_size: file.records.len(),
    };
    if prior.is_empty() {
        None
    } else {
        Some(prior)
    }
}

// ── Current-tree execution state ─────────────────────────────────────────
//
// The journal answers "what happened before?" at the next run. This section
// answers the complementary question the runtime must answer at any time:
// "does the CURRENT working tree have unresolved failing evidence?"
//
// Semantics (deterministic, no model input):
// - Evidence is associated with the working tree it was observed against
//   (tree hash). Any edit changes the hash, so stale evidence never applies
//   to the new state — the state becomes `unverified` until re-run.
// - A failure is RESOLVED only by a later recorded success of the same
//   invocation (identical command; identical filter, or a full run with no
//   filter — including a full run of the same runner that covers a
//   filtered selection) on the same tree. Prose never resolves a failure.
// - `compile_error` and `test_failure` are AUTHORITATIVE: the toolchain ran
//   and reported a defect in the code. They block a "completed" claim.
// - `timeout` / `unknown_failure` are inconclusive (environmental,
//   truncated, or unparsed): surfaced as `unverified`, never blocking on
//   their own — the runtime does not pretend to know what it cannot.

/// Failure classifications that authoritatively indicate a code defect.
pub const AUTHORITATIVE_FAILURE_CLASSES: [&str; 2] = ["compile_error", "test_failure"];

/// Maximum unresolved failures serialized per assessment (bounded output).
pub const MAX_ASSESSED_FAILURES: usize = 3;

/// The execution-state vocabulary for one working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStateKind {
    /// At least one unresolved authoritative (compile/test) failure exists
    /// for the current working tree. Completion must not be claimed.
    Failed,
    /// Recorded passing execution evidence exists for the current tree and
    /// no unresolved failure contradicts it.
    Verified,
    /// No sufficient evidence: no records for this tree, or only
    /// inconclusive (timeout/unknown) failures. Never presented as success.
    Unverified,
    /// The workspace has no capturable tree identity (not a git repository,
    /// or git unavailable), so evidence cannot be associated.
    Unknown,
}

impl ExecutionStateKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionStateKind::Failed => "failed",
            ExecutionStateKind::Verified => "verified",
            ExecutionStateKind::Unverified => "unverified",
            ExecutionStateKind::Unknown => "unknown",
        }
    }
}

/// One failure still applicable to the current working tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnresolvedFailure {
    pub command: String,
    pub classification: String,
    pub exit_code: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_tests: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostic_summary: Vec<String>,
    pub age_seconds: u64,
    pub execution_id: String,
}

/// The most recent passing execution observed on the current working tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastSuccess {
    pub command: String,
    pub classification: String,
    pub age_seconds: u64,
    pub execution_id: String,
}

/// Deterministic assessment of the evidence applicable to one tree state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionStateAssessment {
    pub state: ExecutionStateKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_hash: Option<String>,
    /// Unresolved authoritative failures (max [`MAX_ASSESSED_FAILURES`] shown).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_failures: Vec<UnresolvedFailure>,
    /// Total unresolved authoritative failures (may exceed the shown list).
    #[serde(default)]
    pub unresolved_failures_total: usize,
    /// Unresolved inconclusive failures (timeout/unknown; shown, non-blocking).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inconclusive_failures: Vec<UnresolvedFailure>,
    #[serde(default)]
    pub inconclusive_failures_total: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success: Option<LastSuccess>,
    /// Records considered for this tree.
    pub evidence_records: usize,
    /// Honest one-line description of what the state means.
    pub note: String,
}

impl ExecutionStateAssessment {
    /// True when unresolved authoritative failures forbid a success claim.
    pub fn blocks_completion(&self) -> bool {
        self.state == ExecutionStateKind::Failed
    }

    /// Compact single-line evidence summary for refusal messages.
    pub fn failure_summary(&self) -> String {
        self.unresolved_failures
            .iter()
            .map(|f| {
                format!(
                    "{} → {} (exit {}, {}s ago)",
                    f.command, f.classification, f.exit_code, f.age_seconds
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// Does this token reference a filter entry (exact or wrapped, e.g. go's
/// `^TestFoo$`)?
fn filter_entry_token(token: &str, filter: &[String]) -> bool {
    filter
        .iter()
        .any(|f| !f.is_empty() && (token == f.as_str() || token.contains(f.as_str())))
}

/// Is this token part of a filter expression rather than runner/scope
/// configuration? Filter entries, the selector syntax that embeds them
/// (`-k`, `-run`, pytest's `or`), and wrapped entries qualify; runner and
/// scope selectors (`-p crateA`, `--manifest-path`, `--release`, ...) do
/// not.
fn filter_expression_token(token: &str, filter: &[String]) -> bool {
    matches!(token, "-k" | "-run" | "or") || filter_entry_token(token, filter)
}

/// Does a later successful full run of the same runner exercise the
/// selection that failed?
///
/// The MCP resolver embeds filter names in the resolved command (cargo:
/// `cargo test adds`; pytest: `python -m pytest -q --tb=long -k a or b`;
/// go: `go test -run ^X$ ./...`), so a later unfiltered run of the same
/// runner does not share the exact command string.
///
/// A passing full run covers the filtered selection only when the full
/// command is exactly the failing command with ONE contiguous filter
/// expression removed, every removed token is filter-related, and at least
/// one removed token references a filter entry. This keeps every
/// auto-generated shape covering while refusing to treat an unexplained
/// scope as verified: a root `cargo test` after an explicit
/// `cargo test -p crateA adds` failure is not evidence that crateA (which
/// need not be a default workspace member) was exercised.
fn full_run_covers(success: &ExecutionEvidenceRecord, failure: &ExecutionEvidenceRecord) -> bool {
    if !success.test_filter.is_empty() || failure.test_filter.is_empty() {
        return false;
    }
    if success.runner.is_none() || success.runner != failure.runner {
        return false;
    }
    let success_tokens: Vec<&str> = success.command.split_whitespace().collect();
    let failure_tokens: Vec<&str> = failure.command.split_whitespace().collect();
    if success_tokens.is_empty() || success_tokens.len() >= failure_tokens.len() {
        return false;
    }
    (0..failure_tokens.len()).any(|start| {
        ((start + 1)..=failure_tokens.len()).any(|end| {
            let removed = &failure_tokens[start..end];
            removed
                .iter()
                .all(|token| filter_expression_token(token, &failure.test_filter))
                && removed
                    .iter()
                    .any(|token| filter_entry_token(token, &failure.test_filter))
                && failure_tokens[..start]
                    .iter()
                    .chain(failure_tokens[end..].iter())
                    .copied()
                    .eq(success_tokens.iter().copied())
        })
    })
}

/// Does a later success record resolve an earlier failure record?
/// Identical command with an identical filter or a full run, or a full run
/// of the same runner that covers a filtered failure.
fn success_supersedes_same_tree(
    success: &ExecutionEvidenceRecord,
    failure: &ExecutionEvidenceRecord,
) -> bool {
    success.success
        && success.recorded_at_unix >= failure.recorded_at_unix
        && ((success.command.trim() == failure.command.trim()
            && (success.test_filter == failure.test_filter || success.test_filter.is_empty()))
            || full_run_covers(success, failure))
}

/// Assess the CURRENT working tree: capture its identity and reduce the
/// recorded evidence that applies to it. Read-only (never writes).
pub fn assess(workspace_root: &Path, now_unix: u64) -> ExecutionStateAssessment {
    let root = workspace_root.to_path_buf();
    match codebro_core::RepoState::capture(&root) {
        Some(state) => assess_for_tree(workspace_root, &state.working_tree_hash, now_unix),
        None => ExecutionStateAssessment {
            state: ExecutionStateKind::Unknown,
            tree_hash: None,
            unresolved_failures: Vec::new(),
            unresolved_failures_total: 0,
            inconclusive_failures: Vec::new(),
            inconclusive_failures_total: 0,
            last_success: None,
            evidence_records: 0,
            note: "workspace is not a git repository (or git is unavailable): \
                   execution evidence cannot be associated with the current state"
                .to_string(),
        },
    }
}

/// Assess one explicit tree state. Deterministic; the ceiling on parsed
/// records is [`MAX_RECORDS`] by file retention.
pub fn assess_for_tree(
    workspace_root: &Path,
    tree_hash: &str,
    now_unix: u64,
) -> ExecutionStateAssessment {
    let file = load(workspace_root);
    let records: Vec<&ExecutionEvidenceRecord> = file
        .records
        .iter()
        .filter(|r| r.tree_hash == tree_hash)
        .collect();

    let mut unresolved: Vec<UnresolvedFailure> = Vec::new();
    let mut inconclusive: Vec<UnresolvedFailure> = Vec::new();
    for (idx, r) in records.iter().enumerate() {
        if r.success {
            continue;
        }
        let resolved = records[idx + 1..]
            .iter()
            .any(|s| success_supersedes_same_tree(s, r));
        if resolved {
            continue;
        }
        let entry = UnresolvedFailure {
            command: r.command.clone(),
            classification: r.classification.clone(),
            exit_code: r.exit_code,
            failed_tests: r.failed_tests.clone(),
            diagnostic_summary: r.diagnostic_summary.clone(),
            age_seconds: now_unix.saturating_sub(r.recorded_at_unix),
            execution_id: r.execution_id.clone(),
        };
        if AUTHORITATIVE_FAILURE_CLASSES.contains(&r.classification.as_str()) {
            unresolved.push(entry);
        } else {
            inconclusive.push(entry);
        }
    }
    // Most recent first, then bounded.
    let by_recency =
        |a: &UnresolvedFailure, b: &UnresolvedFailure| a.age_seconds.cmp(&b.age_seconds);
    unresolved.sort_by(by_recency);
    inconclusive.sort_by(by_recency);
    let unresolved_total = unresolved.len();
    let inconclusive_total = inconclusive.len();
    unresolved.truncate(MAX_ASSESSED_FAILURES);
    inconclusive.truncate(MAX_ASSESSED_FAILURES);

    let last_success = records
        .iter()
        .rev()
        .find(|r| r.success)
        .map(|r| LastSuccess {
            command: r.command.clone(),
            classification: r.classification.clone(),
            age_seconds: now_unix.saturating_sub(r.recorded_at_unix),
            execution_id: r.execution_id.clone(),
        });

    let state = if unresolved_total > 0 {
        ExecutionStateKind::Failed
    } else if inconclusive_total > 0 {
        ExecutionStateKind::Unverified
    } else if last_success.is_some() {
        ExecutionStateKind::Verified
    } else {
        ExecutionStateKind::Unverified
    };
    let note = match state {
        ExecutionStateKind::Failed => format!(
            "current working tree has {unresolved_total} unresolved compile/test failure(s) \
             recorded by CodeBro — resolve and re-run the same verification command \
             successfully, or report an explicit failure/partial outcome; prose cannot clear this"
        ),
        ExecutionStateKind::Unverified if inconclusive_total > 0 => format!(
            "current working tree has {inconclusive_total} unresolved inconclusive failure(s) \
             (timeout/unknown) — the outcome is unverified, not verified"
        ),
        ExecutionStateKind::Verified => {
            "current working tree has recorded passing execution evidence".to_string()
        }
        ExecutionStateKind::Unverified => {
            "no execution evidence is recorded for the current working tree — the outcome is \
             unverified until sandbox_build/sandbox_test passes"
                .to_string()
        }
        ExecutionStateKind::Unknown => {
            "execution evidence cannot be associated with the current state".to_string()
        }
    };

    ExecutionStateAssessment {
        state,
        tree_hash: Some(tree_hash.to_string()),
        unresolved_failures: unresolved,
        unresolved_failures_total: unresolved_total,
        inconclusive_failures: inconclusive,
        inconclusive_failures_total: inconclusive_total,
        last_success,
        evidence_records: records.len(),
        note,
    }
}

// ── Read-only health status ──────────────────────────────────────────

/// Read-only health summary for `doctor` / `repository_health`.
///
/// Never writes, never quarantines, never creates directories. A corrupt
/// or wrong-schema file reports `valid=false` so the caller can warn
/// without mutating the workspace (quarantine happens lazily on the next
/// validation run via [`load`], not during health checks).
#[derive(Debug, Clone, PartialEq)]
pub struct JournalStatus {
    pub exists: bool,
    pub valid: bool,
    pub records: usize,
    pub bytes: u64,
    pub newest_age_secs: Option<u64>,
}

pub fn status(workspace_root: &Path, now_unix: u64) -> JournalStatus {
    let path = journal_path(workspace_root);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            return JournalStatus {
                exists: false,
                valid: true,
                records: 0,
                bytes: 0,
                newest_age_secs: None,
            };
        }
    };
    let len = bytes.len() as u64;
    match serde_json::from_slice::<ExecutionEvidenceFile>(&bytes) {
        Ok(file) if file.schema_version == JOURNAL_SCHEMA_VERSION => {
            let newest = file
                .records
                .iter()
                .map(|r| r.recorded_at_unix)
                .max()
                .map(|t| now_unix.saturating_sub(t));
            JournalStatus {
                exists: true,
                valid: true,
                records: file.records.len(),
                bytes: len,
                newest_age_secs: newest,
            }
        }
        _ => JournalStatus {
            exists: true,
            valid: false,
            records: 0,
            bytes: len,
            newest_age_secs: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::ParsedDiagnostic;

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn diag(
        file: Option<&str>,
        line: Option<u32>,
        test: Option<&str>,
        msg: &str,
    ) -> ParsedDiagnostic {
        ParsedDiagnostic {
            severity: "failure".into(),
            code: None,
            message: msg.into(),
            file: file.map(|s| s.into()),
            line,
            column: None,
            test: test.map(|s| s.into()),
        }
    }

    fn input<'a>(
        tree: &'a str,
        cmd: &'a str,
        filter: &'a [String],
        success: bool,
        diags: &'a [ParsedDiagnostic],
    ) -> JournalInput<'a> {
        // Test-only leak: tiny fixed slices outlive the fixture for 'a.
        let modules: &'static [String] = Box::leak(vec!["src/lib".to_string()].into_boxed_slice());
        JournalInput {
            execution_id: "exec-1",
            project_id: Some("proj"),
            tree_hash: tree,
            command: cmd,
            test_filter: filter,
            exit_code: if success { 0 } else { 1 },
            classification: if success { "success" } else { "test_failure" },
            success,
            timed_out: false,
            duration_ms: 1234,
            diagnostics: diags,
            affected_modules: modules,
        }
    }
    fn record_at(root: &Path, tree: &str, cmd: &str, filter: &[String], success: bool, t: u64) {
        let test_name = filter
            .first()
            .cloned()
            .unwrap_or_else(|| "adds".to_string());
        let d = if success {
            vec![]
        } else {
            vec![diag(
                Some("src/lib.rs"),
                Some(5),
                Some(&test_name),
                "assertion failed",
            )]
        };
        let inp = input(tree, cmd, filter, success, &d);
        record(root, &inp, t).unwrap();
    }

    // 1/2. Success and failure executions both record.
    #[test]
    fn records_success_and_failure() {
        let dir = temp_root();
        record_at(dir.path(), "treeA", "cargo test", &[], true, 100);
        record_at(dir.path(), "treeB", "cargo test", &[], false, 200);
        let file = load(dir.path());
        assert_eq!(file.records.len(), 2);
        assert!(file.records[0].success);
        assert!(!file.records[1].success);
        assert_eq!(file.records[1].failed_tests, vec!["adds".to_string()]);
        assert_eq!(file.records[1].classification, "test_failure");
    }

    // 3. Diagnostics/classification survive serialization round-trip.
    #[test]
    fn diagnostic_summary_survives_serialization() {
        let dir = temp_root();
        let diags = vec![diag(
            Some("src/lib.rs"),
            Some(7),
            Some("t_roundtrip"),
            "expected 4, got 5",
        )];
        let inp = input("treeR", "cargo test", &[], false, &diags);
        record(dir.path(), &inp, 50).unwrap();

        // Round-trip through actual file bytes.
        let bytes = std::fs::read(journal_path(dir.path())).unwrap();
        let parsed: ExecutionEvidenceFile = serde_json::from_slice(&bytes).unwrap();
        let r = &parsed.records[0];
        assert_eq!(r.classification, "test_failure");
        assert_eq!(r.failed_tests, vec!["t_roundtrip".to_string()]);
        assert_eq!(r.diagnostic_summary.len(), 1);
        assert!(r.diagnostic_summary[0].contains("src/lib.rs:7"));
        assert!(r.diagnostic_summary[0].contains("expected 4, got 5"));
    }

    // 4. Tree hash recorded and used for association.
    #[test]
    fn tree_hash_is_recorded() {
        let dir = temp_root();
        record_at(dir.path(), "abc123", "cargo test", &[], true, 10);
        let file = load(dir.path());
        assert_eq!(file.records[0].tree_hash, "abc123");
    }

    // 8. Retention limits enforced (record count).
    #[test]
    fn retention_enforces_record_cap() {
        let dir = temp_root();
        for i in 0..(MAX_RECORDS + 25) {
            record_at(
                dir.path(),
                &format!("tree{i}"),
                "cargo test",
                &[],
                true,
                i as u64,
            );
        }
        let file = load(dir.path());
        assert_eq!(file.records.len(), MAX_RECORDS);
        // Oldest dropped: first retained record is the (MAX_RECORDS+25)-MAX_RECORDS-th.
        assert_eq!(file.records[0].tree_hash, "tree25");
    }

    #[test]
    fn retention_enforces_age() {
        let dir = temp_root();
        record_at(dir.path(), "old", "cargo test", &[], true, 0);
        record_at(
            dir.path(),
            "new",
            "cargo test",
            &[],
            true,
            MAX_AGE_SECS + 10,
        );
        let file = load(dir.path());
        assert_eq!(file.records.len(), 1);
        assert_eq!(file.records[0].tree_hash, "new");
    }

    #[test]
    fn retention_enforces_byte_cap() {
        let dir = temp_root();
        // Large summaries to blow past the byte cap quickly.
        let big_msg = "x".repeat(MAX_SUMMARY_LEN);
        for i in 0..40 {
            let mut d = Vec::new();
            for j in 0..MAX_DIAGNOSTIC_SUMMARIES {
                d.push(diag(Some("f.rs"), Some(j as u32), None, &big_msg));
            }
            let tree = format!("t{i}");
            let inp = input(&tree, "cargo test", &[], false, &d);
            record(dir.path(), &inp, i).unwrap();
        }
        let meta = std::fs::metadata(journal_path(dir.path())).unwrap();
        assert!(meta.len() as usize <= MAX_BYTES, "{}", meta.len());
    }

    // 9. Byte-identical serialization of identical fixed inputs.
    #[test]
    fn serialization_is_byte_identical_for_identical_records() {
        let mk_bytes = |dir: &tempfile::TempDir| std::fs::read(journal_path(dir.path())).unwrap();
        let a = temp_root();
        let b = temp_root();
        let filter = vec!["adds".to_string()];
        record_at(a.path(), "treeX", "cargo test", &filter, false, 42);
        record_at(b.path(), "treeX", "cargo test", &filter, false, 42);
        // Same logical content except execution_id/project fields are fixed
        // in the fixture => identical bytes.
        assert_eq!(mk_bytes(&a), mk_bytes(&b));
    }

    // 10. Corruption quarantine / fail-safe.
    #[test]
    fn corrupt_journal_is_quarantined_not_crashed() {
        let dir = temp_root();
        record_at(dir.path(), "t", "cargo test", &[], true, 1);
        let path = journal_path(dir.path());
        std::fs::write(&path, b"{not valid json!!").unwrap();
        let file = load(dir.path());
        assert!(file.records.is_empty());
        // Original bytes preserved beside the journal.
        let quarantined: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains("execution_evidence.json.corrupt"))
            .collect();
        assert_eq!(quarantined.len(), 1, "{quarantined:?}");
        // Wrong-schema files are quarantined too.
        std::fs::write(&path, br#"{"schema_version":"9.9.9","records":[]}"#).unwrap();
        assert!(load(dir.path()).records.is_empty());
    }

    // 11. Secrets are redacted before persistence.
    #[test]
    fn secrets_are_redacted() {
        let dir = temp_root();
        let diags = vec![diag(
            Some("src/api.rs"),
            Some(3),
            Some("t_secret"),
            "request failed with api_key=abcdefghijklmnopqrst and Bearer abcdefghijklmnopqrstuvwx",
        )];
        let inp = JournalInput {
            execution_id: "e",
            project_id: None,
            tree_hash: "t",
            command: "cargo test -- --token supersecretvalue123",
            test_filter: &[],
            exit_code: 1,
            classification: "test_failure",
            success: false,
            timed_out: false,
            duration_ms: 1,
            diagnostics: &diags,
            affected_modules: &[],
        };
        record(dir.path(), &inp, 5).unwrap();
        let bytes = std::fs::read_to_string(journal_path(dir.path())).unwrap();
        assert!(!bytes.contains("supersecretvalue123"), "{bytes}");
        assert!(!bytes.contains("abcdefghijklmnopqrst"), "{bytes}");
        assert!(bytes.contains("[REDACTED]"));
    }

    // 12. Raw stdout/stderr are structurally impossible to persist.
    #[test]
    fn raw_output_never_persisted() {
        let dir = temp_root();
        let marker_stdout = "RAWDATA_STDOUT_MARKER_918273645";
        let marker_stderr = "RAWDATA_STDERR_MARKER_192837465";
        // The JournalInput type has NO stdout/stderr field at all; prove the
        // persisted form cannot contain them even when they appear nearby.
        let execution_like = format!("{marker_stdout}{marker_stderr}");
        let _ = execution_like;
        let diags = vec![diag(None, None, Some("t"), "normal message")];
        let inp = input("tree", "cargo test", &[], false, &diags);
        record(dir.path(), &inp, 3).unwrap();
        let bytes = std::fs::read_to_string(journal_path(dir.path())).unwrap();
        assert!(!bytes.contains(marker_stdout));
        assert!(!bytes.contains(marker_stderr));
    }

    // 13/14. The journal never touches facts or memory state.
    #[test]
    fn journal_never_modifies_facts_or_memory_files() {
        let dir = temp_root();
        let codebro = dir.path().join(".codebro");
        std::fs::create_dir_all(&codebro).unwrap();
        let facts = codebro.join("facts.json");
        let memory = codebro.join("engineering_memory.json");
        let identity = codebro.join("project_identity.json");
        std::fs::write(&facts, br#"{"facts":"untouched"}"#).unwrap();
        std::fs::write(&memory, br#"{"memory":"untouched"}"#).unwrap();
        std::fs::write(&identity, br#"{"identity":"untouched"}"#).unwrap();

        record_at(dir.path(), "treeZ", "go test ./...", &[], false, 77);

        assert_eq!(
            std::fs::read(&facts).unwrap(),
            b"{\"facts\":\"untouched\"}".as_slice()
        );
        assert_eq!(
            std::fs::read(&memory).unwrap(),
            b"{\"memory\":\"untouched\"}".as_slice()
        );
        assert_eq!(
            std::fs::read(&identity).unwrap(),
            b"{\"identity\":\"untouched\"}".as_slice()
        );
    }

    // 15. Journal survives process restart == plain file persistence
    // (each load() is a fresh read; there is no process-local cache).
    #[test]
    fn journal_survives_restart() {
        let dir = temp_root();
        record_at(dir.path(), "treeP", "cargo test", &[], true, 500);
        // A brand-new load in this process models a fresh server process.
        let file = load(dir.path());
        assert_eq!(file.records.len(), 1);
        assert_eq!(file.records[0].execution_id, "exec-1");
    }

    // 19. Concurrent same-process writes stay safe (serialized by callers;
    // here verify interleaved appends never corrupt or lose ordering).
    #[test]
    fn interleaved_appends_remain_consistent() {
        let dir = temp_root();
        for i in 0..20 {
            record_at(
                dir.path(),
                &format!("tree{i}"),
                "cargo test",
                &[],
                i % 2 == 0,
                i,
            );
        }
        let file = load(dir.path());
        assert_eq!(file.records.len(), 20);
        for w in file.records.windows(2) {
            assert!(w[0].recorded_at_unix <= w[1].recorded_at_unix);
        }
    }

    // 20. Malformed entries do not crash lookups.
    #[test]
    fn malformed_entries_fail_safe() {
        let dir = temp_root();
        // Records missing required fields make the whole file invalid under
        // strict parsing => quarantine + empty (fail-safe), not a panic.
        std::fs::create_dir_all(dir.path().join(".codebro")).unwrap();
        std::fs::write(
            journal_path(dir.path()),
            br#"{"schema_version":"1.0.0","records":[{"execution_id":"x"}]}"#,
        )
        .unwrap();
        let prior = summarize_prior(dir.path(), "tree", "cargo test", &[], 999);
        assert!(prior.is_none());
        assert!(load(dir.path()).records.is_empty());
    }

    // ── Lookup semantics ────────────────────────────────────────────────

    // Empty history behaves cleanly.
    #[test]
    fn empty_history_yields_no_prior_evidence() {
        let dir = temp_root();
        assert!(summarize_prior(dir.path(), "t", "cargo test", &[], 1).is_none());
    }

    // 5. Same-tree + same invocation lookup works.
    #[test]
    fn same_tree_prior_run_found() {
        let dir = temp_root();
        let f: Vec<String> = vec!["adds".into()];
        record_at(dir.path(), "treeS", "cargo test", &f, true, 1000);
        let prior = summarize_prior(dir.path(), "treeS", "cargo test", &f, 1600).unwrap();
        let st = prior.same_tree.unwrap();
        assert_eq!(st.outcome, "success");
        assert_eq!(st.age_seconds, 600);
    }

    // 6. Different-tree evidence distinguished from same-tree.
    #[test]
    fn different_tree_is_not_same_tree_but_still_history() {
        let dir = temp_root();
        let f: Vec<String> = vec!["adds".into()];
        record_at(dir.path(), "treeOld", "cargo test", &f, true, 1000);
        // A different tree is never same-tree evidence.
        let prior = summarize_prior(dir.path(), "treeNew", "cargo test", &f, 1100);
        // And a lone success record carries NO reusable pattern sections,
        // so the summary is correctly omitted rather than padded.
        assert!(prior.is_none());
        assert_eq!(load(dir.path()).records.len(), 1);
    }

    // 7. Prior successful validation of same tree/filter surfaces.
    #[test]
    fn prior_success_of_same_tree_and_filter_surfaces() {
        let dir = temp_root();
        let f: Vec<String> = vec!["math".into()];
        record_at(dir.path(), "treeV", "python3 -m pytest -q", &f, true, 100);
        let prior = summarize_prior(dir.path(), "treeV", "python3 -m pytest -q", &f, 150).unwrap();
        assert_eq!(prior.same_tree.as_ref().unwrap().outcome, "success");
    }

    // Repeated failures across distinct trees → repeated_failure pattern.
    #[test]
    fn repeated_failures_across_trees_detected() {
        let dir = temp_root();
        let f: Vec<String> = vec![];
        record_at(dir.path(), "tree1", "cargo test", &f, false, 100);
        record_at(dir.path(), "tree2", "cargo test", &f, false, 200);
        let prior = summarize_prior(dir.path(), "tree3", "cargo test", &f, 300).unwrap();
        assert_eq!(prior.repeated_failures.len(), 1);
        let p = &prior.repeated_failures[0];
        assert_eq!(p.test, "adds");
        assert_eq!(p.failures, 2);
        assert_eq!(p.distinct_trees, 2);
        assert_eq!(p.pattern, "repeated_failure");
    }

    // PASS→FAIL→PASS across distinct trees → historical_variance (never "flaky").
    #[test]
    fn historical_variance_detected_with_honest_label() {
        let dir = temp_root();
        let f: Vec<String> = vec!["wobbly".into()];
        // Failure runs carry failed_tests; success runs attribute passes to
        // tracked tests under the identical invocation.
        record_at(dir.path(), "v1", "cargo test", &f, false, 100);
        record_at(dir.path(), "v2", "cargo test", &f, true, 200);
        record_at(dir.path(), "v3", "cargo test", &f, false, 300);
        let prior = summarize_prior(dir.path(), "v4", "cargo test", &f, 400).unwrap();
        assert!(prior.repeated_failures.is_empty());
        assert_eq!(prior.historical_variance.len(), 1);
        let p = &prior.historical_variance[0];
        assert_eq!(p.test, "wobbly");
        assert_eq!(p.failures, 2);
        assert_eq!(p.successes, 1);
        assert_eq!(p.distinct_trees, 3);
        assert_eq!(p.pattern, "historical_variance");
        assert!(prior
            .historical_variance
            .iter()
            .all(|p| p.pattern != "flaky"));
    }

    // Different invocations do not contaminate each other's history.
    #[test]
    fn invocations_are_isolated() {
        let dir = temp_root();
        let fa: Vec<String> = vec!["a_test".into()];
        let fb: Vec<String> = vec!["b_test".into()];
        record_at(dir.path(), "tree1", "cargo test", &fa, false, 100);
        record_at(dir.path(), "tree2", "cargo test", &fb, true, 200);
        // b_test's pass belongs to a different selection; a_test has one
        // failure in one tree => below every threshold => omitted entirely.
        let prior_a = summarize_prior(dir.path(), "treeX", "cargo test", &fa, 300);
        assert!(prior_a.is_none());
        // And the same-tree lookup does not cross filters either: the full
        // invocation has no records at all, so nothing is surfaced.
        let prior_full = summarize_prior(dir.path(), "tree1", "cargo test", &[], 300);
        assert!(
            prior_full.is_none(),
            "filter mismatch must not match same-tree"
        );
    }

    // Summary output stays bounded regardless of history size.
    #[test]
    fn prior_evidence_summary_is_bounded() {
        let dir = temp_root();
        let f: Vec<String> = vec![];
        for i in 0..30 {
            let mut d = Vec::new();
            for j in 0..MAX_FAILED_TESTS {
                d.push(diag(
                    None,
                    None,
                    Some(&format!("wide_test_{i}_{j}")),
                    "boom",
                ));
                d.push(diag(None, None, Some(&format!("var_test_{i}_{j}")), "boom"));
            }
            // Alternate outcomes so half the tests trend variance-shaped.
            let tree = format!("tree{i}");
            let inp = input(&tree, "cargo test", &f, i % 5 == 0, &d);
            record(dir.path(), &inp, i).unwrap();
        }
        let prior = summarize_prior(dir.path(), "treeNow", "cargo test", &f, 9999).unwrap();
        assert!(prior.repeated_failures.len() <= 5);
        assert!(prior.historical_variance.len() <= 5);
        let json = serde_json::to_string(&prior).unwrap();
        assert!(json.len() < 4096, "{} bytes", json.len());
    }

    // Command truncation keeps pathological commands bounded.
    #[test]
    fn oversized_command_is_truncated() {
        let dir = temp_root();
        let huge = format!("cargo {}", "x".repeat(5000));
        record_at(dir.path(), "t", &huge, &[], true, 9);
        let file = load(dir.path());
        assert!(file.records[0].command.len() <= MAX_COMMAND_LEN);
    }

    // Old-style records with related_fact_ids load safely (serde ignore_unknown).
    #[test]
    fn old_records_with_related_fact_ids_load_safely() {
        let dir = temp_root();
        std::fs::create_dir_all(dir.path().join(".codebro")).unwrap();
        std::fs::write(
            journal_path(dir.path()),
            br#"{"schema_version":"1.0.0","records":[{"schema_version":"1.0.0","execution_id":"e1","recorded_at_unix":100,"tree_hash":"treeA","command":"cargo test","test_filter":[],"exit_code":0,"classification":"success","success":true,"timed_out":false,"duration_ms":100,"failed_tests":[],"diagnostic_summary":[],"affected_modules":[],"related_fact_ids":["fact1","fact2"]}]}"#,
        ).unwrap();
        // Should load without error; related_fact_ids is ignored.
        let file = load(dir.path());
        assert_eq!(file.records.len(), 1);
        assert_eq!(file.records[0].execution_id, "e1");
        assert_eq!(file.records[0].tree_hash, "treeA");
        // New writes must not contain related_fact_ids.
        record_at(dir.path(), "treeB", "cargo test", &[], true, 200);
        let bytes = std::fs::read_to_string(journal_path(dir.path())).unwrap();
        assert!(!bytes.contains("related_fact_ids"));
    }

    // ── Read-only status() ──────────────────────────────────────────

    #[test]
    fn status_absent_reports_no_journal() {
        let dir = temp_root();
        let st = status(dir.path(), 1000);
        assert!(!st.exists);
        assert!(st.valid);
        assert_eq!(st.records, 0);
        assert_eq!(st.bytes, 0);
        assert_eq!(st.newest_age_secs, None);
    }

    #[test]
    fn status_present_reports_counts_and_age() {
        let dir = temp_root();
        record_at(dir.path(), "treeA", "cargo test", &[], true, 100);
        record_at(dir.path(), "treeB", "cargo test", &[], false, 200);
        let st = status(dir.path(), 300);
        assert!(st.exists);
        assert!(st.valid);
        assert_eq!(st.records, 2);
        assert!(st.bytes > 0);
        assert_eq!(st.newest_age_secs, Some(100));
    }

    #[test]
    fn status_corrupt_reports_invalid_without_quarantine_or_mutation() {
        let dir = temp_root();
        std::fs::create_dir_all(dir.path().join(".codebro")).unwrap();
        std::fs::write(journal_path(dir.path()), b"{not valid json!!").unwrap();
        let st = status(dir.path(), 999);
        assert!(st.exists);
        assert!(!st.valid);
        assert_eq!(st.records, 0);
        // status() must not quarantine: no .corrupt-* sidecar, original intact.
        let entries: Vec<String> = std::fs::read_dir(dir.path().join(".codebro"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            !entries.iter().any(|n| n.contains(".corrupt-")),
            "{entries:?}"
        );
        assert_eq!(
            std::fs::read(journal_path(dir.path())).unwrap(),
            b"{not valid json!!"
        );
    }

    #[test]
    fn status_wrong_schema_reports_invalid() {
        let dir = temp_root();
        std::fs::create_dir_all(dir.path().join(".codebro")).unwrap();
        std::fs::write(
            journal_path(dir.path()),
            br#"{"schema_version":"9.9.9","records":[]}"#,
        )
        .unwrap();
        let st = status(dir.path(), 1);
        assert!(st.exists);
        assert!(!st.valid);
    }

    // ── Current-tree execution state assessment ─────────────────────────

    fn timeout_input<'a>(tree: &'a str, cmd: &'a str) -> JournalInput<'a> {
        JournalInput {
            execution_id: "exec-timeout",
            project_id: None,
            tree_hash: tree,
            command: cmd,
            test_filter: &[],
            exit_code: -1,
            classification: "timeout",
            success: false,
            timed_out: true,
            duration_ms: 120_000,
            diagnostics: &[],
            affected_modules: &[],
        }
    }

    #[test]
    fn assessment_without_records_is_unverified_not_verified() {
        let dir = temp_root();
        let a = assess_for_tree(dir.path(), "treeT", 100);
        assert_eq!(a.state, ExecutionStateKind::Unverified);
        assert!(!a.blocks_completion());
        assert!(a.last_success.is_none());
        assert_eq!(a.unresolved_failures_total, 0);
    }

    #[test]
    fn assessment_passing_run_is_verified() {
        let dir = temp_root();
        record_at(dir.path(), "treeT", "cargo test", &[], true, 100);
        let a = assess_for_tree(dir.path(), "treeT", 160);
        assert_eq!(a.state, ExecutionStateKind::Verified);
        assert!(!a.blocks_completion());
        let last = a.last_success.expect("success recorded");
        assert_eq!(last.command, "cargo test");
        assert_eq!(last.age_seconds, 60);
    }

    #[test]
    fn assessment_unresolved_failure_is_failed_and_blocks() {
        let dir = temp_root();
        record_at(dir.path(), "treeT", "cargo test", &[], false, 100);
        let a = assess_for_tree(dir.path(), "treeT", 200);
        assert_eq!(a.state, ExecutionStateKind::Failed);
        assert!(a.blocks_completion());
        assert_eq!(a.unresolved_failures_total, 1);
        let f = &a.unresolved_failures[0];
        assert_eq!(f.classification, "test_failure");
        assert_eq!(f.exit_code, 1);
        assert_eq!(f.failed_tests, vec!["adds".to_string()]);
        assert!(a.failure_summary().contains("cargo test"));
        assert!(a.note.contains("unresolved"));
    }

    #[test]
    fn assessment_same_invocation_success_resolves_failure() {
        let dir = temp_root();
        let f: Vec<String> = vec!["adds".into()];
        record_at(dir.path(), "treeT", "cargo test --lib", &f, false, 100);
        record_at(dir.path(), "treeT", "cargo test --lib", &f, true, 200);
        let a = assess_for_tree(dir.path(), "treeT", 300);
        assert_eq!(a.state, ExecutionStateKind::Verified);
        assert!(!a.blocks_completion());
        assert_eq!(a.unresolved_failures_total, 0);
    }

    #[test]
    fn assessment_different_command_success_does_not_resolve_failure() {
        let dir = temp_root();
        record_at(dir.path(), "treeT", "cargo test", &[], false, 100);
        record_at(dir.path(), "treeT", "cargo clippy", &[], true, 200);
        let a = assess_for_tree(dir.path(), "treeT", 300);
        assert_eq!(
            a.state,
            ExecutionStateKind::Failed,
            "a passing different invocation must not resolve the failure"
        );
        assert_eq!(a.unresolved_failures_total, 1);
    }

    #[test]
    fn assessment_full_run_success_resolves_filtered_failure() {
        // Realistic auto-resolved cargo shape: the filter is embedded in
        // the recorded command ("cargo test adds"), so a later full run
        // ("cargo test", empty filter) must still resolve it.
        let cargo = temp_root();
        let f: Vec<String> = vec!["adds".into()];
        record_at(cargo.path(), "treeT", "cargo test adds", &f, false, 100);
        record_at(cargo.path(), "treeT", "cargo test", &[], true, 200);
        let a = assess_for_tree(cargo.path(), "treeT", 300);
        assert_eq!(
            a.state,
            ExecutionStateKind::Verified,
            "a later full cargo run exercises the filtered tests"
        );

        // Go inserts the filter before the package args.
        let go = temp_root();
        let gf: Vec<String> = vec!["TestFoo".into()];
        record_at(
            go.path(),
            "treeG",
            "go test -run ^TestFoo$ ./...",
            &gf,
            false,
            100,
        );
        record_at(go.path(), "treeG", "go test ./...", &[], true, 200);
        let a = assess_for_tree(go.path(), "treeG", 300);
        assert_eq!(a.state, ExecutionStateKind::Verified, "go full run covers");

        // Pytest appends `-k <names>`.
        let py = temp_root();
        let pf: Vec<String> = vec!["adds".into(), "subs".into()];
        record_at(
            py.path(),
            "treeP",
            "python -m pytest -q --tb=long -k adds or subs",
            &pf,
            false,
            100,
        );
        record_at(
            py.path(),
            "treeP",
            "python -m pytest -q --tb=long",
            &[],
            true,
            200,
        );
        let a = assess_for_tree(py.path(), "treeP", 300);
        assert_eq!(
            a.state,
            ExecutionStateKind::Verified,
            "pytest full run covers"
        );

        // A passing DIFFERENT selection does not cover: still failed.
        let partial = temp_root();
        record_at(partial.path(), "treeX", "cargo test adds", &f, false, 100);
        let sf: Vec<String> = vec!["subs".into()];
        record_at(partial.path(), "treeX", "cargo test subs", &sf, true, 200);
        let a = assess_for_tree(partial.path(), "treeX", 300);
        assert_eq!(
            a.state,
            ExecutionStateKind::Failed,
            "a different filtered selection must not resolve the failure"
        );

        // A different runner's full pass does not cover: still failed.
        let other_runner = temp_root();
        record_at(
            other_runner.path(),
            "treeY",
            "cargo test adds",
            &f,
            false,
            100,
        );
        record_at(
            other_runner.path(),
            "treeY",
            "go test ./...",
            &[],
            true,
            200,
        );
        let a = assess_for_tree(other_runner.path(), "treeY", 300);
        assert_eq!(
            a.state,
            ExecutionStateKind::Failed,
            "a different runner must not resolve the failure"
        );
    }

    /// Red-team regression: a later full run must NOT be treated as
    /// coverage when the failing command carried unexplained scope such as
    /// an explicit package selector. A root `cargo test` does not prove a
    /// package outside the default workspace members was exercised, so
    /// the failure stays unresolved.
    #[test]
    fn assessment_full_run_does_not_cover_unexplained_scope() {
        let dir = temp_root();
        let f: Vec<String> = vec!["adds".into()];
        record_at(
            dir.path(),
            "treeT",
            "cargo test -p crateA adds",
            &f,
            false,
            100,
        );
        record_at(dir.path(), "treeT", "cargo test", &[], true, 200);
        let a = assess_for_tree(dir.path(), "treeT", 300);
        assert_eq!(
            a.state,
            ExecutionStateKind::Failed,
            "an unexplained package scope must not be covered by a root full run"
        );
        assert_eq!(a.unresolved_failures_total, 1);
    }

    /// Tightened coverage still resolves when the full command repeats the
    /// selector syntax exactly (the scope is visible on both sides).
    #[test]
    fn assessment_full_run_covers_matching_explicit_scope() {
        let dir = temp_root();
        let f: Vec<String> = vec!["adds".into()];
        record_at(dir.path(), "treeT", "cargo test --lib adds", &f, false, 100);
        record_at(dir.path(), "treeT", "cargo test --lib", &[], true, 200);
        let a = assess_for_tree(dir.path(), "treeT", 300);
        assert_eq!(a.state, ExecutionStateKind::Verified);
    }

    #[test]
    fn assessment_other_tree_failure_does_not_apply() {
        let dir = temp_root();
        record_at(dir.path(), "treeOld", "cargo test", &[], false, 100);
        let a = assess_for_tree(dir.path(), "treeNew", 200);
        assert_eq!(a.state, ExecutionStateKind::Unverified);
        assert!(!a.blocks_completion());
        assert_eq!(a.unresolved_failures_total, 0);
        assert_eq!(a.evidence_records, 0);
    }

    #[test]
    fn assessment_timeout_is_inconclusive_and_not_blocking() {
        let dir = temp_root();
        let inp = timeout_input("treeT", "cargo test");
        record(dir.path(), &inp, 100).unwrap();
        let a = assess_for_tree(dir.path(), "treeT", 200);
        assert_eq!(a.state, ExecutionStateKind::Unverified);
        assert!(!a.blocks_completion());
        assert_eq!(a.unresolved_failures_total, 0);
        assert_eq!(a.inconclusive_failures_total, 1);
        assert!(a.note.contains("inconclusive"));
    }

    #[test]
    fn assessment_later_success_resolves_timeout_too() {
        let dir = temp_root();
        let inp = timeout_input("treeT", "cargo test");
        record(dir.path(), &inp, 100).unwrap();
        record_at(dir.path(), "treeT", "cargo test", &[], true, 200);
        let a = assess_for_tree(dir.path(), "treeT", 300);
        assert_eq!(a.state, ExecutionStateKind::Verified);
        assert_eq!(a.inconclusive_failures_total, 0);
    }

    #[test]
    fn assessment_shown_failures_are_bounded_but_total_is_honest() {
        let dir = temp_root();
        for i in 0..6 {
            record_at(
                dir.path(),
                "treeT",
                &format!("cargo test -p p{i}"),
                &[],
                false,
                100 + i,
            );
        }
        let a = assess_for_tree(dir.path(), "treeT", 500);
        assert_eq!(a.state, ExecutionStateKind::Failed);
        assert_eq!(a.unresolved_failures.len(), MAX_ASSESSED_FAILURES);
        assert_eq!(a.unresolved_failures_total, 6);
    }

    #[test]
    fn assessment_non_git_workspace_is_unknown() {
        let dir = temp_root();
        let a = assess(dir.path(), 100);
        assert_eq!(a.state, ExecutionStateKind::Unknown);
        assert!(a.tree_hash.is_none());
        assert!(!a.blocks_completion());
    }
}
