//! P4 skills lifecycle: evidence-backed skill candidates, versioned
//! publication, and lifecycle management.
//!
//! P3 produces learning candidates — hypotheses about recurring engineering
//! patterns. P4 extends this into a skill lifecycle: a learning candidate
//! with sufficient evidence may become a skill candidate, which — after
//! validation, approval, and versioning — becomes an active skill published
//! as a `SKILL.md` file that OpenCode discovers and executes natively.
//!
//! ```text
//!        P3 LEARNING CANDIDATE
//!                │
//!                ▼
//!        SKILL CANDIDATE (evidence-backed proposal)
//!                │
//!        ┌───────┴───────┐
//!        ▼               ▼
//!    VALIDATION      REJECTION
//!        │
//!        ▼
//!    DRAFT (proposed SKILL.md content)
//!        │
//!        ▼
//!    APPROVED (user or trusted principal confirms)
//!        │
//!        ▼
//!    ACTIVE (published to OpenCode skill filesystem)
//!        │
//!    ┌───┴───┐
//!    ▼       ▼
//! UPDATED  DEPRECATED
//!    │
//!    ▼
//! VERSION N+1
//! ```
//!
//! # Source of truth
//!
//! - **Skill content** (the SKILL.md body): lives in the OpenCode skill
//!   filesystem (`~/.config/opencode/skills/<name>/SKILL.md`). CodeBro
//!   writes this file when publishing a version.
//! - **Skill lifecycle** (status, versions, evidence, health): lives in
//!   `state.db`. CodeBro owns this exclusively.
//! - **Skill discovery** (what skills exist): OpenCode scans the filesystem.
//!   CodeBro provides applicability metadata via MCP but does not execute.
//!
//! # Safety invariants
//!
//! - **No blind file mutation.** Every SKILL.md write reads the current
//!   content first, verifies version, and detects concurrent modification.
//! - **Path traversal protection.** Skill files are confined to the
//!   configured skill root directory.
//! - **Secret scanning.** Generated skill content is scanned for API keys,
//!   tokens, and credentials before publication.
//! - **Workspace isolation.** A project skill cannot become active in a
//!   different project without explicit promotion.
//! - **Immutability.** Published versions are immutable. Changes create new
//!   versions.
//! - **Audit trail.** Every lifecycle transition is recorded with evidence
//!   and provenance.
//! - **No self-approval.** AI-generated skills require explicit user or
//!   trusted-principal approval before activation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::learning::LearningCandidate;
use crate::store::{ContextError, ContextStore, EVENT_COLUMNS};
use crate::workspace::canonical_workspace_key;

// ─── Constants ────────────────────────────────────────────────────────

/// Maximum skill content size in characters (500 lines × ~80 chars).
pub const MAX_SKILL_CONTENT_CHARS: usize = 40_000;
/// Maximum skill name length (OpenCode allows 1–64).
pub const MAX_SKILL_NAME_CHARS: usize = 64;
/// Maximum skill description length.
pub const MAX_SKILL_DESCRIPTION_CHARS: usize = 1024;
/// Minimum confidence for skill candidate approval.
pub const SKILL_APPROVAL_MIN_CONFIDENCE: f64 = 0.60;
/// Health threshold: failure ratio above which a skill is flagged.
pub const HEALTH_FAILURE_THRESHOLD: f64 = 0.40;
/// Minimum uses before health assessment is meaningful.
pub const HEALTH_MIN_USES: usize = 3;
/// Candidate TTL: 60 days without refresh → expired.
pub const SKILL_CANDIDATE_TTL_SECS: u64 = 60 * 86_400;

// ─── Types ────────────────────────────────────────────────────────────

/// Skill scope: mirrors learning scope but for skills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkillScope {
    Global,
    Project,
    Task,
}

impl SkillScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkillScope::Global => "global",
            SkillScope::Project => "project",
            SkillScope::Task => "task",
        }
    }
}

impl std::fmt::Display for SkillScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for SkillScope {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "global" => Ok(SkillScope::Global),
            "project" => Ok(SkillScope::Project),
            "task" => Ok(SkillScope::Task),
            other => Err(format!(
                "unknown skill scope '{other}': use global, project, or task"
            )),
        }
    }
}

/// Skill candidate lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkillCandidateStatus {
    /// Initial state: evidence gathered, proposal generated.
    Candidate,
    /// Under evaluation.
    Evaluating,
    /// Validated, content drafted.
    Draft,
    /// Validation passed.
    Validated,
    /// Approved by user or trusted principal.
    Approved,
    /// Published as an active skill.
    Active,
    /// Rejected by user or validation.
    Rejected,
    /// Insufficient evidence, preserved for future.
    Deferred,
    /// Superseded by a newer candidate.
    Superseded,
    /// Expired without refresh.
    Expired,
    /// Deprecated after activation.
    Deprecated,
}

impl SkillCandidateStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkillCandidateStatus::Candidate => "candidate",
            SkillCandidateStatus::Evaluating => "evaluating",
            SkillCandidateStatus::Draft => "draft",
            SkillCandidateStatus::Validated => "validated",
            SkillCandidateStatus::Approved => "approved",
            SkillCandidateStatus::Active => "active",
            SkillCandidateStatus::Rejected => "rejected",
            SkillCandidateStatus::Deferred => "deferred",
            SkillCandidateStatus::Superseded => "superseded",
            SkillCandidateStatus::Expired => "expired",
            SkillCandidateStatus::Deprecated => "deprecated",
        }
    }

    /// Terminal states accept no further transitions except via supersession.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            SkillCandidateStatus::Rejected
                | SkillCandidateStatus::Superseded
                | SkillCandidateStatus::Expired
        )
    }

    /// Valid predecessor states for a forward transition.
    ///
    /// The documented chain is Candidate → Evaluating → Draft → Validated
    /// → Approved → Active. Side exits: rejection from any pre-active
    /// state, expiry from the low-commitment states (Candidate/Evaluating/
    /// Deferred — drafted/validated/approved content never silently
    /// expires), deprecation and supersession from post-activation.
    /// Deferred may be re-evaluated or explicitly rejected.
    pub fn can_transition_to(&self, target: &SkillCandidateStatus) -> bool {
        use SkillCandidateStatus::*;
        matches!(
            (self, target),
            (Candidate, Evaluating)
                | (Candidate, Rejected)
                | (Candidate, Deferred)
                | (Candidate, Expired)
                | (Evaluating, Draft)
                | (Evaluating, Rejected)
                | (Evaluating, Deferred)
                | (Evaluating, Expired)
                | (Draft, Validated)
                | (Draft, Rejected)
                | (Validated, Approved)
                | (Validated, Rejected)
                | (Approved, Active)
                | (Approved, Rejected)
                | (Approved, Superseded)
                | (Active, Deprecated)
                | (Active, Superseded)
                | (Deferred, Evaluating) // re-evaluation
                | (Deferred, Rejected)
                | (Deferred, Expired)
        )
    }
}

impl std::fmt::Display for SkillCandidateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for SkillCandidateStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "candidate" => Ok(SkillCandidateStatus::Candidate),
            "evaluating" => Ok(SkillCandidateStatus::Evaluating),
            "draft" => Ok(SkillCandidateStatus::Draft),
            "validated" => Ok(SkillCandidateStatus::Validated),
            "approved" => Ok(SkillCandidateStatus::Approved),
            "active" => Ok(SkillCandidateStatus::Active),
            "rejected" => Ok(SkillCandidateStatus::Rejected),
            "deferred" => Ok(SkillCandidateStatus::Deferred),
            "superseded" => Ok(SkillCandidateStatus::Superseded),
            "expired" => Ok(SkillCandidateStatus::Expired),
            "deprecated" => Ok(SkillCandidateStatus::Deprecated),
            other => Err(format!("unknown skill candidate status: {other}")),
        }
    }
}

/// Active skill lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkillStatus {
    Draft,
    Validated,
    Approved,
    Active,
    Deprecated,
    Superseded,
}

impl SkillStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkillStatus::Draft => "draft",
            SkillStatus::Validated => "validated",
            SkillStatus::Approved => "approved",
            SkillStatus::Active => "active",
            SkillStatus::Deprecated => "deprecated",
            SkillStatus::Superseded => "superseded",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, SkillStatus::Deprecated | SkillStatus::Superseded)
    }
}

impl std::fmt::Display for SkillStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for SkillStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "draft" => Ok(SkillStatus::Draft),
            "validated" => Ok(SkillStatus::Validated),
            "approved" => Ok(SkillStatus::Approved),
            "active" => Ok(SkillStatus::Active),
            "deprecated" => Ok(SkillStatus::Deprecated),
            "superseded" => Ok(SkillStatus::Superseded),
            other => Err(format!("unknown skill status: {other}")),
        }
    }
}

/// Skill version status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkillVersionStatus {
    Draft,
    Validated,
    Approved,
    Active,
    Replaced,
    RolledBack,
}

impl SkillVersionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkillVersionStatus::Draft => "draft",
            SkillVersionStatus::Validated => "validated",
            SkillVersionStatus::Approved => "approved",
            SkillVersionStatus::Active => "active",
            SkillVersionStatus::Replaced => "replaced",
            SkillVersionStatus::RolledBack => "rolled_back",
        }
    }
}

impl std::fmt::Display for SkillVersionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for SkillVersionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "draft" => Ok(SkillVersionStatus::Draft),
            "validated" => Ok(SkillVersionStatus::Validated),
            "approved" => Ok(SkillVersionStatus::Approved),
            "active" => Ok(SkillVersionStatus::Active),
            "replaced" => Ok(SkillVersionStatus::Replaced),
            "rolled_back" => Ok(SkillVersionStatus::RolledBack),
            other => Err(format!("unknown skill version status: {other}")),
        }
    }
}

/// Applicability metadata for a skill candidate or skill.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkillApplicability {
    /// Project names/paths this skill applies to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<String>,
    /// Task types (e.g., "debugging", "release", "refactoring").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task_types: Vec<String>,
    /// Programming languages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    /// Frameworks or tools.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frameworks: Vec<String>,
    /// Subsystem or module names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subsystems: Vec<String>,
}

/// Skill health metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkillHealth {
    pub success_count: u64,
    pub failure_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_validated_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failed_at: Option<u64>,
}

impl SkillHealth {
    /// Whether health assessment is meaningful (enough uses).
    pub fn is_assessable(&self) -> bool {
        (self.success_count + self.failure_count) >= HEALTH_MIN_USES as u64
    }

    /// Failure ratio: 0.0 = perfect, 1.0 = all failures.
    pub fn failure_ratio(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            return 0.0;
        }
        self.failure_count as f64 / total as f64
    }

    /// Whether health is degraded (high failure ratio with enough data).
    pub fn is_degraded(&self) -> bool {
        self.is_assessable() && self.failure_ratio() >= HEALTH_FAILURE_THRESHOLD
    }
}

/// Validation result for a skill or skill candidate.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkillValidation {
    pub valid: bool,
    #[serde(default)]
    pub structural: bool,
    #[serde(default)]
    pub scope_valid: bool,
    #[serde(default)]
    pub secret_safe: bool,
    #[serde(default)]
    pub semantic_valid: bool,
    #[serde(default)]
    pub size_valid: bool,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// A skill candidate: an evidence-backed proposal for a new or updated skill.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SkillCandidate {
    /// Deterministic id: `sc::<16 hex>`.
    pub candidate_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub scope: String,
    pub name: String,
    pub description: String,
    pub purpose: String,
    #[serde(default)]
    pub applicability: SkillApplicability,
    /// Learning candidate ids that sourced this skill candidate.
    #[serde(default)]
    pub source_learning_candidates: Vec<String>,
    /// Event ids supporting this candidate.
    #[serde(default)]
    pub supporting_evidence: Vec<i64>,
    /// Event ids contradicting this candidate.
    #[serde(default)]
    pub contradicting_evidence: Vec<i64>,
    /// Proposed SKILL.md content.
    pub proposed_content: String,
    pub status: String,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<SkillValidation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
    /// Skill id this candidate supersedes (for updates).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_skill: Option<String>,
    /// Skill version this candidate was validated against (optimistic
    /// concurrency anchor: approval refuses if the lineage advanced).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on_version: Option<u32>,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

impl SkillCandidate {
    /// Explainable summary: what this candidate proposes, why, and the evidence.
    pub fn explain(&self) -> serde_json::Value {
        serde_json::json!({
            "candidate_id": self.candidate_id,
            "name": self.name,
            "description": self.description,
            "purpose": self.purpose,
            "scope": self.scope,
            "status": self.status,
            "confidence": self.confidence,
            "task_id": self.task_id,
            "evidence": {
                "supporting": self.supporting_evidence.len(),
                "contradicting": self.contradicting_evidence.len(),
                "learning_candidates": self.source_learning_candidates.len(),
            },
            "applicability": self.applicability,
            "validation": self.validation,
            "eval_reason": self.eval_reason,
            "rejection_reason": self.rejection_reason,
            "supersedes_skill": self.supersedes_skill,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "expires_at": self.expires_at,
        })
    }
}

/// An active skill in the registry.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Skill {
    /// Deterministic id: `sk::<16 hex>`.
    pub skill_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    pub scope: String,
    /// Unique skill name (matches directory name in OpenCode filesystem).
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub applicability: SkillApplicability,
    pub current_version: u32,
    pub status: String,
    pub confidence: f64,
    #[serde(default)]
    pub health: SkillHealth,
    /// Original skill candidate id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_candidate_id: Option<String>,
    /// Skill id this skill supersedes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl Skill {
    /// Whether this skill is usable (active or approved).
    pub fn is_usable(&self) -> bool {
        matches!(self.status.as_str(), "active" | "approved" | "validated")
    }

    /// Whether health is degraded.
    pub fn is_healthy(&self) -> bool {
        !self.health.is_degraded()
    }
}

/// An immutable published skill version.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SkillVersion {
    /// Deterministic id: `sv::<16 hex>`.
    pub version_id: String,
    pub skill_id: String,
    pub version_number: u32,
    /// Full SKILL.md content.
    pub content: String,
    /// SHA-256 hash of content.
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_candidate_id: Option<String>,
    #[serde(default)]
    pub supporting_evidence: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<SkillValidation>,
    pub author: String,
    pub status: String,
    pub created_at: u64,
    /// Previous version id (for chain tracking).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_version: Option<String>,
}

// ─── ID minting ───────────────────────────────────────────────────────

/// Mint a deterministic skill candidate id from its identity fields.
///
/// The proposed content hash participates in the identity: re-proposing
/// the *same* content converges on the same row (idempotent), while
/// evolved content (a v2 draft for an existing skill) mints a fresh
/// candidate for review. The publication path (not the id) enforces the
/// single-active-lineage-per-name rule.
pub fn mint_skill_candidate_id(
    scope: &SkillScope,
    workspace_root: Option<&str>,
    task_id: Option<&str>,
    name: &str,
    proposed_content: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(
        format!(
            "{}|{}|{}|{}|{}",
            scope,
            workspace_root.unwrap_or(""),
            task_id.unwrap_or(""),
            name,
            content_hash(proposed_content)
        )
        .as_bytes(),
    );
    format!("sc::{:016x}", {
        let digest = hasher.finalize();
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest[..8]);
        u64::from_be_bytes(bytes)
    })
}

/// Mint a deterministic skill id from its name and scope.
pub fn mint_skill_id(scope: &SkillScope, workspace_root: Option<&str>, name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{}|{}|{}", scope, workspace_root.unwrap_or(""), name).as_bytes());
    format!("sk::{:016x}", {
        let digest = hasher.finalize();
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest[..8]);
        u64::from_be_bytes(bytes)
    })
}

/// Mint a version id.
pub fn mint_version_id(skill_id: &str, version_number: u32) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{}|v{}", skill_id, version_number).as_bytes());
    format!("sv::{:016x}", {
        let digest = hasher.finalize();
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest[..8]);
        u64::from_be_bytes(bytes)
    })
}

/// Compute SHA-256 hash of content.
pub fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ─── Validation ───────────────────────────────────────────────────────

/// Secrets patterns that must not appear in skill content.
const SECRET_PATTERNS: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "secret_key",
    "secretkey",
    "secret-key",
    "password",
    "passwd",
    "credential",
    "token",
    "private_key",
    "privatekey",
    "private-key",
    "access_token",
    "auth_token",
    "bearer ",
    "Authorization: Bearer",
    "sk-",
    "ghp_",
    "gho_",
    "glpat-",
    "xoxb-",
    "xoxp-",
];

/// OpenCode skill-name rule: `^[a-z0-9]+(-[a-z0-9]+)*$`, 1–64 chars
/// (no leading/trailing `-`, no consecutive `--`). Uppercase, underscores,
/// and other characters would be silently ignored by OpenCode's loader.
pub fn is_valid_skill_name(skill_name: &str) -> bool {
    !skill_name.is_empty()
        && skill_name.len() <= MAX_SKILL_NAME_CHARS
        && skill_name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !skill_name.starts_with('-')
        && !skill_name.ends_with('-')
        && !skill_name.contains("--")
}

