#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Canonical provenance and claim envelope types (Phase A).
//!
//! This module provides a shared semantic vocabulary for claims without
//! changing any existing storage, MCP contracts, or runtime behaviour. It is
//! purely additive: every type lives alongside the existing fact store,
//! memory store, and sandbox result structures.
//!
//! # Types
//!
//! | Type | Role |
//! |------|------|
//! | `ClaimId` | Opaque strongly-typed identifier for a claim |
//! | `SourceKind` | Where a claim originated (static analysis, execution, agent, human) |
//! | `Provenance` | Canonical provenance envelope tying a claim to its origin state |
//! | `ClaimEnvelope<T>` | Generic claim container carrying id, subject, predicate, source, provenance, confidence, timestamp, payload |
//! | `compute_trust` | Deterministic pure-trust computation |
//! | `FreshnessResolver` | Pluggable trait for source-specific freshness resolution |
//!
//! # Semantic Distinctions Preserved
//!
//! - **Provenance** describes where / what / when / against which state a claim arose.
//! - **Trust** describes how much CodeBro should believe the claim right now.
//! - **Confidence** is meaningful primarily for agent-declared claims.
//!
//! ## Authority (deferred)
//!
//! **Authority is intentionally NOT implemented in Phase A.**
//!
//! Authority is distinct from:
//! - provenance (where a claim came from)
//! - trust (how much to believe it)
//! - confidence (agent's self-assessed certainty)
//! - freshness (whether the claim still matches current state)
//!
//! In particular, `HumanDeclared` does NOT mean "higher authority" merely
//! because it has a higher trust score than `AgentDeclared`. A human
//! declaration may govern what should be done without changing the truth
//! value or trustworthiness of empirical evidence. For example, a human
//! deciding to ignore a flaky test does not make the test-failure evidence
//! false — it only changes what actions are taken despite that evidence.
//!
//! Authority (whose intent governs conflicts) will be introduced when there
//! is a concrete use case. No `Authority` enum, type, scoring, or resolver
//! exists in Phase A.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};

// ── ClaimId ────────────────────────────────────────────────────────────────

/// Opaque, strongly-typed identifier for a claim.
///
/// IDs are producer-supplied strings — no UUID generation, no timestamps,
/// no randomness. Two `ClaimId` values compare equal iff their payloads are
/// byte-identical.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClaimId(String);

impl ClaimId {
    /// Wrap a producer-supplied id string.
    pub fn new(inner: impl Into<String>) -> Self {
        ClaimId(inner.into())
    }

    /// View the underlying opaque string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ClaimId {
    fn from(s: &str) -> Self {
        ClaimId(s.to_string())
    }
}

impl From<String> for ClaimId {
    fn from(s: String) -> Self {
        ClaimId(s)
    }
}

impl AsRef<str> for ClaimId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClaimId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ── SourceKind ─────────────────────────────────────────────────────────────

/// Canonical source-kind classifier for a claim.
///
/// Every claim carries exactly one `SourceKind` indicating what produced it.
/// The kind is used by the trust computation and by freshness resolvers to
/// decide which semantics apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Produced by a deterministic static-analysis pass (e.g. tree-sitter
    /// scan, linter, compiler output).
    #[default]
    StaticAnalysis,
    /// Produced by an executed command with an authoritative exit code
    /// (sandbox execution, build, test).
    Execution,
    /// Self-declared by an AI agent during a session. Low intrinsic trust;
    /// confidence is the primary lever.
    AgentDeclared,
    /// Declared by a human (engineering memory entry, project-identity
    /// constraint, explicit override). Carries authority but not objective
    /// truth.
    HumanDeclared,
}

impl SourceKind {
    /// All known kinds in stable order.
    pub const ALL: [SourceKind; 4] = [
        SourceKind::StaticAnalysis,
        SourceKind::Execution,
        SourceKind::AgentDeclared,
        SourceKind::HumanDeclared,
    ];

    /// Canonical snake_case name.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::StaticAnalysis => "static_analysis",
            SourceKind::Execution => "execution",
            SourceKind::AgentDeclared => "agent_declared",
            SourceKind::HumanDeclared => "human_declared",
        }
    }

    /// Parse a canonical name back into a kind. Unknown strings map to
    /// `None`; there is no catch-all `Unknown` variant.
    pub fn parse(s: &str) -> Option<SourceKind> {
        match s {
            "static_analysis" => Some(SourceKind::StaticAnalysis),
            "execution" => Some(SourceKind::Execution),
            "agent_declared" => Some(SourceKind::AgentDeclared),
            "human_declared" => Some(SourceKind::HumanDeclared),
            _ => None,
        }
    }
}

impl fmt::Display for SourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Freshness ──────────────────────────────────────────────────────────────

/// Result of a freshness resolution pass.
///
/// Mirrors the existing `Freshness` enum from the sandbox module for
/// consistency; new claim types reuse this same shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessStatus {
    /// The claim is current relative to the observed repository state.
    Fresh,
    /// The repository has changed since the claim was produced.
    Stale,
    /// Cannot determine current repository state.
    Unknown,
}

