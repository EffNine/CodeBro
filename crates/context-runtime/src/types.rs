//! Context-runtime domain model: authority, record kinds, records, events.

use serde::{Deserialize, Serialize};

/// Maximum characters in a record's canonical content.
pub const MAX_CONTENT_CHARS: usize = 4096;
/// Maximum characters in a record's kind namespace.
pub const MAX_NAMESPACE_CHARS: usize = 256;
/// Maximum evidence ids a record may cite.
pub const MAX_EVIDENCE_IDS: usize = 32;
/// Maximum related record ids a record may reference.
pub const MAX_RELATED_IDS: usize = 32;
/// Maximum characters in a task identity string.
pub const MAX_TASK_ID_CHARS: usize = 256;
/// Maximum characters of the extensible `extra_json` metadata object.
pub const MAX_EXTRA_JSON_CHARS: usize = 2048;
/// Maximum bytes of an event payload (evidence blobs are bounded).
pub const MAX_EVENT_PAYLOAD_BYTES: usize = 8192;
/// Maximum characters in an event kind string.
pub const MAX_EVENT_KIND_CHARS: usize = 64;
/// Maximum characters in a history summary (the FTS-indexed human-readable
/// line describing what happened; longer text belongs in `payload`).
pub const MAX_HISTORY_SUMMARY_CHARS: usize = 2000;
/// Maximum characters in a history dedup key (client-supplied idempotency).
pub const MAX_DEDUP_KEY_CHARS: usize = 256;
/// Maximum characters in a history source label.
pub const MAX_HISTORY_SOURCE_CHARS: usize = 256;

/// Authority of a durable context record — who stands behind it.
///
/// This is the provenance spine of the context store. The hierarchy is
/// *not* a ranking to be summed; it tells retrieval how to weigh a record
/// and how confidently a record may be revised:
///
/// | Authority | Meaning |
/// |-----------|---------|
/// | `UserConfirmed` | The user explicitly stated or approved this. Strongest; revisions require new confirmation. |
/// | `AiInferred` | An agent inferred this from observed work. Weaker, revisable; must cite evidence. |
/// | `Observed` | Machine-observed behaviour (an event, a repeated pattern) without semantic interpretation. Must cite evidence. |
/// | `ProjectDerived` | Produced by deterministic project pipelines (indexer, identity inference). |
/// | `Imported` | Came from an external source (export/import, another tool). Source recorded in `import_origin`. |
/// | `SystemDerived` | Computed by CodeBro itself (deterministic derivation). |
///
/// AI inference is never stored as user-confirmed truth: writing an
/// `AiInferred` or `Observed` record without at least one evidence event id
/// is rejected by the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    UserConfirmed,
    AiInferred,
    Observed,
    ProjectDerived,
    Imported,
    SystemDerived,
}

impl Authority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Authority::UserConfirmed => "user_confirmed",
            Authority::AiInferred => "ai_inferred",
            Authority::Observed => "observed",
            Authority::ProjectDerived => "project_derived",
            Authority::Imported => "imported",
            Authority::SystemDerived => "system_derived",
        }
    }
}

impl std::fmt::Display for Authority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for Authority {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user_confirmed" => Ok(Authority::UserConfirmed),
            "ai_inferred" => Ok(Authority::AiInferred),
            "observed" => Ok(Authority::Observed),
            "project_derived" => Ok(Authority::ProjectDerived),
            "imported" => Ok(Authority::Imported),
            "system_derived" => Ok(Authority::SystemDerived),
            other => Err(format!("unknown authority: {other}")),
        }
    }
}

