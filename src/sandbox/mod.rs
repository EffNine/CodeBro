#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Sandbox execution abstraction for CodeBro.
//!
//! The sandbox module provides a trait-based execution backend that the MCP
//! server uses to run commands in isolated environments. Two backends exist:
//!
//! - **Local** — runs commands directly in the workspace via PTY, bounded by
//!   timeout, output caps, and the command policy (same authority as the
//!   Testing subagent).
//! - **OpenSandbox** — forwards requests to a remote OpenSandbox HTTP API.
//!   This backend is disabled until an `OPEN_SANDBOX_URL` is configured.
//!
//! The MCP `sandbox_exec` tool routes through this abstraction so that
//! OpenSandbox-specific details never leak into the MCP layer.

pub mod local;
pub mod opensandbox;

pub use local::LocalSandboxBackend;
pub use opensandbox::OpenSandboxBackend;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Digest};
use std::collections::HashMap;
use std::hash::Hasher;
use std::path::PathBuf;

/// The operational mode for sandbox execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxMode {
    #[default]
    Local,
    OpenSandbox,
}

impl std::fmt::Display for SandboxMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxMode::Local => write!(f, "local"),
            SandboxMode::OpenSandbox => write!(f, "opensandbox"),
        }
    }
}

/// A policy that governs what commands may run in the sandbox.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SandboxPolicy {
    /// Maximum wall-clock seconds per command.
    pub timeout_secs: u64,
    /// Maximum bytes of command output retained.
    pub max_output_bytes: usize,
    /// Whether network access is allowed (local backend only; ignored by
    /// OpenSandbox which manages its own network policy).
    pub allow_network: bool,
    /// Whether the command is allowed to write files outside the workspace.
    pub allow_writes: bool,
    /// Extra environment variables injected into the execution environment.
    pub env: HashMap<String, String>,
}

impl SandboxPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    pub fn with_max_output(mut self, bytes: usize) -> Self {
        self.max_output_bytes = bytes;
        self
    }

    pub fn with_network(mut self, allowed: bool) -> Self {
        self.allow_network = allowed;
        self
    }

    pub fn with_writes(mut self, allowed: bool) -> Self {
        self.allow_writes = allowed;
        self
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }
}

/// The command to execute inside the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxCommand {
    /// The command string to run (e.g. `"cargo test --lib"`).
    pub command: String,
    /// Working directory relative to the workspace root.
    #[serde(default)]
    pub working_directory: Option<String>,
    /// Override the default policy for this execution.
    #[serde(default)]
    pub policy: Option<SandboxPolicy>,
    /// Arbitrary metadata attached to the request (not executed, returned
    /// verbatim in the result for agent correlation).
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// The authoritative structured result of a sandbox execution.
///
/// This is the machine-fact record: exit code, duration, output, denial
/// reason — never model prose. The model interprets the result; it cannot
/// override the exit code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// The command that was requested.
    pub command: String,
    /// The original requested command (may differ from resolved_command when
    /// the MCP layer auto-resolves a semantic operation to an explicit command).
    #[serde(default)]
    pub requested_command: String,
    /// The exact command string that was executed.
    #[serde(default)]
    pub resolved_command: String,
    /// The working directory the command ran in.
    pub working_directory: String,
    /// The authoritative process exit code. `-1` means no process ran
    /// (denied, timeout, or spawn failure).
    pub exit_code: i32,
    /// `exit_code == 0 && !timeout && !cancelled && !denied`.
    pub success: bool,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u128,
    /// ISO 8601 timestamp when execution started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// Captured stdout (redacted, truncated).
    pub stdout: String,
    /// Captured stderr (redacted, truncated).
    pub stderr: String,
    /// Whether the command exceeded the per-command timeout.
    pub timeout: bool,
    /// Whether the command was terminated by cancellation.
    pub cancelled: bool,
    /// Whether the command was rejected by policy (never ran).
    pub denied: bool,
    /// The policy denial reason, when `denied`.
    pub denied_reason: Option<String>,
    /// Sandbox backend that produced this result.
    pub backend: String,
    /// Mode the sandbox ran in.
    pub mode: String,
    /// Unique execution identifier (UUID v4).
    #[serde(default)]
    pub execution_id: String,
    /// Repository identity at execution time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_identity: Option<RepoIdentity>,
    /// Repository state at execution time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_state: Option<RepoState>,
    /// Sandbox capabilities active for this execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_capabilities: Option<SandboxCapabilities>,
    /// Reproducibility classification.
    #[serde(default)]
    pub reproducibility: Reproducibility,
    /// Artifacts explicitly associated with this execution.
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    /// Freshness relative to current repository state (computed on read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness: Option<Freshness>,
    /// Arbitrary metadata echoed back from the request.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl ExecutionResult {
    /// Constructed by the sandbox runtime after execution completes.
    pub fn from_local(
        command: &str,
        working_directory: &str,
        stdout: &str,
        stderr: &str,
        exit_code: i32,
        duration_ms: u128,
        timeout: bool,
        cancelled: bool,
        metadata: HashMap<String, String>,
    ) -> Self {
        let success = exit_code == 0 && !timeout && !cancelled;
        ExecutionResult {
            command: command.to_string(),
            requested_command: command.to_string(),
            resolved_command: command.to_string(),
            working_directory: working_directory.to_string(),
            exit_code,
            success,
            duration_ms,
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            timeout,
            cancelled,
            denied: false,
            denied_reason: None,
            backend: "local".to_string(),
            mode: SandboxMode::Local.to_string(),
            execution_id: uuid::Uuid::new_v4().to_string(),
            repo_identity: None,
            repo_state: None,
            sandbox_capabilities: None,
            reproducibility: Reproducibility::default(),
            artifacts: Vec::new(),
            freshness: None,
            metadata,
        }
    }

    /// A record for a command denied before any process ran.
    pub fn denied(
        command: &str,
        working_directory: &str,
        reason: &str,
        metadata: HashMap<String, String>,
    ) -> Self {
        ExecutionResult {
            command: command.to_string(),
            requested_command: command.to_string(),
            resolved_command: command.to_string(),
            working_directory: working_directory.to_string(),
            exit_code: -1,
            success: false,
            duration_ms: 0,
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
            stdout: String::new(),
            stderr: String::new(),
            timeout: false,
            cancelled: false,
            denied: true,
            denied_reason: Some(reason.to_string()),
            backend: "local".to_string(),
            mode: SandboxMode::Local.to_string(),
            execution_id: uuid::Uuid::new_v4().to_string(),
            repo_identity: None,
            repo_state: None,
            sandbox_capabilities: None,
            reproducibility: Reproducibility::default(),
            artifacts: Vec::new(),
            freshness: None,
            metadata,
        }
    }

    /// Whether this result represents a successful execution.
    pub fn is_success(&self) -> bool {
        self.success
    }

    /// Compact one-line summary for observability.
    pub fn summary_line(&self) -> String {
        format!(
            "[sandbox_exec] cmd={} exit={} success={} timeout={} denied={} duration={}ms backend={}",
            truncate_cmd(&self.command, 60),
            self.exit_code,
            self.success,
            self.timeout,
            self.denied,
            self.duration_ms,
            self.backend,
        )
    }
}