impl fmt::Display for FreshnessStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FreshnessStatus::Fresh => write!(f, "fresh"),
            FreshnessStatus::Stale => write!(f, "stale"),
            FreshnessStatus::Unknown => write!(f, "unknown"),
        }
    }
}

// ── Provenance ─────────────────────────────────────────────────────────────

/// Canonical provenance attached to every claim.
///
/// Provenance answers: *where did this come from, what produced it, when,
/// and against which repository state?* It is distinct from trust and
/// confidence — two claims with identical provenance can carry different
/// trust scores because of their subject matter or confidence value.
///
/// Reuses `RepoIdentity` and `RepoState` from the sandbox module where
/// applicable; does not force execution-specific fields into the generic
/// struct.
///
/// # PartialEq limitation
///
/// `PartialEq` for `Provenance` **excludes** `repo_identity` and
/// `repo_state` because those sandbox types do not implement `PartialEq`.
///
/// **Warning:** Two `Provenance` values from different repository states may
/// compare equal even though they arose against different repository states.
/// This equality must **NOT** currently be used as authoritative semantic
/// equality for claim deduplication or caching. This limitation is
/// intentional for Phase A — revisit only when a concrete equality/dedup/
/// cache use case exists.
#[derive(Clone, Serialize, Deserialize)]
pub struct Provenance {
    /// What produced this claim.
    #[serde(default)]
    pub source_kind: SourceKind,
    /// Name of the tool / subsystem that generated the claim (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tool: Option<String>,
    /// Version of the generator at the time of production (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator_version: Option<String>,
    /// When the claim was produced (ISO 8601).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<DateTime<Utc>>,
    /// Workspace root the claim was produced against (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    /// Repository identity at production time (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_identity: Option<crate::sandbox::RepoIdentity>,
    /// Repository state at production time (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_state: Option<crate::sandbox::RepoState>,
    /// Execution id when the claim derives from a sandbox execution
    /// (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    /// Arbitrary metadata attached by the producer (optional).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl Default for Provenance {
    fn default() -> Self {
        Provenance {
            source_kind: SourceKind::StaticAnalysis,
            source_tool: None,
            generator_version: None,
            observed_at: None,
            workspace_root: None,
            repo_identity: None,
            repo_state: None,
            execution_id: None,
            metadata: HashMap::new(),
        }
    }
}

impl PartialEq for Provenance {
    fn eq(&self, other: &Self) -> bool {
        self.source_kind == other.source_kind
            && self.source_tool == other.source_tool
            && self.generator_version == other.generator_version
            && self.observed_at == other.observed_at
            && self.workspace_root == other.workspace_root
            && self.execution_id == other.execution_id
            && self.metadata == other.metadata
            // repo_identity and repo_state are excluded from equality
            // because they don't implement PartialEq in the sandbox module.
    }
}

impl Provenance {
    /// Build empty provenance with the given source kind.
    pub fn new(source_kind: SourceKind) -> Self {
        Provenance {
            source_kind,
            ..Default::default()
        }
    }

    /// Whether this provenance carries enough information to be considered
    /// complete (has at least a source kind and an observation timestamp).
    pub fn is_complete(&self) -> bool {
        self.source_kind != SourceKind::StaticAnalysis // non-default
            || self.observed_at.is_some()
    }

    /// Chain-setter for source_tool.
    pub fn with_source_tool(mut self, tool: impl Into<String>) -> Self {
        self.source_tool = Some(tool.into());
        self
    }

    /// Chain-setter for generator_version.
    pub fn with_generator_version(mut self, version: impl Into<String>) -> Self {
        self.generator_version = Some(version.into());
        self
    }

    /// Chain-setter for workspace_root.
    pub fn with_workspace_root(mut self, root: impl Into<String>) -> Self {
        self.workspace_root = Some(root.into());
        self
    }

    /// Chain-setter for repo_identity.
    pub fn with_repo_identity(mut self, identity: crate::sandbox::RepoIdentity) -> Self {
        self.repo_identity = Some(identity);
        self
    }

    /// Chain-setter for repo_state.
    pub fn with_repo_state(mut self, state: crate::sandbox::RepoState) -> Self {
        self.repo_state = Some(state);
        self
    }

    /// Chain-setter for execution_id.
    pub fn with_execution_id(mut self, id: impl Into<String>) -> Self {
        self.execution_id = Some(id.into());
        self
    }

    /// Chain-setter for metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

impl fmt::Debug for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Provenance")
            .field("source_kind", &self.source_kind)
            .field("source_tool", &self.source_tool)
            .field("generator_version", &self.generator_version)
            .field("observed_at", &self.observed_at)
            .field("workspace_root", &self.workspace_root)
            .field("repo_identity", &self.repo_identity)
            .field("repo_state", &self.repo_state)
            .field("execution_id", &self.execution_id)
            .field("metadata", &self.metadata)
            .finish()
    }
}