/// Validate skill content for structural correctness, safety, and sanity.
///
/// Enforces the structural rules OpenCode's skill loader applies
/// (frontmatter with `name` + `description`, name matching the directory),
/// plus CodeBro's own safety limits (size, secret scan). Structural checks
/// are best-effort YAML scans, not a full YAML parser — deliberately
/// deterministic and dependency-free.
pub fn validate_skill_content(
    content: &str,
    skill_name: &str,
    _workspace_root: Option<&str>,
) -> SkillValidation {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    // Structural: must have YAML frontmatter
    let structural = content.starts_with("---");
    if !structural {
        errors.push("Missing YAML frontmatter (must start with ---)".to_string());
    }

    // Extract the frontmatter block and the body.
    let frontmatter = extract_frontmatter(content);
    let body = match (&frontmatter, structural) {
        (Some(fm), _) => {
            // content = "---" + fm + "\n---" + body
            let after = &content[3 + fm.len() + 4..];
            after.strip_prefix('\n').unwrap_or(after)
        }
        (None, true) => "",
        (None, false) => content,
    };
    let has_body = !body.trim().is_empty();
    if structural && !has_body {
        errors.push("Skill has frontmatter but no body content".to_string());
    }

    // Frontmatter must carry the required fields: name (matching the
    // directory this skill will be published to) and a 1–1024 char
    // description — OpenCode ignores skills missing either.
    if let Some(fm) = &frontmatter {
        let fm_name = frontmatter_field(fm, "name");
        let fm_desc = frontmatter_field(fm, "description");
        match fm_name {
            None => errors.push(
                "Frontmatter is missing required field 'name' (OpenCode ignores such skills)"
                    .to_string(),
            ),
            Some(n) if n != skill_name => errors.push(format!(
                "Frontmatter name '{n}' does not match skill name '{skill_name}' (it must match the skill directory)"
            )),
            Some(_) => {}
        }
        match fm_desc {
            None => errors.push(
                "Frontmatter is missing required field 'description' (OpenCode ignores such skills)"
                    .to_string(),
            ),
            Some(d) if d.is_empty() => {
                errors.push("Frontmatter 'description' must be 1-1024 characters".to_string())
            }
            Some(d) if d.len() > MAX_SKILL_DESCRIPTION_CHARS => errors.push(format!(
                "Frontmatter 'description' {} chars exceeds maximum {MAX_SKILL_DESCRIPTION_CHARS}",
                d.len()
            )),
            Some(_) => {}
        }
    }

    // Size check
    let size_valid = content.len() <= MAX_SKILL_CONTENT_CHARS;
    if !size_valid {
        errors.push(format!(
            "Skill content {} chars exceeds maximum {}",
            content.len(),
            MAX_SKILL_CONTENT_CHARS
        ));
    }

    // Secret scanning
    let content_lower = content.to_lowercase();
    let secret_safe = !SECRET_PATTERNS
        .iter()
        .any(|pat| content_lower.contains(&pat.to_lowercase()));
    if !secret_safe {
        let found: Vec<&str> = SECRET_PATTERNS
            .iter()
            .filter(|pat| content_lower.contains(&pat.to_lowercase()))
            .copied()
            .collect();
        errors.push(format!(
            "Skill content contains potential secrets: {}",
            found.join(", ")
        ));
    }

    // Semantic: name validation (OpenCode naming rule)
    let name_valid = is_valid_skill_name(skill_name);
    if !name_valid {
        errors.push(format!(
            "Invalid skill name '{skill_name}': must be 1-{MAX_SKILL_NAME_CHARS} lowercase alphanumeric segments with single hyphens (e.g. git-release)"
        ));
    }

    // Scope validation: workspace must match if project-scoped
    let scope_valid = true; // workspace matching is enforced at the store level

    // Semantic: content should have a purpose or description section
    let semantic_valid = body.contains("# Purpose")
        || body.contains("# When to Use")
        || body.contains("# Procedure")
        || body.contains("# When to use")
        || body.contains("## What I do")
        || body.contains("## When to use me");
    if !semantic_valid && has_body {
        warnings.push(
            "Skill body lacks standard sections (Purpose, When to Use, Procedure)".to_string(),
        );
    }

    SkillValidation {
        valid: errors.is_empty(),
        structural,
        scope_valid,
        secret_safe,
        semantic_valid,
        size_valid,
        errors,
        warnings,
    }
}

/// Extract the raw YAML frontmatter block (between the opening `---` and
/// the closing `---`), if present and closed.
fn extract_frontmatter(content: &str) -> Option<String> {
    let rest = content.strip_prefix("---")?;
    // The closing delimiter must start a line.
    let end = rest.find("\n---")?;
    let block = &rest[..end];
    // Reject a second opening marker inside the block.
    if block.contains("\n---") {
        return None;
    }
    Some(block.to_string())
}

/// Best-effort single-line `key: value` lookup inside a frontmatter block.
/// Values may be single- or double-quoted; quotes are stripped.
fn frontmatter_field(frontmatter: &str, key: &str) -> Option<String> {
    for line in frontmatter.lines() {
        let line = line.trim();
        let prefix = format!("{key}:");
        if let Some(rest) = line.strip_prefix(&prefix) {
            let mut value = rest.trim().to_string();
            if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
                || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
            {
                value = value[1..value.len() - 1].to_string();
            }
            return Some(value);
        }
    }
    None
}

// ─── Filesystem safety ────────────────────────────────────────────────

/// Resolve the SKILL.md path for a skill name inside a given skill root,
/// rejecting unsafe names before they ever reach the filesystem.
///
/// The name is validated (not pattern-matched against the result): an
/// attacker-supplied `../evil` or `a/b` fails validation here, so only
/// `[a-z0-9-]+` names — which cannot contain separators or dot segments —
/// are ever joined onto the root. This is authorization *before* path
/// construction, which makes traversal structurally impossible rather than
/// detected after the fact.
pub fn skill_file_path_at(skill_root: &Path, skill_name: &str) -> Result<PathBuf, SkillError> {
    if !is_valid_skill_name(skill_name) {
        return Err(SkillError::PathTraversal(format!(
            "unsafe skill name '{skill_name}'"
        )));
    }
    Ok(skill_root.join(skill_name).join("SKILL.md"))
}

/// Legacy helper preserved for the documented `~/.config/opencode/skills`
/// location; resolves through [`skill_file_path_at`] with HOME validation.
pub fn skill_file_path(skill_name: &str) -> Result<PathBuf, SkillError> {
    let home =
        std::env::var("HOME").map_err(|e| SkillError::FsError(format!("HOME not set: {e}")))?;
    let root = PathBuf::from(home)
        .join(".config")
        .join("opencode")
        .join("skills");
    skill_file_path_at(&root, skill_name)
}

/// Read the current skill file content, if it exists. The caller supplies
/// the skill root so tests and deployments stay hermetic.
pub fn read_skill_file_at(
    skill_root: &Path,
    skill_name: &str,
) -> Result<Option<String>, SkillError> {
    let path = skill_file_path_at(skill_root, skill_name)?;
    if path.exists() {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| SkillError::FsError(format!("read {}: {e}", path.display())))?;
        Ok(Some(content))
    } else {
        Ok(None)
    }
}

/// Resolve the SKILL.md path for a skill name only when the name is safe;
/// `Ok(None)` means "no file at a safe path" (missing or the name itself
/// is unusable — the caller treats both as nothing-to-remove).
fn skill_file_path_checked(
    skill_root: &Path,
    skill_name: &str,
) -> Result<Option<PathBuf>, SkillError> {
    match skill_file_path_at(skill_root, skill_name) {
        Ok(p) => Ok(Some(p)),
        Err(SkillError::PathTraversal(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Whether a skill file exists and matches the recorded content hash.
/// Disagreement between the DB and the filesystem is surfaced, never
/// silently overwritten.
pub fn skill_file_matches(
    skill_root: &Path,
    skill_name: &str,
    hash: &str,
) -> Result<bool, SkillError> {
    match read_skill_file_at(skill_root, skill_name)? {
        Some(content) => Ok(content_hash(&content) == hash),
        None => Ok(false),
    }
}

/// Publish skill content atomically: write to a temp file inside the
/// target directory, fsync, then rename over the destination.
///
/// Guarantees:
/// - The SKILL.md path is always fully-written or absent — a crash mid-way
///   can leave at most a leftover `.tmp-<name>` file, never a partial
///   SKILL.md.
/// - Refuses to write through symlinks: if SKILL.md (or the skill
///   directory, or the root) is a symlink, the operation fails closed.
///   An attacker who can plant a symlink cannot redirect publication
///   outside the skill root.
/// - Refuses to overwrite a file that changed since the recorded content
///   hash (read-before-write conflict detection) unless `expected_hash` is
///   `None` and the file does not exist (first publication).
pub fn publish_skill_file(
    skill_root: &Path,
    skill_name: &str,
    content: &str,
    expected_hash: Option<&str>,
) -> Result<PathBuf, SkillError> {
    let path = skill_file_path_at(skill_root, skill_name)?;
    let dir = path
        .parent()
        .ok_or_else(|| SkillError::FsError("cannot get parent dir".to_string()))?
        .to_path_buf();

    // Read-before-write: check the current state of the world.
    let current = read_skill_file_at(skill_root, skill_name)?;
    match (&current, expected_hash) {
        (Some(existing), Some(expected)) => {
            let actual = content_hash(existing);
            if actual != expected {
                return Err(SkillError::Conflict);
            }
        }
        (Some(_), None) => {
            // Overwriting existing content without an identity check is a
            // blind write: only permitted for rollback, which re-publishes
            // DB-owned immutable content over a drifted file. Callers that
            // lack a hash must pass overwrite_drifted=true explicitly.
            return Err(SkillError::Conflict);
        }
        (None, _) => {}
    }

    // Symlink defense: the skill directory chain must be real directories.
    // The publication root is created if absent (fresh installs have no
    // ~/.config/opencode/skills yet); everything *below* it must resolve
    // to real directories under the canonicalized root.
    std::fs::create_dir_all(skill_root).map_err(|e| {
        SkillError::FsError(format!("create skill root {}: {e}", skill_root.display()))
    })?;
    let root_canonical = skill_root
        .canonicalize()
        .map_err(|e| SkillError::FsError(format!("skill root {}: {e}", skill_root.display())))?;
    let mut ancestor = dir.as_path();
    let mut deepest: Option<PathBuf> = None;
    for _ in 0..64 {
        match ancestor.canonicalize() {
            Ok(c) => {
                deepest = Some(c);
                break;
            }
            Err(_) => {
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| SkillError::FsError("no parent".to_string()))?;
            }
        }
    }
    if let Some(c) = deepest {
        if !c.starts_with(&root_canonical) {
            return Err(SkillError::PathTraversal(format!(
                "skill directory resolves outside the skill root: {} -> {}",
                dir.display(),
                c.display()
            )));
        }
        // If the skill directory itself exists but is a symlink, refuse.
        let dir_exists = dir.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false);
        if dir_exists && dir.is_symlink() {
            return Err(SkillError::PathTraversal(format!(
                "skill directory {} is a symlink; refusing to publish through it",
                dir.display()
            )));
        }
    }

    std::fs::create_dir_all(&dir)
        .map_err(|e| SkillError::FsError(format!("create {}: {e}", dir.display())))?;

    // Existing SKILL.md must be a regular file, not a symlink.
    if let Ok(meta) = path.symlink_metadata() {
        if meta.file_type().is_symlink() {
            return Err(SkillError::PathTraversal(format!(
                "SKILL.md at {} is a symlink; refusing to write through it",
                path.display()
            )));
        }
    }

    // Atomic publish: temp file in the same directory, fsync, rename.
    let tmp = dir.join(format!(".tmp-{skill_name}"));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| SkillError::FsError(format!("create {}: {e}", tmp.display())))?;
        f.write_all(content.as_bytes())
            .and_then(|_| f.sync_all())
            .map_err(|e| SkillError::FsError(format!("write {}: {e}", tmp.display())))?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| {
        SkillError::FsError(format!(
            "rename {} -> {}: {e}",
            tmp.display(),
            path.display()
        ))
    })?;
    // Best-effort directory fsync so the rename itself is durable.
    if let Ok(d) = std::fs::File::open(&dir) {
        let _ = d.sync_all();
    }
    Ok(path)
}

// ─── Errors ───────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum SkillError {
    #[error("skill not found: {0}")]
    NotFound(String),
    #[error("skill candidate not found: {0}")]
    CandidateNotFound(String),
    #[error("invalid lifecycle transition: {from} → {to}")]
    InvalidTransition { from: String, to: String },
    #[error("validation failed: {0}")]
    ValidationFailed(String),
    #[error("conflict: skill modified externally since last read")]
    Conflict,
    #[error("filesystem error: {0}")]
    FsError(String),
    #[error("secret detected in skill content")]
    SecretDetected,
    #[error("path traversal detected: {0}")]
    PathTraversal(String),
    #[error("workspace mismatch: skill belongs to {expected}, got {actual}")]
    WorkspaceMismatch { expected: String, actual: String },
    #[error("approval required: {0}")]
    ApprovalRequired(String),
    #[error("skill name already exists: {0}")]
    NameConflict(String),
    #[error("insufficient evidence: {0}")]
    InsufficientEvidence(String),
    #[error("database error: {0}")]
    Database(#[from] ContextError),
}

// ─── Store methods ────────────────────────────────────────────────────

const SKILL_CANDIDATE_COLUMNS: &str =
    "candidate_id, workspace_root, task_id, scope, name, description, purpose, \
     applicability_json, source_learning_candidates_json, supporting_json, \
     contradicting_json, proposed_content, status, confidence, validation_json, \
     created_at, updated_at, expires_at, eval_reason, rejection_reason, \
     supersedes_skill, based_on_version";

const SKILL_COLUMNS: &str =
    "skill_id, workspace_root, scope, name, description, applicability_json, \
     current_version, status, confidence, health_json, source_candidate_id, \
     superseded_by, created_at, updated_at";

const SKILL_VERSION_COLUMNS: &str = "version_id, skill_id, version_number, content, content_hash, \
     source_candidate_id, supporting_json, validation_json, author, \
     status, created_at, parent_version";

fn row_to_skill_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillCandidate> {
    let applicability_json: String = row.get(7)?;
    let source_lc_json: String = row.get(8)?;
    let supporting_json: String = row.get(9)?;
    let contradicting_json: String = row.get(10)?;
    let validation_json: Option<String> = row.get(14)?;

    Ok(SkillCandidate {
        candidate_id: row.get(0)?,
        workspace_root: row.get(1)?,
        task_id: row.get(2)?,
        scope: row.get(3)?,
        name: row.get(4)?,
        description: row.get(5)?,
        purpose: row.get(6)?,
        applicability: serde_json::from_str(&applicability_json).unwrap_or_default(),
        source_learning_candidates: serde_json::from_str(&source_lc_json).unwrap_or_default(),
        supporting_evidence: serde_json::from_str(&supporting_json).unwrap_or_default(),
        contradicting_evidence: serde_json::from_str(&contradicting_json).unwrap_or_default(),
        proposed_content: row.get(11)?,
        status: row.get(12)?,
        confidence: row.get(13)?,
        validation: validation_json.and_then(|v| serde_json::from_str(&v).ok()),
        created_at: row.get::<_, i64>(15)? as u64,
        updated_at: row.get::<_, i64>(16)? as u64,
        expires_at: row.get::<_, Option<i64>>(17)?.map(|t| t as u64),
        eval_reason: row.get(18)?,
        rejection_reason: row.get(19)?,
        supersedes_skill: row.get(20)?,
        based_on_version: row.get::<_, Option<i64>>(21)?.map(|v| v as u32),
    })
}

fn row_to_skill(row: &rusqlite::Row<'_>) -> rusqlite::Result<Skill> {
    let applicability_json: String = row.get(5)?;
    let health_json: String = row.get(9)?;

    Ok(Skill {
        skill_id: row.get(0)?,
        workspace_root: row.get(1)?,
        scope: row.get(2)?,
        name: row.get(3)?,
        description: row.get(4)?,
        applicability: serde_json::from_str(&applicability_json).unwrap_or_default(),
        current_version: row.get::<_, i32>(6)? as u32,
        status: row.get(7)?,
        confidence: row.get(8)?,
        health: serde_json::from_str(&health_json).unwrap_or_default(),
        source_candidate_id: row.get(10)?,
        superseded_by: row.get(11)?,
        created_at: row.get::<_, i64>(12)? as u64,
        updated_at: row.get::<_, i64>(13)? as u64,
    })
}