/// Expected-result contract for semantic sandbox operations.
///
/// Callers express expectations (exit code, success flag); the system
/// records whether the actual execution satisfied them. The underlying
/// `ExecutionResult` is always preserved so agents can inspect the raw
/// evidence regardless of whether the contract passed or failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    /// The underlying execution evidence — always present.
    pub execution: ExecutionResult,
    /// Whether the execution satisfied all declared expectations.
    pub verified: bool,
    /// Human-readable verification summary.
    pub summary: String,
    /// List of expectation violations, empty when `verified == true`.
    #[serde(default)]
    pub violations: Vec<String>,
    /// Optional caller-supplied fact IDs associated with the change being
    /// verified. These IDs provide correlation context only and are not
    /// themselves verification evidence. None means the caller did not
    /// supply correlation context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impacted_fact_ids: Option<Vec<String>>,
}

impl VerificationResult {
    /// Build a verification result with no explicit expectations.
    ///
    /// All expectations pass by default; `verified` is true iff the
    /// execution itself was successful (success == true).
    pub fn from_execution(execution: ExecutionResult) -> Self {
        let verified = execution.success;
        let summary = if verified {
            format!(
                "Execution succeeded: exit={} duration={}ms backend={}",
                execution.exit_code, execution.duration_ms, execution.backend
            )
        } else if execution.denied {
            format!(
                "Execution denied: {}",
                execution.denied_reason.as_deref().unwrap_or("policy violation")
            )
        } else if execution.timeout {
            format!(
                "Execution timed out after {}ms: exit={}",
                execution.duration_ms, execution.exit_code
            )
        } else {
            format!(
                "Execution failed: exit={} duration={}ms backend={}",
                execution.exit_code, execution.duration_ms, execution.backend
            )
        };
        let violations = if verified {
            Vec::new()
        } else {
            let mut v = Vec::new();
            if !execution.success {
                v.push(format!("exit_code={} (expected 0)", execution.exit_code));
            }
            if execution.timeout {
                v.push("timeout exceeded".to_string());
            }
            if execution.denied {
                if let Some(ref reason) = execution.denied_reason {
                    v.push(format!("denied: {reason}"));
                }
            }
            v
        };
        VerificationResult {
            execution,
            verified,
            summary,
            violations,
            impacted_fact_ids: None,
        }
    }

    /// Build a verification result with explicit expectations.
    ///
    /// `expected_exit_code` and `expected_success` are optional; omitting
    /// one means that expectation is not checked.
    pub fn from_execution_with_expectations(
        execution: ExecutionResult,
        expected_exit_code: Option<i32>,
        expected_success: Option<bool>,
    ) -> Self {
        let mut violations = Vec::new();

        if let Some(explicit_success) = expected_success {
            if execution.success != explicit_success {
                violations.push(format!(
                    "expected success={explicit_success}, got success={}",
                    execution.success
                ));
            }
        }

        if let Some(explicit_exit) = expected_exit_code {
            if execution.exit_code != explicit_exit {
                violations.push(format!(
                    "expected exit_code={explicit_exit}, got {}",
                    execution.exit_code
                ));
            }
        }

        // When no explicit success expectation is set but exit_code != 0,
        // treat non-zero as a failure violation (common default).
        if expected_success.is_none() && expected_exit_code.is_none() {
            if !execution.success {
                violations.push(format!(
                    "non-zero exit: {}",
                    execution.exit_code
                ));
            }
        }

        let verified = violations.is_empty();
        let summary = if verified {
            format!(
                "Verification passed: exit={} duration={}ms backend={}",
                execution.exit_code, execution.duration_ms, execution.backend
            )
        } else {
            let details = violations.join("; ");
            format!(
                "Verification failed ({details}): exit={} duration={}ms backend={}",
                execution.exit_code, execution.duration_ms, execution.backend
            )
        };

        VerificationResult {
            execution,
            verified,
            summary,
            violations,
            impacted_fact_ids: None,
        }
    }