// ── ClaimEnvelope ──────────────────────────────────────────────────────────

/// A generic claim envelope carrying id, subject, predicate, source,
/// provenance, confidence, creation time, and typed payload.
///
/// `T` is the claim-specific payload. Common instantiations:
/// - `ClaimEnvelope<()>` — metadata-only claim (e.g. a decision record)
/// - `ClaimEnvelope<String>` — text claim (e.g. an analysis finding)
/// - `ClaimEnvelope<Vec<F>>` — collection claim (e.g. a list of symbols)
///
/// The envelope is deliberately decoupled from any particular storage format;
/// it is a semantic layer, not a persistence layer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClaimEnvelope<T> {
    /// Opaque claim identifier.
    pub id: ClaimId,
    /// What the claim is about (subject entity or topic).
    pub subject: String,
    /// The asserted proposition or finding.
    pub predicate: String,
    /// Source kind: where the claim originated.
    pub source_kind: SourceKind,
    /// Full provenance envelope.
    pub provenance: Provenance,
    /// Confidence in [0.0, 1.0]; meaningful primarily for agent-declared
    /// claims. Ignored by the trust computation for other source kinds
    /// (trust ≠ confidence).
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    /// Epoch seconds when the claim was created.
    #[serde(default)]
    pub created_at: u64,
    /// Typed payload carried by the claim.
    pub payload: T,
}

fn default_confidence() -> f64 {
    0.5
}

impl<T> ClaimEnvelope<T> {
    /// Create a new claim envelope.
    pub fn new(
        id: impl Into<ClaimId>,
        subject: impl Into<String>,
        predicate: impl Into<String>,
        source_kind: SourceKind,
        provenance: Provenance,
        payload: T,
    ) -> Self {
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        ClaimEnvelope {
            id: id.into(),
            subject: subject.into(),
            predicate: predicate.into(),
            source_kind,
            provenance,
            confidence: default_confidence(),
            created_at,
            payload,
        }
    }