/// What a context record is about.
///
/// Fact / Decision / Constraint records keep living in their canonical
/// JSON homes (verified facts, engineering memory, project identity); a
/// context record of those kinds may *reference* the canonical entity via
/// `related_ids` but never duplicates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    /// How the user tends to work / what they value (fingerprint attribute).
    Preference,
    /// A durable goal with rationale, distinct from a decision ("why", not "what").
    Intent,
    /// A hard or soft constraint that must be respected.
    Constraint,
    /// A recurring approach that proved useful.
    Pattern,
    /// A learned outcome — success, failure, rejection, supersession.
    Experience,
    /// A stated engineering principle.
    Principle,
    /// Communication/verbosity/terminology style.
    Style,
    /// Visual/design/product taste.
    Taste,
    /// Reserved: mirrors a verified fact (reference only).
    Fact,
    /// Reserved: mirrors an engineering decision (reference only).
    Decision,
    /// Reserved: skill-candidate metadata (skill lifecycle phase).
    Skill,
    /// Anything not covered above; `namespace` disambiguates.
    Other,
}

impl RecordKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordKind::Preference => "preference",
            RecordKind::Intent => "intent",
            RecordKind::Constraint => "constraint",
            RecordKind::Pattern => "pattern",
            RecordKind::Experience => "experience",
            RecordKind::Principle => "principle",
            RecordKind::Style => "style",
            RecordKind::Taste => "taste",
            RecordKind::Fact => "fact",
            RecordKind::Decision => "decision",
            RecordKind::Skill => "skill",
            RecordKind::Other => "other",
        }
    }
}

impl std::fmt::Display for RecordKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for RecordKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "preference" => Ok(RecordKind::Preference),
            "intent" => Ok(RecordKind::Intent),
            "constraint" => Ok(RecordKind::Constraint),
            "pattern" => Ok(RecordKind::Pattern),
            "experience" => Ok(RecordKind::Experience),
            "principle" => Ok(RecordKind::Principle),
            "style" => Ok(RecordKind::Style),
            "taste" => Ok(RecordKind::Taste),
            "fact" => Ok(RecordKind::Fact),
            "decision" => Ok(RecordKind::Decision),
            "skill" => Ok(RecordKind::Skill),
            "other" => Ok(RecordKind::Other),
            other => Err(format!("unknown record kind: {other}")),
        }
    }
}

/// Where a record applies. Current task intent can override project
/// records, which can override global ones; scope is the mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordScope {
    /// Applies across all projects (global fingerprint).
    Global,
    /// Applies to one workspace (project identity layer).
    Project,
    /// Applies to a single task only (short-lived override).
    Task,
}

impl RecordScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordScope::Global => "global",
            RecordScope::Project => "project",
            RecordScope::Task => "task",
        }
    }
}

impl std::fmt::Display for RecordScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for RecordScope {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "global" => Ok(RecordScope::Global),
            "project" => Ok(RecordScope::Project),
            "task" => Ok(RecordScope::Task),
            other => Err(format!("unknown record scope: {other}")),
        }
    }
}

/// Lifecycle state of a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
    Active,
    Superseded,
    Expired,
    Rejected,
}

impl RecordStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordStatus::Active => "active",
            RecordStatus::Superseded => "superseded",
            RecordStatus::Expired => "expired",
            RecordStatus::Rejected => "rejected",
        }
    }
}

impl std::fmt::Display for RecordStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for RecordStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(RecordStatus::Active),
            "superseded" => Ok(RecordStatus::Superseded),
            "expired" => Ok(RecordStatus::Expired),
            "rejected" => Ok(RecordStatus::Rejected),
            other => Err(format!("unknown record status: {other}")),
        }
    }
}

/// Highest knowledge-lifecycle stage a record has reached.
///
/// `Observed → Inferred → Confirmed`: an observation becomes an inference
/// when an agent interprets it, and confirmation promotes the record by
/// *superseding* the weaker one with `UserConfirmed` authority — the
/// original record stays in the store as `Superseded` for the audit trail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStage {
    Observed,
    Inferred,
    Confirmed,
}

impl LifecycleStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            LifecycleStage::Observed => "observed",
            LifecycleStage::Inferred => "inferred",
            LifecycleStage::Confirmed => "confirmed",
        }
    }
}