    /// Build a verification result with explicit expectations and optional
    /// correlation metadata.
    ///
    /// `impacted_fact_ids` are caller-supplied fact IDs associated with the
    /// change being verified. They are correlation context only — not
    /// independently verified evidence.
    pub fn from_execution_with_impacted_fact_ids(
        execution: ExecutionResult,
        expected_exit_code: Option<i32>,
        expected_success: Option<bool>,
        impacted_fact_ids: Option<Vec<String>>,
    ) -> Self {
        let mut violations = Vec::new();

        if let Some(explicit_success) = expected_success {
            if execution.success != explicit_success {
                violations.push(format!(
                    "expected success={explicit_success}, got success={}",
                    execution.success
                ));
            }
        }

        if let Some(explicit_exit) = expected_exit_code {
            if execution.exit_code != explicit_exit {
                violations.push(format!(
                    "expected exit_code={explicit_exit}, got {}",
                    execution.exit_code
                ));
            }
        }

        // When no explicit success expectation is set but exit_code != 0,
        // treat non-zero as a failure violation (common default).
        if expected_success.is_none() && expected_exit_code.is_none() {
            if !execution.success {
                violations.push(format!(
                    "non-zero exit: {}",
                    execution.exit_code
                ));
            }
        }

        let verified = violations.is_empty();
        let summary = if verified {
            format!(
                "Verification passed: exit={} duration={}ms backend={}",
                execution.exit_code, execution.duration_ms, execution.backend
            )
        } else {
            let details = violations.join("; ");
            format!(
                "Verification failed ({details}): exit={} duration={}ms backend={}",
                execution.exit_code, execution.duration_ms, execution.backend
            )
        };

        VerificationResult {
            execution,
            verified,
            summary,
            violations,
            impacted_fact_ids,
        }
    }
}

fn truncate_cmd(cmd: &str, max: usize) -> String {
    if cmd.len() <= max {
        cmd.to_string()
    } else {
        format!("{}…", &cmd[..max])
    }
}

// ── Evidence provenance types (P1.2) ──────────────────────────────────

/// A unique execution identifier.
pub type ExecutionId = String;

/// Repository identity: what project is being tested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoIdentity {
    /// Deterministic project identifier derived from workspace root + manifest.
    pub project_id: String,
    /// Absolute workspace root path.
    pub root: String,
    /// Detected repository type: "cargo", "go", "npm", "unknown".
    pub repository_type: String,
}

impl RepoIdentity {
    /// Derive from a workspace root. Uses project_identity runtime where available.
    pub fn from_workspace(workspace_root: &PathBuf) -> Self {
        let root = workspace_root.to_string_lossy().to_string();
        let repository_type = if workspace_root.join("Cargo.toml").exists() {
            "cargo".to_string()
        } else if workspace_root.join("go.mod").exists() {
            "go".to_string()
        } else if workspace_root.join("package.json").exists() {
            "npm".to_string()
        } else {
            "unknown".to_string()
        };
        // project_id: hash of root for deterministic short identifier
        let project_id = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(&root, &mut hasher);
            format!("{:x}", hasher.finish())
        };
        RepoIdentity {
            project_id,
            root,
            repository_type,
        }
    }
}

/// Repository state at the time of capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoState {
    /// Current HEAD commit SHA (full or short). Empty if not a git repo.
    pub commit_sha: String,
    /// Whether the working tree has uncommitted changes.
    pub working_tree_dirty: bool,
    /// Deterministic hash of relevant working-tree state.
    /// For a clean repo this is the commit SHA; for dirty it includes diff.
    pub working_tree_hash: String,
}

impl RepoState {
    /// Capture repository state from the workspace root.
    /// Returns None if not a git repository or git is unavailable.
    pub fn capture(workspace_root: &PathBuf) -> Option<Self> {
        let output = std::process::Command::new("git")
            .current_dir(workspace_root)
            .args(["rev-parse", "--verify", "HEAD"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let commit_sha = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Check dirty: uncommitted changes or untracked files.
        let dirty_output = std::process::Command::new("git")
            .current_dir(workspace_root)
            .args(["status", "--porcelain"])
            .output()
            .ok()?;
        let working_tree_dirty = !dirty_output.status.success()
            || !String::from_utf8_lossy(&dirty_output.stdout).trim().is_empty();

        // Compute a deterministic hash of working-tree state.
        // Strategy: hash(sorted tracked files + diff + untracked).
        let working_tree_hash = {
            let mut parts: Vec<Vec<u8>> = Vec::new();

            // Tracked files list (sorted)
            let ls_output = std::process::Command::new("git")
                .current_dir(workspace_root)
                .args(["ls-files"])
                .output()
                .ok();
            if let Some(ref ls) = ls_output {
                if ls.status.success() {
                    parts.push(ls.stdout.clone());
                }
            }

            // Uncommitted changes
            let diff_output = std::process::Command::new("git")
                .current_dir(workspace_root)
                .args(["diff", "HEAD"])
                .output()
                .ok();
            if let Some(ref diff) = diff_output {
                if diff.status.success() {
                    parts.push(diff.stdout.clone());
                }
            }

            // Untracked files
            let untracked_output = std::process::Command::new("git")
                .current_dir(workspace_root)
                .args(["ls-files", "--others", "--exclude-standard"])
                .output()
                .ok();
            if let Some(ref ut) = untracked_output {
                if ut.status.success() {
                    parts.push(ut.stdout.clone());
                }
            }

            // Sort each part for determinism
            for part in &mut parts {
                part.sort();
            }

            // Concatenate and hash
            let mut hasher = sha2::Sha256::new();
            for part in &parts {
                sha2::Digest::update(&mut hasher, part);
            }
            if parts.is_empty() {
                // Fallback: just hash the commit SHA
                sha2::Digest::update(&mut hasher, commit_sha.as_bytes());
            }
            format!("{:x}", hasher.finalize())
        };

        Some(RepoState {
            commit_sha,
            working_tree_dirty,
            working_tree_hash,
        })
    }
}

/// Reproducibility classification for an execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reproducibility {
    /// Deterministic: same inputs always produce same output (e.g. cargo test
    /// on a dependency-free fixture with no parallelism).
    Deterministic,
    /// Likely deterministic: mostly reproducible but may vary under load or
    /// with system state (e.g. cargo test on the full workspace).
    LikelyDeterministic,
    /// Non-deterministic: output intentionally varies (tests with randomness,
    /// network-dependent commands, wall-clock timers).
    NonDeterministic,
    /// Unknown: cannot classify from available information.
    Unknown,
}

impl Default for Reproducibility {
    fn default() -> Self {
        Reproducibility::Unknown
    }
}

impl std::fmt::Display for Reproducibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reproducibility::Deterministic => write!(f, "deterministic"),
            Reproducibility::LikelyDeterministic => write!(f, "likely_deterministic"),
            Reproducibility::NonDeterministic => write!(f, "non_deterministic"),
            Reproducibility::Unknown => write!(f, "unknown"),
        }
    }
}