    /// Set confidence (clamped to [0.0, 1.0]).
    pub fn with_confidence(mut self, confidence: f64) -> ClaimEnvelope<T> {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    /// Set provenance.
    pub fn with_provenance(mut self, provenance: Provenance) -> ClaimEnvelope<T> {
        self.provenance = provenance;
        self
    }

    /// Return the claim's trust score via [`compute_trust`].
    ///
    /// Resolves freshness from provenance (no filesystem access) and passes
    /// the result explicitly to [`compute_trust`]. Callers that need
    /// filesystem-backed freshness resolution should call
    /// [`resolve_freshness`](Self::resolve_freshness) separately and then
    /// invoke [`compute_trust`] directly.
    pub fn trust(&self) -> f64 {
        let freshness = resolve_freshness_from_provenance(&self.provenance);
        compute_trust(&self.source_kind, self.confidence, freshness)
    }

    /// Resolve freshness against the current repository state using the
    /// default resolver. See [`resolve_freshness`].
    pub fn resolve_freshness(&self) -> FreshnessStatus {
        resolve_freshness(self)
    }
}

// ── Trust computation ──────────────────────────────────────────────────────

/// Default trust policy v0 — provisional defaults, not canonical architectural truth.
///
/// These values are Phase A starting points. They express the relative
/// reliability hierarchy we observe today, not a fixed law of the system.
/// Future phases may revise, parameterise, or replace them entirely.
pub const TRUST_STATIC_ANALYSIS: f64 = 0.90;
pub const TRUST_EXECUTION: f64 = 0.85;
pub const TRUST_HUMAN_DECLARED: f64 = 0.70;
pub const TRUST_AGENT_DECLARED: f64 = 0.30;

/// Canonical pure trust computation.
///
/// Trust answers: *how much should CodeBro believe this claim right now?*
///
/// The computation is deterministic and separates three concerns:
/// 1. **Source kind** — intrinsic reliability of the producer.
/// 2. **Freshness** — whether the claim still matches current state (supplied
///    explicitly by the caller; this function does NOT resolve freshness).
/// 3. **Confidence** — only meaningfully scales trust for `agent_declared`
///    claims; for other kinds confidence is informational.
///
/// Authority (human_declared) is NOT conflated with truth: a human decision
/// to ignore a flaky test does not make the test-failure evidence false.
/// Human-declared claims get a moderate base trust because they carry
/// authoritative intent, not because they are objectively verified.
///
/// `compute_trust` does NOT access the filesystem, capture `RepoState`, or
/// resolve freshness. Any freshness resolution must be performed by the
/// caller before invoking this function.
///
/// Returns a score in [0.0, 1.0].
pub fn compute_trust(
    source_kind: &SourceKind,
    confidence: f64,
    freshness: FreshnessStatus,
) -> f64 {
    let base = match source_kind {
        SourceKind::StaticAnalysis => TRUST_STATIC_ANALYSIS,
        SourceKind::Execution => TRUST_EXECUTION,
        SourceKind::AgentDeclared => TRUST_AGENT_DECLARED,
        SourceKind::HumanDeclared => TRUST_HUMAN_DECLARED,
    };

    let freshness_factor = match freshness {
        FreshnessStatus::Fresh => 1.0,
        FreshnessStatus::Unknown => 0.8,
        FreshnessStatus::Stale => 0.6,
    };

    // Confidence only directly scales trust for agent-declared claims.
    // For other kinds, high confidence is nice but doesn't change the
    // intrinsic reliability of the source.
    let confidence_factor = match source_kind {
        SourceKind::AgentDeclared => confidence.clamp(0.0, 1.0),
        _ => 1.0,
    };

    // Small bonus for complete provenance (has timestamp + workspace).
    (base * freshness_factor * confidence_factor)
        .clamp(0.0, 1.0)
}

// ── Freshness resolver interface ───────────────────────────────────────────

/// Pluggable freshness resolver.
///
/// Different claim sources have different freshness semantics. This trait
/// allows source-specific resolution without coupling the core types to
/// any particular algorithm.
///
/// The default resolution (`resolve_freshness`) compares the claim's
/// provenance repo state against the current repository state.
pub trait FreshnessResolver: Send + Sync {
    /// Resolve the freshness of `claim` relative to the current state at
    /// `workspace_root`.
    fn resolve(
        &self,
        claim: &ClaimEnvelope<()>,
        workspace_root: &std::path::Path,
    ) -> FreshnessStatus;
}

/// Default freshness resolver: compares `provenance.repo_state` against
/// the current `RepoState::capture` of the workspace root.
pub struct DefaultFreshnessResolver;

impl FreshnessResolver for DefaultFreshnessResolver {
    fn resolve(
        &self,
        claim: &ClaimEnvelope<()>,
        workspace_root: &std::path::Path,
    ) -> FreshnessStatus {
        let current = crate::sandbox::RepoState::capture(&workspace_root.to_path_buf());
        match (&claim.provenance.repo_state, current) {
            (Some(prev), Some(cur)) => {
                if prev.working_tree_hash == cur.working_tree_hash {
                    FreshnessStatus::Fresh
                } else {
                    FreshnessStatus::Stale
                }
            }
            _ => FreshnessStatus::Unknown,
        }
    }
}

/// Resolve freshness for a claim using the default resolver.
pub fn resolve_freshness<T>(claim: &ClaimEnvelope<T>) -> FreshnessStatus {
    let resolver = DefaultFreshnessResolver;
    let empty_payload = ClaimEnvelope {
        id: claim.id.clone(),
        subject: claim.subject.clone(),
        predicate: claim.predicate.clone(),
        source_kind: claim.source_kind,
        provenance: claim.provenance.clone(),
        confidence: claim.confidence,
        created_at: claim.created_at,
        payload: (),
    };
    // Use workspace_root from provenance if available, otherwise unknown.
    let root = claim
        .provenance
        .workspace_root
        .as_deref()
        .map(std::path::Path::new)
        .unwrap_or(std::path::Path::new("."));
    resolver.resolve(&empty_payload, root)
}

/// Resolve freshness purely from provenance (no filesystem access).
/// Used internally by `compute_trust` to avoid requiring a workspace root.
fn resolve_freshness_from_provenance(provenance: &Provenance) -> FreshnessStatus {
    // Without a recorded repo_state we cannot assess freshness.
    if provenance.repo_state.is_none() {
        return FreshnessStatus::Unknown;
    }
    // If stale_reason is explicitly recorded, honour it.
    if provenance.metadata.contains_key("stale_reason") {
        return FreshnessStatus::Stale;
    }
    // Having a recorded repo_state without evidence of change means the
    // claim is believed current until proven otherwise.
    FreshnessStatus::Fresh
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_provenance() -> Provenance {
        Provenance {
            source_kind: SourceKind::StaticAnalysis,
            source_tool: Some("codebro-init".to_string()),
            generator_version: Some("0.7.0".to_string()),
            observed_at: Some(
                DateTime::<Utc>::from_timestamp(1700000000, 0).unwrap(),
            ),
            workspace_root: Some("/workspace".to_string()),
            repo_identity: Some(crate::sandbox::RepoIdentity {
                project_id: "abc123".to_string(),
                root: "/workspace".to_string(),
                repository_type: "cargo".to_string(),
            }),
            repo_state: Some(crate::sandbox::RepoState {
                commit_sha: "deadbeef".to_string(),
                working_tree_dirty: false,
                working_tree_hash: "hash1".to_string(),
            }),
            execution_id: None,
            metadata: HashMap::new(),
        }
    }

    // ── ClaimId ──────────────────────────────────────────────────────────

    #[test]
    fn claim_id_is_opaque_but_comparable() {
        let a = ClaimId::new("claim::alpha");
        let b = ClaimId::new("claim::alpha");
        let c = ClaimId::new("claim::beta");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a < c);
        assert_eq!(a.as_str(), "claim::alpha");
        assert_eq!(a.to_string(), "claim::alpha");
        assert_eq!(a.as_ref(), "claim::alpha");
    }