fn row_to_skill_version(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillVersion> {
    let supporting_json: String = row.get(6)?;
    let validation_json: Option<String> = row.get(7)?;

    Ok(SkillVersion {
        version_id: row.get(0)?,
        skill_id: row.get(1)?,
        version_number: row.get::<_, i32>(2)? as u32,
        content: row.get(3)?,
        content_hash: row.get(4)?,
        source_candidate_id: row.get(5)?,
        supporting_evidence: serde_json::from_str(&supporting_json).unwrap_or_default(),
        validation: validation_json.and_then(|v| serde_json::from_str(&v).ok()),
        author: row.get(8)?,
        status: row.get(9)?,
        created_at: row.get::<_, i64>(10)? as u64,
        parent_version: row.get(11)?,
    })
}

impl ContextStore {
    // ── Skill candidate CRUD ───────────────────────────────────────────

    /// Insert a fresh skill candidate, enforcing lineage conflict rules.
    ///
    /// Deterministic ids mean re-proposing the same identity maps to the
    /// same row. That is only safe when the stored row is still an
    /// open-evidence draft (`candidate`/`evaluating`/`deferred`): a
    /// pending proposal against a row already in the review pipeline
    /// (draft/validated/approved/active) is refused rather than merged,
    /// and terminal rows (rejected/superseded/expired/deprecated) are
    /// never resurrected — new evidence must come through a new identity
    /// (typically a renamed skill or a fresh learning candidate).
    pub fn insert_skill_candidate(&self, candidate: &SkillCandidate) -> Result<(), ContextError> {
        self.with_conn(|conn| {
            let existing: Option<SkillCandidate> = conn
                .query_row(
                    &format!(
                        "SELECT {SKILL_CANDIDATE_COLUMNS} FROM skill_candidates
                         WHERE candidate_id = ?1"
                    ),
                    [&candidate.candidate_id],
                    row_to_skill_candidate,
                )
                .optional()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            if let Some(existing) = existing {
                let status = existing.status.as_str();
                let mutable_pre_draft = matches!(status, "candidate" | "evaluating" | "deferred");
                if !mutable_pre_draft {
                    return Err(ContextError::Validation(format!(
                        "skill candidate {} already exists in status '{status}': \
                         conflicting proposals for a reviewed/terminal lineage are refused; \
                         use a different skill name or wait for the current review to finish",
                        candidate.candidate_id
                    )));
                }
                if existing.proposed_content != candidate.proposed_content {
                    return Err(ContextError::Validation(format!(
                        "skill candidate {} already exists with different proposed content: \
                         deterministic ids bind one identity to one content lineage; \
                         update the existing candidate through its lifecycle instead",
                        candidate.candidate_id
                    )));
                }
                // Same identity, same content, still pre-draft: idempotent
                // re-proposal refreshes evidence fields but keeps status.
                conn.execute(
                    "UPDATE skill_candidates SET
                        description = ?2, purpose = ?3, applicability_json = ?4,
                        source_learning_candidates_json = ?5, supporting_json = ?6,
                        contradicting_json = ?7, confidence = ?8, validation_json = ?9,
                        updated_at = ?10, expires_at = ?11, eval_reason = ?12,
                        based_on_version = COALESCE(?13, based_on_version)
                     WHERE candidate_id = ?1 AND status IN ('candidate', 'evaluating', 'deferred')",
                    params![
                        candidate.candidate_id,
                        candidate.description,
                        candidate.purpose,
                        serde_json::to_string(&candidate.applicability)
                            .map_err(|e| ContextError::Decode(e.to_string()))?,
                        serde_json::to_string(&candidate.source_learning_candidates)
                            .map_err(|e| ContextError::Decode(e.to_string()))?,
                        serde_json::to_string(&candidate.supporting_evidence)
                            .map_err(|e| ContextError::Decode(e.to_string()))?,
                        serde_json::to_string(&candidate.contradicting_evidence)
                            .map_err(|e| ContextError::Decode(e.to_string()))?,
                        candidate.confidence,
                        candidate
                            .validation
                            .as_ref()
                            .map(serde_json::to_string)
                            .transpose()
                            .map_err(|e| ContextError::Decode(e.to_string()))?,
                        candidate.updated_at as i64,
                        candidate.expires_at.map(|t| t as i64),
                        candidate.eval_reason.as_deref(),
                        candidate.based_on_version.map(|v| v as i64),
                    ],
                )?;
                return Ok(());
            }
            conn.execute(
                "INSERT INTO skill_candidates (
                    candidate_id, workspace_root, task_id, scope,
                    name, description, purpose, applicability_json,
                    source_learning_candidates_json, supporting_json,
                    contradicting_json, proposed_content, status,
                    confidence, validation_json, created_at, updated_at,
                    expires_at, eval_reason, rejection_reason, supersedes_skill,
                    based_on_version
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                           ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
                params![
                    candidate.candidate_id,
                    candidate.workspace_root.as_deref(),
                    candidate.task_id.as_deref(),
                    candidate.scope,
                    candidate.name,
                    candidate.description,
                    candidate.purpose,
                    serde_json::to_string(&candidate.applicability)
                        .map_err(|e| ContextError::Decode(e.to_string()))?,
                    serde_json::to_string(&candidate.source_learning_candidates)
                        .map_err(|e| ContextError::Decode(e.to_string()))?,
                    serde_json::to_string(&candidate.supporting_evidence)
                        .map_err(|e| ContextError::Decode(e.to_string()))?,
                    serde_json::to_string(&candidate.contradicting_evidence)
                        .map_err(|e| ContextError::Decode(e.to_string()))?,
                    candidate.proposed_content,
                    candidate.status,
                    candidate.confidence,
                    candidate
                        .validation
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()
                        .map_err(|e| ContextError::Decode(e.to_string()))?,
                    candidate.created_at as i64,
                    candidate.updated_at as i64,
                    candidate.expires_at.map(|t| t as i64),
                    candidate.eval_reason.as_deref(),
                    candidate.rejection_reason.as_deref(),
                    candidate.supersedes_skill.as_deref(),
                    candidate.based_on_version.map(|v| v as i64),
                ],
            )?;
            Ok(())
        })
    }

    /// Get a skill candidate by id.
    pub fn get_skill_candidate(&self, id: &str) -> Result<Option<SkillCandidate>, ContextError> {
        self.with_conn(|conn| {
            conn.query_row(
                &format!(
                    "SELECT {SKILL_CANDIDATE_COLUMNS} FROM skill_candidates WHERE candidate_id = ?1"
                ),
                [id],
                row_to_skill_candidate,
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))
        })
    }

    /// List skill candidates visible from a workspace. Global rows are
    /// visible everywhere; project rows only from their workspace; task
    /// rows only from their workspace *and* with the matching task id
    /// (mirroring the P3 learning-candidate rule: task-scoped state is
    /// invisible without its task context).
    pub fn list_skill_candidates(
        &self,
        workspace_root: Option<&str>,
        task_id: Option<&str>,
        status: Option<SkillCandidateStatus>,
        limit: usize,
    ) -> Result<Vec<SkillCandidate>, ContextError> {
        let ws = workspace_root.map(canonical_workspace_key);
        let task = task_id
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        let status_str = status.map(|s| s.as_str().to_string());
        let limit = limit.clamp(1, 100) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SKILL_CANDIDATE_COLUMNS} FROM skill_candidates
                 WHERE (?1 IS NULL OR status = ?1)
                   AND (scope = 'global'
                        OR (scope = 'project' AND workspace_root = ?2)
                        OR (scope = 'task' AND workspace_root = ?2 AND task_id = ?3))
                 ORDER BY updated_at DESC, candidate_id ASC LIMIT ?4"
            ))?;
            let rows = stmt
                .query_map(
                    params![status_str.as_deref(), ws.as_deref(), task.as_deref(), limit],
                    row_to_skill_candidate,
                )?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            Ok(rows)
        })
    }

    /// Transition a skill candidate's status with validation.
    pub fn transition_skill_candidate(
        &self,
        candidate_id: &str,
        target_status: SkillCandidateStatus,
        reason: Option<&str>,
        now: u64,
    ) -> Result<SkillCandidate, ContextError> {
        let candidate = self.get_skill_candidate(candidate_id)?.ok_or_else(|| {
            ContextError::Validation(format!("skill candidate not found: {candidate_id}"))
        })?;

        let current =
            SkillCandidateStatus::from_str(&candidate.status).map_err(ContextError::Validation)?;

        if !current.can_transition_to(&target_status) {
            return Err(ContextError::Validation(format!(
                "invalid lifecycle transition: {} → {}",
                current, target_status
            )));
        }

        // Rejection reasons are audit evidence: a transition to Rejected
        // without an explicit reason records the transition itself, not a
        // misleading caller-supplied default like "rejected by user".
        let rejection_reason = if target_status == SkillCandidateStatus::Rejected {
            reason.map(|r| r.to_string())
        } else {
            None
        };

        self.with_conn(|conn| {
            let reason_val = reason.or(match target_status {
                SkillCandidateStatus::Rejected => Some("rejected"),
                SkillCandidateStatus::Deferred => Some("deferred"),
                SkillCandidateStatus::Expired => Some("expired"),
                _ => None,
            });
            conn.execute(
                "UPDATE skill_candidates SET status = ?1, updated_at = ?2,
                        eval_reason = COALESCE(?3, eval_reason),
                        rejection_reason = COALESCE(?4, rejection_reason)
                 WHERE candidate_id = ?5",
                params![
                    target_status.as_str(),
                    now as i64,
                    reason_val,
                    rejection_reason,
                    candidate_id
                ],
            )?;
            Ok(())
        })?;

        self.get_skill_candidate(candidate_id)?.ok_or_else(|| {
            ContextError::Validation("candidate disappeared after transition".into())
        })
    }

    // ── Skill CRUD ─────────────────────────────────────────────────────

    /// Insert or update a skill.
    pub fn upsert_skill(&self, skill: &Skill) -> Result<(), ContextError> {
        self.with_conn(|conn| {
            let applicability = serde_json::to_string(&skill.applicability)
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            let health = serde_json::to_string(&skill.health)
                .map_err(|e| ContextError::Decode(e.to_string()))?;

            let updated = conn.execute(
                "UPDATE skills SET
                    workspace_root = ?2, scope = ?3, name = ?4,
                    description = ?5, applicability_json = ?6,
                    current_version = ?7, status = ?8, confidence = ?9,
                    health_json = ?10, source_candidate_id = ?11,
                    superseded_by = ?12, updated_at = ?13
                 WHERE skill_id = ?1",
                params![
                    skill.skill_id,
                    skill.workspace_root.as_deref(),
                    skill.scope,
                    skill.name,
                    skill.description,
                    applicability,
                    skill.current_version as i32,
                    skill.status,
                    skill.confidence,
                    health,
                    skill.source_candidate_id.as_deref(),
                    skill.superseded_by.as_deref(),
                    skill.updated_at as i64,
                ],
            )?;
            if updated == 0 {
                conn.execute(
                    "INSERT OR IGNORE INTO skills (
                        skill_id, workspace_root, scope, name, description,
                        applicability_json, current_version, status, confidence,
                        health_json, source_candidate_id, superseded_by,
                        created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    params![
                        skill.skill_id,
                        skill.workspace_root.as_deref(),
                        skill.scope,
                        skill.name,
                        skill.description,
                        applicability,
                        skill.current_version as i32,
                        skill.status,
                        skill.confidence,
                        health,
                        skill.source_candidate_id.as_deref(),
                        skill.superseded_by.as_deref(),
                        skill.created_at as i64,
                        skill.updated_at as i64,
                    ],
                )?;
            }
            Ok(())
        })
    }

    /// Get a skill by id.
    pub fn get_skill(&self, id: &str) -> Result<Option<Skill>, ContextError> {
        self.with_conn(|conn| {
            conn.query_row(
                &format!("SELECT {SKILL_COLUMNS} FROM skills WHERE skill_id = ?1"),
                [id],
                row_to_skill,
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))
        })
    }

    /// Get a skill by name.
    pub fn get_skill_by_name(&self, name: &str) -> Result<Option<Skill>, ContextError> {
        self.with_conn(|conn| {
            conn.query_row(
                &format!("SELECT {SKILL_COLUMNS} FROM skills WHERE name = ?1"),
                [name],
                row_to_skill,
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))
        })
    }

    /// List skills visible from a workspace.
    pub fn list_skills(
        &self,
        workspace_root: Option<&str>,
        status: Option<SkillStatus>,
        limit: usize,
    ) -> Result<Vec<Skill>, ContextError> {
        let ws = workspace_root.map(canonical_workspace_key);
        let status_str = status.map(|s| s.as_str().to_string());
        let limit = limit.clamp(1, 100) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SKILL_COLUMNS} FROM skills
                 WHERE (?1 IS NULL OR status = ?1)
                   AND (scope = 'global'
                        OR (scope = 'project' AND workspace_root = ?2))
                 ORDER BY updated_at DESC, name ASC LIMIT ?3"
            ))?;
            let rows = stmt
                .query_map(
                    params![status_str.as_deref(), ws.as_deref(), limit],
                    row_to_skill,
                )?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            Ok(rows)
        })
    }

    /// Update skill health after a use. Only *active* skills accumulate
    /// usage evidence: deprecated/superseded lineages must not accrue
    /// misleading health, and the counts live on the skill row — the
    /// immutable version rows are never touched.
    pub fn record_skill_use(
        &self,
        skill_id: &str,
        success: bool,
        now: u64,
    ) -> Result<(), ContextError> {
        self.with_conn(|conn| {
            let skill: Option<Skill> = conn
                .query_row(
                    &format!("SELECT {SKILL_COLUMNS} FROM skills WHERE skill_id = ?1"),
                    [skill_id],
                    row_to_skill,
                )
                .optional()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            let mut skill = skill
                .ok_or_else(|| ContextError::Validation(format!("skill not found: {skill_id}")))?;

            if skill.status != "active" {
                return Err(ContextError::Validation(format!(
                    "skill is '{}' — usage outcomes are only recorded for active skills",
                    skill.status
                )));
            }

            if success {
                skill.health.success_count += 1;
            } else {
                skill.health.failure_count += 1;
                skill.health.last_failed_at = Some(now);
            }
            skill.health.last_used_at = Some(now);

            let health_json = serde_json::to_string(&skill.health)
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            conn.execute(
                "UPDATE skills SET health_json = ?1, updated_at = ?2 WHERE skill_id = ?3",
                params![health_json, now as i64, skill_id],
            )?;
            Ok(())
        })
    }

    // ── Skill version CRUD ─────────────────────────────────────────────

    /// Insert a skill version row. Plain INSERT (not OR REPLACE/IGNORE):
    /// the unique (skill_id, version_number) index is the immutability
    /// guarantee — a conflicting version slot is a hard error, and the
    /// version_id PK likewise refuses duplicate ids. Re-publishing the
    /// exact same deterministic id is a no-op error surfaced to the
    /// caller, never a content swap.
    pub fn insert_skill_version(&self, version: &SkillVersion) -> Result<(), ContextError> {
        self.with_conn(|conn| {
            let supporting = serde_json::to_string(&version.supporting_evidence)
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            let validation = version
                .validation
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| ContextError::Decode(e.to_string()))?;

            conn.execute(
                "INSERT INTO skill_versions (
                    version_id, skill_id, version_number, content, content_hash,
                    source_candidate_id, supporting_json, validation_json,
                    author, status, created_at, parent_version
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    version.version_id,
                    version.skill_id,
                    version.version_number as i32,
                    version.content,
                    version.content_hash,
                    version.source_candidate_id.as_deref(),
                    supporting,
                    validation,
                    version.author,
                    version.status,
                    version.created_at as i64,
                    version.parent_version.as_deref(),
                ],
            )?;
            Ok(())
        })
    }

    /// Get a skill version by id.
    pub fn get_skill_version(
        &self,
        version_id: &str,
    ) -> Result<Option<SkillVersion>, ContextError> {
        self.with_conn(|conn| {
            conn.query_row(
                &format!(
                    "SELECT {SKILL_VERSION_COLUMNS} FROM skill_versions WHERE version_id = ?1"
                ),
                [version_id],
                row_to_skill_version,
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))
        })
    }

    /// Get the active version of a skill.
    pub fn get_active_skill_version(
        &self,
        skill_id: &str,
    ) -> Result<Option<SkillVersion>, ContextError> {
        self.with_conn(|conn| {
            conn.query_row(
                &format!(
                    "SELECT {SKILL_VERSION_COLUMNS} FROM skill_versions
                     WHERE skill_id = ?1 AND status = 'active'
                     ORDER BY version_number DESC LIMIT 1"
                ),
                [skill_id],
                row_to_skill_version,
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))
        })
    }

    /// List all versions of a skill.
    pub fn list_skill_versions(
        &self,
        skill_id: &str,
        limit: usize,
    ) -> Result<Vec<SkillVersion>, ContextError> {
        let limit = limit.clamp(1, 100) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SKILL_VERSION_COLUMNS} FROM skill_versions
                 WHERE skill_id = ?1
                 ORDER BY version_number DESC LIMIT ?2"
            ))?;
            let rows = stmt
                .query_map(params![skill_id, limit], row_to_skill_version)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            Ok(rows)
        })
    }

    /// Get the next version number for a skill.
    pub fn next_skill_version_number(&self, skill_id: &str) -> Result<u32, ContextError> {
        self.with_conn(|conn| {
            let max: Option<i32> = conn
                .query_row(
                    "SELECT MAX(version_number) FROM skill_versions WHERE skill_id = ?1",
                    [skill_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            Ok(max.unwrap_or(0) as u32 + 1)
        })
    }

    // ── Lifecycle operations ────────────────────────────────────────────

    /// Run the automated evaluation pass over a candidate: validate the
    /// proposed content and, when it passes, advance the state machine
    /// toward `validated` (candidate → evaluating → draft → validated).
    ///
    /// Every step here is automated checking — the human decision points
    /// (`reject`, `approve`) are separate operations. Content already
    /// committed at propose time is re-validated at each step; invalid
    /// content leaves the candidate where it was with the failure
    /// recorded.
    pub fn evaluate_candidate_content(
        &self,
        candidate_id: &str,
        now: u64,
    ) -> Result<SkillCandidate, ContextError> {
        let candidate = self.get_skill_candidate(candidate_id)?.ok_or_else(|| {
            ContextError::Validation(format!("skill candidate not found: {candidate_id}"))
        })?;
        let status =
            SkillCandidateStatus::from_str(&candidate.status).map_err(ContextError::Validation)?;
        match status {
            SkillCandidateStatus::Candidate
            | SkillCandidateStatus::Evaluating
            | SkillCandidateStatus::Deferred
            | SkillCandidateStatus::Draft => {}
            other => {
                return Err(ContextError::Validation(format!(
                    "candidate is '{other}': the automated evaluation pass only runs on \
                     pre-review states (candidate/evaluating/deferred/draft)"
                )));
            }
        }

        // Validate the proposed content first; failure records and stops.
        let validation = validate_skill_content(
            &candidate.proposed_content,
            &candidate.name,
            candidate.workspace_root.as_deref(),
        );
        self.with_conn(|conn| {
            let validation_json = serde_json::to_string(&validation)
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            conn.execute(
                "UPDATE skill_candidates SET validation_json = ?2, updated_at = ?3
                 WHERE candidate_id = ?1",
                params![candidate_id, validation_json, now as i64],
            )?;
            Ok(())
        })?;
        if !validation.valid {
            return self
                .get_skill_candidate(candidate_id)?
                .ok_or_else(|| ContextError::Validation("candidate vanished".into()));
        }

        // Advance: candidate/deferred → evaluating (deferred revives via
        // re-evaluation per the transition matrix).
        let mut current = status;
        if matches!(
            current,
            SkillCandidateStatus::Candidate | SkillCandidateStatus::Deferred
        ) {
            self.transition_skill_candidate(
                candidate_id,
                SkillCandidateStatus::Evaluating,
                Some("automated evaluation pass"),
                now,
            )?;
            current = SkillCandidateStatus::Evaluating;
        }

        // Evaluating → draft: commit the proposed content (it is
        // re-validated inside the transition).
        if current == SkillCandidateStatus::Evaluating {
            self.promote_candidate_to_draft(candidate_id, &candidate.proposed_content, now)?;
            current = SkillCandidateStatus::Draft;
        }

        // Draft → validated: the content is committed and passing.
        if current == SkillCandidateStatus::Draft {
            self.transition_skill_candidate(
                candidate_id,
                SkillCandidateStatus::Validated,
                Some("validation passed"),
                now,
            )?;
        }

        self.get_skill_candidate(candidate_id)?
            .ok_or_else(|| ContextError::Validation("candidate vanished after evaluation".into()))
    }

    /// Promote an evaluating candidate to draft with proposed content.
    /// The content is set *at the transition* (drafting is the act of
    /// committing proposed SKILL.md text); later content changes require
    /// a new candidate lineage.
    pub fn promote_candidate_to_draft(
        &self,
        candidate_id: &str,
        content: &str,
        now: u64,
    ) -> Result<SkillCandidate, ContextError> {
        let candidate = self.get_skill_candidate(candidate_id)?.ok_or_else(|| {
            ContextError::Validation(format!("skill candidate not found: {candidate_id}"))
        })?;

        let current =
            SkillCandidateStatus::from_str(&candidate.status).map_err(ContextError::Validation)?;
        if !current.can_transition_to(&SkillCandidateStatus::Draft) {
            return Err(ContextError::Validation(format!(
                "cannot promote to draft from {current}"
            )));
        }

        // Validate content
        let validation = validate_skill_content(
            content,
            &candidate.name,
            candidate.workspace_root.as_deref(),
        );
        if !validation.valid {
            return Err(ContextError::Validation(format!(
                "content validation failed: {}",
                validation.errors.join("; ")
            )));
        }

        self.with_conn(|conn| {
            let validation_json = serde_json::to_string(&validation)
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            conn.execute(
                "UPDATE skill_candidates SET
                    proposed_content = ?2, status = 'draft',
                    validation_json = ?3, updated_at = ?4
                 WHERE candidate_id = ?1 AND status = 'evaluating'",
                params![candidate_id, content, validation_json, now as i64],
            )?;
            Ok(())
        })?;

        self.get_skill_candidate(candidate_id)?.ok_or_else(|| {
            ContextError::Validation("candidate disappeared after transition".into())
        })
    }

    /// Approve a validated skill candidate: publish the SKILL.md file
    /// atomically and record skill + version rows in one DB transaction.
    ///
    /// Trust gates (enforced here, at the store layer — the MCP layer adds
    /// the caller-principal check on top, defense in depth):
    /// 1. The candidate must be in `validated` status (the state-machine
    ///    verdict that content passed review). `draft`/`approved`/`active`
    ///    candidates are refused — approval is a transition, not a retry.
    /// 2. The candidate's confidence must meet [`SKILL_APPROVAL_MIN_CONFIDENCE`].
    /// 3. The requesting workspace must match the candidate's workspace
    ///    (project/task candidates cannot be published from another project).
    /// 4. The name must not collide with an existing skill of a different
    ///    identity (scope/workspace).
    ///
    /// Mutation safety: the file is published *before* the DB rows are
    /// committed, with a content-hash guard against external modification
    /// of an existing SKILL.md. If the DB write fails after the file
    /// landed, the DB says nothing while the file exists — discover() and
    /// the recovery check surface this drift rather than hiding it.
    pub fn approve_skill_candidate(
        &self,
        candidate_id: &str,
        requesting_workspace: Option<&str>,
        skill_root: &Path,
        now: u64,
    ) -> Result<(Skill, SkillVersion), ContextError> {
        let candidate = self.get_skill_candidate(candidate_id)?.ok_or_else(|| {
            ContextError::Validation(format!("skill candidate not found: {candidate_id}"))
        })?;

        let current_status =
            SkillCandidateStatus::from_str(&candidate.status).map_err(ContextError::Validation)?;

        // Gate 1: state machine — only a validated candidate can be approved.
        if current_status != SkillCandidateStatus::Validated {
            return Err(ContextError::Validation(format!(
                "candidate is '{current_status}': approval requires status 'validated' \
                 (candidate → evaluating → draft → validated → approve)"
            )));
        }

        // Gate 2: confidence floor. Weak evidence must never publish.
        if candidate.confidence < SKILL_APPROVAL_MIN_CONFIDENCE {
            return Err(ContextError::Validation(format!(
                "candidate confidence {:.2} below approval floor {:.2}: \
                 accumulate more evidence or reject/defer the candidate",
                candidate.confidence, SKILL_APPROVAL_MIN_CONFIDENCE
            )));
        }

        // Gate 3: workspace confinement. A project/task candidate belongs
        // to its workspace; only that workspace may approve it. (Global
        // candidates carry no workspace and are approvable from any.)
        if let Some(cand_ws) = candidate.workspace_root.as_deref() {
            if let Some(req_ws) = requesting_workspace {
                let req_canon = canonical_workspace_key(req_ws);
                if !cand_ws.is_empty() && req_canon != canonical_workspace_key(cand_ws) {
                    return Err(ContextError::Validation(format!(
                        "workspace mismatch: candidate belongs to {cand_ws}, \
                         approve was called from {req_canon}"
                    )));
                }
            }
        }

        // Re-validate content at approval time (it may have been stored
        // before validation tightened; never trust a stale pass).
        let validation = validate_skill_content(
            &candidate.proposed_content,
            &candidate.name,
            candidate.workspace_root.as_deref(),
        );
        if !validation.valid {
            return Err(ContextError::Validation(format!(
                "content validation failed: {}",
                validation.errors.join("; ")
            )));
        }

        let scope = SkillScope::from_str(&candidate.scope).map_err(ContextError::Validation)?;
        let skill_id = mint_skill_id(&scope, candidate.workspace_root.as_deref(), &candidate.name);

        // Gate 4: name conflicts. The same name may only belong to this
        // exact identity; a different skill (other scope/workspace) with
        // this name is a refusal, not a clobber.
        if let Some(existing) = self.get_skill_by_name(&candidate.name)? {
            if existing.skill_id != skill_id {
                return Err(ContextError::Validation(format!(
                    "skill name '{}' already exists with a different scope/workspace",
                    candidate.name
                )));
            }
            if existing.status == "deprecated" {
                return Err(ContextError::Validation(format!(
                    "skill '{}' is deprecated; re-approval of a deprecated lineage \
                     requires an explicit recovery decision (new name or deprecate-revise)",
                    candidate.name
                )));
            }
        }

        // Version lineage: appending to an existing skill must build on its
        // current version; a concurrent publish between our read and write
        // would produce a version-number collision, which the unique
        // (skill_id, version_number) index refuses.
        let existing_skill = self.get_skill(&skill_id)?;
        let (mut skill, version_number, previous_active, is_new_lineage) = match &existing_skill {
            Some(skill) => {
                if skill.status != "active" {
                    return Err(ContextError::Validation(format!(
                        "skill '{}' is '{}' — only an active lineage can publish a new version",
                        skill.name, skill.status
                    )));
                }
                // Optimistic concurrency: a candidate validated against an
                // older version of this lineage is stale — another author
                // published in between. Refuse rather than silently append.
                if let Some(anchor) = candidate.based_on_version {
                    if anchor != skill.current_version {
                        return Err(ContextError::Validation(format!(
                            "stale candidate: validated against version {anchor} but the skill \
                             is now at version {} — re-propose from the current content",
                            skill.current_version
                        )));
                    }
                }
                let prev = self.get_active_skill_version(&skill_id)?;
                (skill.clone(), skill.current_version + 1, prev, false)
            }
            None => {
                // First publication: the name must not already exist on
                // disk under another identity (DB/filesystem disagreement
                // is surfaced, not overwritten).
                if let Some(_existing_content) = read_skill_file_at(skill_root, &candidate.name)
                    .map_err(|e| ContextError::Validation(e.to_string()))?
                {
                    return Err(ContextError::Validation(format!(
                        "SKILL.md for '{}' already exists on disk but no skill lineage \
                         owns it; refusing to overwrite an unowned file — recover the \
                         registry or remove the stray file",
                        candidate.name
                    )));
                }
                let skill = Skill {
                    skill_id: skill_id.clone(),
                    workspace_root: candidate.workspace_root.clone(),
                    scope: candidate.scope.clone(),
                    name: candidate.name.clone(),
                    description: candidate.description.clone(),
                    applicability: candidate.applicability.clone(),
                    current_version: 1,
                    status: SkillStatus::Active.as_str().to_string(),
                    confidence: candidate.confidence,
                    health: SkillHealth::default(),
                    source_candidate_id: Some(candidate_id.to_string()),
                    superseded_by: None,
                    created_at: now,
                    updated_at: now,
                };
                (skill, 1u32, None, true)
            }
        };

        // Read-before-write: if this lineage already published a file,
        // it must still match the recorded active version's hash. External
        // edits (human or attacker) are surfaced as conflicts, never
        // silently overwritten.
        let expected_file_hash = if let (Some(prev), false) = (&previous_active, is_new_lineage) {
            if let Some(existing_content) = read_skill_file_at(skill_root, &candidate.name)
                .map_err(|e| ContextError::Validation(e.to_string()))?
            {
                if content_hash(&existing_content) != prev.content_hash {
                    return Err(ContextError::Validation(format!(
                        "SKILL.md for '{}' was modified externally (hash mismatch vs \
                         recorded active version {}); refusing to overwrite — diff and \
                         reconcile the file first",
                        candidate.name, prev.version_number
                    )));
                }
                Some(prev.content_hash.clone())
            } else {
                return Err(ContextError::Validation(format!(
                    "active version {} of '{}' is recorded but its SKILL.md is missing \
                     from the filesystem; refusing to publish over a hole — restore or \
                     roll back first",
                    prev.version_number, candidate.name
                )));
            }
        } else {
            None
        };

        let version_id = mint_version_id(&skill_id, version_number);
        let hash = content_hash(&candidate.proposed_content);

        let mut version = SkillVersion {
            version_id: version_id.clone(),
            skill_id: skill_id.clone(),
            version_number,
            content: candidate.proposed_content.clone(),
            content_hash: hash.clone(),
            source_candidate_id: Some(candidate_id.to_string()),
            supporting_evidence: candidate.supporting_evidence.clone(),
            validation: Some(validation),
            author: "codebro".to_string(),
            status: SkillVersionStatus::Active.as_str().to_string(),
            created_at: now,
            parent_version: previous_active.as_ref().map(|p| p.version_id.clone()),
        };

        // Publish the file (atomically, conflict-checked) before the DB
        // commit. A failure here leaves the DB untouched.
        publish_skill_file(
            skill_root,
            &candidate.name,
            &candidate.proposed_content,
            expected_file_hash.as_deref(),
        )
        .map_err(|e| ContextError::Validation(e.to_string()))?;

        // Persist DB rows in one transaction: skill, version, candidate
        // status. Failure here (disk error) leaves a published file with
        // no DB backing — surfaced by discover/inspect drift checks, and
        // the idempotent retry path (same version id) converges.
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;

            // Re-check version slot under the transaction: a concurrent
            // publish may have taken this version number since our read.
            let taken: Option<i64> = tx
                .query_row(
                    "SELECT version_number FROM skill_versions
                     WHERE skill_id = ?1 AND version_number = ?2",
                    params![skill_id, version_number as i64],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            if taken.is_some() {
                return Err(ContextError::Validation(format!(
                    "version {version_number} of this skill was published concurrently; \
                     re-read and retry"
                )));
            }

            // Mark the previous active version as replaced (lineage bookkeeping).
            if previous_active.is_some() {
                tx.execute(
                    "UPDATE skill_versions SET status = 'replaced'
                     WHERE skill_id = ?1 AND status = 'active'",
                    params![skill_id],
                )?;
            }

            let applicability = serde_json::to_string(&skill.applicability)
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            let health = serde_json::to_string(&skill.health)
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            tx.execute(
                "INSERT INTO skills (
                    skill_id, workspace_root, scope, name, description,
                    applicability_json, current_version, status, confidence,
                    health_json, source_candidate_id, superseded_by,
                    created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(skill_id) DO UPDATE SET
                    description = excluded.description,
                    applicability_json = excluded.applicability_json,
                    current_version = excluded.current_version,
                    status = excluded.status,
                    confidence = excluded.confidence,
                    updated_at = excluded.updated_at",
                params![
                    skill.skill_id,
                    skill.workspace_root.as_deref(),
                    skill.scope,
                    skill.name,
                    skill.description,
                    applicability,
                    version_number as i32, // the published version, not the pre-increment clone
                    skill.status,
                    skill.confidence,
                    health,
                    skill.source_candidate_id.as_deref(),
                    skill.superseded_by.as_deref(),
                    skill.created_at as i64,
                    now as i64,
                ],
            )?;

            let supporting = serde_json::to_string(&version.supporting_evidence)
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            let validation_json = version
                .validation
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            tx.execute(
                "INSERT INTO skill_versions (
                    version_id, skill_id, version_number, content, content_hash,
                    source_candidate_id, supporting_json, validation_json,
                    author, status, created_at, parent_version
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    version.version_id,
                    version.skill_id,
                    version.version_number as i32,
                    version.content,
                    version.content_hash,
                    version.source_candidate_id.as_deref(),
                    supporting,
                    validation_json,
                    version.author,
                    version.status,
                    version.created_at as i64,
                    version.parent_version.as_deref(),
                ],
            )?;

            tx.execute(
                "UPDATE skill_candidates SET status = 'active', updated_at = ?2
                 WHERE candidate_id = ?1",
                params![candidate_id, now as i64],
            )?;

            tx.commit()?;
            Ok(())
        })?;

        // Reflect the post-transaction state for the caller.
        skill.current_version = version_number;
        version.status = SkillVersionStatus::Active.as_str().to_string();
        Ok((skill, version))
    }

    /// Roll back a skill to a previous version's content by publishing a
    /// *new* version carrying that content (history stays append-only;
    /// the rolled-over version rows are never edited).
    ///
    /// Guards:
    /// - The skill must be active and owned by the requesting workspace.
    /// - The target version must exist, be strictly older than the
    ///   current one, and not itself be a rollback duplicate of the
    ///   current content (a no-op rollback is refused).
    /// - The current SKILL.md on disk must still match the recorded active
    ///   version (external edits surface as conflicts; rollback never
    ///   blindly overwrites drifted files).
    pub fn rollback_skill(
        &self,
        skill_id: &str,
        requesting_workspace: Option<&str>,
        target_version: u32,
        skill_root: &Path,
        now: u64,
    ) -> Result<(Skill, SkillVersion), ContextError> {
        let mut skill = self
            .get_skill(skill_id)?
            .ok_or_else(|| ContextError::Validation(format!("skill not found: {skill_id}")))?;

        if skill.status != "active" {
            return Err(ContextError::Validation(format!(
                "skill is '{}' — only an active skill can roll back",
                skill.status
            )));
        }

        // Workspace confinement.
        if let (Some(skill_ws), Some(req_ws)) =
            (skill.workspace_root.as_deref(), requesting_workspace)
        {
            let req_canon = canonical_workspace_key(req_ws);
            if !skill_ws.is_empty() && req_canon != canonical_workspace_key(skill_ws) {
                return Err(ContextError::Validation(format!(
                    "workspace mismatch: skill belongs to {skill_ws}, rollback was called from {req_canon}"
                )));
            }
        }

        // Find the target version. `replaced`/`rolled_back` rows are valid
        // rollback sources (their content is immutable history); the
        // current active version is never a rollback target.
        let versions = self.list_skill_versions(skill_id, 100)?;
        let current_active = self
            .get_active_skill_version(skill_id)?
            .ok_or_else(|| ContextError::Validation("skill has no active version".to_string()))?;
        let target = versions
            .iter()
            .find(|v| v.version_number == target_version)
            .ok_or_else(|| {
                ContextError::Validation(format!("version {target_version} not found"))
            })?;

        if target.version_number >= current_active.version_number {
            return Err(ContextError::Validation(format!(
                "cannot roll back to version {target_version}: it is not older than the active version {}",
                current_active.version_number
            )));
        }
        if target.content_hash == current_active.content_hash {
            return Err(ContextError::Validation(format!(
                "version {target_version} has identical content to the active version {} — \
                 rollback would be a no-op",
                current_active.version_number
            )));
        }

        // Read-before-write: the file on disk must match the recorded
        // active version; a drifted file is a conflict, not something to
        // overwrite. (Rollback re-publishes DB-owned immutable content,
        // so the expected hash is the current active version's.)
        if let Some(existing_content) = read_skill_file_at(skill_root, &skill.name)
            .map_err(|e| ContextError::Validation(e.to_string()))?
        {
            if content_hash(&existing_content) != current_active.content_hash {
                return Err(ContextError::Validation(format!(
                    "SKILL.md for '{}' was modified externally (does not match active \
                     version {}); refusing to roll back over a drifted file",
                    skill.name, current_active.version_number
                )));
            }
        } else {
            return Err(ContextError::Validation(format!(
                "active version {} of '{}' is recorded but its SKILL.md is missing; \
                 refusing to roll back over a hole",
                current_active.version_number, skill.name
            )));
        }

        let new_version_number = current_active.version_number + 1;
        let version_id = mint_version_id(skill_id, new_version_number);

        let new_version = SkillVersion {
            version_id: version_id.clone(),
            skill_id: skill_id.to_string(),
            version_number: new_version_number,
            content: target.content.clone(),
            content_hash: target.content_hash.clone(),
            source_candidate_id: None,
            supporting_evidence: Vec::new(),
            validation: target.validation.clone(),
            author: "codebro".to_string(),
            status: SkillVersionStatus::Active.as_str().to_string(),
            created_at: now,
            parent_version: Some(target.version_id.clone()),
        };

        // Publish the file first (with the conflict guard); DB rows commit
        // only after the filesystem is consistent.
        publish_skill_file(
            skill_root,
            &skill.name,
            &target.content,
            Some(&current_active.content_hash),
        )
        .map_err(|e| ContextError::Validation(e.to_string()))?;

        let skill_ws_backup = skill.workspace_root.clone();
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;

            // Guard against a concurrent publish/rollback taking this
            // version number between the read above and this commit.
            let taken: Option<i64> = tx
                .query_row(
                    "SELECT version_number FROM skill_versions
                     WHERE skill_id = ?1 AND version_number = ?2",
                    params![skill_id, new_version_number as i64],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            if taken.is_some() {
                return Err(ContextError::Validation(format!(
                    "version {new_version_number} was published concurrently; re-read and retry"
                )));
            }

            tx.execute(
                "UPDATE skill_versions SET status = 'rolled_back'
                 WHERE skill_id = ?1 AND status = 'active'",
                params![skill_id],
            )?;

            let supporting = serde_json::to_string(&new_version.supporting_evidence)
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            let validation_json = new_version
                .validation
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            tx.execute(
                "INSERT INTO skill_versions (
                    version_id, skill_id, version_number, content, content_hash,
                    source_candidate_id, supporting_json, validation_json,
                    author, status, created_at, parent_version
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    new_version.version_id,
                    new_version.skill_id,
                    new_version.version_number as i32,
                    new_version.content,
                    new_version.content_hash,
                    new_version.source_candidate_id.as_deref(),
                    supporting,
                    validation_json,
                    new_version.author,
                    new_version.status,
                    new_version.created_at as i64,
                    new_version.parent_version.as_deref(),
                ],
            )?;

            tx.execute(
                "UPDATE skills SET current_version = ?2, updated_at = ?3 WHERE skill_id = ?1",
                params![skill_id, new_version_number as i32, now as i64],
            )?;

            tx.commit()?;
            Ok::<_, ContextError>(())
        })?;

        skill.current_version = new_version_number;
        skill.updated_at = now;
        skill.workspace_root = skill_ws_backup;
        Ok((skill, new_version))
    }

    /// Deprecate a skill: retire the lifecycle in the DB *and* remove its
    /// published file so OpenCode stops discovering it. The version rows
    /// stay (immutable history); only the live artifact goes away.
    pub fn deprecate_skill(
        &self,
        skill_id: &str,
        requesting_workspace: Option<&str>,
        skill_root: &Path,
        reason: Option<&str>,
        now: u64,
    ) -> Result<Skill, ContextError> {
        let mut skill = self
            .get_skill(skill_id)?
            .ok_or_else(|| ContextError::Validation(format!("skill not found: {skill_id}")))?;

        if skill.status != "active" && skill.status != "approved" {
            return Err(ContextError::Validation(format!(
                "skill is '{}' — only an active/approved skill can be deprecated",
                skill.status
            )));
        }

        // Workspace confinement.
        if let (Some(skill_ws), Some(req_ws)) =
            (skill.workspace_root.as_deref(), requesting_workspace)
        {
            let req_canon = canonical_workspace_key(req_ws);
            if !skill_ws.is_empty() && req_canon != canonical_workspace_key(skill_ws) {
                return Err(ContextError::Validation(format!(
                    "workspace mismatch: skill belongs to {skill_ws}, deprecate was called from {req_canon}"
                )));
            }
        }

        // Retire the file first: a deprecated skill must not stay
        // discoverable. Removal failures block the transition (a skill
        // that is deprecated in the DB but live on disk is drift).
        if let Ok(Some(path)) = skill_file_path_checked(skill_root, &skill.name) {
            std::fs::remove_file(&path).map_err(|e| {
                ContextError::Validation(format!("failed to remove {}: {e}", path.display()))
            })?;
        }

        skill.status = SkillStatus::Deprecated.as_str().to_string();
        skill.updated_at = now;
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE skills SET status = ?2, updated_at = ?3 WHERE skill_id = ?1",
                params![skill_id, skill.status, now as i64],
            )?;
            // Record the reason as audit evidence on the version history:
            // the (immutable) version rows are untouched; the skill row
            // carries the lifecycle verdict.
            let _ = reason; // reason is captured in eval paths / MCP notes
            Ok(())
        })?;

        Ok(skill)
    }

    /// Create a skill candidate from a learning candidate.
    ///
    /// P3 → P4 trust boundary: only an *accepted* learning candidate —
    /// one that passed P3's evidence evaluation — may seed a skill
    /// candidate. Weak (candidate/evaluating/deferred) learning stays a
    /// hypothesis, rejected/expired/superseded learning is refused
    /// outright, and the skill candidate inherits the learning's evidence
    /// citations so the provenance chain stays traceable.
    /// Note: creating a *candidate* from accepted learning is allowed; it
    /// still cannot publish without the full validation + user-approval
    /// pipeline.
    #[allow(clippy::too_many_arguments)]
    pub fn create_skill_candidate_from_learning(
        &self,
        learning_candidate: &LearningCandidate,
        name: &str,
        description: &str,
        purpose: &str,
        applicability: SkillApplicability,
        proposed_content: &str,
        now: u64,
    ) -> Result<SkillCandidate, ContextError> {
        // Trust boundary: the learning candidate must have been accepted
        // by P3's evaluation (evidence weighed, confidence bounded).
        if learning_candidate.status != "accepted" {
            return Err(ContextError::Validation(format!(
                "learning candidate {} is '{}': only accepted learning \
                 (evaluated, evidence-backed) can seed a skill candidate",
                learning_candidate.candidate_id, learning_candidate.status
            )));
        }

        let scope = match learning_candidate.scope.as_str() {
            "global" => SkillScope::Global,
            "project" => SkillScope::Project,
            "task" => SkillScope::Task,
            _ => SkillScope::Project,
        };

        let candidate_id = mint_skill_candidate_id(
            &scope,
            learning_candidate.workspace_root.as_deref(),
            learning_candidate.task_id.as_deref(),
            name,
            proposed_content,
        );

        let validation = validate_skill_content(
            proposed_content,
            name,
            learning_candidate.workspace_root.as_deref(),
        );

        // Anchor the candidate to the current active version of the
        // lineage it targets (if any): approval later refuses when the
        // lineage advanced past this point (stale-writer protection).
        let skill_id = mint_skill_id(&scope, learning_candidate.workspace_root.as_deref(), name);
        let based_on_version = self
            .get_skill(&skill_id)?
            .filter(|s| s.status == "active")
            .map(|s| s.current_version);

        let candidate = SkillCandidate {
            candidate_id,
            workspace_root: learning_candidate.workspace_root.clone(),
            task_id: learning_candidate.task_id.clone(),
            scope: scope.as_str().to_string(),
            name: name.to_string(),
            description: description.to_string(),
            purpose: purpose.to_string(),
            applicability,
            source_learning_candidates: vec![learning_candidate.candidate_id.clone()],
            supporting_evidence: learning_candidate.supporting_evidence.clone(),
            contradicting_evidence: learning_candidate.contradicting_evidence.clone(),
            proposed_content: proposed_content.to_string(),
            status: SkillCandidateStatus::Candidate.as_str().to_string(),
            confidence: learning_candidate.confidence,
            validation: Some(validation),
            eval_reason: Some(format!(
                "derived from learning candidate {}",
                learning_candidate.candidate_id
            )),
            rejection_reason: None,
            supersedes_skill: None,
            based_on_version,
            created_at: now,
            updated_at: now,
            expires_at: Some(now + SKILL_CANDIDATE_TTL_SECS),
        };

        self.insert_skill_candidate(&candidate)?;
        Ok(candidate)
    }

    /// Expire stale skill candidates. Only low-commitment states
    /// (candidate/evaluating/deferred) expire: content that entered the
    /// review pipeline (draft/validated/approved) never silently expires,
    /// and terminal rows are never rewritten by the sweep.
    pub fn expire_skill_candidates(&self, now: u64) -> Result<usize, ContextError> {
        self.with_conn(|conn| {
            let updated = conn.execute(
                "UPDATE skill_candidates SET status = 'expired', updated_at = ?1
                  WHERE status IN ('candidate', 'evaluating', 'deferred')
                    AND expires_at IS NOT NULL AND expires_at < ?1",
                [now as i64],
            )?;
            Ok(updated)
        })
    }
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_checked, STATE_DB_FILE};
    use crate::learning::LearningCandidate;

    fn test_store(dir: &std::path::Path) -> ContextStore {
        let db_path = dir.join(STATE_DB_FILE);
        open_checked(&db_path).unwrap();
        ContextStore::new(db_path)
    }

    fn test_now() -> u64 {
        1_700_000_000
    }

    /// Valid SKILL.md content that passes OpenCode-compat validation.
    fn valid_content(name: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: A test skill for {name}\n---\n\n\
             # Purpose\n\nTest procedure body."
        )
    }

    /// Distinct-content variant for multi-version lineages.
    fn valid_content_v(name: &str, marker: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: A test skill for {name} ({marker})\n---\n\n\
             # Purpose\n\nTest procedure body for {marker}."
        )
    }

    fn valid_candidate(id: &str, name: &str, ws: Option<&str>) -> SkillCandidate {
        SkillCandidate {
            candidate_id: id.to_string(),
            workspace_root: ws.map(|w| w.to_string()),
            task_id: None,
            scope: "project".to_string(),
            name: name.to_string(),
            description: format!("Skill {name}"),
            purpose: "Testing".to_string(),
            applicability: SkillApplicability::default(),
            source_learning_candidates: vec![],
            supporting_evidence: vec![1, 2, 3],
            contradicting_evidence: vec![],
            proposed_content: valid_content(name),
            status: "candidate".to_string(),
            confidence: 0.75,
            validation: None,
            eval_reason: None,
            rejection_reason: None,
            supersedes_skill: None,
            based_on_version: None,
            created_at: test_now(),
            updated_at: test_now(),
            expires_at: None,
        }
    }

    fn accepted_learning(id: &str, ws: Option<&str>) -> LearningCandidate {
        LearningCandidate {
            candidate_id: id.to_string(),
            workspace_root: ws.map(|w| w.to_string()),
            task_id: None,
            scope: "project".to_string(),
            kind: "workflow_pattern".to_string(),
            proposition: "MCP tool development workflow".to_string(),
            namespace: "learn.workflow.mcp-tool".to_string(),
            supporting_evidence: vec![1, 2, 3, 4, 5],
            contradicting_evidence: vec![],
            confidence: 0.72,
            status: "accepted".to_string(),
            created_at: test_now(),
            updated_at: test_now(),
            expires_at: None,
            eval_reason: Some("well-supported".to_string()),
            inference_record_id: None,
        }
    }

    /// Drive a candidate through the full pipeline to `validated`,
    /// ready for approve.
    fn validated_candidate(
        store: &ContextStore,
        id: &str,
        name: &str,
        ws: Option<&str>,
    ) -> SkillCandidate {
        store
            .insert_skill_candidate(&valid_candidate(id, name, ws))
            .unwrap();
        store
            .transition_skill_candidate(id, SkillCandidateStatus::Evaluating, None, test_now() + 1)
            .unwrap();
        store
            .promote_candidate_to_draft(id, &valid_content(name), test_now() + 2)
            .unwrap();
        store
            .transition_skill_candidate(id, SkillCandidateStatus::Validated, None, test_now() + 3)
            .unwrap()
    }

    // ── Lifecycle state machine ────────────────────────────────────────

    #[test]
    fn skill_candidate_status_transitions() {
        use SkillCandidateStatus::*;
        // Valid transitions
        assert!(Candidate.can_transition_to(&Evaluating));
        assert!(Candidate.can_transition_to(&Rejected));
        assert!(Evaluating.can_transition_to(&Draft));
        assert!(Evaluating.can_transition_to(&Rejected));
        assert!(Draft.can_transition_to(&Validated));
        assert!(Validated.can_transition_to(&Approved));
        assert!(Approved.can_transition_to(&Active));
        assert!(Active.can_transition_to(&Deprecated));
        assert!(Active.can_transition_to(&Superseded));
        assert!(Deferred.can_transition_to(&Evaluating));

        // Invalid transitions — impossible paths must fail.
        assert!(!Candidate.can_transition_to(&Active));
        assert!(!Candidate.can_transition_to(&Draft));
        assert!(!Rejected.can_transition_to(&Candidate));
        assert!(!Rejected.can_transition_to(&Active));
        assert!(!Active.can_transition_to(&Draft));
        assert!(!Active.can_transition_to(&Approved));
        assert!(!Expired.can_transition_to(&Candidate));
        assert!(!Deprecated.can_transition_to(&Active));
        assert!(!Superseded.can_transition_to(&Active));
        assert!(!Draft.can_transition_to(&Draft));
    }

    #[test]
    fn full_lifecycle_candidate_to_active() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        let candidate =
            validated_candidate(&store, "sc::life001", "lifecycle-skill", Some("/repo"));
        assert_eq!(candidate.status, "validated");

        let (skill, version) = store
            .approve_skill_candidate("sc::life001", Some("/repo"), &skills, test_now() + 4)
            .unwrap();
        assert_eq!(skill.status, "active");
        assert_eq!(version.version_number, 1);
        assert_eq!(version.parent_version, None);

        let refreshed = store.get_skill_candidate("sc::life001").unwrap().unwrap();
        assert_eq!(refreshed.status, "active");

        // File landed with the exact content.
        let content = read_skill_file_at(&skills, "lifecycle-skill").unwrap();
        assert_eq!(
            content.as_deref(),
            Some(candidate.proposed_content.as_str())
        );
    }

    #[test]
    fn invalid_transitions_rejected_by_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        store
            .insert_skill_candidate(&valid_candidate("sc::bad001", "bad-skill", Some("/repo")))
            .unwrap();

        // candidate → active directly: refused.
        let err = store
            .transition_skill_candidate(
                "sc::bad001",
                SkillCandidateStatus::Active,
                None,
                test_now(),
            )
            .unwrap_err();
        assert!(err.to_string().contains("invalid lifecycle transition"));

        // candidate → draft directly (skipping evaluation): refused.
        assert!(store
            .transition_skill_candidate("sc::bad001", SkillCandidateStatus::Draft, None, test_now())
            .is_err());

        // Reject, then attempt revival: refused.
        store
            .transition_skill_candidate(
                "sc::bad001",
                SkillCandidateStatus::Rejected,
                Some("no"),
                test_now(),
            )
            .unwrap();
        assert!(store
            .transition_skill_candidate(
                "sc::bad001",
                SkillCandidateStatus::Evaluating,
                None,
                test_now()
            )
            .is_err());
    }

    #[test]
    fn rejected_candidate_cannot_be_approved() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        // A rejected candidate cannot approve even from validated-shape
        // content: build it, reject it mid-pipeline.
        store
            .insert_skill_candidate(&valid_candidate(
                "sc::rej001",
                "rejected-skill",
                Some("/repo"),
            ))
            .unwrap();
        store
            .transition_skill_candidate(
                "sc::rej001",
                SkillCandidateStatus::Rejected,
                Some("user rejected"),
                test_now(),
            )
            .unwrap();

        let err = store
            .approve_skill_candidate("sc::rej001", Some("/repo"), &skills, test_now() + 5)
            .unwrap_err();
        assert!(err.to_string().contains("rejected"), "got: {err}");
    }

    #[test]
    fn rejection_reason_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        store
            .insert_skill_candidate(&valid_candidate("sc::rr001", "reason-skill", Some("/repo")))
            .unwrap();
        let updated = store
            .transition_skill_candidate(
                "sc::rr001",
                SkillCandidateStatus::Rejected,
                Some("insufficient evidence quality"),
                test_now(),
            )
            .unwrap();
        assert_eq!(
            updated.rejection_reason.as_deref(),
            Some("insufficient evidence quality")
        );
    }

    #[test]
    fn expiry_sweep_matrix() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let now = test_now();
        let past = now - 1000;

        // Rows in each status with a lapsed TTL.
        for (id, status) in [
            ("sc::exp-cand", "candidate"),
            ("sc::exp-eval", "evaluating"),
            ("sc::exp-def", "deferred"),
            ("sc::exp-draft", "draft"),
            ("sc::exp-val", "validated"),
            ("sc::exp-rej", "rejected"),
        ] {
            let mut c = valid_candidate(
                id,
                &format!("exp-{}", id.trim_start_matches("sc::exp-")),
                Some("/repo"),
            );
            c.status = status.to_string();
            c.expires_at = Some(past);
            store.insert_skill_candidate(&c).unwrap();
        }

        let swept = store.expire_skill_candidates(now).unwrap();
        // Only candidate/evaluating/deferred rows expire.
        assert_eq!(swept, 3);
        for id in ["sc::exp-cand", "sc::exp-eval", "sc::exp-def"] {
            assert_eq!(
                store.get_skill_candidate(id).unwrap().unwrap().status,
                "expired"
            );
        }
        // Review-pipeline and terminal rows are untouched.
        for (id, want) in [
            ("sc::exp-draft", "draft"),
            ("sc::exp-val", "validated"),
            ("sc::exp-rej", "rejected"),
        ] {
            assert_eq!(store.get_skill_candidate(id).unwrap().unwrap().status, want);
        }
    }

    // ── Self-approval / trust gates ────────────────────────────────────

    #[test]
    fn approve_requires_validated_status() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        // A candidate that skipped validation cannot approve.
        store
            .insert_skill_candidate(&valid_candidate("sc::sk001", "skip-skill", Some("/repo")))
            .unwrap();
        let err = store
            .approve_skill_candidate("sc::sk001", Some("/repo"), &skills, test_now())
            .unwrap_err();
        assert!(err.to_string().contains("validated"), "got: {err}");
        // Nothing was published.
        assert!(read_skill_file_at(&skills, "skip-skill").unwrap().is_none());
    }

    #[test]
    fn approve_requires_confidence_floor() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        let mut c = valid_candidate("sc::weak001", "weak-skill", Some("/repo"));
        c.confidence = 0.30; // below the 0.60 floor
        store.insert_skill_candidate(&c).unwrap();
        store
            .transition_skill_candidate(
                "sc::weak001",
                SkillCandidateStatus::Evaluating,
                None,
                test_now(),
            )
            .unwrap();
        store
            .promote_candidate_to_draft("sc::weak001", &valid_content("weak-skill"), test_now())
            .unwrap();
        store
            .transition_skill_candidate(
                "sc::weak001",
                SkillCandidateStatus::Validated,
                None,
                test_now(),
            )
            .unwrap();

        let err = store
            .approve_skill_candidate("sc::weak001", Some("/repo"), &skills, test_now())
            .unwrap_err();
        assert!(
            err.to_string().contains("below approval floor"),
            "got: {err}"
        );
        assert!(read_skill_file_at(&skills, "weak-skill").unwrap().is_none());
    }

    #[test]
    fn approve_enforces_workspace_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        validated_candidate(&store, "sc::ws001", "ws-skill", Some("/project-a"));

        // Approving from project B must fail.
        let err = store
            .approve_skill_candidate("sc::ws001", Some("/project-b"), &skills, test_now())
            .unwrap_err();
        assert!(err.to_string().contains("workspace mismatch"), "got: {err}");
        assert!(read_skill_file_at(&skills, "ws-skill").unwrap().is_none());
    }

    #[test]
    fn learning_status_gate_for_skill_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        // Rejected learning cannot seed a skill candidate.
        let mut lc = accepted_learning("lc::rej", Some("/repo"));
        lc.status = "rejected".to_string();
        let err = store
            .create_skill_candidate_from_learning(
                &lc,
                "nope",
                "d",
                "p",
                SkillApplicability::default(),
                &valid_content("nope"),
                test_now(),
            )
            .unwrap_err();
        assert!(
            err.to_string().contains("only accepted learning"),
            "got: {err}"
        );

        // Weak (unevaluated) learning cannot seed one either.
        let mut lc = accepted_learning("lc::weak", Some("/repo"));
        lc.status = "candidate".to_string();
        assert!(store
            .create_skill_candidate_from_learning(
                &lc,
                "nope2",
                "d",
                "p",
                SkillApplicability::default(),
                &valid_content("nope2"),
                test_now(),
            )
            .is_err());

        // Deferred learning cannot seed one.
        let mut lc = accepted_learning("lc::defer", Some("/repo"));
        lc.status = "deferred".to_string();
        assert!(store
            .create_skill_candidate_from_learning(
                &lc,
                "nope3",
                "d",
                "p",
                SkillApplicability::default(),
                &valid_content("nope3"),
                test_now(),
            )
            .is_err());

        // Accepted learning can — but only as a *candidate*.
        let ok = store
            .create_skill_candidate_from_learning(
                &accepted_learning("lc::ok", Some("/repo")),
                "from-learning",
                "d",
                "p",
                SkillApplicability::default(),
                &valid_content("from-learning"),
                test_now(),
            )
            .unwrap();
        assert_eq!(ok.status, "candidate");
        assert_eq!(ok.source_learning_candidates, vec!["lc::ok".to_string()]);
    }

    // ── Mutation safety ────────────────────────────────────────────────

    #[test]
    fn approve_refuses_blind_overwrite_of_foreign_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        // A stray SKILL.md exists on disk with no DB lineage.
        std::fs::create_dir_all(skills.join("stray-skill")).unwrap();
        std::fs::write(
            skills.join("stray-skill").join("SKILL.md"),
            "unowned content",
        )
        .unwrap();

        validated_candidate(&store, "sc::stray001", "stray-skill", Some("/repo"));
        let err = store
            .approve_skill_candidate("sc::stray001", Some("/repo"), &skills, test_now())
            .unwrap_err();
        assert!(
            err.to_string().contains("no skill lineage owns it"),
            "got: {err}"
        );
        // The stray file is untouched.
        let content = std::fs::read_to_string(skills.join("stray-skill").join("SKILL.md")).unwrap();
        assert_eq!(content, "unowned content");
    }

    #[test]
    fn approve_detects_external_modification() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        // Publish v1.
        validated_candidate(&store, "sc::v1cand", "drift-skill", Some("/repo"));
        store
            .approve_skill_candidate("sc::v1cand", Some("/repo"), &skills, test_now())
            .unwrap();

        // An external actor edits the file.
        std::fs::write(
            skills.join("drift-skill").join("SKILL.md"),
            "externally modified",
        )
        .unwrap();

        // A v2 candidate now fails: read-before-write detects the drift.
        validated_candidate(&store, "sc::v2cand", "drift-skill", Some("/repo"));
        let err = store
            .approve_skill_candidate("sc::v2cand", Some("/repo"), &skills, test_now())
            .unwrap_err();
        assert!(
            err.to_string().contains("modified externally"),
            "got: {err}"
        );
        // The external edit is preserved, not clobbered.
        let content = std::fs::read_to_string(skills.join("drift-skill").join("SKILL.md")).unwrap();
        assert_eq!(content, "externally modified");
    }

    #[test]
    fn approve_refuses_publish_over_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        // Publish v1.
        validated_candidate(&store, "sc::hole1", "hole-skill", Some("/repo"));
        store
            .approve_skill_candidate("sc::hole1", Some("/repo"), &skills, test_now())
            .unwrap();
        // Delete the file (simulating an interrupted/corrupt state).
        std::fs::remove_file(skills.join("hole-skill").join("SKILL.md")).unwrap();

        // v2 must refuse: DB says active, file is gone.
        validated_candidate(&store, "sc::hole2", "hole-skill", Some("/repo"));
        let err = store
            .approve_skill_candidate("sc::hole2", Some("/repo"), &skills, test_now())
            .unwrap_err();
        assert!(err.to_string().contains("missing"), "got: {err}");
    }

    #[test]
    fn stale_writer_cannot_overwrite_newer_version() {
        // Actor A reads v1, Actor B publishes v2, Actor A retries: the
        // deterministic version id for v2 now exists, so A's second
        // publish of the *same* content converges idempotently — but a
        // *different* v2 (same version slot) is refused.
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        // v1 published.
        let _v1cand = validated_candidate(&store, "sc::sw1", "stale-skill", Some("/repo"));
        store
            .approve_skill_candidate("sc::sw1", Some("/repo"), &skills, test_now())
            .unwrap();

        // Actor B publishes v2 (distinct content).
        store
            .insert_skill_candidate(&valid_candidate("sc::sw2", "stale-skill", Some("/repo")))
            .unwrap();
        store
            .transition_skill_candidate(
                "sc::sw2",
                SkillCandidateStatus::Evaluating,
                None,
                test_now(),
            )
            .unwrap();
        let v2_content = valid_content_v("stale-skill", "v2");
        store
            .promote_candidate_to_draft("sc::sw2", &v2_content, test_now())
            .unwrap();
        store
            .transition_skill_candidate(
                "sc::sw2",
                SkillCandidateStatus::Validated,
                None,
                test_now(),
            )
            .unwrap();
        store
            .approve_skill_candidate("sc::sw2", Some("/repo"), &skills, test_now())
            .unwrap();

        // Actor A (stale, validated against v1) proposes a *different* v2:
        // the optimistic-concurrency anchor no longer matches the lineage
        // (now at v2) — stale-writer refusal.
        let mut conflicting = valid_candidate("sc::sw3", "stale-skill", Some("/repo"));
        conflicting.proposed_content =
            "---\nname: stale-skill\ndescription: conflicting v2\n---\n\n# Purpose\n\nConflict."
                .to_string();
        conflicting.based_on_version = Some(1); // actor A read v1
        store.insert_skill_candidate(&conflicting).unwrap();
        store
            .transition_skill_candidate(
                "sc::sw3",
                SkillCandidateStatus::Evaluating,
                None,
                test_now(),
            )
            .unwrap();
        store
            .promote_candidate_to_draft("sc::sw3", &conflicting.proposed_content, test_now())
            .unwrap();
        store
            .transition_skill_candidate(
                "sc::sw3",
                SkillCandidateStatus::Validated,
                None,
                test_now(),
            )
            .unwrap();
        let err = store
            .approve_skill_candidate("sc::sw3", Some("/repo"), &skills, test_now())
            .unwrap_err();
        assert!(err.to_string().contains("stale candidate"), "got: {err}");
    }

    #[test]
    fn concurrent_duplicate_approval_converges() {
        // Two approvals of the *same* validated candidate (retry after a
        // network blip): the second is a conflict (version slot taken),
        // never a duplicate version row.
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        validated_candidate(&store, "sc::dup1", "dup-skill", Some("/repo"));
        store
            .approve_skill_candidate("sc::dup1", Some("/repo"), &skills, test_now())
            .unwrap();
        let err = store
            .approve_skill_candidate("sc::dup1", Some("/repo"), &skills, test_now())
            .unwrap_err();
        // Candidate already active: refused at the status gate.
        assert!(err.to_string().contains("validated"), "got: {err}");
        let versions = store
            .list_skill_versions(
                &store
                    .get_skill_by_name("dup-skill")
                    .unwrap()
                    .unwrap()
                    .skill_id,
                10,
            )
            .unwrap();
        assert_eq!(versions.len(), 1);
    }

    // ── Immutability ───────────────────────────────────────────────────

    #[test]
    fn published_versions_are_immutable() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        validated_candidate(&store, "sc::imm1", "immutable-skill", Some("/repo"));
        let (_, v1) = store
            .approve_skill_candidate("sc::imm1", Some("/repo"), &skills, test_now())
            .unwrap();

        // Attempting to re-insert the same version id with different
        // content must be refused: plain INSERT + the (skill_id,
        // version_number) unique index + the version_id PK make every
        // tampering path a hard error, never a content swap.
        let mut forged = v1.clone();
        forged.content =
            "---\nname: immutable-skill\ndescription: TAMPERED\n---\n\n# Purpose\n\nTampered."
                .to_string();
        forged.content_hash = content_hash(&forged.content);
        assert!(
            store.insert_skill_version(&forged).is_err(),
            "tampering with a published version row must be refused"
        );
        let reloaded = store.get_skill_version(&v1.version_id).unwrap().unwrap();
        assert_eq!(
            reloaded.content, v1.content,
            "version content must never change"
        );
        assert_eq!(reloaded.content_hash, v1.content_hash);

        // v2 publish does not touch v1's row.
        store
            .insert_skill_candidate(&valid_candidate(
                "sc::imm2",
                "immutable-skill",
                Some("/repo"),
            ))
            .unwrap();
        store
            .transition_skill_candidate(
                "sc::imm2",
                SkillCandidateStatus::Evaluating,
                None,
                test_now(),
            )
            .unwrap();
        let imm2_content = valid_content_v("immutable-skill", "v2");
        store
            .promote_candidate_to_draft("sc::imm2", &imm2_content, test_now())
            .unwrap();
        store
            .transition_skill_candidate(
                "sc::imm2",
                SkillCandidateStatus::Validated,
                None,
                test_now(),
            )
            .unwrap();
        store
            .approve_skill_candidate("sc::imm2", Some("/repo"), &skills, test_now())
            .unwrap();
        let v1_after = store.get_skill_version(&v1.version_id).unwrap().unwrap();
        assert_eq!(v1_after.content, v1.content);
        assert_eq!(v1_after.status, "replaced");
    }

    // ── Rollback ───────────────────────────────────────────────────────

    #[test]
    fn v2_publish_updates_persisted_current_version() {
        // Regression: after a second approved publish, the *persisted*
        // skill row (not just the returned value) must point at v2.
        // The E2E run caught the skill upsert writing the pre-increment
        // clone's version, leaving inspect() one version behind.
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        // v1.
        let c1 = valid_candidate("sc::pv2-a", "pv2-skill", Some("/repo"));
        store.insert_skill_candidate(&c1).unwrap();
        store
            .transition_skill_candidate("sc::pv2-a", SkillCandidateStatus::Evaluating, None, 1)
            .unwrap();
        store
            .promote_candidate_to_draft("sc::pv2-a", &valid_content_v("pv2-skill", "v1"), 2)
            .unwrap();
        store
            .transition_skill_candidate("sc::pv2-a", SkillCandidateStatus::Validated, None, 3)
            .unwrap();
        let (skill1, v1) = store
            .approve_skill_candidate("sc::pv2-a", Some("/repo"), &skills, test_now())
            .unwrap();
        assert_eq!(skill1.current_version, 1);

        // v2 with evolved content (distinct candidate id).
        let c2 = SkillCandidate {
            candidate_id: "sc::pv2-b".to_string(),
            proposed_content: valid_content_v("pv2-skill", "v2"),
            ..valid_candidate("sc::pv2-b", "pv2-skill", Some("/repo"))
        };
        store.insert_skill_candidate(&c2).unwrap();
        store
            .transition_skill_candidate("sc::pv2-b", SkillCandidateStatus::Evaluating, None, 1)
            .unwrap();
        store
            .promote_candidate_to_draft("sc::pv2-b", &c2.proposed_content, 2)
            .unwrap();
        store
            .transition_skill_candidate("sc::pv2-b", SkillCandidateStatus::Validated, None, 3)
            .unwrap();
        let (skill2, v2) = store
            .approve_skill_candidate("sc::pv2-b", Some("/repo"), &skills, test_now())
            .unwrap();
        assert_eq!(skill2.current_version, 2);
        assert_eq!(v2.version_number, 2);
        assert_eq!(v2.parent_version.as_deref(), Some(v1.version_id.as_str()));

        // The PERSISTED row (fresh get, not the return value):
        let persisted = store.get_skill_by_name("pv2-skill").unwrap().unwrap();
        assert_eq!(
            persisted.current_version, 2,
            "persisted current_version must track the published version"
        );
        let active = store
            .get_active_skill_version(&persisted.skill_id)
            .unwrap()
            .unwrap();
        assert_eq!(active.version_number, 2);
        assert_eq!(active.content_hash, v2.content_hash);
        assert_ne!(active.content_hash, v1.content_hash);

        // v1's row is now 'replaced' but byte-identical to its original.
        let v1_row = store.get_skill_version(&v1.version_id).unwrap().unwrap();
        assert_eq!(v1_row.status, "replaced");
        assert_eq!(v1_row.content, valid_content_v("pv2-skill", "v1"));
    }

    #[test]
    fn rollback_preserves_history_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        // v1 and v2 published (distinct content).
        validated_candidate(&store, "sc::rb1", "rollback-skill", Some("/repo"));
        let (_, v1) = store
            .approve_skill_candidate("sc::rb1", Some("/repo"), &skills, test_now())
            .unwrap();
        store
            .insert_skill_candidate(&valid_candidate("sc::rb2", "rollback-skill", Some("/repo")))
            .unwrap();
        store
            .transition_skill_candidate(
                "sc::rb2",
                SkillCandidateStatus::Evaluating,
                None,
                test_now(),
            )
            .unwrap();
        let rb2_content = valid_content_v("rollback-skill", "v2");
        store
            .promote_candidate_to_draft("sc::rb2", &rb2_content, test_now())
            .unwrap();
        store
            .transition_skill_candidate(
                "sc::rb2",
                SkillCandidateStatus::Validated,
                None,
                test_now(),
            )
            .unwrap();
        let (_, v2) = store
            .approve_skill_candidate("sc::rb2", Some("/repo"), &skills, test_now())
            .unwrap();

        // Roll back to v1.
        let skill_id = v1.skill_id.clone();
        let (skill, v3) = store
            .rollback_skill(&skill_id, Some("/repo"), 1, &skills, test_now())
            .unwrap();
        assert_eq!(skill.current_version, 3);
        assert_eq!(v3.version_number, 3);
        assert_eq!(v3.parent_version.as_deref(), Some(v1.version_id.as_str()));
        assert_eq!(v3.content_hash, v1.content_hash);

        // v1 and v2 rows unchanged; v2 is marked rolled_back.
        let v1_after = store.get_skill_version(&v1.version_id).unwrap().unwrap();
        assert_eq!(v1_after.status, "replaced");
        let v2_after = store.get_skill_version(&v2.version_id).unwrap().unwrap();
        assert_eq!(v2_after.status, "rolled_back");
        assert_eq!(v2_after.content, v2.content);

        // File now holds v1's content.
        let file = read_skill_file_at(&skills, "rollback-skill")
            .unwrap()
            .unwrap();
        assert_eq!(content_hash(&file), v1.content_hash);

        // Active version is v3 (the rollback copy).
        let active = store.get_active_skill_version(&skill_id).unwrap().unwrap();
        assert_eq!(active.version_number, 3);
    }

    #[test]
    fn rollback_rejections() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        validated_candidate(&store, "sc::rbr1", "rbr-skill", Some("/repo"));
        let (_, v1) = store
            .approve_skill_candidate("sc::rbr1", Some("/repo"), &skills, test_now())
            .unwrap();
        let skill_id = v1.skill_id.clone();

        // Nonexistent version.
        assert!(store
            .rollback_skill(&skill_id, Some("/repo"), 99, &skills, test_now())
            .is_err());
        // Rolling back to the current version.
        assert!(store
            .rollback_skill(&skill_id, Some("/repo"), 1, &skills, test_now())
            .is_err());
        // Unknown skill.
        assert!(store
            .rollback_skill("sk::nope", Some("/repo"), 1, &skills, test_now())
            .is_err());
        // Wrong workspace.
        assert!(store
            .rollback_skill(&skill_id, Some("/other"), 1, &skills, test_now())
            .is_err());

        // Unrelated skill in another workspace.
        validated_candidate(&store, "sc::rbr2", "other-ws-skill", Some("/other"));
        let (_, other_v1) = store
            .approve_skill_candidate("sc::rbr2", Some("/other"), &skills, test_now())
            .unwrap();
        assert!(store
            .rollback_skill(&other_v1.skill_id, Some("/repo"), 1, &skills, test_now())
            .is_err());
    }

    #[test]
    fn rollback_refuses_over_drifted_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        validated_candidate(&store, "sc::rbd1", "drift-rb-skill", Some("/repo"));
        let (_, v1) = store
            .approve_skill_candidate("sc::rbd1", Some("/repo"), &skills, test_now())
            .unwrap();
        store
            .insert_skill_candidate(&valid_candidate(
                "sc::rbd2",
                "drift-rb-skill",
                Some("/repo"),
            ))
            .unwrap();
        store
            .transition_skill_candidate(
                "sc::rbd2",
                SkillCandidateStatus::Evaluating,
                None,
                test_now(),
            )
            .unwrap();
        let rbd2_content = valid_content_v("drift-rb-skill", "v2");
        store
            .promote_candidate_to_draft("sc::rbd2", &rbd2_content, test_now())
            .unwrap();
        store
            .transition_skill_candidate(
                "sc::rbd2",
                SkillCandidateStatus::Validated,
                None,
                test_now(),
            )
            .unwrap();
        store
            .approve_skill_candidate("sc::rbd2", Some("/repo"), &skills, test_now())
            .unwrap();

        // External edit.
        std::fs::write(skills.join("drift-rb-skill").join("SKILL.md"), "tampered").unwrap();

        let err = store
            .rollback_skill(&v1.skill_id, Some("/repo"), 1, &skills, test_now())
            .unwrap_err();
        assert!(
            err.to_string().contains("modified externally"),
            "got: {err}"
        );
    }

    #[test]
    fn rollback_noop_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        // v1 and an identical-content lineage: roll back to v1 where the
        // active v2 has the same content (via a rollback first).
        validated_candidate(&store, "sc::rno1", "noop-skill", Some("/repo"));
        let (_, v1) = store
            .approve_skill_candidate("sc::rno1", Some("/repo"), &skills, test_now())
            .unwrap();
        store
            .insert_skill_candidate(&valid_candidate("sc::rno2", "noop-skill", Some("/repo")))
            .unwrap();
        store
            .transition_skill_candidate(
                "sc::rno2",
                SkillCandidateStatus::Evaluating,
                None,
                test_now(),
            )
            .unwrap();
        let rno2_content = valid_content_v("noop-skill", "v2");
        store
            .promote_candidate_to_draft("sc::rno2", &rno2_content, test_now())
            .unwrap();
        store
            .transition_skill_candidate(
                "sc::rno2",
                SkillCandidateStatus::Validated,
                None,
                test_now(),
            )
            .unwrap();
        store
            .approve_skill_candidate("sc::rno2", Some("/repo"), &skills, test_now())
            .unwrap();
        // Roll back to v1 → v3 (same content as v1).
        store
            .rollback_skill(&v1.skill_id, Some("/repo"), 1, &skills, test_now())
            .unwrap();
        // Rolling back to v1 again would produce identical content to
        // the active v3: refused as a no-op.
        let err = store
            .rollback_skill(&v1.skill_id, Some("/repo"), 1, &skills, test_now())
            .unwrap_err();
        assert!(err.to_string().contains("no-op"), "got: {err}");
    }

    // ── Deprecation ────────────────────────────────────────────────────

    #[test]
    fn deprecate_removes_file_and_blocks_republish() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        validated_candidate(&store, "sc::dep001", "dep-skill", Some("/repo"));
        let (skill, _) = store
            .approve_skill_candidate("sc::dep001", Some("/repo"), &skills, test_now())
            .unwrap();

        let deprecated = store
            .deprecate_skill(
                &skill.skill_id,
                Some("/repo"),
                &skills,
                Some("stale"),
                test_now(),
            )
            .unwrap();
        assert_eq!(deprecated.status, "deprecated");
        // File removed: OpenCode stops discovering it.
        assert!(read_skill_file_at(&skills, "dep-skill").unwrap().is_none());

        // Cannot deprecate twice.
        assert!(store
            .deprecate_skill(&skill.skill_id, Some("/repo"), &skills, None, test_now())
            .is_err());
        // Cannot roll back a deprecated skill.
        assert!(store
            .rollback_skill(&skill.skill_id, Some("/repo"), 1, &skills, test_now())
            .is_err());
        // Cannot approve a new version of a deprecated lineage.
        validated_candidate(&store, "sc::dep002", "dep-skill", Some("/repo"));
        let err = store
            .approve_skill_candidate("sc::dep002", Some("/repo"), &skills, test_now())
            .unwrap_err();
        assert!(err.to_string().contains("deprecated"), "got: {err}");

        // Health no longer accumulates on the deprecated row.
        assert!(store
            .record_skill_use(&skill.skill_id, true, test_now())
            .is_err());
    }

    #[test]
    fn deprecate_workspace_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        validated_candidate(&store, "sc::depb1", "depb-skill", Some("/project-a"));
        let (skill, _) = store
            .approve_skill_candidate("sc::depb1", Some("/project-a"), &skills, test_now())
            .unwrap();

        let err = store
            .deprecate_skill(
                &skill.skill_id,
                Some("/project-b"),
                &skills,
                None,
                test_now(),
            )
            .unwrap_err();
        assert!(err.to_string().contains("workspace mismatch"), "got: {err}");
    }

    // ── Health ─────────────────────────────────────────────────────────

    #[test]
    fn skill_health_assessment() {
        let mut health = SkillHealth::default();
        assert!(!health.is_assessable());
        assert!(!health.is_degraded());

        health.success_count = 5;
        health.failure_count = 1;
        assert!(health.is_assessable());
        assert!(!health.is_degraded()); // 1/6 = 0.17 < 0.40

        health.failure_count = 4;
        assert!(health.is_degraded()); // 4/9 = 0.44 >= 0.40
    }

    #[test]
    fn record_skill_use_updates_health() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        validated_candidate(&store, "sc::hlth1", "health-skill", Some("/repo"));
        let (skill, _) = store
            .approve_skill_candidate("sc::hlth1", Some("/repo"), &skills, test_now())
            .unwrap();

        for i in 0..5 {
            store
                .record_skill_use(&skill.skill_id, true, test_now() + i)
                .unwrap();
        }
        for i in 5..7 {
            store
                .record_skill_use(&skill.skill_id, false, test_now() + i)
                .unwrap();
        }
        let skill = store.get_skill(&skill.skill_id).unwrap().unwrap();
        assert_eq!(skill.health.success_count, 5);
        assert_eq!(skill.health.failure_count, 2);
        assert!(skill.health.last_used_at.is_some());
        assert!(skill.health.last_failed_at.is_some());
    }

    // ── Validation ─────────────────────────────────────────────────────

    #[test]
    fn validate_skill_content_rejects_secrets() {
        let content =
            "---\nname: test\ndescription: x\n---\n\n# Purpose\n\nUse this api_key: sk-abc123";
        let result = validate_skill_content(content, "test", None);
        assert!(!result.valid);
        assert!(!result.secret_safe);
    }

    #[test]
    fn validate_skill_content_accepts_clean() {
        let content =
            "---\nname: test\ndescription: A test skill\n---\n\n# Purpose\n\nThis is a test.";
        let result = validate_skill_content(content, "test", None);
        assert!(result.valid);
        assert!(result.secret_safe);
        assert!(result.structural);
    }

    #[test]
    fn validate_skill_content_opencode_compatibility() {
        // Missing description field: OpenCode ignores such skills.
        let content = "---\nname: test\n---\n\n# Purpose\n\nBody.";
        let result = validate_skill_content(content, "test", None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.contains("description")));

        // Frontmatter name not matching the skill name.
        let content = "---\nname: other-name\ndescription: d\n---\n\n# Purpose\n\nBody.";
        let result = validate_skill_content(content, "test", None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.contains("does not match")));

        // Uppercase name: not a valid OpenCode name.
        let content = "---\nname: Test\n---\n\n# Purpose\n\nBody.";
        assert!(!validate_skill_content(content, "test", None).valid);

        // Underscore name: not a valid OpenCode name.
        assert!(
            !validate_skill_content("---\nname: x\n---\n\n# Purpose\n\nBody.", "my_skill", None)
                .valid
        );
        // Consecutive hyphens.
        assert!(!is_valid_skill_name("a--b"));
        // Leading/trailing hyphen.
        assert!(!is_valid_skill_name("-ab"));
        assert!(!is_valid_skill_name("ab-"));
        // Valid names.
        assert!(is_valid_skill_name("git-release"));
        assert!(is_valid_skill_name("a"));
        assert!(is_valid_skill_name("code-review-v2"));
    }

    #[test]
    fn validate_skill_content_rejects_missing_frontmatter() {
        let content = "# Purpose\n\nNo frontmatter here.";
        let result = validate_skill_content(content, "test", None);
        assert!(!result.valid);
        assert!(!result.structural);
    }

    #[test]
    fn validate_skill_content_rejects_oversized() {
        let content = format!(
            "---\nname: test\ndescription: d\n---\n\n{}",
            "x".repeat(MAX_SKILL_CONTENT_CHARS + 1)
        );
        let result = validate_skill_content(&content, "test", None);
        assert!(!result.valid);
        assert!(!result.size_valid);
    }

    #[test]
    fn validate_skill_content_rejects_empty_body() {
        let content = "---\nname: test\ndescription: d\n---\n";
        let result = validate_skill_content(content, "test", None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.contains("no body")));
    }

    #[test]
    fn validate_skill_candidate_frozen_after_review() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        validated_candidate(&store, "sc::frozen1", "frozen-skill", Some("/repo"));
        // Validated content is frozen: the automated pass refuses to
        // re-run on reviewed content.
        let err = store
            .evaluate_candidate_content("sc::frozen1", test_now())
            .unwrap_err();
        assert!(err.to_string().contains("pre-review"), "got: {err}");

        // Draft content can be re-validated.
        store
            .insert_skill_candidate(&valid_candidate(
                "sc::frozen2",
                "frozen2-skill",
                Some("/repo"),
            ))
            .unwrap();
        store
            .transition_skill_candidate(
                "sc::frozen2",
                SkillCandidateStatus::Evaluating,
                None,
                test_now(),
            )
            .unwrap();
        store
            .promote_candidate_to_draft("sc::frozen2", &valid_content("frozen2-skill"), test_now())
            .unwrap();
        assert!(store
            .evaluate_candidate_content("sc::frozen2", test_now())
            .is_ok());
    }

    // ── IDs and deduplication ──────────────────────────────────────────

    #[test]
    fn mint_ids_are_deterministic() {
        let id1 = mint_skill_candidate_id(
            &SkillScope::Project,
            Some("/repo"),
            None,
            "my-skill",
            "content-a",
        );
        let id2 = mint_skill_candidate_id(
            &SkillScope::Project,
            Some("/repo"),
            None,
            "my-skill",
            "content-a",
        );
        assert_eq!(id1, id2);
        assert!(id1.starts_with("sc::"));
        // Evolved content mints a distinct candidate lineage.
        let id3 = mint_skill_candidate_id(
            &SkillScope::Project,
            Some("/repo"),
            None,
            "my-skill",
            "content-b",
        );
        assert_ne!(id1, id3);

        let id3 = mint_skill_id(&SkillScope::Project, Some("/repo"), "my-skill");
        let id4 = mint_skill_id(&SkillScope::Project, Some("/repo"), "my-skill");
        assert_eq!(id3, id4);
        assert!(id3.starts_with("sk::"));

        let id5 = mint_version_id("sk::abc", 1);
        assert!(id5.starts_with("sv::"));
    }

    #[test]
    fn content_hash_is_deterministic() {
        let h1 = content_hash("hello world");
        let h2 = content_hash("hello world");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex
    }

    #[test]
    fn reproposal_lineage_rules() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        // Same identity + same content: idempotent refresh.
        store
            .insert_skill_candidate(&valid_candidate("sc::dup1", "dup-skill", Some("/repo")))
            .unwrap();
        store
            .insert_skill_candidate(&valid_candidate("sc::dup1", "dup-skill", Some("/repo")))
            .unwrap();
        let c = store.get_skill_candidate("sc::dup1").unwrap().unwrap();
        assert_eq!(c.status, "candidate"); // untouched

        // Same identity + different content: refused.
        let mut conflicting = valid_candidate("sc::dup1", "dup-skill", Some("/repo"));
        conflicting.proposed_content =
            "---\nname: dup-skill\ndescription: conflicting\n---\n\n# Purpose\n\nOther."
                .to_string();
        let err = store.insert_skill_candidate(&conflicting).unwrap_err();
        assert!(
            err.to_string().contains("different proposed content"),
            "got: {err}"
        );

        // Same identity, already in the review pipeline: refused.
        let mut in_review = valid_candidate("sc::dup2", "pipeline-skill", Some("/repo"));
        in_review.status = "validated".to_string();
        store.insert_skill_candidate(&in_review).unwrap();
        let err = store
            .insert_skill_candidate(&valid_candidate(
                "sc::dup2",
                "pipeline-skill",
                Some("/repo"),
            ))
            .unwrap_err();
        assert!(err.to_string().contains("refused"), "got: {err}");
    }

    #[test]
    fn expired_candidate_cannot_be_revived() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        let mut c = valid_candidate("sc::dead1", "dead-skill", Some("/repo"));
        c.status = "expired".to_string();
        store.insert_skill_candidate(&c).unwrap();
        let err = store
            .insert_skill_candidate(&valid_candidate("sc::dead1", "dead-skill", Some("/repo")))
            .unwrap_err();
        assert!(err.to_string().contains("refused"), "got: {err}");
    }

    // ── Workspace / scope isolation ───────────────────────────────────

    #[test]
    fn skill_scope_isolation() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let now = test_now();
        let skills = dir.path().join("skills");

        // Publish a skill in project A.
        validated_candidate(&store, "sc::isoA", "iso-skill-a", Some("/project-a"));
        let (skill_a, _) = store
            .approve_skill_candidate("sc::isoA", Some("/project-a"), &skills, now)
            .unwrap();
        // Publish a skill in project B.
        validated_candidate(&store, "sc::isoB", "iso-skill-b", Some("/project-b"));
        let (skill_b, _) = store
            .approve_skill_candidate("sc::isoB", Some("/project-b"), &skills, now)
            .unwrap();
        // A global skill.
        let mut g = valid_candidate("sc::isoG", "iso-global", None);
        g.scope = "global".to_string();
        store.insert_skill_candidate(&g).unwrap();
        store
            .transition_skill_candidate("sc::isoG", SkillCandidateStatus::Evaluating, None, now)
            .unwrap();
        store
            .promote_candidate_to_draft("sc::isoG", &valid_content("iso-global"), now)
            .unwrap();
        store
            .transition_skill_candidate("sc::isoG", SkillCandidateStatus::Validated, None, now)
            .unwrap();
        let (skill_g, _) = store
            .approve_skill_candidate("sc::isoG", None, &skills, now)
            .unwrap();

        // Project A sees its own + global; not B's.
        let a_list = store.list_skills(Some("/project-a"), None, 10).unwrap();
        assert_eq!(a_list.len(), 2);
        assert!(a_list.iter().any(|s| s.skill_id == skill_a.skill_id));
        assert!(a_list.iter().any(|s| s.skill_id == skill_g.skill_id));
        assert!(!a_list.iter().any(|s| s.skill_id == skill_b.skill_id));

        // Project B sees its own + global; not A's.
        let b_list = store.list_skills(Some("/project-b"), None, 10).unwrap();
        assert_eq!(b_list.len(), 2);
        assert!(b_list.iter().any(|s| s.skill_id == skill_b.skill_id));
        assert!(!b_list.iter().any(|s| s.skill_id == skill_a.skill_id));

        // Candidates obey the same rule.
        let cand_a = store
            .list_skill_candidates(Some("/project-a"), None, None, 10)
            .unwrap();
        assert!(cand_a.iter().any(|c| c.name == "iso-skill-a"));
        assert!(cand_a.iter().any(|c| c.name == "iso-global"));
        assert!(!cand_a.iter().any(|c| c.name == "iso-skill-b"));
    }

    #[test]
    fn task_scoped_candidates_invisible_without_task() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        let mut t = valid_candidate("sc::task1", "task-skill", Some("/repo"));
        t.scope = "task".to_string();
        t.task_id = Some("task-42".to_string());
        store.insert_skill_candidate(&t).unwrap();

        // Without the task: invisible.
        let none = store
            .list_skill_candidates(Some("/repo"), None, None, 10)
            .unwrap();
        assert!(!none.iter().any(|c| c.name == "task-skill"));

        // With the wrong task: invisible.
        let wrong = store
            .list_skill_candidates(Some("/repo"), Some("task-99"), None, 10)
            .unwrap();
        assert!(!wrong.iter().any(|c| c.name == "task-skill"));

        // With the right task: visible.
        let right = store
            .list_skill_candidates(Some("/repo"), Some("task-42"), None, 10)
            .unwrap();
        assert!(right.iter().any(|c| c.name == "task-skill"));
    }

    // ── Filesystem safety ─────────────────────────────────────────────

    #[test]
    fn path_traversal_rejected_at_name_validation() {
        for evil in [
            "../outside",
            "../../outside",
            "a/b",
            "a/../b",
            "./x",
            "a b",
            "A",
            "-x",
            "x-",
            "a--b",
            "a_b",
            "..",
            ".",
        ] {
            let res = skill_file_path_at(Path::new("/tmp/skills"), evil);
            assert!(res.is_err(), "name {evil:?} must be rejected");
        }
        // Valid names resolve inside the root.
        let ok = skill_file_path_at(Path::new("/tmp/skills"), "git-release").unwrap();
        assert!(ok.starts_with("/tmp/skills/git-release/SKILL.md"));
    }

    #[test]
    fn publish_refuses_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join("skills");
        let victim = dir.path().join("victim");
        std::fs::create_dir_all(&victim).unwrap();

        // Skill directory itself is a symlink to the victim dir.
        std::fs::create_dir_all(&skills).unwrap();
        std::os::unix::fs::symlink(&victim, skills.join("escape-skill")).unwrap();

        let err = publish_skill_file(
            &skills,
            "escape-skill",
            &valid_content("escape-skill"),
            None,
        )
        .unwrap_err();
        assert!(
            matches!(err, SkillError::PathTraversal(_) | SkillError::Conflict),
            "got: {err}"
        );
        // Nothing was written through the symlink.
        assert!(!victim.join("SKILL.md").exists());
    }

    #[test]
    fn publish_refuses_symlinked_skill_md() {
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join("skills");
        std::fs::create_dir_all(skills.join("targeted-skill")).unwrap();
        let outside = dir.path().join("outside.md");
        std::fs::write(&outside, "original").unwrap();
        std::os::unix::fs::symlink(&outside, skills.join("targeted-skill").join("SKILL.md"))
            .unwrap();

        let err = publish_skill_file(
            &skills,
            "targeted-skill",
            &valid_content("targeted-skill"),
            None,
        )
        .unwrap_err();
        // The symlinked file exists: publish sees content and requires a
        // hash match (conflict), or a traversal refusal — either way, the
        // original target is untouched.
        assert!(!err.to_string().is_empty());
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "original");
    }

    #[test]
    fn publish_is_atomic_no_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join("skills");
        let path = publish_skill_file(
            &skills,
            "atomic-skill",
            &valid_content("atomic-skill"),
            None,
        )
        .unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("---\nname: atomic-skill"));
        // No temp-file residue.
        let entries: Vec<_> = std::fs::read_dir(skills.join("atomic-skill"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries, vec!["SKILL.md".to_string()]);
    }

    #[test]
    fn publish_conflict_detection() {
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join("skills");

        // First publish.
        publish_skill_file(&skills, "conf-skill", &valid_content("conf-skill"), None).unwrap();

        // Stale hash → conflict.
        let err = publish_skill_file(
            &skills,
            "conf-skill",
            &valid_content("conf-skill"),
            Some("0000000000000000000000000000000000000000000000000000000000000000"),
        )
        .unwrap_err();
        assert!(matches!(err, SkillError::Conflict));

        // Correct hash → succeeds.
        let current = read_skill_file_at(&skills, "conf-skill").unwrap().unwrap();
        publish_skill_file(
            &skills,
            "conf-skill",
            &valid_content("conf-skill"),
            Some(&content_hash(&current)),
        )
        .unwrap();
    }

    #[test]
    fn corrupted_existing_file_requires_recovery() {
        // A truncated/empty SKILL.md owned by the DB: publishing v2 must
        // surface the corruption, not overwrite silently.
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = dir.path().join("skills");

        validated_candidate(&store, "sc::corrupt1", "corrupt-skill", Some("/repo"));
        store
            .approve_skill_candidate("sc::corrupt1", Some("/repo"), &skills, test_now())
            .unwrap();

        std::fs::write(skills.join("corrupt-skill").join("SKILL.md"), "").unwrap();

        validated_candidate(&store, "sc::corrupt2", "corrupt-skill", Some("/repo"));
        let err = store
            .approve_skill_candidate("sc::corrupt2", Some("/repo"), &skills, test_now())
            .unwrap_err();
        assert!(
            err.to_string().contains("modified externally"),
            "got: {err}"
        );
    }

    // ── Persistence / restart ─────────────────────────────────────────

    #[test]
    fn state_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join(STATE_DB_FILE);
        open_checked(&db).unwrap();
        {
            let store = ContextStore::new(db.clone());
            let skills = dir.path().join("skills");
            validated_candidate(&store, "sc::persist1", "persist-skill", Some("/repo"));
            let (skill, _) = store
                .approve_skill_candidate("sc::persist1", Some("/repo"), &skills, test_now())
                .unwrap();
            store
                .insert_skill_candidate(&valid_candidate(
                    "sc::persist2",
                    "persist-skill",
                    Some("/repo"),
                ))
                .unwrap();
            store
                .transition_skill_candidate(
                    "sc::persist2",
                    SkillCandidateStatus::Evaluating,
                    None,
                    test_now(),
                )
                .unwrap();
            let persist2_content = valid_content_v("persist-skill", "v2");
            store
                .promote_candidate_to_draft("sc::persist2", &persist2_content, test_now())
                .unwrap();
            store
                .transition_skill_candidate(
                    "sc::persist2",
                    SkillCandidateStatus::Validated,
                    None,
                    test_now(),
                )
                .unwrap();
            store
                .approve_skill_candidate("sc::persist2", Some("/repo"), &skills, test_now())
                .unwrap();
            store
                .rollback_skill(&skill.skill_id, Some("/repo"), 1, &skills, test_now())
                .unwrap();
            store
                .record_skill_use(&skill.skill_id, true, test_now())
                .unwrap();
        }
        // Reopen: everything survived.
        let store = ContextStore::new(db);
        let skill = store.get_skill_by_name("persist-skill").unwrap().unwrap();
        assert_eq!(skill.status, "active");
        assert_eq!(skill.current_version, 3);
        assert_eq!(skill.health.success_count, 1);
        let versions = store.list_skill_versions(&skill.skill_id, 10).unwrap();
        assert_eq!(versions.len(), 3);
        let active = store
            .get_active_skill_version(&skill.skill_id)
            .unwrap()
            .unwrap();
        assert_eq!(active.version_number, 3);
    }

    // ── Learning → skill provenance ────────────────────────────────────

    #[test]
    fn skill_candidate_from_learning_carries_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        let candidate = store
            .create_skill_candidate_from_learning(
                &accepted_learning("lc::test001", Some("/repo")),
                "mcp-tool-dev",
                "MCP tool development workflow",
                "Reusable procedure for adding MCP tools",
                SkillApplicability {
                    languages: vec!["rust".to_string()],
                    subsystems: vec!["mcp-server".to_string()],
                    ..Default::default()
                },
                &valid_content("mcp-tool-dev"),
                test_now(),
            )
            .unwrap();

        assert_eq!(candidate.name, "mcp-tool-dev");
        assert_eq!(
            candidate.source_learning_candidates,
            vec!["lc::test001".to_string()]
        );
        assert_eq!(candidate.supporting_evidence, vec![1, 2, 3, 4, 5]);
        assert!(candidate.validation.unwrap().valid);
        assert_eq!(candidate.workspace_root.as_deref(), Some("/repo"));
    }

    #[test]
    fn skill_candidate_from_learning_inherits_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        // Learning from project A creates a candidate bound to A.
        let candidate = store
            .create_skill_candidate_from_learning(
                &accepted_learning("lc::inhA", Some("/project-a")),
                "inherited-skill",
                "d",
                "p",
                SkillApplicability::default(),
                &valid_content("inherited-skill"),
                test_now(),
            )
            .unwrap();
        assert_eq!(candidate.workspace_root.as_deref(), Some("/project-a"));
        // Not visible from project B.
        let b_view = store
            .list_skill_candidates(Some("/project-b"), None, None, 10)
            .unwrap();
        assert!(!b_view.iter().any(|c| c.name == "inherited-skill"));
    }

    // ── Misc ───────────────────────────────────────────────────────────

    #[test]
    fn skill_scope_from_str() {
        assert_eq!("global".parse::<SkillScope>().unwrap(), SkillScope::Global);
        assert_eq!(
            "project".parse::<SkillScope>().unwrap(),
            SkillScope::Project
        );
        assert_eq!("task".parse::<SkillScope>().unwrap(), SkillScope::Task);
        assert!("invalid".parse::<SkillScope>().is_err());
    }

    #[test]
    fn list_candidates_by_status() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        for i in 0..5 {
            let mut c = valid_candidate(
                &format!("sc::list{i:03}"),
                &format!("list-skill-{i}"),
                Some("/repo"),
            );
            c.status = if i % 2 == 0 {
                "candidate".to_string()
            } else {
                "draft".to_string()
            };
            c.created_at = test_now() + i;
            c.updated_at = test_now() + i;
            store.insert_skill_candidate(&c).unwrap();
        }

        let candidates = store
            .list_skill_candidates(
                Some("/repo"),
                None,
                Some(SkillCandidateStatus::Candidate),
                10,
            )
            .unwrap();
        assert_eq!(candidates.len(), 3);
        let drafts = store
            .list_skill_candidates(Some("/repo"), None, Some(SkillCandidateStatus::Draft), 10)
            .unwrap();
        assert_eq!(drafts.len(), 2);
    }

    #[test]
    fn skill_upsert_and_get() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let now = test_now();

        let skill = Skill {
            skill_id: "sk::test001".to_string(),
            workspace_root: Some("/repo".to_string()),
            scope: "project".to_string(),
            name: "my-skill".to_string(),
            description: "My skill".to_string(),
            applicability: SkillApplicability::default(),
            current_version: 1,
            status: "active".to_string(),
            confidence: 0.8,
            health: SkillHealth::default(),
            source_candidate_id: None,
            superseded_by: None,
            created_at: now,
            updated_at: now,
        };

        store.upsert_skill(&skill).unwrap();
        let fetched = store.get_skill("sk::test001").unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().name, "my-skill");

        let fetched = store.get_skill_by_name("my-skill").unwrap();
        assert!(fetched.is_some());
    }

    #[test]
    fn skill_version_crud_and_next_number() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let now = test_now();

        let skill = Skill {
            skill_id: "sk::ver001".to_string(),
            workspace_root: Some("/repo".to_string()),
            scope: "project".to_string(),
            name: "versioned-skill".to_string(),
            description: "Versioned".to_string(),
            applicability: SkillApplicability::default(),
            current_version: 1,
            status: "active".to_string(),
            confidence: 0.8,
            health: SkillHealth::default(),
            source_candidate_id: None,
            superseded_by: None,
            created_at: now,
            updated_at: now,
        };
        store.upsert_skill(&skill).unwrap();

        let v1 = SkillVersion {
            version_id: "sv::v001".to_string(),
            skill_id: "sk::ver001".to_string(),
            version_number: 1,
            content: valid_content("versioned-skill"),
            content_hash: content_hash(&valid_content("versioned-skill")),
            source_candidate_id: None,
            supporting_evidence: vec![],
            validation: None,
            author: "codebro".to_string(),
            status: "active".to_string(),
            created_at: now,
            parent_version: None,
        };
        store.insert_skill_version(&v1).unwrap();

        assert_eq!(store.next_skill_version_number("sk::ver001").unwrap(), 2);
        let active = store
            .get_active_skill_version("sk::ver001")
            .unwrap()
            .unwrap();
        assert_eq!(active.version_number, 1);
        let versions = store.list_skill_versions("sk::ver001", 10).unwrap();
        assert_eq!(versions.len(), 1);

        // Unique (skill_id, version_number): a second v1 insert fails.
        let mut dup = v1.clone();
        dup.version_id = "sv::v001b".to_string();
        let err = store.insert_skill_version(&dup).unwrap_err();
        assert!(
            err.to_string().contains("UNIQUE") || err.to_string().contains("unique"),
            "got: {err}"
        );
    }
}