/// Freshness of evidence relative to current repository state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    /// Evidence was produced from the current repository state.
    Fresh,
    /// Repository state has changed since this evidence was produced.
    Stale,
    /// Cannot determine current repository state.
    Unknown,
}

impl std::fmt::Display for Freshness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Freshness::Fresh => write!(f, "fresh"),
            Freshness::Stale => write!(f, "stale"),
            Freshness::Unknown => write!(f, "unknown"),
        }
    }
}

/// A named artifact produced by an execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// Path to the artifact, relative to workspace root.
    pub path: String,
    /// Kind of artifact: "log", "binary", "report", "coverage", "other".
    pub kind: String,
    /// File size in bytes, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// SHA256 hash of the artifact content, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// Isolation level provided by the sandbox backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationLevel {
    /// No isolation — runs in the host process / shell.
    None,
    /// Process-level isolation (process group, cgroup, namespace).
    Process,
    /// Container-level isolation.
    Container,
    /// Remote service isolation.
    Remote,
    /// Not yet determined.
    Unknown,
}

impl std::fmt::Display for IsolationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IsolationLevel::None => write!(f, "none"),
            IsolationLevel::Process => write!(f, "process"),
            IsolationLevel::Container => write!(f, "container"),
            IsolationLevel::Remote => write!(f, "remote"),
            IsolationLevel::Unknown => write!(f, "unknown"),
        }
    }
}

/// Filesystem scope enforced by the sandbox backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemScope {
    /// Bounded by CodeBro policy (workspace root + command allowlist).
    PolicyBounded,
    /// Scoped to a sandbox-controlled filesystem root.
    SandboxScoped,
    /// Unrestricted (not recommended).
    Unrestricted,
    /// Not yet determined.
    Unknown,
}

impl std::fmt::Display for FilesystemScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilesystemScope::PolicyBounded => write!(f, "policy_bounded"),
            FilesystemScope::SandboxScoped => write!(f, "sandbox_scoped"),
            FilesystemScope::Unrestricted => write!(f, "unrestricted"),
            FilesystemScope::Unknown => write!(f, "unknown"),
        }
    }
}

/// Network access policy of the sandbox backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAccess {
    /// Full host network access.
    Host,
    /// No network access.
    None,
    /// Controlled / restricted network access.
    Controlled,
    /// Not yet determined.
    Unknown,
}

impl std::fmt::Display for NetworkAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkAccess::Host => write!(f, "host"),
            NetworkAccess::None => write!(f, "none"),
            NetworkAccess::Controlled => write!(f, "controlled"),
            NetworkAccess::Unknown => write!(f, "unknown"),
        }
    }
}

/// Environment control offered by the sandbox backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentControl {
    /// Only a restricted subset of host environment variables.
    Restricted,
    /// Caller-injected environment variables within policy bounds.
    Controlled,
    /// Full host environment passthrough.
    Passthrough,
    /// Not yet determined.
    Unknown,
}

impl std::fmt::Display for EnvironmentControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvironmentControl::Restricted => write!(f, "restricted"),
            EnvironmentControl::Controlled => write!(f, "controlled"),
            EnvironmentControl::Passthrough => write!(f, "passthrough"),
            EnvironmentControl::Unknown => write!(f, "unknown"),
        }
    }
}

/// Formal capability descriptor for a sandbox backend.
///
/// An agent MUST inspect these capabilities before deciding whether a
/// particular execution is safe or appropriate for the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxCapabilities {
    /// Isolation level provided.
    pub isolation: IsolationLevel,
    /// Filesystem scope enforced.
    pub filesystem_scope: FilesystemScope,
    /// Network access policy.
    pub network_access: NetworkAccess,
    /// Whether resource limits (CPU, memory) are enforced.
    pub resource_limits: bool,
    /// Whether timeout enforcement is guaranteed.
    pub timeout_enforcement: bool,
    /// Whether output size is bounded.
    pub output_limits: bool,
    /// Environment variable control level.
    pub environment_control: EnvironmentControl,
}

impl SandboxCapabilities {
    /// Capabilities for the Local backend.
    ///
    /// Local runs commands directly in the workspace with no real isolation.
    /// It is NOT equivalent to a real sandbox.
    pub fn local() -> Self {
        SandboxCapabilities {
            isolation: IsolationLevel::None,
            filesystem_scope: FilesystemScope::PolicyBounded,
            network_access: NetworkAccess::Host,
            resource_limits: false,
            timeout_enforcement: true,
            output_limits: true,
            environment_control: EnvironmentControl::Restricted,
        }
    }

    /// Capabilities for the OpenSandbox backend (declared intent; actual
    /// guarantees depend on the remote service contract).
    pub fn opensandbox() -> Self {
        SandboxCapabilities {
            isolation: IsolationLevel::Remote,
            filesystem_scope: FilesystemScope::SandboxScoped,
            network_access: NetworkAccess::Unknown,
            resource_limits: false,
            timeout_enforcement: true,
            output_limits: true,
            environment_control: EnvironmentControl::Controlled,
        }
    }
}

/// Structured status response for `sandbox_status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxStatusResponse {
    /// Active backend name.
    pub backend: String,
    /// Operational mode.
    pub mode: String,
    /// Whether the backend is available.
    pub available: bool,
    /// Explicit capability descriptor for the active backend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<SandboxCapabilities>,
    /// Whether OpenSandbox was explicitly configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opensandbox_configured: Option<bool>,
}