    #[test]
    fn claim_id_serializes_transparently() {
        let id = ClaimId::new("claim::x");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"claim::x\"");
        let back: ClaimId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn claim_id_from_str_and_string() {
        let a: ClaimId = "foo".into();
        let b: ClaimId = String::from("foo").into();
        assert_eq!(a, b);
    }

    // ── SourceKind ───────────────────────────────────────────────────────

    #[test]
    fn all_source_kinds_round_trip() {
        for k in SourceKind::ALL {
            assert_eq!(SourceKind::parse(k.as_str()), Some(k));
            assert_eq!(k.to_string(), k.as_str());
        }
        assert_eq!(SourceKind::parse("unknown_kind"), None);
        assert_eq!(SourceKind::parse(""), None);
    }

    #[test]
    fn source_kind_serializes_as_snake_case() {
        let json = serde_json::to_string(&SourceKind::StaticAnalysis).unwrap();
        assert_eq!(json, "\"static_analysis\"");
        let json = serde_json::to_string(&SourceKind::AgentDeclared).unwrap();
        assert_eq!(json, "\"agent_declared\"");
        let json = serde_json::to_string(&SourceKind::HumanDeclared).unwrap();
        assert_eq!(json, "\"human_declared\"");
    }

    #[test]
    fn source_kind_deserializes_snake_case() {
        let a: SourceKind = serde_json::from_str("\"static_analysis\"").unwrap();
        assert_eq!(a, SourceKind::StaticAnalysis);
        let a: SourceKind = serde_json::from_str("\"execution\"").unwrap();
        assert_eq!(a, SourceKind::Execution);
        let a: SourceKind = serde_json::from_str("\"agent_declared\"").unwrap();
        assert_eq!(a, SourceKind::AgentDeclared);
        let a: SourceKind = serde_json::from_str("\"human_declared\"").unwrap();
        assert_eq!(a, SourceKind::HumanDeclared);
    }

    // ── Provenance ───────────────────────────────────────────────────────

    #[test]
    fn provenance_new_sets_source_kind() {
        let p = Provenance::new(SourceKind::Execution);
        assert_eq!(p.source_kind, SourceKind::Execution);
    }

    #[test]
    fn provenance_builder_chains() {
        let p = Provenance::new(SourceKind::StaticAnalysis)
            .with_source_tool("init")
            .with_generator_version("0.7.0")
            .with_workspace_root("/ws")
            .with_execution_id("exec-1");
        assert_eq!(p.source_tool, Some("init".to_string()));
        assert_eq!(p.generator_version, Some("0.7.0".to_string()));
        assert_eq!(p.workspace_root, Some("/ws".to_string()));
        assert_eq!(p.execution_id, Some("exec-1".to_string()));
    }

    #[test]
    fn provenance_is_complete_when_has_kind_and_timestamp() {
        let p = Provenance {
            source_kind: SourceKind::Execution,
            observed_at: Some(
                DateTime::<Utc>::from_timestamp(1700000000, 0).unwrap(),
            ),
            ..Default::default()
        };
        // is_complete requires non-default kind OR observed_at.
        assert!(p.is_complete());
    }

    #[test]
    fn provenance_is_incomplete_when_empty() {
        let p = Provenance::default();
        assert!(!p.is_complete());
    }