impl std::fmt::Display for LifecycleStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for LifecycleStage {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "observed" => Ok(LifecycleStage::Observed),
            "inferred" => Ok(LifecycleStage::Inferred),
            "confirmed" => Ok(LifecycleStage::Confirmed),
            other => Err(format!("unknown lifecycle stage: {other}")),
        }
    }
}

/// Lifecycle floor for an authority: the weakest lifecycle stage a record
/// of that authority may claim.
///
/// | Authority | Floor |
/// |-----------|-------|
/// | `UserConfirmed` | `Confirmed` |
/// | `AiInferred` | `Inferred` |
/// | `Observed` | `Observed` |
/// | `ProjectDerived` / `SystemDerived` / `Imported` | `Observed` (pipelines
/// declare their own stage; anything at or above counts) |
///
/// `LifecycleStage` and `Authority` describe the same knowledge journey from
/// two sides (stage reached vs who stands behind it). They are kept as
/// separate fields for backward compatibility with the P0 schema, but they
/// must never contradict: a record claiming `UserConfirmed` authority with
/// an `Observed` lifecycle stage is incoherent. The `remember` MCP
/// capability assigns both together via this function; direct store callers
/// should do the same. The store's shape validation accepts any stage
/// (existing P0 rows predate the rule) — the invariant is enforced at the
/// semantic write layer, where the caller principal is known.
pub fn lifecycle_for_authority(authority: Authority) -> LifecycleStage {
    match authority {
        Authority::UserConfirmed => LifecycleStage::Confirmed,
        Authority::AiInferred => LifecycleStage::Inferred,
        Authority::Observed
        | Authority::ProjectDerived
        | Authority::Imported
        | Authority::SystemDerived => LifecycleStage::Observed,
    }
}

/// Authority precedence rank for conflict resolution (higher wins).
///
/// Confirmation outranks derivation, derivation outranks observation, and
/// bare inference ranks lowest: an inferred project record must never
/// silently override a confirmed global one. Within equal authority, scope
/// specificity decides (see `fingerprint::scope_rank`).
pub fn authority_rank(authority: Authority) -> u8 {
    match authority {
        Authority::UserConfirmed => 100,
        Authority::ProjectDerived => 50,
        Authority::SystemDerived => 40,
        Authority::Imported => 30,
        Authority::Observed => 20,
        Authority::AiInferred => 10,
    }
}

/// A durable context record — the knowledge unit of the user-context store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextRecord {
    /// Opaque stable id (uuid). Unlike fact ids it is not path-derived:
    /// records are mutable entities.
    pub id: String,
    pub kind: RecordKind,
    /// Machine-facing attribute namespace, e.g. `fp.communication.verbosity`
    /// or `intent.portability`. The resolution unit of the fingerprint
    /// hierarchy (task > project > global).
    pub namespace: String,
    /// Canonical semantic content. For multilingual users this holds the
    /// interpreted meaning, not a transcription of the user's wording.
    pub content: String,
    /// Verbatim original text when the record was interpreted from user
    /// wording (evidence of the interpretation, distinct from its meaning).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_text: Option<String>,
    /// BCP-47-ish language/dialect tag of the original exchange
    /// (e.g. `en`, `ms`, `manglish`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub authority: Authority,
    /// Caller-declared confidence in [0,1]. Decays at retrieval time when
    /// evidence does not refresh the record; never permanently high.
    pub confidence: f64,
    /// Importance in [0,1] for tie-breaking and excerpt ordering.
    pub importance: f64,
    pub scope: RecordScope,
    /// Canonical workspace root for `Project`/`Task` scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    /// Task identity for `Task` scope (e.g. an OpenCode session or task
    /// id). Required when `scope` is `Task`, forbidden otherwise — a
    /// task-scoped record without a task binding is indistinguishable from
    /// another task's record and must be refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Extensible semantic metadata as a JSON object string (e.g. intent
    /// `rationale` / `priority` / `intent_status`). One schemaless column
    /// instead of dozens of rigid preference columns; the fingerprint and
    /// intent modules define the keys they own. Must parse as a JSON object
    /// and fit [`MAX_EXTRA_JSON_CHARS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_json: Option<String>,
    pub status: RecordStatus,
    pub lifecycle: LifecycleStage,
    /// Id of the record this one supersedes (audit trail).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// Provenance source label (free text, e.g. a session id or milestone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// For `Imported` records: where they came from (tool, export file...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_origin: Option<String>,
    /// Evidence: ids of the `events` that back this record. Required for
    /// `AiInferred` and `Observed` authorities.
    #[serde(default)]
    pub evidence: Vec<String>,
    /// References to canonical entities in other stores (fact/memory/
    /// identity ids) — never a duplicate copy of them.
    #[serde(default)]
    pub related_ids: Vec<String>,
    /// Unix seconds.
    pub created_at: u64,
    /// Unix seconds; bumped on every write.
    pub updated_at: u64,
    /// Unix seconds; when passed while active the record is expired by
    /// `expire_sweep`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