/// The sandbox execution backend trait.
///
/// Implementations provide the actual execution mechanism. The MCP layer
/// calls `execute()` and receives an `ExecutionResult` — it does not know
/// whether the command ran locally or remotely.
pub trait SandboxBackend: Send + Sync {
    /// Execute a command in the sandbox and return the structured result.
    fn execute(
        &self,
        workspace_root: &PathBuf,
        cmd: SandboxCommand,
        policy: &SandboxPolicy,
    ) -> ExecutionResult;

    /// Return the backend name for observability.
    fn name(&self) -> &str {
        "unknown"
    }

    /// Return the backend mode.
    fn mode(&self) -> SandboxMode {
        SandboxMode::Local
    }

    /// Health check: whether the backend is available.
    fn is_available(&self) -> bool {
        true
    }

    /// Return the formal capability descriptor for this backend.
    /// Agents should inspect this BEFORE executing to understand guarantees.
    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities::local()
    }
}

/// The sandbox runtime: selects the appropriate backend based on
/// configuration and dispatches execution requests.
#[derive(Clone)]
pub struct SandboxRuntime {
    mode: SandboxMode,
    local: LocalSandboxBackend,
    opensandbox: Option<OpenSandboxBackend>,
    /// Whether OpenSandbox was explicitly configured (vs. auto-fallback).
    opensandbox_explicit: bool,
}

impl SandboxRuntime {
    /// Build the sandbox runtime from environment configuration.
    ///
    /// If `OPEN_SANDBOX_URL` is set, the OpenSandbox backend is initialized
    /// and used when `mode` is `OpenSandbox`. Otherwise, the local backend
    /// is used.
    pub fn from_env() -> Self {
        let url = std::env::var("OPEN_SANDBOX_URL").ok();
        let opensandbox_explicit = url.is_some();
        let mode = if url.is_some() {
            SandboxMode::OpenSandbox
        } else {
            SandboxMode::Local
        };
        let opensandbox = url.map(|u| OpenSandboxBackend::new(u));
        SandboxRuntime {
            mode,
            local: LocalSandboxBackend::new(),
            opensandbox,
            opensandbox_explicit,
        }
    }

    /// Build with an explicit mode (for tests).
    pub fn new(mode: SandboxMode) -> Self {
        SandboxRuntime {
            mode,
            local: LocalSandboxBackend::new(),
            opensandbox: None,
            opensandbox_explicit: false,
        }
    }

    /// Build with an explicit OpenSandbox URL (for tests that need it).
    pub fn with_opensandbox(url: String) -> Self {
        SandboxRuntime {
            mode: SandboxMode::OpenSandbox,
            local: LocalSandboxBackend::new(),
            opensandbox: Some(OpenSandboxBackend::new(url)),
            opensandbox_explicit: true,
        }
    }

    /// Execute a command through the configured backend.
    ///
    /// If OpenSandbox is explicitly configured but unavailable, this fails
    /// closed: returns a denied result instead of silently falling back to
    /// local. Silent fallback is a security-boundary downgrade.
    pub fn execute(
        &self,
        workspace_root: &PathBuf,
        cmd: SandboxCommand,
        policy: &SandboxPolicy,
    ) -> ExecutionResult {
        match self.mode {
            SandboxMode::Local => {
                let mut result = self.local.execute(workspace_root, cmd, policy);
                self.enrich_with_provenance(workspace_root, &mut result);
                result
            }
            SandboxMode::OpenSandbox => {
                if let Some(ref backend) = self.opensandbox {
                    if !backend.is_available() {
                        // Fail closed: OpenSandbox explicitly configured but
                        // unreachable. Do NOT fall back to local.
                        return ExecutionResult {
                            command: cmd.command.clone(),
                            requested_command: cmd.command.clone(),
                            resolved_command: cmd.command.clone(),
                            working_directory: workspace_root.to_string_lossy().to_string(),
                            exit_code: -1,
                            success: false,
                            duration_ms: 0,
                            timestamp: Some(chrono::Utc::now().to_rfc3339()),
                            stdout: String::new(),
                            stderr: "OpenSandbox configured but service unavailable".to_string(),
                            timeout: false,
                            cancelled: false,
                            denied: true,
                            denied_reason: Some(
                                "OpenSandbox backend unavailable; not falling back to local".to_string(),
                            ),
                            backend: "opensandbox".to_string(),
                            mode: SandboxMode::OpenSandbox.to_string(),
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            repo_identity: None,
                            repo_state: None,
                            sandbox_capabilities: Some(SandboxCapabilities::opensandbox()),
                            reproducibility: Reproducibility::default(),
                            artifacts: Vec::new(),
                            freshness: None,
                            metadata: cmd.metadata,
                        };
                    }
                    let mut result = backend.execute(workspace_root, cmd, policy);
                    self.enrich_with_provenance(workspace_root, &mut result);
                    result
                } else {
                    // OpenSandbox mode requested but no backend configured.
                    // This should not happen when built via from_env(), but
                    // if it does, fail closed rather than silently falling back.
                    tracing::warn!(
                        "OpenSandbox mode requested but no backend configured; failing closed"
                    );
                    ExecutionResult {
                        command: cmd.command.clone(),
                        requested_command: cmd.command.clone(),
                        resolved_command: cmd.command.clone(),
                        working_directory: workspace_root.to_string_lossy().to_string(),
                        exit_code: -1,
                        success: false,
                        duration_ms: 0,
                        timestamp: Some(chrono::Utc::now().to_rfc3339()),
                        stdout: String::new(),
                        stderr: "OpenSandbox mode requested but backend not configured".to_string(),
                        timeout: false,
                        cancelled: false,
                        denied: true,
                        denied_reason: Some(
                            "OpenSandbox backend not configured".to_string(),
                        ),
                        backend: "opensandbox".to_string(),
                        mode: SandboxMode::OpenSandbox.to_string(),
                        execution_id: uuid::Uuid::new_v4().to_string(),
                        repo_identity: None,
                        repo_state: None,
                        sandbox_capabilities: Some(SandboxCapabilities::opensandbox()),
                        reproducibility: Reproducibility::default(),
                        artifacts: Vec::new(),
                        freshness: None,
                        metadata: cmd.metadata,
                    }
                }
            }
        }
    }