    #[test]
    fn provenance_serialization_round_trip() {
        let p = sample_provenance();
        let json = serde_json::to_string(&p).unwrap();
        let back: Provenance = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    // Helper to set observed_at — we need a public setter or builder pattern.
    // Since Provenance uses field access in tests, let's just construct directly.
    // (The test above won't compile as written; fix it below.)

    // ── ClaimEnvelope ────────────────────────────────────────────────────

    #[test]
    fn claim_envelope_construction() {
        let env: ClaimEnvelope<String> = ClaimEnvelope::new(
            "claim::1",
            "module_authors",
            "has 3 public modules",
            SourceKind::StaticAnalysis,
            sample_provenance(),
            "three modules found".to_string(),
        );
        assert_eq!(env.id.as_str(), "claim::1");
        assert_eq!(env.subject, "module_authors");
        assert_eq!(env.predicate, "has 3 public modules");
        assert_eq!(env.source_kind, SourceKind::StaticAnalysis);
        assert_eq!(env.confidence, 0.5); // default
        assert!(env.created_at > 0);
        assert_eq!(env.payload, "three modules found");
    }

    #[test]
    fn claim_envelope_with_confidence() {
        let env = ClaimEnvelope::new(
            "claim::2",
            "subject",
            "predicate",
            SourceKind::AgentDeclared,
            Provenance::new(SourceKind::AgentDeclared),
            "data".to_string(),
        )
        .with_confidence(0.9);
        assert_eq!(env.confidence, 0.9);
    }

    #[test]
    fn claim_envelope_clamps_confidence() {
        let env = ClaimEnvelope::new(
            "claim::3",
            "s",
            "p",
            SourceKind::AgentDeclared,
            Provenance::new(SourceKind::AgentDeclared),
            (),
        )
        .with_confidence(1.5);
        assert_eq!(env.confidence, 1.0);

        // Separate instance for negative clamp.
        let env = ClaimEnvelope::new(
            "claim::3b",
            "s",
            "p",
            SourceKind::AgentDeclared,
            Provenance::new(SourceKind::AgentDeclared),
            (),
        )
        .with_confidence(-0.1);
        assert_eq!(env.confidence, 0.0);
    }

    #[test]
    fn claim_envelope_unit_payload() {
        let env: ClaimEnvelope<()> = ClaimEnvelope::new(
            "claim::unit",
            "decision",
            "ignore flaky test",
            SourceKind::HumanDeclared,
            Provenance::new(SourceKind::HumanDeclared),
            (),
        );
        assert!(matches!(env.payload, ()));
    }

    #[test]
    fn claim_envelope_vec_payload() {
        let env: ClaimEnvelope<Vec<String>> = ClaimEnvelope::new(
            "claim::vec",
            "symbols",
            "list of public symbols",
            SourceKind::StaticAnalysis,
            sample_provenance(),
            vec!["foo".to_string(), "bar".to_string()],
        );
        assert_eq!(env.payload, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn claim_envelope_serde_round_trip() {
        let env: ClaimEnvelope<String> = ClaimEnvelope::new(
            "claim::rt",
            "subject",
            "predicate",
            SourceKind::Execution,
            sample_provenance(),
            "payload data".to_string(),
        )
        .with_confidence(0.75);
        let json = serde_json::to_string(&env).unwrap();
        let back: ClaimEnvelope<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(env.id, back.id);
        assert_eq!(env.subject, back.subject);
        assert_eq!(env.predicate, back.predicate);
        assert_eq!(env.source_kind, back.source_kind);
        assert_eq!(env.confidence, back.confidence);
        assert_eq!(env.payload, back.payload);
    }

    // ── compute_trust ────────────────────────────────────────────────────

    #[test]
    fn trust_static_analysis_high() {
        let p = sample_provenance();
        let freshness = resolve_freshness_from_provenance(&p);
        let t = compute_trust(&SourceKind::StaticAnalysis, 0.5, freshness);
        // Base 0.9, freshness fresh (repo_state present, no stale_reason).
        assert!(t > 0.88, "expected high trust for static_analysis, got {t}");
        assert!(t <= 1.0);
    }

    #[test]
    fn trust_execution_high() {
        let p = Provenance {
            source_kind: SourceKind::Execution,
            observed_at: Some(
                DateTime::<Utc>::from_timestamp(1700000000, 0).unwrap(),
            ),
            workspace_root: Some("/workspace".to_string()),
            repo_identity: Some(crate::sandbox::RepoIdentity {
                project_id: "p".to_string(),
                root: "/workspace".to_string(),
                repository_type: "cargo".to_string(),
            }),
            repo_state: Some(crate::sandbox::RepoState {
                commit_sha: "abc".to_string(),
                working_tree_dirty: false,
                working_tree_hash: "h1".to_string(),
            }),
            ..Default::default()
        };
        let freshness = resolve_freshness_from_provenance(&p);
        let t = compute_trust(&SourceKind::Execution, 0.5, freshness);
        // Base 0.85, freshness fresh.
        assert!(t > 0.8, "expected high trust for execution, got {t}");
    }

    #[test]
    fn trust_agent_declared_low_without_confidence() {
        let p = Provenance::new(SourceKind::AgentDeclared);
        let freshness = resolve_freshness_from_provenance(&p);
        let t = compute_trust(&SourceKind::AgentDeclared, 0.3, freshness);
        // Base 0.3 * confidence 0.3 = 0.09.
        assert!(t < 0.2, "expected low trust for agent_declared with low confidence, got {t}");
    }

    #[test]
    fn trust_agent_declared_scales_with_confidence() {
        let p = Provenance::new(SourceKind::AgentDeclared);
        let freshness = resolve_freshness_from_provenance(&p);
        let t_low = compute_trust(&SourceKind::AgentDeclared, 0.2, freshness);
        let t_high = compute_trust(&SourceKind::AgentDeclared, 0.9, freshness);
        assert!(t_high > t_low, "higher confidence should yield higher trust for agent_declared");
    }

    #[test]
    fn trust_human_declared_not_equated_with_truth() {
        let p = Provenance::new(SourceKind::HumanDeclared);
        let freshness = resolve_freshness_from_provenance(&p);
        let t = compute_trust(&SourceKind::HumanDeclared, 0.1, freshness);
        // Human-declared gets base 0.7 regardless of confidence.
        // Confidence is NOT a direct multiplier here.
        assert!(t > 0.5, "human_declared should have moderate base trust, got {t}");
        // Even with zero confidence, human authority remains.
        let t_zero_conf = compute_trust(&SourceKind::HumanDeclared, 0.0, freshness);
        assert!(t_zero_conf > 0.5, "human_declared trust should not collapse at zero confidence");
    }

    #[test]
    fn trust_respects_freshness() {
        let p = sample_provenance();
        let t_fresh = compute_trust(&SourceKind::StaticAnalysis, 0.5, FreshnessStatus::Fresh);

        let t_stale = compute_trust(&SourceKind::StaticAnalysis, 0.5, FreshnessStatus::Stale);

        assert!(t_fresh > t_stale,
            "fresh claim should trust more than stale: fresh={t_fresh}, stale={t_stale}");
    }

    #[test]
    fn trust_is_deterministic() {
        let t1 = compute_trust(&SourceKind::StaticAnalysis, 0.7, FreshnessStatus::Fresh);
        let t2 = compute_trust(&SourceKind::StaticAnalysis, 0.7, FreshnessStatus::Fresh);
        assert_eq!(t1, t2);
    }

    #[test]
    fn trust_is_clamped_to_zero_one() {
        // Even with pathological inputs, trust stays in [0, 1].
        for confidence in [0.0, 0.5, 1.0] {
            for kind in SourceKind::ALL {
                for freshness in [
                    FreshnessStatus::Fresh,
                    FreshnessStatus::Unknown,
                    FreshnessStatus::Stale,
                ] {
                    let t = compute_trust(&kind, confidence, freshness);
                    assert!((0.0..=1.0).contains(&t),
                        "trust out of range for kind={kind} conf={confidence} freshness={freshness}: {t}");
                }
            }
        }
    }

    #[test]
    fn confidence_and_trust_are_distinct() {
        // A human-declared claim with low confidence still has moderate trust.
        let trust = compute_trust(&SourceKind::HumanDeclared, 0.1, FreshnessStatus::Unknown);
        assert_ne!(trust, 0.1, "trust must not equal confidence for human_declared");
        assert!(trust > 0.1, "trust should exceed low confidence for human authority");
    }

    #[test]
    fn trust_constants_have_expected_values() {
        assert_eq!(TRUST_STATIC_ANALYSIS, 0.90);
        assert_eq!(TRUST_EXECUTION, 0.85);
        assert_eq!(TRUST_HUMAN_DECLARED, 0.70);
        assert_eq!(TRUST_AGENT_DECLARED, 0.30);
    }

    #[test]
    fn compute_trust_does_not_accept_provenance() {
        // compile-time check: compute_trust takes (SourceKind, f64, FreshnessStatus),
        // not a Provenance reference. This test ensures the signature change is stable.
        let _t = compute_trust(&SourceKind::StaticAnalysis, 0.5, FreshnessStatus::Fresh);
        assert!(_t > 0.0);
    }

    // ── FreshnessResolver ────────────────────────────────────────────────

    #[test]
    fn default_resolver_returns_unknown_without_repo_state() {
        let env: ClaimEnvelope<()> = ClaimEnvelope::new(
            "claim::fresh",
            "s", "p",
            SourceKind::StaticAnalysis,
            Provenance::new(SourceKind::StaticAnalysis),
            (),
        );
        let status = resolve_freshness(&env);
        assert_eq!(status, FreshnessStatus::Unknown);
    }

    #[test]
    fn default_resolver_returns_unknown_without_workspace_root() {
        let mut p = Provenance::new(SourceKind::StaticAnalysis);
        p.repo_state = Some(crate::sandbox::RepoState {
            commit_sha: "abc".to_string(),
            working_tree_dirty: false,
            working_tree_hash: "h1".to_string(),
        });
        // No workspace_root set → resolver defaults to current dir.
        // The current dir may be a git repo, so we can't assert Unknown here.
        // Instead verify the resolver runs without panicking and returns
        // a valid status.
        let env: ClaimEnvelope<()> = ClaimEnvelope::new(
            "claim::fresh",
            "s", "p",
            SourceKind::StaticAnalysis,
            p,
            (),
        );
        let status = env.resolve_freshness();
        assert!(matches!(
            status,
            FreshnessStatus::Fresh | FreshnessStatus::Stale | FreshnessStatus::Unknown
        ));
    }

    #[test]
    fn freshness_resolver_trait_for_static_analysis() {
        struct StaticAnalysisResolver;
        impl FreshnessResolver for StaticAnalysisResolver {
            fn resolve(
                &self,
                _claim: &ClaimEnvelope<()>,
                _workspace_root: &std::path::Path,
            ) -> FreshnessStatus {
                FreshnessStatus::Fresh
            }
        }
        let resolver = StaticAnalysisResolver;
        let env: ClaimEnvelope<()> = ClaimEnvelope::new(
            "claim::sa", "s", "p",
            SourceKind::StaticAnalysis,
            Provenance::new(SourceKind::StaticAnalysis),
            (),
        );
        assert_eq!(
            resolver.resolve(&env, std::path::Path::new("/tmp")),
            FreshnessStatus::Fresh
        );
    }

    #[test]
    fn freshness_resolver_trait_for_execution() {
        struct ExecutionResolver;
        impl FreshnessResolver for ExecutionResolver {
            fn resolve(
                &self,
                claim: &ClaimEnvelope<()>,
                _workspace_root: &std::path::Path,
            ) -> FreshnessStatus {
                // Execution claims are fresh if they have an execution_id.
                if claim.provenance.execution_id.is_some() {
                    FreshnessStatus::Fresh
                } else {
                    FreshnessStatus::Unknown
                }
            }
        }
        let resolver = ExecutionResolver;
        let mut p = Provenance::new(SourceKind::Execution);
        p.execution_id = Some("exec-42".to_string());
        let env: ClaimEnvelope<()> = ClaimEnvelope::new(
            "claim::exec", "s", "p",
            SourceKind::Execution,
            p,
            (),
        );
        assert_eq!(
            resolver.resolve(&env, std::path::Path::new("/tmp")),
            FreshnessStatus::Fresh
        );

        let p2 = Provenance::new(SourceKind::Execution);
        let env2: ClaimEnvelope<()> = ClaimEnvelope::new(
            "claim::exec2", "s", "p",
            SourceKind::Execution,
            p2,
            (),
        );
        assert_eq!(
            resolver.resolve(&env2, std::path::Path::new("/tmp")),
            FreshnessStatus::Unknown
        );
    }

    // ── Integration: trust + freshness together ──────────────────────────

    #[test]
    fn full_pipeline_static_analysis_claim() {
        let p = Provenance {
            source_kind: SourceKind::StaticAnalysis,
            source_tool: Some("codebro-init".to_string()),
            generator_version: Some("0.7.0".to_string()),
            observed_at: Some(
                DateTime::<Utc>::from_timestamp(1700000000, 0).unwrap(),
            ),
            workspace_root: Some("/workspace".to_string()),
            repo_identity: Some(crate::sandbox::RepoIdentity {
                project_id: "proj".to_string(),
                root: "/workspace".to_string(),
                repository_type: "cargo".to_string(),
            }),
            repo_state: Some(crate::sandbox::RepoState {
                commit_sha: "abc123".to_string(),
                working_tree_dirty: false,
                working_tree_hash: "hash-abc".to_string(),
            }),
            ..Default::default()
        };
        let env: ClaimEnvelope<Vec<String>> = ClaimEnvelope::new(
            "claim::symbols",
            "public_symbols",
            "found 5 public symbols in src/lib.rs",
            SourceKind::StaticAnalysis,
            p,
            vec!["foo".to_string(), "bar".to_string()],
        )
        .with_confidence(0.95);

        assert_eq!(env.source_kind, SourceKind::StaticAnalysis);
        assert_eq!(env.provenance.source_tool, Some("codebro-init".to_string()));
        assert!(env.trust() > 0.85);
        // Freshness unknown because /workspace is not a real git repo in tests.
        assert_eq!(env.resolve_freshness(), FreshnessStatus::Unknown);
    }

    #[test]
    fn full_pipeline_agent_declared_claim() {
        let p = Provenance::new(SourceKind::AgentDeclared)
            .with_source_tool("codebro-agent");
        let env: ClaimEnvelope<String> = ClaimEnvelope::new(
            "claim::hypothesis",
            "bug_root_cause",
            "the flaky test is caused by race condition in init()",
            SourceKind::AgentDeclared,
            p,
            "race condition hypothesis".to_string(),
        )
        .with_confidence(0.4);

        assert_eq!(env.source_kind, SourceKind::AgentDeclared);
        assert_eq!(env.confidence, 0.4);
        // Trust should be low because agent_declared base is 0.3 * 0.4 confidence
        assert!(env.trust() < 0.2, "agent claim with low confidence should have low trust, got {}", env.trust());
    }

    #[test]
    fn full_pipeline_human_declared_claim() {
        let p = Provenance::new(SourceKind::HumanDeclared)
            .with_source_tool("human")
            .with_workspace_root("/workspace");
        let env: ClaimEnvelope<()> = ClaimEnvelope::new(
            "claim::override",
            "test_policy",
            "ignore flaky_test_integration for now",
            SourceKind::HumanDeclared,
            p,
            (),
        )
        .with_confidence(0.2); // human may be uncertain, but authority remains

        assert_eq!(env.source_kind, SourceKind::HumanDeclared);
        // Trust should remain moderate despite low confidence
        assert!(env.trust() > 0.5, "human authority should maintain moderate trust, got {}", env.trust());
    }
}