impl ContextRecord {
    /// Construct with safe defaults; callers override the fields they need.
    /// Timestamps are store-managed (`put_record` overwrites them).
    pub fn new(
        id: impl Into<String>,
        kind: RecordKind,
        namespace: impl Into<String>,
        content: impl Into<String>,
        authority: Authority,
    ) -> Self {
        ContextRecord {
            id: id.into(),
            kind,
            namespace: namespace.into(),
            content: content.into(),
            original_text: None,
            language: None,
            authority,
            confidence: 0.5,
            importance: 0.5,
            scope: RecordScope::Global,
            workspace_root: None,
            task_id: None,
            extra_json: None,
            status: RecordStatus::Active,
            lifecycle: LifecycleStage::Observed,
            supersedes: None,
            source: None,
            import_origin: None,
            evidence: Vec::new(),
            related_ids: Vec::new(),
            created_at: 0,
            updated_at: 0,
            expires_at: None,
        }
    }
}

/// One entry in the append-only observation event log.
///
/// Events are the evidence layer: records cite them, learning (later
/// phase) consumes them. Payloads are bounded and digested at write time so
/// evidence integrity can be verified without trusting the blob.
///
/// P2 (sessions + history + recall) extends the P0 shape with four additive
/// optional fields: `task_id` binds task-scoped history to its task (the
/// same identity rule as records — task history stays invisible without the
/// task); `summary` is the FTS-indexed human-readable line ("why did we
/// choose SQLite"); `dedup_key` is a client-supplied idempotency key;
/// `source` names the producer (e.g. `mcp:remember`, `mcp:sandbox_test`).
/// All four default to `None`, so P0/P1 rows and callers are unaffected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    /// Assigned by the store on insert (autoincrement).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// Session this event belongs to (session clustering, later phase).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Canonical workspace root the event occurred in.
    pub workspace_root: String,
    /// Task identity for task-scoped history. Task-scoped events are
    /// invisible to recall unless the caller names the task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Free-form event kind (e.g. `tool_called`, `mutation`, `verification`).
    /// The P2 history layer defines the closed [`crate::history::HistoryKind`]
    /// taxonomy it consumes; this field stays free-form for P0/P1 callers.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Short human-readable line describing what happened. FTS-indexed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Bounded JSON blob with the observed detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    /// Client-supplied idempotency key: recording the same key twice
    /// returns the original event instead of duplicating history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_key: Option<String>,
    /// Producer label (free text, e.g. `mcp:remember`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// SHA-256 hex of the payload bytes (set by the store).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// Unix seconds (store-managed).
    pub created_at: u64,
}