    /// Enrich an execution result with provenance metadata (repo identity,
    /// repo state, capabilities, execution ID, timestamp).
    fn enrich_with_provenance(
        &self,
        workspace_root: &PathBuf,
        result: &mut ExecutionResult,
    ) {
        result.execution_id = uuid::Uuid::new_v4().to_string();
        result.timestamp = Some(chrono::Utc::now().to_rfc3339());
        result.requested_command = result.command.clone();
        result.resolved_command = result.command.clone();
        result.repo_identity = Some(RepoIdentity::from_workspace(workspace_root));
        result.repo_state = RepoState::capture(workspace_root);
        result.sandbox_capabilities = Some(self.capabilities());
    }

    /// Return the currently active backend name.
    pub fn active_backend(&self) -> &str {
        match self.mode {
            SandboxMode::Local => "local",
            SandboxMode::OpenSandbox => self
                .opensandbox
                .as_ref()
                .map(|b| b.name())
                .unwrap_or("opensandbox (unavailable)"),
        }
    }

    /// Whether the configured backend is available.
    pub fn is_available(&self) -> bool {
        match self.mode {
            SandboxMode::Local => true,
            SandboxMode::OpenSandbox => self
                .opensandbox
                .as_ref()
                .map(|b| b.is_available())
                .unwrap_or(false),
        }
    }

    /// Return structured status including capabilities.
    pub fn status(&self) -> SandboxStatusResponse {
        SandboxStatusResponse {
            backend: self.active_backend().to_string(),
            mode: self.mode.to_string(),
            available: self.is_available(),
            capabilities: Some(self.capabilities()),
            opensandbox_configured: if self.opensandbox_explicit {
                Some(true)
            } else {
                None
            },
        }
    }

    /// Return the capabilities of the active backend.
    pub fn capabilities(&self) -> SandboxCapabilities {
        match self.mode {
            SandboxMode::Local => self.local.capabilities(),
            SandboxMode::OpenSandbox => {
                self.opensandbox
                    .as_ref()
                    .map(|b| b.capabilities())
                    .unwrap_or(SandboxCapabilities::opensandbox())
            }
        }
    }

    /// Whether OpenSandbox was explicitly configured.
    pub fn opensandbox_explicit(&self) -> bool {
        self.opensandbox_explicit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_result_success_from_exit_code() {
        let r = ExecutionResult::from_local(
            "true",
            "/workspace",
            "",
            "",
            0,
            5,
            false,
            false,
            HashMap::new(),
        );
        assert!(r.success);
        assert_eq!(r.exit_code, 0);
        assert!(!r.denied);
        assert!(!r.timeout);
    }

    #[test]
    fn test_execution_result_denied() {
        let r = ExecutionResult::denied("rm -rf /", "/workspace", "denied by policy", HashMap::new());
        assert!(!r.success);
        assert_eq!(r.exit_code, -1);
        assert!(r.denied);
        assert_eq!(r.denied_reason, Some("denied by policy".to_string()));
    }

    #[test]
    fn test_sandbox_runtime_default_mode_is_local() {
        let rt = SandboxRuntime::new(SandboxMode::Local);
        assert_eq!(rt.active_backend(), "local");
        assert!(rt.is_available());
    }

    #[test]
    fn test_execution_result_summary_line() {
        let r = ExecutionResult::from_local(
            "cargo test",
            "/workspace",
            "test result: ok",
            "",
            0,
            1200,
            false,
            false,
            HashMap::new(),
        );
        let line = r.summary_line();
        assert!(line.contains("cargo test"));
        assert!(line.contains("exit=0"));
        assert!(line.contains("success=true"));
        assert!(line.contains("backend=local"));
    }

    #[test]
    fn test_verification_result_from_execution_passes() {
        let exec = ExecutionResult::from_local(
            "true",
            "/workspace",
            "",
            "",
            0,
            5,
            false,
            false,
            HashMap::new(),
        );
        let v = VerificationResult::from_execution(exec);
        assert!(v.verified);
        assert!(v.violations.is_empty());
        assert!(v.summary.contains("succeeded"));
    }

    #[test]
    fn test_verification_result_from_execution_fails() {
        let exec = ExecutionResult::from_local(
            "false",
            "/workspace",
            "",
            "",
            1,
            3,
            false,
            false,
            HashMap::new(),
        );
        let v = VerificationResult::from_execution(exec);
        assert!(!v.verified);
        assert!(!v.violations.is_empty());
        assert!(v.summary.contains("failed"));
    }

    #[test]
    fn test_verification_result_with_expectations() {
        let exec = ExecutionResult::from_local(
            "true",
            "/workspace",
            "",
            "",
            0,
            5,
            false,
            false,
            HashMap::new(),
        );
        let v =
            VerificationResult::from_execution_with_expectations(exec, Some(0), Some(true));
        assert!(v.verified);
        assert!(v.violations.is_empty());
    }

    #[test]
    fn test_verification_result_with_wrong_expectations() {
        let exec = ExecutionResult::from_local(
            "false",
            "/workspace",
            "",
            "",
            1,
            3,
            false,
            false,
            HashMap::new(),
        );
        let v =
            VerificationResult::from_execution_with_expectations(exec, Some(0), Some(true));
        assert!(!v.verified);
        assert_eq!(v.violations.len(), 2);
        assert!(v.violations[0].contains("expected success=true"));
        assert!(v.violations[1].contains("expected exit_code=0"));
    }

    #[test]
    fn test_verification_result_denied_command() {
        let exec = ExecutionResult::denied(
            "rm -rf /",
            "/workspace",
            "denied by policy",
            HashMap::new(),
        );
        let v = VerificationResult::from_execution(exec);
        assert!(!v.verified);
        assert!(v.summary.contains("denied"));
    }

    #[test]
    fn test_verification_result_timeout() {
        let exec = ExecutionResult::from_local(
            "sleep 10",
            "/workspace",
            "",
            "",
            -1,
            1000,
            true,
            false,
            HashMap::new(),
        );
        let v = VerificationResult::from_execution(exec);
        assert!(!v.verified);
        assert!(v.summary.contains("timed out"));
    }

    // ── P1.2 evidence provenance tests ──────────────────────────────────

    #[test]
    fn test_execution_result_has_execution_id() {
        let r = ExecutionResult::from_local(
            "true",
            "/workspace",
            "",
            "",
            0,
            5,
            false,
            false,
            HashMap::new(),
        );
        assert!(!r.execution_id.is_empty());
    }

    #[test]
    fn test_execution_result_has_timestamp() {
        let r = ExecutionResult::from_local(
            "true",
            "/workspace",
            "",
            "",
            0,
            5,
            false,
            false,
            HashMap::new(),
        );
        assert!(r.timestamp.is_some());
        let ts = r.timestamp.as_ref().unwrap();
        // Validate ISO 8601 format
        assert!(ts.contains('T') || ts.contains('Z'));
    }

    #[test]
    fn test_execution_result_has_resolved_and_requested_command() {
        let r = ExecutionResult::from_local(
            "cargo test --lib",
            "/workspace",
            "",
            "",
            0,
            100,
            false,
            false,
            HashMap::new(),
        );
        assert_eq!(r.command, "cargo test --lib");
        assert_eq!(r.requested_command, "cargo test --lib");
        assert_eq!(r.resolved_command, "cargo test --lib");
    }

    #[test]
    fn test_reproducibility_display() {
        assert_eq!(format!("{}", Reproducibility::Deterministic), "deterministic");
        assert_eq!(
            format!("{}", Reproducibility::LikelyDeterministic),
            "likely_deterministic"
        );
        assert_eq!(
            format!("{}", Reproducibility::NonDeterministic),
            "non_deterministic"
        );
        assert_eq!(format!("{}", Reproducibility::Unknown), "unknown");
    }

    #[test]
    fn test_freshness_display() {
        assert_eq!(format!("{}", Freshness::Fresh), "fresh");
        assert_eq!(format!("{}", Freshness::Stale), "stale");
        assert_eq!(format!("{}", Freshness::Unknown), "unknown");
    }

    #[test]
    fn test_sandbox_capabilities_local() {
        let caps = SandboxCapabilities::local();
        assert_eq!(caps.isolation, IsolationLevel::None);
        assert_eq!(caps.filesystem_scope, FilesystemScope::PolicyBounded);
        assert_eq!(caps.network_access, NetworkAccess::Host);
        assert!(!caps.resource_limits);
        assert!(caps.timeout_enforcement);
        assert!(caps.output_limits);
        assert_eq!(caps.environment_control, EnvironmentControl::Restricted);
    }

    #[test]
    fn test_sandbox_capabilities_opensandbox() {
        let caps = SandboxCapabilities::opensandbox();
        assert_eq!(caps.isolation, IsolationLevel::Remote);
        assert_eq!(caps.filesystem_scope, FilesystemScope::SandboxScoped);
        assert_eq!(caps.network_access, NetworkAccess::Unknown);
        assert!(caps.timeout_enforcement);
        assert!(caps.output_limits);
    }

    #[test]
    fn test_repo_identity_from_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let identity = RepoIdentity::from_workspace(&dir.path().to_path_buf());
        assert_eq!(identity.repository_type, "cargo");
        assert!(!identity.project_id.is_empty());
        assert!(!identity.root.is_empty());
    }