/// Validate a record's shape before persistence. Returns a description of
/// the first problem found.
pub fn validate_record(record: &ContextRecord) -> Result<(), String> {
    if record.id.is_empty() {
        return Err("record id must not be empty".to_string());
    }
    if record.namespace.is_empty() {
        return Err("record namespace must not be empty".to_string());
    }
    if record.namespace.chars().count() > MAX_NAMESPACE_CHARS {
        return Err(format!(
            "record namespace exceeds {MAX_NAMESPACE_CHARS} characters"
        ));
    }
    if record.content.trim().is_empty() {
        return Err("record content must not be empty".to_string());
    }
    if record.content.chars().count() > MAX_CONTENT_CHARS {
        return Err(format!(
            "record content exceeds {MAX_CONTENT_CHARS} characters"
        ));
    }
    if !(0.0..=1.0).contains(&record.confidence) {
        return Err(format!("confidence out of range: {}", record.confidence));
    }
    if !(0.0..=1.0).contains(&record.importance) {
        return Err(format!("importance out of range: {}", record.importance));
    }
    if record.evidence.len() > MAX_EVIDENCE_IDS {
        return Err(format!(
            "record cites more than {MAX_EVIDENCE_IDS} evidence ids"
        ));
    }
    if record.related_ids.len() > MAX_RELATED_IDS {
        return Err(format!(
            "record references more than {MAX_RELATED_IDS} related ids"
        ));
    }
    match record.authority {
        // AI inference and raw observation must always cite evidence.
        Authority::AiInferred | Authority::Observed if record.evidence.is_empty() => {
            return Err(format!(
                "{} records must cite at least one evidence event id",
                record.authority
            ));
        }
        _ => {}
    }
    match record.scope {
        RecordScope::Global => {
            if record.workspace_root.is_some() {
                return Err("global scope must not carry a workspace_root".to_string());
            }
            if record.task_id.is_some() {
                return Err("global scope must not carry a task_id".to_string());
            }
        }
        RecordScope::Project => {
            let ws = record.workspace_root.as_deref().unwrap_or("");
            if ws.trim().is_empty() {
                return Err(format!("{} scope requires a workspace_root", record.scope));
            }
            if record.task_id.is_some() {
                return Err("project scope must not carry a task_id (use task scope)".to_string());
            }
        }
        RecordScope::Task => {
            let ws = record.workspace_root.as_deref().unwrap_or("");
            if ws.trim().is_empty() {
                return Err(format!("{} scope requires a workspace_root", record.scope));
            }
            // Task-scope identity rule: without a task binding, one task's
            // override is indistinguishable from another's. Refuse it.
            let task = record.task_id.as_deref().unwrap_or("");
            if task.trim().is_empty() {
                return Err("task scope requires a task_id".to_string());
            }
            if task.chars().count() > MAX_TASK_ID_CHARS {
                return Err(format!("task_id exceeds {MAX_TASK_ID_CHARS} characters"));
            }
        }
    }
    // Reference-only reservation: Fact / Decision / Skill records mirror
    // canonical entities that live in the JSON stores. They may reference
    // them via `related_ids` but must never duplicate their content, so a
    // bare record of these kinds (no backlink) is refused.
    match record.kind {
        RecordKind::Fact | RecordKind::Decision | RecordKind::Skill
            if record.related_ids.is_empty() =>
        {
            return Err(format!(
                "{} records are reference-only: cite the canonical entity via related_ids",
                record.kind
            ));
        }
        _ => {}
    }
    if let Some(extra) = record.extra_json.as_deref() {
        if extra.chars().count() > MAX_EXTRA_JSON_CHARS {
            return Err(format!(
                "extra_json exceeds {MAX_EXTRA_JSON_CHARS} characters"
            ));
        }
        let parsed: serde_json::Value = serde_json::from_str(extra)
            .map_err(|e| format!("extra_json must be a JSON object: {e}"))?;
        if !parsed.is_object() {
            return Err("extra_json must be a JSON object".to_string());
        }
    }
    if let Some(supersedes) = record.supersedes.as_deref() {
        if supersedes == record.id {
            return Err("a record cannot supersede itself".to_string());
        }
    }
    Ok(())
}

/// Validate an event's shape before persistence.
pub fn validate_event(event: &EventRecord) -> Result<(), String> {
    if event.workspace_root.trim().is_empty() {
        return Err("event workspace_root must not be empty".to_string());
    }
    if event.kind.trim().is_empty() {
        return Err("event kind must not be empty".to_string());
    }
    if event.kind.chars().count() > MAX_EVENT_KIND_CHARS {
        return Err(format!(
            "event kind exceeds {MAX_EVENT_KIND_CHARS} characters"
        ));
    }
    if let Some(payload) = event.payload.as_deref() {
        if payload.len() > MAX_EVENT_PAYLOAD_BYTES {
            return Err(format!(
                "event payload exceeds {MAX_EVENT_PAYLOAD_BYTES} bytes ({} given)",
                payload.len()
            ));
        }
    }
    if let Some(summary) = event.summary.as_deref() {
        if summary.chars().count() > MAX_HISTORY_SUMMARY_CHARS {
            return Err(format!(
                "event summary exceeds {MAX_HISTORY_SUMMARY_CHARS} characters"
            ));
        }
    }
    if let Some(key) = event.dedup_key.as_deref() {
        if key.trim().is_empty() {
            return Err("event dedup_key must not be blank when supplied".to_string());
        }
        if key.chars().count() > MAX_DEDUP_KEY_CHARS {
            return Err(format!(
                "event dedup_key exceeds {MAX_DEDUP_KEY_CHARS} characters"
            ));
        }
    }
    if let Some(source) = event.source.as_deref() {
        if source.chars().count() > MAX_HISTORY_SOURCE_CHARS {
            return Err(format!(
                "event source exceeds {MAX_HISTORY_SOURCE_CHARS} characters"
            ));
        }
    }
    if let Some(task) = event.task_id.as_deref() {
        if task.trim().is_empty() {
            return Err("event task_id must not be blank when supplied".to_string());
        }
        if task.chars().count() > MAX_TASK_ID_CHARS {
            return Err(format!(
                "event task_id exceeds {MAX_TASK_ID_CHARS} characters"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(kind: RecordKind, authority: Authority) -> ContextRecord {
        ContextRecord::new("ctx::t1", kind, "fp.test.ns", "test content", authority)
    }

    #[test]
    fn authorities_roundtrip_through_strings() {
        for a in [
            Authority::UserConfirmed,
            Authority::AiInferred,
            Authority::Observed,
            Authority::ProjectDerived,
            Authority::Imported,
            Authority::SystemDerived,
        ] {
            assert_eq!(a.to_string().parse::<Authority>().unwrap(), a);
        }
        assert!("nonsense".parse::<Authority>().is_err());
    }

    #[test]
    fn record_kinds_roundtrip_through_strings() {
        for k in [
            RecordKind::Preference,
            RecordKind::Intent,
            RecordKind::Constraint,
            RecordKind::Pattern,
            RecordKind::Experience,
            RecordKind::Principle,
            RecordKind::Style,
            RecordKind::Taste,
            RecordKind::Fact,
            RecordKind::Decision,
            RecordKind::Skill,
            RecordKind::Other,
        ] {
            assert_eq!(k.to_string().parse::<RecordKind>().unwrap(), k);
        }
    }

    #[test]
    fn scopes_statuses_stages_roundtrip() {
        for s in [RecordScope::Global, RecordScope::Project, RecordScope::Task] {
            assert_eq!(s.to_string().parse::<RecordScope>().unwrap(), s);
        }
        for s in [
            RecordStatus::Active,
            RecordStatus::Superseded,
            RecordStatus::Expired,
            RecordStatus::Rejected,
        ] {
            assert_eq!(s.to_string().parse::<RecordStatus>().unwrap(), s);
        }
        for s in [
            LifecycleStage::Observed,
            LifecycleStage::Inferred,
            LifecycleStage::Confirmed,
        ] {
            assert_eq!(s.to_string().parse::<LifecycleStage>().unwrap(), s);
        }
    }

    #[test]
    fn inference_requires_evidence() {
        let rec = base(RecordKind::Preference, Authority::AiInferred);
        assert!(validate_record(&rec).unwrap_err().contains("evidence"));
        let mut rec = rec;
        rec.evidence.push("ev:1".to_string());
        assert!(validate_record(&rec).is_ok());
    }

    #[test]
    fn observed_requires_evidence_user_confirmed_does_not() {
        assert!(validate_record(&base(RecordKind::Preference, Authority::Observed)).is_err());
        assert!(validate_record(&base(RecordKind::Preference, Authority::UserConfirmed)).is_ok());
    }

    #[test]
    fn project_scope_requires_workspace_root() {
        let mut rec = base(RecordKind::Preference, Authority::UserConfirmed);
        rec.scope = RecordScope::Project;
        assert!(validate_record(&rec)
            .unwrap_err()
            .contains("workspace_root"));
        rec.workspace_root = Some("/work".to_string());
        assert!(validate_record(&rec).is_ok());
    }

    #[test]
    fn task_scope_requires_task_identity() {
        // Workspace alone is not enough: task scope without a task binding
        // is refused.
        let mut rec = base(RecordKind::Preference, Authority::UserConfirmed);
        rec.scope = RecordScope::Task;
        rec.workspace_root = Some("/work".to_string());
        assert!(validate_record(&rec).unwrap_err().contains("task_id"));
        // Empty/blank task ids are not identities either.
        rec.task_id = Some("   ".to_string());
        assert!(validate_record(&rec).unwrap_err().contains("task_id"));
        rec.task_id = Some("task-42".to_string());
        assert!(validate_record(&rec).is_ok());
    }

    #[test]
    fn task_id_is_forbidden_outside_task_scope() {
        // A task binding on a broader scope would silently narrow (or leak)
        // visibility; refuse it so the scope always means what it says.
        let mut rec = base(RecordKind::Preference, Authority::UserConfirmed);
        rec.scope = RecordScope::Project;
        rec.workspace_root = Some("/work".to_string());
        rec.task_id = Some("task-1".to_string());
        assert!(validate_record(&rec).unwrap_err().contains("task_id"));

        let mut global = base(RecordKind::Preference, Authority::UserConfirmed);
        global.task_id = Some("task-1".to_string());
        assert!(validate_record(&global).unwrap_err().contains("task_id"));

        let mut global_ws = base(RecordKind::Preference, Authority::UserConfirmed);
        global_ws.workspace_root = Some("/work".to_string());
        assert!(validate_record(&global_ws)
            .unwrap_err()
            .contains("workspace_root"));
    }

    #[test]
    fn oversized_task_id_rejected() {
        let mut rec = base(RecordKind::Preference, Authority::UserConfirmed);
        rec.scope = RecordScope::Task;
        rec.workspace_root = Some("/work".to_string());
        rec.task_id = Some("t".repeat(MAX_TASK_ID_CHARS + 1));
        assert!(validate_record(&rec).unwrap_err().contains("task_id"));
    }

    #[test]
    fn fact_decision_skill_kinds_require_canonical_backlink() {
        // Reference-only reservation: these kinds mirror the JSON stores
        // and must cite the canonical entity instead of duplicating it.
        for kind in [RecordKind::Fact, RecordKind::Decision, RecordKind::Skill] {
            let bare = base(kind, Authority::ProjectDerived);
            assert!(
                validate_record(&bare)
                    .unwrap_err()
                    .contains("reference-only"),
                "{kind} without related_ids must be refused"
            );
            let mut linked = base(kind, Authority::ProjectDerived);
            linked
                .related_ids
                .push("fact::some-canonical-id".to_string());
            assert!(
                validate_record(&linked).is_ok(),
                "{kind} with a backlink must be accepted"
            );
        }
        // Fingerprint kinds are unaffected.
        let mut pref = base(RecordKind::Preference, Authority::UserConfirmed);
        pref.related_ids.push("fact::optional-link".to_string());
        assert!(validate_record(&pref).is_ok());
        assert!(validate_record(&base(RecordKind::Intent, Authority::UserConfirmed)).is_ok());
    }

    #[test]
    fn extra_json_must_be_a_bounded_object() {
        let mut rec = base(RecordKind::Intent, Authority::UserConfirmed);
        rec.extra_json = Some(r#"{"intent_status":"active"}"#.to_string());
        assert!(validate_record(&rec).is_ok());

        rec.extra_json = Some(r#"["not","an","object"]"#.to_string());
        assert!(validate_record(&rec).unwrap_err().contains("object"));

        rec.extra_json = Some("not json at all".to_string());
        assert!(validate_record(&rec).is_err());

        rec.extra_json = Some(format!(
            "{{\"pad\":\"{}\"}}",
            "x".repeat(MAX_EXTRA_JSON_CHARS)
        ));
        assert!(validate_record(&rec).unwrap_err().contains("extra_json"));
    }

    #[test]
    fn lifecycle_floor_follows_authority() {
        assert_eq!(
            lifecycle_for_authority(Authority::UserConfirmed),
            LifecycleStage::Confirmed
        );
        assert_eq!(
            lifecycle_for_authority(Authority::AiInferred),
            LifecycleStage::Inferred
        );
        assert_eq!(
            lifecycle_for_authority(Authority::Observed),
            LifecycleStage::Observed
        );
    }

    #[test]
    fn authority_rank_orders_confirmation_above_inference() {
        assert!(
            authority_rank(Authority::UserConfirmed) > authority_rank(Authority::ProjectDerived)
        );
        assert!(authority_rank(Authority::ProjectDerived) > authority_rank(Authority::Observed));
        assert!(authority_rank(Authority::Observed) > authority_rank(Authority::AiInferred));
    }

    #[test]
    fn cannot_supersede_itself() {
        let mut rec = base(RecordKind::Preference, Authority::UserConfirmed);
        rec.supersedes = Some(rec.id.clone());
        assert!(validate_record(&rec).is_err());
    }

    #[test]
    fn oversized_content_and_namespace_rejected() {
        let rec = ContextRecord::new(
            "ctx::big",
            RecordKind::Preference,
            "ns",
            "x".repeat(MAX_CONTENT_CHARS + 1),
            Authority::UserConfirmed,
        );
        assert!(validate_record(&rec).is_err());

        let rec = ContextRecord::new(
            "ctx::big",
            RecordKind::Preference,
            "y".repeat(MAX_NAMESPACE_CHARS + 1),
            "content",
            Authority::UserConfirmed,
        );
        assert!(validate_record(&rec).is_err());
    }

    #[test]
    fn event_validation_bounds_payload_and_requires_workspace() {
        let ok = EventRecord {
            id: None,
            session_id: None,
            workspace_root: "/work".to_string(),
            task_id: None,
            kind: "verification".to_string(),
            tool: None,
            path: None,
            outcome: None,
            summary: None,
            payload: Some("{}".to_string()),
            dedup_key: None,
            source: None,
            digest: None,
            created_at: 0,
        };
        assert!(validate_event(&ok).is_ok());

        let mut no_ws = ok.clone();
        no_ws.workspace_root = "".to_string();
        assert!(validate_event(&no_ws).is_err());

        let mut big = ok.clone();
        big.payload = Some("x".repeat(MAX_EVENT_PAYLOAD_BYTES + 1));
        assert!(validate_event(&big).is_err());
    }

    #[test]
    fn serde_shape_is_snake_case_and_portable() {
        let rec = base(RecordKind::Intent, Authority::UserConfirmed);
        let json = serde_json::to_value(&rec).unwrap();
        assert_eq!(json["kind"], "intent");
        assert_eq!(json["authority"], "user_confirmed");
        assert_eq!(json["scope"], "global");
        assert_eq!(json["status"], "active");
        assert_eq!(json["lifecycle"], "observed");
        // Optional fields are omitted when absent: clean, small exports.
        assert!(json.get("original_text").is_none());
        assert!(json.get("workspace_root").is_none());
    }
}