    #[test]
    fn test_repo_identity_unknown_type() {
        let dir = tempfile::tempdir().unwrap();
        let identity = RepoIdentity::from_workspace(&dir.path().to_path_buf());
        assert_eq!(identity.repository_type, "unknown");
    }

    #[test]
    fn test_repo_state_capture_clean_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        // Initialize a git repo
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["init"])
            .output()
            .ok();
        std::fs::write(dir.path().join("foo.txt"), "hello").unwrap();
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["add", "."])
            .output()
            .ok();
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["commit", "-m", "initial"])
            .output()
            .ok();

        let state = RepoState::capture(&dir.path().to_path_buf());
        assert!(state.is_some());
        let state = state.unwrap();
        assert!(!state.commit_sha.is_empty());
        assert!(!state.working_tree_hash.is_empty());
        assert!(!state.working_tree_dirty);
    }

    #[test]
    fn test_repo_state_capture_dirty_repo() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["init"])
            .output()
            .ok();
        std::fs::write(dir.path().join("foo.txt"), "hello").unwrap();
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["add", "."])
            .output()
            .ok();
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["commit", "-m", "initial"])
            .output()
            .ok();

        // Modify a file to make the tree dirty
        std::fs::write(dir.path().join("foo.txt"), "modified").unwrap();

        let state = RepoState::capture(&dir.path().to_path_buf());
        assert!(state.is_some());
        let state = state.unwrap();
        assert!(state.working_tree_dirty);
    }

    #[test]
    fn test_repo_state_capture_non_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("foo.txt"), "hello").unwrap();
        let state = RepoState::capture(&dir.path().to_path_buf());
        assert!(state.is_none());
    }

    #[test]
    fn test_repo_state_deterministic_hash() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["init"])
            .output()
            .ok();
        std::fs::write(dir.path().join("foo.txt"), "hello").unwrap();
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["add", "."])
            .output()
            .ok();
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["commit", "-m", "initial"])
            .output()
            .ok();

        let state1 = RepoState::capture(&dir.path().to_path_buf()).unwrap();
        let state2 = RepoState::capture(&dir.path().to_path_buf()).unwrap();
        // Hash must be deterministic across captures of the same state.
        assert_eq!(state1.working_tree_hash, state2.working_tree_hash);
    }

    #[test]
    fn test_sandbox_runtime_status_local() {
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let status = rt.status();
        assert_eq!(status.backend, "local");
        assert!(status.available);
        assert!(status.capabilities.is_some());
    }

    #[test]
    fn test_sandbox_runtime_fail_closed_opensandbox_unavailable() {
        // Explicitly configure OpenSandbox with an unreachable URL.
        let rt = SandboxRuntime::with_opensandbox("http://localhost:1".to_string());
        assert!(rt.opensandbox_explicit());
        // is_available() checks config (non-empty URL), not connectivity.
        // The fail-closed behavior is verified through execute().
        assert_eq!(rt.active_backend(), "opensandbox");

        // Execute should fail closed, not silently fall back to local.
        let cmd = SandboxCommand {
            command: "echo hi".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new();
        let result = rt.execute(&PathBuf::from("/tmp"), cmd, &policy);
        assert!(!result.success);
        assert!(result.denied);
        assert_eq!(result.backend, "opensandbox");
        assert!(result
            .denied_reason
            .as_deref()
            .unwrap_or("")
            .contains("unavailable"));
    }

    #[test]
    fn test_sandbox_runtime_exposes_capabilities() {
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let caps = rt.capabilities();
        assert_eq!(caps.isolation, IsolationLevel::None);
        assert_eq!(caps.network_access, NetworkAccess::Host);
    }

    // ── Freshness tests ───────────────────────────────────────────────

    #[test]
    fn test_freshness_fresh_when_state_matches() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["init"])
            .output()
            .ok();
        std::fs::write(dir.path().join("f.txt"), "a").unwrap();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["add", "."])
            .output()
            .ok();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["commit", "-m", "init"])
            .output()
            .ok();

        let state1 = RepoState::capture(&dir.path().to_path_buf()).unwrap();
        let state2 = RepoState::capture(&dir.path().to_path_buf()).unwrap();
        assert_eq!(state1.working_tree_hash, state2.working_tree_hash);
        // Freshness is computed by comparing evidence state to current state.
        let freshness = compute_freshness(&state1, Some(&state2));
        assert_eq!(freshness, Freshness::Fresh);
    }

    #[test]
    fn test_freshness_stale_when_state_changed() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["init"])
            .output()
            .ok();
        std::fs::write(dir.path().join("f.txt"), "a").unwrap();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["add", "."])
            .output()
            .ok();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["commit", "-m", "init"])
            .output()
            .ok();

        let state1 = RepoState::capture(&dir.path().to_path_buf()).unwrap();
        // Modify the repo
        std::fs::write(dir.path().join("f.txt"), "b").unwrap();
        let state2 = RepoState::capture(&dir.path().to_path_buf()).unwrap();
        assert_ne!(state1.working_tree_hash, state2.working_tree_hash);
        let freshness = compute_freshness(&state1, Some(&state2));
        assert_eq!(freshness, Freshness::Stale);
    }

    #[test]
    fn test_freshness_unknown_when_current_unavailable() {
        // Simulate the case where current state cannot be determined
        // (e.g., not a git repo at time of freshness check).
        let dir = tempfile::tempdir().unwrap();
        // Create a git repo and capture initial state
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["init"])
            .output()
            .ok();
        std::fs::write(dir.path().join("f.txt"), "a").unwrap();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["add", "."])
            .output()
            .ok();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["commit", "-m", "init"])
            .output()
            .ok();
        let state1 = RepoState::capture(&dir.path().to_path_buf()).unwrap();
        // Unknown freshness when current state capture fails (simulated by passing None)
        let freshness = compute_freshness(&state1, None);
        assert_eq!(freshness, Freshness::Unknown);
    }

    /// Compute freshness by comparing evidence state to current state.
    fn compute_freshness(
        evidence_state: &RepoState,
        current_state: Option<&RepoState>,
    ) -> Freshness {
        match current_state {
            Some(current) => {
                if evidence_state.working_tree_hash == current.working_tree_hash {
                    Freshness::Fresh
                } else {
                    Freshness::Stale
                }
            }
            None => Freshness::Unknown,
        }
    }

    // ── M3 verification correlation tests ─────────────────────────────

    #[test]
    fn verification_without_impacted_fact_ids_remains_unchanged() {
        let exec = ExecutionResult::from_local(
            "true",
            "/workspace",
            "",
            "",
            0,
            5,
            false,
            false,
            HashMap::new(),
        );
        let v = VerificationResult::from_execution(exec);
        assert!(v.verified);
        assert_eq!(v.impacted_fact_ids, None);
    }

    #[test]
    fn verification_preserves_caller_supplied_impacted_fact_ids() {
        let exec = ExecutionResult::from_local(
            "cargo test",
            "/workspace",
            "test result: ok",
            "",
            0,
            1200,
            false,
            false,
            HashMap::new(),
        );
        let ids = vec![
            "sym::src/lib.rs::foo_0".to_string(),
            "rel::foo_calls_bar".to_string(),
        ];
        let v = VerificationResult::from_execution_with_impacted_fact_ids(
            exec,
            None,
            None,
            Some(ids.clone()),
        );
        assert!(v.verified);
        assert_eq!(v.impacted_fact_ids, Some(ids));
    }

    #[test]
    fn verification_serializes_impacted_fact_ids_when_present() {
        let exec = ExecutionResult::from_local(
            "true",
            "/workspace",
            "",
            "",
            0,
            5,
            false,
            false,
            HashMap::new(),
        );
        let ids = vec!["sym::x".to_string()];
        let v = VerificationResult::from_execution_with_impacted_fact_ids(
            exec,
            None,
            None,
            Some(ids),
        );
        let json = serde_json::to_string(&v).expect("serializes");
        assert!(json.contains("impacted_fact_ids"));
        assert!(json.contains("sym::x"));
    }

    #[test]
    fn verification_omits_impacted_fact_ids_when_none() {
        let exec = ExecutionResult::from_local(
            "true",
            "/workspace",
            "",
            "",
            0,
            5,
            false,
            false,
            HashMap::new(),
        );
        let v = VerificationResult::from_execution(exec);
        let json = serde_json::to_string(&v).expect("serializes");
        assert!(!json.contains("impacted_fact_ids"));
    }
}
