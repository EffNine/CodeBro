//! WS5 portability: explicit, verified export/import of durable memory.
//!
//! CodeBro's durable state lives in the user-level SQLite store
//! (`~/.codebro/state.db`): context records (preferences, intents,
//! constraints, learned experience), history sessions/events, and skills
//! with their immutable versions. Portability makes that state movable
//! across devices **without silent synchronization and without weakening
//! any trust boundary**:
//!
//! - **Explicit transport.** `codebro export` writes a directory;
//!   `codebro import` reads one. There is no daemon, no watcher, no cloud
//!   call — the operator moves bytes.
//! - **Deterministic format.** A versioned `manifest.json` (format
//!   version, counts, column lists, SHA-256 `data_hash`) plus one JSONL
//!   file per table. The layout matches the pre-existing portable mirror
//!   (`~/memory/export`) so legacy mirrors remain importable.
//! - **Integrity verified.** Import recomputes the hash over the parsed
//!   rows and refuses on mismatch. Legacy manifests without a format
//!   version fall back to count verification (their exporter may
//!   canonicalize floats differently) and say so in the report.
//! - **Redacted at both ends.** Every string value is passed through the
//!   canonical secret-redaction authority on export *and* on import
//!   (defense in depth), and event writes re-redact through the history
//!   seam.
//! - **Trust preserved.** Import copies authority verbatim: a
//!   `user_confirmed` record stays confirmed, an `ai_inferred` record
//!   never becomes confirmed. Evidence citations are remapped to the
//!   local events table — a citation that resolves nowhere is refused,
//!   never fabricated.
//! - **No newer state overwritten.** A record whose local copy has a
//!   newer `updated_at` is skipped; equal timestamps with different
//!   content are reported as conflicts; an import never downgrades a
//!   local `user_confirmed` record to a weaker authority.
//! - **Skills keep their lifecycle.** Import inserts missing lineages and
//!   candidates verbatim (validated: safe name, known scope/status,
//!   content hashes recomputed) and never approves, evaluates, or
//!   transitions anything. Existing local lineages are never merged.
//!
//! Import is a two-phase controlled state transition: everything is
//! parsed, verified, remapped, and validated **before** the first write.
//! Any malformed or corrupted row aborts the import with nothing written.
//! Apply itself is idempotent (records upsert by id; events dedup by a
//! content-addressed key; skills insert-if-absent), so a re-run converges
//! after an unexpected mid-apply failure.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::history::SessionRecord;
use crate::skills::{Skill, SkillCandidate, SkillVersion};
use crate::store::{ContextError, ContextStore};
use crate::types::{ContextRecord, EventRecord};

/// Current portable-format version. Bump when the manifest shape changes
/// incompatibly; imports refuse manifests newer than this.
pub const PORTABLE_FORMAT_VERSION: u32 = 1;
/// Manifest file name inside an export directory.
pub const MANIFEST_FILE: &str = "manifest.json";
/// Per-table row bound for one import.
pub const MAX_IMPORT_ROWS_PER_TABLE: usize = 50_000;
/// Total row bound for one import.
pub const MAX_IMPORT_TOTAL_ROWS: usize = 200_000;
/// Per-file byte bound for one import section.
pub const MAX_IMPORT_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Total byte bound for one import bundle.
pub const MAX_IMPORT_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

/// Tables written by an export (mirror completeness; matches the existing
/// `~/memory/export` layout).
pub const EXPORT_TABLES: &[&str] = &[
    "context_records",
    "events",
    "learning_candidates",
    "sessions",
    "skill_approval_requests",
    "skill_candidates",
    "skill_versions",
    "skills",
    "task_checkpoints",
    "tasks",
];

/// Tables import understands. Everything else is exported for mirror
/// completeness but reported as `not_imported` (device-local workflow
/// state that must not be silently resurrected on another machine).
pub const IMPORTED_TABLES: &[&str] = &[
    "sessions",
    "events",
    "context_records",
    "skills",
    "skill_versions",
    "skill_candidates",
];

// ── Manifest / reports ───────────────────────────────────────────────────

/// The versioned portable manifest. Unknown fields are ignored on read;
/// `format_version` is absent in legacy (pre-WS5 curator) mirrors.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableManifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_version: Option<u32>,
    /// Informational; seconds since the Unix epoch for CodeBro exports,
    /// free-form in legacy mirrors.
    #[serde(default)]
    pub generated_at: String,
    /// SHA-256 over the exported rows (sorted tables, compact JSON).
    pub data_hash: String,
    #[serde(default)]
    pub counts: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_db: Option<String>,
    /// Table → column names, as exported.
    #[serde(default)]
    pub tables: BTreeMap<String, Vec<String>>,
}

/// Result of one export.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportReport {
    pub out_dir: PathBuf,
    pub data_hash: String,
    pub counts: BTreeMap<String, usize>,
    /// Number of string values rewritten by secret redaction.
    pub redacted_values: usize,
}

/// Options for one import.
#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// Validate and report without writing anything.
    pub dry_run: bool,
    /// Explicit source-root → local-root remapping (repeatable in the CLI).
    pub workspace_map: BTreeMap<String, String>,
    /// Local workspace root applied to the bundle's single source root
    /// (ignored when the bundle spans several roots — explicit `--map` is
    /// then required).
    pub default_workspace: Option<String>,
    /// Provenance label written to imported records' `import_origin`.
    pub import_origin: String,
    /// OpenCode skills directory; when set, newly imported active skills
    /// publish their SKILL.md (never over an existing differing file).
    pub skills_root: Option<PathBuf>,
}

/// Per-table import outcome counts.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TableOutcome {
    pub inserted: usize,
    pub updated: usize,
    pub duplicates: usize,
    pub conflicts: usize,
    pub skipped: usize,
    pub rejected: usize,
}

/// Bounded import report: what happened (or would happen) per table.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportReport {
    pub dry_run: bool,
    /// `hash_verified` (strict) or `counts_verified` (legacy manifest).
    pub integrity: String,
    pub format_version: Option<u32>,
    pub tables: BTreeMap<String, TableOutcome>,
    /// Tables present in the bundle but deliberately not imported.
    pub not_imported: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

impl ImportReport {
    fn outcome(&mut self, table: &str) -> &mut TableOutcome {
        self.tables.entry(table.to_string()).or_default()
    }
}

// ── Export ───────────────────────────────────────────────────────────────

/// Export the portable tables from a state database into `out_dir`.
///
/// Read-only with respect to the live store (the connection is opened
/// `READ_ONLY`), deterministic (sorted tables/keys), and redacted: every
/// string value passes through the canonical secret-redaction authority
/// before it is written.
pub fn export_mirror(db_path: &Path, out_dir: &Path) -> Result<ExportReport, String> {
    if !db_path.is_file() {
        return Err(format!(
            "state database not found: {} (nothing to export)",
            db_path.display()
        ));
    }
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("open {} read-only: {e}", db_path.display()))?;

    let mut tables: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut columns: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut redacted_values = 0usize;

    for table in EXPORT_TABLES {
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |_| Ok(()),
            )
            .optional()
            .map_err(|e| format!("probe table {table}: {e}"))?
            .is_some();
        if !exists {
            continue;
        }
        let cols = table_columns(&conn, table)?;
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .map_err(|e| format!("select {table}: {e}"))?;
        let mut rows = Vec::new();
        let mapped = stmt
            .query_map([], |row| {
                let mut map = Map::new();
                for (i, col) in cols.iter().enumerate() {
                    map.insert(col.clone(), sql_value_to_json(row.get_ref(i)?));
                }
                Ok(Value::Object(map))
            })
            .map_err(|e| format!("read {table}: {e}"))?;
        for row in mapped {
            let mut value = row.map_err(|e| format!("read {table} row: {e}"))?;
            redact_value(&mut value, &mut redacted_values);
            if *table == "skill_versions" {
                // Redaction can rewrite skill content; the recorded hash
                // must describe the exported bytes or import would refuse
                // the lineage as corrupted.
                rehash_redacted_skill_version(&mut value);
            }
            rows.push(value);
        }
        columns.insert(table.to_string(), cols);
        tables.insert(table.to_string(), rows);
    }

    let data_hash = canonical_hash(&tables);
    let counts: BTreeMap<String, usize> = tables
        .iter()
        .map(|(table, rows)| (table.clone(), rows.len()))
        .collect();

    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    for (table, rows) in &tables {
        let mut body = String::new();
        for row in rows {
            body.push_str(&serde_json::to_string(row).map_err(|e| e.to_string())?);
            body.push('\n');
        }
        codebro_core::persistence::write_atomic(
            &out_dir.join(format!("{table}.jsonl")),
            body.as_bytes(),
        )
        .map_err(|e| format!("write {table}.jsonl: {e}"))?;
    }

    let manifest = PortableManifest {
        format_version: Some(PORTABLE_FORMAT_VERSION),
        generated_at: unix_now_secs().to_string(),
        data_hash: data_hash.clone(),
        counts: counts.clone(),
        source_db: Some(db_path.display().to_string()),
        tables: columns,
    };
    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(|e| format!("manifest: {e}"))?;
    codebro_core::persistence::write_atomic(&out_dir.join(MANIFEST_FILE), manifest_json.as_bytes())
        .map_err(|e| format!("write manifest: {e}"))?;
    codebro_core::persistence::write_atomic(&out_dir.join("README.md"), EXPORT_README.as_bytes())
        .map_err(|e| format!("write README: {e}"))?;

    Ok(ExportReport {
        out_dir: out_dir.to_path_buf(),
        data_hash,
        counts,
        redacted_values,
    })
}

const EXPORT_README: &str = "\
# CodeBro portable memory export

Generated by `codebro export`. Do not hand-edit: regenerate instead.

- `manifest.json` — format version, per-table counts, column lists, and the
  SHA-256 `data_hash` over the exported rows.
- `<table>.jsonl` — one JSON object per row, keys sorted.

Import with `codebro import --file <this directory> --root <workspace>`
(add `--dry-run` first). Import is explicit, verified, redacted, and never
overwrites newer local state or promotes authority.
";

// ── Load / verify ────────────────────────────────────────────────────────

struct LoadedBundle {
    manifest: PortableManifest,
    rows: BTreeMap<String, Vec<Value>>,
    integrity: &'static str,
}

fn load_bundle(dir: &Path) -> Result<LoadedBundle, String> {
    let manifest_path = dir.join(MANIFEST_FILE);
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("read {}: {e}", manifest_path.display()))?;
    let manifest: PortableManifest =
        serde_json::from_str(&raw).map_err(|e| format!("malformed manifest: {e}"))?;
    if let Some(version) = manifest.format_version {
        if version > PORTABLE_FORMAT_VERSION {
            return Err(format!(
                "manifest format_version {version} is newer than supported {PORTABLE_FORMAT_VERSION}; upgrade CodeBro to import it"
            ));
        }
    }
    if manifest.data_hash.trim().is_empty() {
        return Err("malformed manifest: data_hash is missing".to_string());
    }

    let mut rows: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut total = 0usize;
    let mut total_bytes = 0u64;
    for table in manifest.tables.keys() {
        // Table names become file names: only known sections are legal.
        // This is authorization *before* path construction — a crafted
        // manifest cannot make the importer read outside the bundle.
        if !EXPORT_TABLES.contains(&table.as_str()) {
            return Err(format!("malformed manifest: unknown table '{table}'"));
        }
        let path = dir.join(format!("{table}.jsonl"));
        if !path.is_file() {
            continue; // an absent section is an empty section
        }
        let size = std::fs::metadata(&path)
            .map_err(|e| format!("stat {}: {e}", path.display()))?
            .len();
        if size > MAX_IMPORT_FILE_BYTES {
            return Err(format!(
                "{table}.jsonl exceeds the {MAX_IMPORT_FILE_BYTES}-byte import bound"
            ));
        }
        total_bytes += size;
        if total_bytes > MAX_IMPORT_TOTAL_BYTES {
            return Err(format!(
                "bundle exceeds the {MAX_IMPORT_TOTAL_BYTES}-byte import bound"
            ));
        }
        let body =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let mut table_rows: Vec<Value> = Vec::new();
        for (index, line) in body.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(line)
                .map_err(|e| format!("malformed {table}.jsonl line {}: {e}", index + 1))?;
            if !value.is_object() {
                return Err(format!(
                    "{table}.jsonl line {} is not a JSON object",
                    index + 1
                ));
            }
            table_rows.push(value);
            if table_rows.len() > MAX_IMPORT_ROWS_PER_TABLE {
                return Err(format!(
                    "{table} exceeds the {MAX_IMPORT_ROWS_PER_TABLE}-row import bound"
                ));
            }
        }
        total += table_rows.len();
        if total > MAX_IMPORT_TOTAL_ROWS {
            return Err(format!(
                "bundle exceeds the {MAX_IMPORT_TOTAL_ROWS}-row import bound"
            ));
        }
        rows.insert(table.clone(), table_rows);
    }

    let recomputed = canonical_hash(&rows);
    let integrity = if recomputed == manifest.data_hash {
        "hash_verified"
    } else if manifest.format_version.is_some() {
        return Err(
            "integrity check failed: data_hash mismatch — the export is corrupted or was modified after writing"
                .to_string(),
        );
    } else {
        // Legacy mirror: its exporter may canonicalize floats differently,
        // so fall back to count verification and say so.
        let counts_ok = rows.iter().all(|(table, table_rows)| {
            manifest
                .counts
                .get(table)
                .map(|expected| *expected == table_rows.len())
                .unwrap_or(true)
        });
        if !counts_ok {
            return Err(
                "integrity check failed: data_hash mismatch and row counts disagree".to_string(),
            );
        }
        "counts_verified"
    };

    Ok(LoadedBundle {
        manifest,
        rows,
        integrity,
    })
}

// ── Import ───────────────────────────────────────────────────────────────

/// Import one exported mirror into the local store.
///
/// Two-phase: parse/verify/remap/validate everything first (any corrupted
/// row aborts with nothing written), then apply in dependency order
/// (sessions → events → records → skills → candidates). Idempotent.
pub fn import_mirror(
    store: &ContextStore,
    dir: &Path,
    options: &ImportOptions,
) -> Result<ImportReport, String> {
    let bundle = load_bundle(dir)?;
    let plan = build_plan(store, &bundle, options)?;

    let mut report = ImportReport {
        dry_run: options.dry_run,
        integrity: bundle.integrity.to_string(),
        format_version: bundle.manifest.format_version,
        tables: BTreeMap::new(),
        not_imported: BTreeMap::new(),
        warnings: plan.warnings.clone(),
    };
    for table in bundle.manifest.tables.keys() {
        if !IMPORTED_TABLES.contains(&table.as_str()) {
            report.not_imported.insert(
                table.clone(),
                "exported for mirror completeness; not imported (device-local workflow state)"
                    .to_string(),
            );
        }
    }

    if options.dry_run {
        report.tables.insert(
            "sessions".to_string(),
            TableOutcome {
                inserted: plan.sessions.len(),
                skipped: plan.skipped.get("sessions").copied().unwrap_or(0),
                ..Default::default()
            },
        );
        report.tables.insert(
            "events".to_string(),
            TableOutcome {
                inserted: plan.events.len(),
                skipped: plan.skipped.get("events").copied().unwrap_or(0),
                ..Default::default()
            },
        );
        let mut records = TableOutcome {
            skipped: plan.skipped.get("context_records").copied().unwrap_or(0),
            ..Default::default()
        };
        for record in &plan.records {
            match record.action {
                RecordAction::Insert => records.inserted += 1,
                RecordAction::Update => records.updated += 1,
                RecordAction::Duplicate => records.duplicates += 1,
                RecordAction::Conflict(_) => records.conflicts += 1,
                RecordAction::SkipOlder => records.skipped += 1,
            }
        }
        report.tables.insert("context_records".to_string(), records);
        report.tables.insert(
            "skills".to_string(),
            TableOutcome {
                inserted: plan.skills.len(),
                skipped: plan.skipped.get("skills").copied().unwrap_or(0),
                ..Default::default()
            },
        );
        report.tables.insert(
            "skill_versions".to_string(),
            TableOutcome {
                inserted: plan.skills.iter().map(|s| s.versions.len()).sum(),
                ..Default::default()
            },
        );
        report.tables.insert(
            "skill_candidates".to_string(),
            TableOutcome {
                inserted: plan.candidates.len(),
                skipped: plan.skipped.get("skill_candidates").copied().unwrap_or(0),
                ..Default::default()
            },
        );
        return Ok(report);
    }

    // ── Apply: sessions first so event linkage survives ──
    let mut session_ids: Vec<String> = Vec::new();
    for session in &plan.sessions {
        match store.import_session(session) {
            Ok(true) => {
                session_ids.push(session.id.clone());
                report.outcome("sessions").inserted += 1;
            }
            Ok(false) => report.outcome("sessions").duplicates += 1,
            Err(e) => return Err(format!("import session {}: {e}", session.id)),
        }
    }
    for _ in 0..plan.skipped.get("sessions").copied().unwrap_or(0) {
        report.outcome("sessions").skipped += 1;
    }

    // ── Apply: events, building the original→local id map ──
    let mut event_map: BTreeMap<i64, i64> = BTreeMap::new();
    for event in &plan.events {
        let original_id = event.id.unwrap_or_default();
        match store.import_event(event) {
            Ok((new_id, duplicate)) => {
                event_map.insert(original_id, new_id);
                if duplicate {
                    report.outcome("events").duplicates += 1;
                } else {
                    report.outcome("events").inserted += 1;
                }
            }
            Err(e) => {
                return Err(format!(
                    "import event {}: {e}",
                    event.id.map(|i| i.to_string()).unwrap_or_default()
                ))
            }
        }
    }
    for _ in 0..plan.skipped.get("events").copied().unwrap_or(0) {
        report.outcome("events").skipped += 1;
    }
    if !session_ids.is_empty() {
        let _ = store.recount_sessions(&session_ids);
    }

    // ── Apply: context records (evidence remapped to local ids) ──
    for record_plan in &plan.records {
        match &record_plan.action {
            RecordAction::Insert | RecordAction::Update => {
                let mut record = record_plan.record.clone();
                record.evidence = record
                    .evidence
                    .iter()
                    .map(
                        |id| match id.parse::<i64>().ok().and_then(|old| event_map.get(&old)) {
                            Some(new_id) => new_id.to_string(),
                            None => id.clone(),
                        },
                    )
                    .collect();
                record.import_origin = Some(options.import_origin.clone());
                if let Err(e) = store.import_record(&record) {
                    return Err(format!("import record {}: {e}", record.id));
                }
                match record_plan.action {
                    RecordAction::Insert => report.outcome("context_records").inserted += 1,
                    _ => report.outcome("context_records").updated += 1,
                }
            }
            RecordAction::Duplicate => report.outcome("context_records").duplicates += 1,
            RecordAction::Conflict(reason) => {
                report.outcome("context_records").conflicts += 1;
                if report.warnings.len() < 16 {
                    report.warnings.push(format!(
                        "conflict: record {} — {reason}",
                        record_plan.record.id
                    ));
                }
            }
            RecordAction::SkipOlder => report.outcome("context_records").skipped += 1,
        }
    }
    for _ in 0..plan.skipped.get("context_records").copied().unwrap_or(0) {
        report.outcome("context_records").skipped += 1;
    }

    // ── Apply: skills (insert-if-absent, then publish missing artifacts) ──
    for skill_plan in &plan.skills {
        match store.import_skill_lineage(&skill_plan.skill, &skill_plan.versions) {
            Ok(true) => {
                report.outcome("skills").inserted += 1;
                report.outcome("skill_versions").inserted += skill_plan.versions.len();
                if let Some(root) = options.skills_root.as_deref() {
                    publish_imported_skill(
                        root,
                        &skill_plan.skill,
                        &skill_plan.versions,
                        &mut report,
                    );
                }
            }
            Ok(false) => {
                report.outcome("skills").duplicates += 1;
                report.outcome("skill_versions").duplicates += skill_plan.versions.len();
            }
            Err(e) => return Err(format!("import skill {}: {e}", skill_plan.skill.skill_id)),
        }
    }
    for _ in 0..plan.skipped.get("skills").copied().unwrap_or(0) {
        report.outcome("skills").skipped += 1;
    }

    // ── Apply: skill candidates (status preserved verbatim) ──
    for candidate in &plan.candidates {
        match store.import_skill_candidate(candidate) {
            Ok(true) => report.outcome("skill_candidates").inserted += 1,
            Ok(false) => report.outcome("skill_candidates").duplicates += 1,
            Err(e) => {
                return Err(format!(
                    "import skill candidate {}: {e}",
                    candidate.candidate_id
                ))
            }
        }
    }
    for _ in 0..plan.skipped.get("skill_candidates").copied().unwrap_or(0) {
        report.outcome("skill_candidates").skipped += 1;
    }

    Ok(report)
}

fn publish_imported_skill(
    root: &Path,
    skill: &Skill,
    versions: &[SkillVersion],
    report: &mut ImportReport,
) {
    if !matches!(skill.status.as_str(), "active" | "approved") {
        return;
    }
    let Some(current) = versions
        .iter()
        .find(|v| v.version_number == skill.current_version)
    else {
        return;
    };
    match crate::skills::read_skill_file_at(root, &skill.name) {
        Ok(None) => {
            if let Err(e) =
                crate::skills::publish_skill_file(root, &skill.name, &current.content, None)
            {
                report.warnings.push(format!(
                    "skill '{}' imported but not published: {e}",
                    skill.name
                ));
            }
        }
        Ok(Some(existing)) => {
            if crate::skills::content_hash(&existing) != current.content_hash
                && report.warnings.len() < 16
            {
                report.warnings.push(format!(
                    "skill '{}' imported, but an existing SKILL.md differs — left untouched",
                    skill.name
                ));
            }
        }
        Err(e) => report
            .warnings
            .push(format!("skill '{}' file check failed: {e}", skill.name)),
    }
}

// ── Planning ─────────────────────────────────────────────────────────────

enum RecordAction {
    Insert,
    Update,
    Duplicate,
    Conflict(String),
    SkipOlder,
}

struct RecordPlan {
    record: ContextRecord,
    action: RecordAction,
}

struct SkillPlan {
    skill: Skill,
    versions: Vec<SkillVersion>,
}

struct ImportPlan {
    sessions: Vec<SessionRecord>,
    events: Vec<EventRecord>,
    records: Vec<RecordPlan>,
    skills: Vec<SkillPlan>,
    candidates: Vec<SkillCandidate>,
    warnings: Vec<String>,
    skipped: BTreeMap<String, usize>,
}

impl ImportPlan {
    fn skip(&mut self, table: &str) {
        *self.skipped.entry(table.to_string()).or_default() += 1;
    }
}

fn build_plan(
    store: &ContextStore,
    bundle: &LoadedBundle,
    options: &ImportOptions,
) -> Result<ImportPlan, String> {
    let mut plan = ImportPlan {
        sessions: Vec::new(),
        events: Vec::new(),
        records: Vec::new(),
        skills: Vec::new(),
        candidates: Vec::new(),
        warnings: Vec::new(),
        skipped: BTreeMap::new(),
    };

    // Distinct non-global source roots decide whether the default
    // workspace mapping is unambiguous.
    let mut source_roots: BTreeSet<String> = BTreeSet::new();
    for table in IMPORTED_TABLES {
        for row in bundle.rows.get(*table).into_iter().flatten() {
            if let Some(ws) = row.get("workspace_root").and_then(Value::as_str) {
                if !ws.trim().is_empty() {
                    source_roots.insert(ws.to_string());
                }
            }
        }
    }
    let auto_default: Option<String> = match (&options.default_workspace, source_roots.len()) {
        (Some(target), 1) => Some(target.clone()),
        (Some(_), n) if n > 1 => {
            plan.warnings.push(format!(
                "bundle spans {n} workspaces; the default workspace mapping is ambiguous — pass explicit --map entries (unmapped project records and skills are skipped; history is imported verbatim)"
            ));
            None
        }
        _ => None,
    };
    let remap = |ws: &str, plan: &mut ImportPlan, table: &str| -> Option<String> {
        if let Some(mapped) = options.workspace_map.get(ws) {
            return Some(crate::workspace::canonical_workspace_key(mapped));
        }
        if let Some(target) = &auto_default {
            return Some(crate::workspace::canonical_workspace_key(target));
        }
        plan.skip(table);
        None
    };

    // Sessions and events are *evidence*, not project knowledge: they are
    // imported verbatim (remapped only when their workspace is mapped) so
    // citations from imported global records always resolve. Their
    // original workspace root is preserved otherwise — history stays
    // truthful even when the other project is not checked out here.
    for row in bundle.rows.get("sessions").into_iter().flatten() {
        let mut session = session_from_row(row)?;
        if let Some(mapped) = mapped_workspace(&session.workspace_root, options, &auto_default) {
            session.workspace_root = mapped;
        }
        plan.sessions.push(session);
    }

    for row in bundle.rows.get("events").into_iter().flatten() {
        let mut event = event_from_row(row)?;
        if let Some(mapped) = mapped_workspace(&event.workspace_root, options, &auto_default) {
            event.workspace_root = mapped;
        }
        plan.events.push(event);
    }
    let planned_event_ids: BTreeSet<i64> = plan.events.iter().filter_map(|e| e.id).collect();

    // Records
    for row in bundle.rows.get("context_records").into_iter().flatten() {
        let mut record = record_from_row(row)?;
        if record.scope != crate::types::RecordScope::Global {
            let source_ws = record.workspace_root.clone().unwrap_or_default();
            match remap(&source_ws, &mut plan, "context_records") {
                Some(target) => record.workspace_root = Some(target),
                None => continue,
            }
        }
        // Evidence citations must resolve locally or inside this bundle;
        // nothing is ever fabricated or silently dropped.
        for cited in &record.evidence {
            let parsed = cited.trim().parse::<i64>().ok();
            let resolves = match parsed {
                Some(id) if planned_event_ids.contains(&id) => true,
                Some(id) => store
                    .get_event(id)
                    .map_err(|e| format!("resolve evidence {id}: {e}"))?
                    .is_some(),
                None => false,
            };
            if !resolves {
                return Err(format!(
                    "corrupted record {}: evidence id {cited:?} resolves neither to a bundled event nor to local history",
                    record.id
                ));
            }
        }
        let action = match store
            .get_record(&record.id)
            .map_err(|e| format!("read local record {}: {e}", record.id))?
        {
            None => RecordAction::Insert,
            Some(local) => decide_merge(&local, &record),
        };
        plan.records.push(RecordPlan { record, action });
    }

    // Skills: group versions by lineage, validate every version has its
    // skill (a dangling version is corruption, not a skip).
    let mut versions_by_skill: BTreeMap<String, Vec<SkillVersion>> = BTreeMap::new();
    for row in bundle.rows.get("skill_versions").into_iter().flatten() {
        let version = skill_version_from_row(row)?;
        versions_by_skill
            .entry(version.skill_id.clone())
            .or_default()
            .push(version);
    }
    for row in bundle.rows.get("skills").into_iter().flatten() {
        let mut skill = skill_from_row(row)?;
        // Take the versions with the skill even when the lineage is
        // skipped: an unmapped project's versions must not linger as
        // dangling references.
        let versions = versions_by_skill
            .remove(&skill.skill_id)
            .unwrap_or_default();
        if let Some(ws) = skill.workspace_root.clone() {
            match remap(&ws, &mut plan, "skills") {
                Some(target) => skill.workspace_root = Some(target),
                None => continue,
            }
        }
        if versions.is_empty() {
            return Err(format!(
                "corrupted skill {}: no version rows in the bundle",
                skill.skill_id
            ));
        }
        plan.skills.push(SkillPlan { skill, versions });
    }
    if let Some(skill_id) = versions_by_skill.keys().next() {
        return Err(format!(
            "corrupted bundle: skill_versions reference unknown skill {skill_id}"
        ));
    }

    // Candidates
    for row in bundle.rows.get("skill_candidates").into_iter().flatten() {
        let mut candidate = skill_candidate_from_row(row)?;
        if let Some(ws) = candidate.workspace_root.clone() {
            match remap(&ws, &mut plan, "skill_candidates") {
                Some(target) => candidate.workspace_root = Some(target),
                None => continue,
            }
        }
        plan.candidates.push(candidate);
    }

    Ok(plan)
}

/// The mapped local root for a source workspace, when a mapping applies.
fn mapped_workspace(
    ws: &str,
    options: &ImportOptions,
    auto_default: &Option<String>,
) -> Option<String> {
    if let Some(mapped) = options.workspace_map.get(ws) {
        return Some(crate::workspace::canonical_workspace_key(mapped));
    }
    auto_default
        .as_ref()
        .map(|target| crate::workspace::canonical_workspace_key(target))
}

/// Merge decision for one imported record against its local copy.
/// Trust rules: never overwrite newer local state, never downgrade a
/// confirmed local record to a weaker authority, never rewrite
/// equal-timestamp different-content rows (report the conflict).
fn decide_merge(local: &ContextRecord, imported: &ContextRecord) -> RecordAction {
    if imported.updated_at < local.updated_at {
        return RecordAction::SkipOlder;
    }
    if imported.updated_at == local.updated_at {
        if same_record(local, imported) {
            return RecordAction::Duplicate;
        }
        return RecordAction::Conflict(
            "same updated_at with different content (deterministic refusal)".to_string(),
        );
    }
    let imported_rank = crate::types::authority_rank(imported.authority);
    let local_rank = crate::types::authority_rank(local.authority);
    if imported_rank < local_rank {
        return RecordAction::Conflict(format!(
            "imported authority {} is weaker than local {} (never downgrade confirmed knowledge)",
            imported.authority, local.authority
        ));
    }
    RecordAction::Update
}

fn same_record(a: &ContextRecord, b: &ContextRecord) -> bool {
    // Evidence ids are local storage coordinates: after an import they are
    // remapped into this device's event id space, so equal-length
    // citations are equivalent provenance, not a content difference.
    // Different citation counts remain a conflict.
    if a.evidence.len() != b.evidence.len() {
        return false;
    }
    let mut a = a.clone();
    let mut b = b.clone();
    // Import origin is bookkeeping, not content.
    a.import_origin = None;
    b.import_origin = None;
    a.evidence.clear();
    b.evidence.clear();
    a == b
}

// ── Row mapping ──────────────────────────────────────────────────────────

fn as_object<'a>(row: &'a Value, table: &str) -> Result<&'a Map<String, Value>, String> {
    row.as_object()
        .ok_or_else(|| format!("{table} row is not a JSON object"))
}

fn required<'a>(row: &'a Map<String, Value>, key: &str, table: &str) -> Result<&'a Value, String> {
    match row.get(key) {
        Some(Value::Null) | None => Err(format!("{table} row is missing '{key}'")),
        Some(value) => Ok(value),
    }
}

fn required_str(row: &Map<String, Value>, key: &str, table: &str) -> Result<String, String> {
    required(row, key, table)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{table} row field '{key}' is not a string"))
}

fn optional_str(row: &Map<String, Value>, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).map(str::to_string)
}

fn required_u64(row: &Map<String, Value>, key: &str, table: &str) -> Result<u64, String> {
    required(row, key, table)?
        .as_u64()
        .ok_or_else(|| format!("{table} row field '{key}' is not an unsigned integer"))
}

fn optional_u64(row: &Map<String, Value>, key: &str) -> Option<u64> {
    row.get(key).and_then(Value::as_u64)
}

fn required_f64(row: &Map<String, Value>, key: &str, table: &str) -> Result<f64, String> {
    required(row, key, table)?
        .as_f64()
        .ok_or_else(|| format!("{table} row field '{key}' is not a number"))
}

/// JSON column that is stored as a JSON string (`evidence_json`); an
/// unparseable non-empty string is corruption, never silently dropped.
fn json_column(row: &Map<String, Value>, key: &str, table: &str) -> Result<Option<Value>, String> {
    match row.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => serde_json::from_str(s)
            .map(Some)
            .map_err(|e| format!("{table} row field '{key}' is not valid JSON: {e}")),
        Some(value @ (Value::Array(_) | Value::Object(_))) => Ok(Some(value.clone())),
        Some(other) => Err(format!(
            "{table} row field '{key}' has unexpected type {}",
            other
        )),
    }
}

fn record_from_row(row: &Value) -> Result<ContextRecord, String> {
    let row = as_object(row, "context_records")?;
    let mut mapped = Map::new();
    mapped.insert("id".into(), required(row, "id", "context_records")?.clone());
    mapped.insert(
        "kind".into(),
        required(row, "record_type", "context_records")?.clone(),
    );
    mapped.insert(
        "namespace".into(),
        required(row, "namespace", "context_records")?.clone(),
    );
    mapped.insert(
        "content".into(),
        required(row, "content", "context_records")?.clone(),
    );
    if let Some(v) = optional_str(row, "original_text") {
        mapped.insert("original_text".into(), Value::String(v));
    }
    if let Some(v) = optional_str(row, "language") {
        mapped.insert("language".into(), Value::String(v));
    }
    mapped.insert(
        "authority".into(),
        required(row, "authority", "context_records")?.clone(),
    );
    mapped.insert(
        "confidence".into(),
        required(row, "confidence", "context_records")?.clone(),
    );
    mapped.insert(
        "importance".into(),
        required(row, "importance", "context_records")?.clone(),
    );
    mapped.insert(
        "scope".into(),
        required(row, "scope", "context_records")?.clone(),
    );
    if let Some(v) = optional_str(row, "workspace_root") {
        mapped.insert("workspace_root".into(), Value::String(v));
    }
    if let Some(v) = optional_str(row, "task_id") {
        mapped.insert("task_id".into(), Value::String(v));
    }
    if let Some(v) = optional_str(row, "extra_json") {
        mapped.insert("extra_json".into(), Value::String(v));
    }
    mapped.insert(
        "status".into(),
        required(row, "status", "context_records")?.clone(),
    );
    mapped.insert(
        "lifecycle".into(),
        required(row, "lifecycle", "context_records")?.clone(),
    );
    if let Some(v) = optional_str(row, "supersedes") {
        mapped.insert("supersedes".into(), Value::String(v));
    }
    if let Some(v) = optional_str(row, "source") {
        mapped.insert("source".into(), Value::String(v));
    }
    mapped.insert(
        "evidence".into(),
        json_column(row, "evidence_json", "context_records")?
            .unwrap_or_else(|| Value::Array(Vec::new())),
    );
    mapped.insert(
        "related_ids".into(),
        json_column(row, "related_json", "context_records")?
            .unwrap_or_else(|| Value::Array(Vec::new())),
    );
    mapped.insert(
        "created_at".into(),
        required(row, "created_at", "context_records")?.clone(),
    );
    mapped.insert(
        "updated_at".into(),
        required(row, "updated_at", "context_records")?.clone(),
    );
    if let Some(v) = optional_u64(row, "expires_at") {
        mapped.insert("expires_at".into(), Value::from(v));
    }
    let record: ContextRecord = serde_json::from_value(Value::Object(mapped))
        .map_err(|e| format!("corrupted context_records row: {e}"))?;
    crate::types::validate_record(&record)
        .map_err(|e| format!("corrupted context_records row: {e}"))?;
    Ok(record)
}

fn event_from_row(row: &Value) -> Result<EventRecord, String> {
    let row = as_object(row, "events")?;
    let event = EventRecord {
        id: optional_u64(row, "id").map(|v| v as i64),
        session_id: optional_str(row, "session_id"),
        workspace_root: required_str(row, "workspace_root", "events")?,
        task_id: optional_str(row, "task_id"),
        kind: required_str(row, "kind", "events")?,
        tool: optional_str(row, "tool"),
        path: optional_str(row, "path"),
        outcome: optional_str(row, "outcome"),
        summary: optional_str(row, "summary"),
        payload: optional_str(row, "payload_json"),
        dedup_key: optional_str(row, "dedup_key"),
        source: optional_str(row, "source"),
        digest: optional_str(row, "digest"),
        created_at: required_u64(row, "created_at", "events")?,
    };
    crate::types::validate_event(&event).map_err(|e| format!("corrupted events row: {e}"))?;
    Ok(event)
}

fn session_from_row(row: &Value) -> Result<SessionRecord, String> {
    let row = as_object(row, "sessions")?;
    let started_at = required_u64(row, "opened_at", "sessions")?;
    let status_raw = optional_str(row, "status").unwrap_or_else(|| "active".to_string());
    let status = status_raw
        .parse::<crate::history::SessionStatus>()
        .map_err(|_| format!("corrupted sessions row: unknown status '{status_raw}'"))?;
    Ok(SessionRecord {
        id: required_str(row, "id", "sessions")?,
        workspace_root: required_str(row, "workspace_root", "sessions")?,
        task_id: optional_str(row, "task_id"),
        title: optional_str(row, "title"),
        source: optional_str(row, "source"),
        status,
        parent_session_id: optional_str(row, "parent_session_id"),
        started_at,
        updated_at: optional_u64(row, "updated_at").unwrap_or(started_at),
        ended_at: optional_u64(row, "closed_at"),
        end_reason: optional_str(row, "end_reason"),
        event_count: 0,
        snapshot_json: optional_str(row, "context_snapshot_json"),
    })
}

fn skill_from_row(row: &Value) -> Result<Skill, String> {
    let row = as_object(row, "skills")?;
    let mut mapped = Map::new();
    for key in [
        "skill_id",
        "workspace_root",
        "scope",
        "name",
        "description",
        "current_version",
        "status",
        "confidence",
        "source_candidate_id",
        "superseded_by",
        "created_at",
        "updated_at",
    ] {
        if let Some(value) = row.get(key) {
            if !value.is_null() {
                mapped.insert(key.to_string(), value.clone());
            }
        }
    }
    if let Some(v) = json_column(row, "applicability_json", "skills")? {
        mapped.insert("applicability".into(), v);
    }
    if let Some(v) = json_column(row, "health_json", "skills")? {
        mapped.insert("health".into(), v);
    }
    serde_json::from_value(Value::Object(mapped)).map_err(|e| format!("corrupted skills row: {e}"))
}

fn skill_version_from_row(row: &Value) -> Result<SkillVersion, String> {
    let row = as_object(row, "skill_versions")?;
    let mut mapped = Map::new();
    for key in [
        "version_id",
        "skill_id",
        "version_number",
        "content",
        "content_hash",
        "source_candidate_id",
        "author",
        "status",
        "created_at",
        "parent_version",
    ] {
        if let Some(value) = row.get(key) {
            if !value.is_null() {
                mapped.insert(key.to_string(), value.clone());
            }
        }
    }
    if let Some(v) = json_column(row, "supporting_json", "skill_versions")? {
        mapped.insert("supporting_evidence".into(), v);
    }
    if let Some(v) = json_column(row, "validation_json", "skill_versions")? {
        mapped.insert("validation".into(), v);
    }
    serde_json::from_value(Value::Object(mapped))
        .map_err(|e| format!("corrupted skill_versions row: {e}"))
}

fn skill_candidate_from_row(row: &Value) -> Result<SkillCandidate, String> {
    let row = as_object(row, "skill_candidates")?;
    let mut mapped = Map::new();
    for key in [
        "candidate_id",
        "workspace_root",
        "task_id",
        "scope",
        "name",
        "description",
        "purpose",
        "proposed_content",
        "status",
        "confidence",
        "eval_reason",
        "rejection_reason",
        "supersedes_skill",
        "based_on_version",
        "created_at",
        "updated_at",
        "expires_at",
    ] {
        if let Some(value) = row.get(key) {
            if !value.is_null() {
                mapped.insert(key.to_string(), value.clone());
            }
        }
    }
    for (column, field) in [
        ("applicability_json", "applicability"),
        (
            "source_learning_candidates_json",
            "source_learning_candidates",
        ),
        ("supporting_json", "supporting_evidence"),
        ("contradicting_json", "contradicting_evidence"),
        ("validation_json", "validation"),
    ] {
        if let Some(v) = json_column(row, column, "skill_candidates")? {
            mapped.insert(field.into(), v);
        }
    }
    serde_json::from_value(Value::Object(mapped))
        .map_err(|e| format!("corrupted skill_candidates row: {e}"))
}

// ── SQLite value mapping / redaction / hashing ───────────────────────────

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| format!("columns of {table}: {e}"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("columns of {table}: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("columns of {table}: {e}"))?;
    Ok(columns)
}

fn sql_value_to_json(value: rusqlite::types::ValueRef<'_>) -> Value {
    match value {
        rusqlite::types::ValueRef::Null => Value::Null,
        rusqlite::types::ValueRef::Integer(i) => Value::from(i),
        rusqlite::types::ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        rusqlite::types::ValueRef::Text(bytes) => {
            Value::String(String::from_utf8_lossy(bytes).to_string())
        }
        rusqlite::types::ValueRef::Blob(bytes) => {
            let mut hex = String::with_capacity(bytes.len() * 2);
            for byte in bytes {
                hex.push_str(&format!("{byte:02x}"));
            }
            Value::String(format!("0x{hex}"))
        }
    }
}

/// Redact every string value in a row (defense in depth; the store already
/// redacts at write time, but exports must never leak a secret that
/// predates a redaction fix).
fn redact_value(value: &mut Value, counter: &mut usize) {
    match value {
        Value::String(s) => {
            let redacted = codebro_core::tools::shell::redact_secrets_public(s);
            if &redacted != s {
                *counter += 1;
                *s = redacted;
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_value(item, counter);
            }
        }
        Value::Object(map) => {
            for (_, item) in map.iter_mut() {
                redact_value(item, counter);
            }
        }
        _ => {}
    }
}

/// Recompute a skill version's content hash when export redaction changed
/// its content, so the exported row stays self-consistent.
fn rehash_redacted_skill_version(row: &mut Value) {
    let Some(object) = row.as_object_mut() else {
        return;
    };
    let content = object
        .get("content")
        .and_then(Value::as_str)
        .map(str::to_string);
    let recorded = object
        .get("content_hash")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let (Some(content), Some(recorded)) = (content, recorded) {
        let recomputed = crate::skills::content_hash(&content);
        if recomputed != recorded {
            object.insert("content_hash".into(), Value::String(recomputed));
        }
    }
}

/// Canonical hash over exported rows: sorted table names, compact JSON
/// with sorted keys (serde_json's default map is ordered), matching the
/// pre-existing mirror's `data_hash` construction.
fn canonical_hash(tables: &BTreeMap<String, Vec<Value>>) -> String {
    let mut hasher = Sha256::new();
    for (table, rows) in tables {
        hasher.update(table.as_bytes());
        let body = serde_json::to_string(rows).unwrap_or_default();
        hasher.update(body.as_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{HistoryInput, HistoryKind, OpenSession};
    use crate::skills::{
        content_hash, Skill, SkillApplicability, SkillCandidate, SkillHealth, SkillVersion,
    };
    use crate::types::{Authority, ContextRecord, RecordKind, RecordScope};
    use tempfile::TempDir;

    const SECRET: &str = "sk-live-abcdef1234567890";

    fn store_at(dir: &TempDir) -> ContextStore {
        ContextStore::at_state_dir(dir.path().to_path_buf())
    }

    fn db_path(dir: &TempDir) -> PathBuf {
        dir.path().join(crate::db::STATE_DB_FILE)
    }

    #[allow(clippy::too_many_arguments)]
    fn put(
        store: &ContextStore,
        id: &str,
        kind: RecordKind,
        scope: RecordScope,
        content: &str,
        authority: Authority,
        ws: Option<&str>,
        updated_at: u64,
    ) {
        let mut record = ContextRecord::new(id, kind, format!("ns.{id}"), content, authority);
        record.scope = scope;
        record.workspace_root = ws.map(str::to_string);
        record.created_at = updated_at.saturating_sub(100);
        record.updated_at = updated_at;
        store.put_record(&record, updated_at).unwrap();
    }

    fn skill(name: &str, ws: Option<&str>, status: &str, version: u32) -> (Skill, SkillVersion) {
        let skill_id = format!("sk::{name}");
        let content = format!("# {name}\n\nDo the thing safely.\n");
        let version = SkillVersion {
            version_id: format!("sv::{name}::{version}"),
            skill_id: skill_id.clone(),
            version_number: version,
            content: content.clone(),
            content_hash: content_hash(&content),
            source_candidate_id: None,
            supporting_evidence: Vec::new(),
            validation: None,
            author: "codebro".to_string(),
            status: "active".to_string(),
            created_at: 500,
            parent_version: None,
        };
        let skill = Skill {
            skill_id,
            workspace_root: ws.map(str::to_string),
            scope: if ws.is_some() { "project" } else { "global" }.to_string(),
            name: name.to_string(),
            description: "portable test skill".to_string(),
            applicability: SkillApplicability::default(),
            current_version: 1,
            status: status.to_string(),
            confidence: 0.8,
            health: SkillHealth::default(),
            source_candidate_id: None,
            superseded_by: None,
            created_at: 500,
            updated_at: 600,
        };
        (skill, version)
    }

    fn candidate(name: &str, ws: Option<&str>, status: &str) -> SkillCandidate {
        SkillCandidate {
            candidate_id: format!("sc::{name}"),
            workspace_root: ws.map(str::to_string),
            task_id: None,
            scope: if ws.is_some() { "project" } else { "global" }.to_string(),
            name: name.to_string(),
            description: "candidate description".to_string(),
            purpose: "candidate purpose".to_string(),
            applicability: SkillApplicability::default(),
            source_learning_candidates: Vec::new(),
            supporting_evidence: Vec::new(),
            contradicting_evidence: Vec::new(),
            proposed_content: format!("# {name}\n\nProposed.\n"),
            status: status.to_string(),
            confidence: 0.7,
            validation: None,
            eval_reason: None,
            rejection_reason: None,
            supersedes_skill: None,
            based_on_version: None,
            created_at: 500,
            updated_at: 600,
            expires_at: None,
        }
    }

    fn seed(store: &ContextStore, ws: &str) {
        put(
            store,
            "ctx::confirmed",
            RecordKind::Preference,
            RecordScope::Project,
            "Prefer the simplest reasonable implementation",
            Authority::UserConfirmed,
            Some(ws),
            1000,
        );
        let session = store
            .open_session(
                ws,
                &OpenSession {
                    task_id: None,
                    title: Some("portable session".to_string()),
                    source: Some("test".to_string()),
                    parent_session_id: None,
                },
                900,
            )
            .unwrap();
        let mut input = HistoryInput::new(ws, HistoryKind::Validation, "cargo test passed");
        input.session_id = Some(session.id.clone());
        input.created_at = Some(950);
        input.source = Some("test".to_string());
        let (event_id, _) = store.record_history(&input, 950).unwrap();
        let mut inferred = ContextRecord::new(
            "ctx::inferred",
            RecordKind::Experience,
            "learn.test.pattern",
            "The workflow 'cargo passed' repeatedly succeeded",
            Authority::AiInferred,
        );
        inferred.scope = RecordScope::Project;
        inferred.workspace_root = Some(ws.to_string());
        inferred.evidence = vec![event_id.to_string()];
        inferred.created_at = 960;
        inferred.updated_at = 980;
        store.put_record(&inferred, 980).unwrap();

        let (skill, version) = skill("portable-skill", Some(ws), "active", 1);
        store.import_skill_lineage(&skill, &[version]).unwrap();
        store
            .import_skill_candidate(&candidate("portable-candidate", Some(ws), "validated"))
            .unwrap();
    }

    fn export_of(store_dir: &TempDir, out: &TempDir) -> ExportReport {
        export_mirror(&db_path(store_dir), out.path()).unwrap()
    }

    fn options(target: &str) -> ImportOptions {
        ImportOptions {
            dry_run: false,
            workspace_map: BTreeMap::new(),
            default_workspace: Some(target.to_string()),
            import_origin: "test-export".to_string(),
            skills_root: None,
        }
    }

    #[test]
    fn round_trip_preserves_records_events_sessions_and_skills() {
        let source = tempfile::tempdir().unwrap();
        let source_ws = tempfile::tempdir().unwrap();
        let ws = source_ws.path().display().to_string();
        let store = store_at(&source);
        seed(&store, &ws);
        let export = tempfile::tempdir().unwrap();
        let report = export_of(&source, &export);
        assert_eq!(report.counts.get("context_records"), Some(&2));
        assert_eq!(report.counts.get("sessions"), Some(&1));
        assert_eq!(report.counts.get("events"), Some(&1));
        assert_eq!(report.counts.get("skills"), Some(&1));

        let target = tempfile::tempdir().unwrap();
        let target_ws = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        let import = import_mirror(
            &target_store,
            export.path(),
            &options(&target_ws.path().display().to_string()),
        )
        .unwrap();
        assert_eq!(import.integrity, "hash_verified");
        assert_eq!(import.format_version, Some(PORTABLE_FORMAT_VERSION));
        assert_eq!(import.tables["context_records"].inserted, 2);
        assert_eq!(import.tables["events"].inserted, 1);
        assert_eq!(import.tables["sessions"].inserted, 1);
        assert_eq!(import.tables["skills"].inserted, 1);
        assert_eq!(import.tables["skill_versions"].inserted, 1);
        assert_eq!(import.tables["skill_candidates"].inserted, 1);

        let confirmed = target_store.get_record("ctx::confirmed").unwrap().unwrap();
        assert_eq!(confirmed.authority, Authority::UserConfirmed);
        assert_eq!(confirmed.updated_at, 1000);
        assert_eq!(confirmed.import_origin.as_deref(), Some("test-export"));
        assert_eq!(
            confirmed.workspace_root.as_deref(),
            Some(
                crate::workspace::canonical_workspace_key(&target_ws.path().display().to_string())
                    .as_str()
            )
        );

        let inferred = target_store.get_record("ctx::inferred").unwrap().unwrap();
        assert_eq!(inferred.authority, Authority::AiInferred);
        assert_eq!(inferred.evidence.len(), 1);
        let local_event_id: i64 = inferred.evidence[0].parse().unwrap();
        let local_event = target_store.get_event(local_event_id).unwrap().unwrap();
        assert_eq!(local_event.summary.as_deref(), Some("cargo test passed"));

        let sessions = target_store
            .list_sessions(
                &target_ws.path().display().to_string(),
                &crate::history::SessionFilter::default(),
                2000,
            )
            .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].event_count, 1, "recount after event import");
        assert_eq!(sessions[0].title.as_deref(), Some("portable session"));

        let imported_skill = target_store.get_skill("sk::portable-skill").unwrap();
        assert!(imported_skill.is_some());
        assert_eq!(imported_skill.unwrap().status, "active");
        let candidate = target_store
            .get_skill_candidate("sc::portable-candidate")
            .unwrap()
            .unwrap();
        assert_eq!(candidate.status, "validated", "never auto-approved");
    }

    #[test]
    fn export_is_deterministic() {
        let source = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let store = store_at(&source);
        seed(&store, &ws.path().display().to_string());
        let out_a = tempfile::tempdir().unwrap();
        let out_b = tempfile::tempdir().unwrap();
        let a = export_of(&source, &out_a);
        let b = export_of(&source, &out_b);
        assert_eq!(a.data_hash, b.data_hash);
        for table in EXPORT_TABLES {
            let path_a = out_a.path().join(format!("{table}.jsonl"));
            if path_a.is_file() {
                let path_b = out_b.path().join(format!("{table}.jsonl"));
                assert_eq!(
                    std::fs::read_to_string(&path_a).unwrap(),
                    std::fs::read_to_string(&path_b).unwrap(),
                    "{table} export must be byte-stable"
                );
            }
        }
    }

    #[test]
    fn tampered_export_is_refused() {
        let source = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let store = store_at(&source);
        seed(&store, &ws.path().display().to_string());
        let export = tempfile::tempdir().unwrap();
        export_of(&source, &export);
        let path = export.path().join("context_records.jsonl");
        let body = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, body.replace("simplest", "tampered")).unwrap();

        let target = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        let err = import_mirror(&target_store, export.path(), &options("/tmp/ws")).unwrap_err();
        assert!(err.contains("integrity"), "unexpected error: {err}");
        assert!(target_store.get_record("ctx::confirmed").unwrap().is_none());
    }

    #[test]
    fn duplicate_import_is_idempotent() {
        let source = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let store = store_at(&source);
        seed(&store, &ws.path().display().to_string());
        let export = tempfile::tempdir().unwrap();
        export_of(&source, &export);

        let target = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        let opts = options(&ws.path().display().to_string());
        let first = import_mirror(&target_store, export.path(), &opts).unwrap();
        assert_eq!(first.tables["context_records"].inserted, 2);
        let second = import_mirror(&target_store, export.path(), &opts).unwrap();
        assert_eq!(second.tables["context_records"].inserted, 0);
        assert_eq!(second.tables["context_records"].updated, 0);
        assert_eq!(second.tables["context_records"].duplicates, 2);
        assert_eq!(second.tables["events"].inserted, 0);
        assert_eq!(second.tables["events"].duplicates, 1);
        assert_eq!(second.tables["sessions"].duplicates, 1);
        assert_eq!(second.tables["skills"].duplicates, 1);
        assert_eq!(second.tables["skill_candidates"].duplicates, 1);
        assert_eq!(
            target_store
                .count_visible_with_task(Some(&ws.path().display().to_string()), None)
                .unwrap(),
            2
        );
    }

    #[test]
    fn duplicate_detection_survives_evidence_remap() {
        // The source event id space starts at 2 (a prior unrelated event),
        // while the fresh target assigns id 1 to the imported event: the
        // remapped evidence must still classify the second import as a
        // duplicate, not a conflict.
        let source = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let ws_str = ws.path().display().to_string();
        let store = store_at(&source);
        let mut filler = HistoryInput::new(&ws_str, HistoryKind::Observation, "unrelated");
        filler.created_at = Some(500);
        store.record_history(&filler, 500).unwrap();
        let mut input = HistoryInput::new(&ws_str, HistoryKind::Validation, "evidence event");
        input.created_at = Some(600);
        let (event_id, _) = store.record_history(&input, 600).unwrap();
        assert_ne!(event_id, 1, "source id space must not start at 1");
        let mut inferred = ContextRecord::new(
            "ctx::remap",
            RecordKind::Experience,
            "learn.remap",
            "evidence remap probe",
            Authority::AiInferred,
        );
        inferred.scope = RecordScope::Project;
        inferred.workspace_root = Some(ws_str.clone());
        inferred.evidence = vec![event_id.to_string()];
        inferred.created_at = 700;
        inferred.updated_at = 700;
        store.put_record(&inferred, 700).unwrap();
        let export = tempfile::tempdir().unwrap();
        export_of(&source, &export);

        let target = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        // Shift the target's event id space so the remap is observable.
        let mut local_filler = HistoryInput::new(
            &ws_str,
            HistoryKind::Observation,
            "pre-existing local event",
        );
        local_filler.created_at = Some(400);
        target_store.record_history(&local_filler, 400).unwrap();
        let opts = options(&ws_str);
        let first = import_mirror(&target_store, export.path(), &opts).unwrap();
        assert_eq!(first.tables["context_records"].inserted, 1);
        let second = import_mirror(&target_store, export.path(), &opts).unwrap();
        assert_eq!(second.tables["context_records"].duplicates, 1);
        assert_eq!(second.tables["context_records"].conflicts, 0);
        let local = target_store.get_record("ctx::remap").unwrap().unwrap();
        let local_event_id: i64 = local.evidence[0].parse().unwrap();
        assert_ne!(local_event_id.to_string(), event_id.to_string());
        let event = target_store.get_event(local_event_id).unwrap().unwrap();
        assert_eq!(event.summary.as_deref(), Some("evidence event"));
    }

    #[test]
    fn newer_local_state_is_not_overwritten() {
        let source = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let ws_str = ws.path().display().to_string();
        let store = store_at(&source);
        put(
            &store,
            "ctx::same",
            RecordKind::Preference,
            RecordScope::Project,
            "imported older value",
            Authority::UserConfirmed,
            Some(&ws_str),
            1000,
        );
        let export = tempfile::tempdir().unwrap();
        export_of(&source, &export);

        let target = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        put(
            &target_store,
            "ctx::same",
            RecordKind::Preference,
            RecordScope::Project,
            "local newer value",
            Authority::UserConfirmed,
            Some(&ws_str),
            2000,
        );
        let report = import_mirror(&target_store, export.path(), &options(&ws_str)).unwrap();
        assert_eq!(report.tables["context_records"].skipped, 1);
        assert_eq!(report.tables["context_records"].updated, 0);
        let local = target_store.get_record("ctx::same").unwrap().unwrap();
        assert_eq!(local.content, "local newer value");
        assert_eq!(local.updated_at, 2000);
    }

    #[test]
    fn newer_imported_record_updates_local() {
        let source = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let ws_str = ws.path().display().to_string();
        let store = store_at(&source);
        put(
            &store,
            "ctx::same",
            RecordKind::Preference,
            RecordScope::Project,
            "imported newer value",
            Authority::UserConfirmed,
            Some(&ws_str),
            3000,
        );
        let export = tempfile::tempdir().unwrap();
        export_of(&source, &export);

        let target = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        put(
            &target_store,
            "ctx::same",
            RecordKind::Preference,
            RecordScope::Project,
            "local older value",
            Authority::UserConfirmed,
            Some(&ws_str),
            1000,
        );
        let report = import_mirror(&target_store, export.path(), &options(&ws_str)).unwrap();
        assert_eq!(report.tables["context_records"].updated, 1);
        let local = target_store.get_record("ctx::same").unwrap().unwrap();
        assert_eq!(local.content, "imported newer value");
        assert_eq!(local.updated_at, 3000);
    }

    #[test]
    fn equal_timestamp_conflict_is_reported_not_written() {
        let source = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let ws_str = ws.path().display().to_string();
        let store = store_at(&source);
        put(
            &store,
            "ctx::same",
            RecordKind::Preference,
            RecordScope::Project,
            "imported variant",
            Authority::UserConfirmed,
            Some(&ws_str),
            1000,
        );
        let export = tempfile::tempdir().unwrap();
        export_of(&source, &export);

        let target = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        put(
            &target_store,
            "ctx::same",
            RecordKind::Preference,
            RecordScope::Project,
            "local variant",
            Authority::UserConfirmed,
            Some(&ws_str),
            1000,
        );
        let report = import_mirror(&target_store, export.path(), &options(&ws_str)).unwrap();
        assert_eq!(report.tables["context_records"].conflicts, 1);
        assert!(report.warnings.iter().any(|w| w.contains("ctx::same")));
        let local = target_store.get_record("ctx::same").unwrap().unwrap();
        assert_eq!(local.content, "local variant");
    }

    #[test]
    fn confirmed_local_is_never_downgraded_by_newer_inference() {
        let source = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let ws_str = ws.path().display().to_string();
        let store = store_at(&source);
        let mut inferred = ContextRecord::new(
            "ctx::same",
            RecordKind::Preference,
            "ns.ctx::same",
            "inferred replacement",
            Authority::AiInferred,
        );
        inferred.scope = RecordScope::Project;
        inferred.workspace_root = Some(ws_str.clone());
        inferred.evidence = vec!["1".to_string()];
        inferred.created_at = 1000;
        inferred.updated_at = 3000;
        // Seed the evidence event locally so the source record validates.
        let mut input = HistoryInput::new(&ws_str, HistoryKind::Observation, "observed pattern");
        input.created_at = Some(900);
        let (event_id, _) = store.record_history(&input, 900).unwrap();
        inferred.evidence = vec![event_id.to_string()];
        store.put_record(&inferred, 3000).unwrap();
        let export = tempfile::tempdir().unwrap();
        export_of(&source, &export);

        let target = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        put(
            &target_store,
            "ctx::same",
            RecordKind::Preference,
            RecordScope::Project,
            "user confirmed truth",
            Authority::UserConfirmed,
            Some(&ws_str),
            1000,
        );
        let report = import_mirror(&target_store, export.path(), &options(&ws_str)).unwrap();
        assert_eq!(report.tables["context_records"].conflicts, 1);
        assert_eq!(report.tables["context_records"].updated, 0);
        let local = target_store.get_record("ctx::same").unwrap().unwrap();
        assert_eq!(local.authority, Authority::UserConfirmed);
        assert_eq!(local.content, "user confirmed truth");
    }

    #[test]
    fn secrets_are_redacted_in_export_and_import() {
        let source = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let ws_str = ws.path().display().to_string();
        let store = store_at(&source);
        put(
            &store,
            "ctx::secret",
            RecordKind::Preference,
            RecordScope::Project,
            &format!("the token {SECRET} must never leak"),
            Authority::UserConfirmed,
            Some(&ws_str),
            1000,
        );
        let export = tempfile::tempdir().unwrap();
        let report = export_of(&source, &export);
        assert!(report.redacted_values >= 1);
        let body = std::fs::read_to_string(export.path().join("context_records.jsonl")).unwrap();
        assert!(!body.contains(SECRET), "export leaked a secret");
        assert!(body.contains("[REDACTED]"));

        let target = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        import_mirror(&target_store, export.path(), &options(&ws_str)).unwrap();
        let imported = target_store.get_record("ctx::secret").unwrap().unwrap();
        assert!(!imported.content.contains(SECRET));
        assert!(imported.content.contains("[REDACTED]"));
    }

    #[test]
    fn redacted_skill_content_rehashes_so_the_bundle_stays_importable() {
        let source = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let ws_str = ws.path().display().to_string();
        let store = store_at(&source);
        let (skill, mut version) = skill("secret-skill", Some(&ws_str), "active", 1);
        version.content = format!("# secret-skill\n\nUse token {SECRET} carefully.\n");
        version.content_hash = content_hash(&version.content);
        store
            .import_skill_lineage(&skill, &[version])
            .expect("seed lineage");
        let export = tempfile::tempdir().unwrap();
        let report = export_of(&source, &export);
        assert!(report.redacted_values >= 1);

        let target = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        let import = import_mirror(&target_store, export.path(), &options(&ws_str)).unwrap();
        assert_eq!(import.tables["skills"].inserted, 1);
        assert_eq!(import.tables["skill_versions"].inserted, 1);
        let imported = target_store.get_skill("sk::secret-skill").unwrap().unwrap();
        let versions = target_store
            .list_skill_versions(&imported.skill_id, 10)
            .unwrap();
        assert!(!versions[0].content.contains(SECRET));
        assert_eq!(versions[0].content_hash, content_hash(&versions[0].content));
    }

    #[test]
    fn crafted_manifest_table_names_cannot_escape_the_bundle() {
        let parent = tempfile::tempdir().unwrap();
        let bundle = parent.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(
            parent.path().join("victim.jsonl"),
            "{\"id\":\"ctx::victim\"}\n",
        )
        .unwrap();
        let manifest = PortableManifest {
            format_version: Some(1),
            generated_at: "0".to_string(),
            data_hash: "0".repeat(64),
            counts: BTreeMap::new(),
            source_db: None,
            tables: BTreeMap::from([("../victim".to_string(), Vec::new())]),
        };
        std::fs::write(
            bundle.join(MANIFEST_FILE),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();

        let target = tempfile::tempdir().unwrap();
        let err = import_mirror(&store_at(&target), &bundle, &options("/tmp/ws")).unwrap_err();
        assert!(err.contains("unknown table"), "unexpected: {err}");
        assert!(!target.path().join(crate::db::STATE_DB_FILE).exists());
    }

    #[test]
    fn malformed_manifest_is_refused() {
        let export = tempfile::tempdir().unwrap();
        std::fs::write(export.path().join(MANIFEST_FILE), "{not json").unwrap();
        let target = tempfile::tempdir().unwrap();
        let err =
            import_mirror(&store_at(&target), export.path(), &options("/tmp/ws")).unwrap_err();
        assert!(err.contains("malformed manifest"), "unexpected: {err}");
    }

    #[test]
    fn corrupted_record_aborts_before_writing() {
        let source = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let ws_str = ws.path().display().to_string();
        let store = store_at(&source);
        seed(&store, &ws_str);
        let export = tempfile::tempdir().unwrap();
        export_of(&source, &export);

        // Corrupt a row *and* refresh the manifest hash so integrity
        // verification passes and row validation is what refuses.
        let manifest_path = export.path().join(MANIFEST_FILE);
        let mut manifest: PortableManifest =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        let mut tables: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for table in manifest.tables.keys() {
            let body =
                std::fs::read_to_string(export.path().join(format!("{table}.jsonl"))).unwrap();
            let rows: Vec<Value> = body
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            tables.insert(table.clone(), rows);
        }
        let rows = tables.get_mut("context_records").unwrap();
        rows[0].as_object_mut().unwrap().remove("namespace");
        for (table, rows) in &tables {
            let mut body = String::new();
            for row in rows {
                body.push_str(&serde_json::to_string(row).unwrap());
                body.push('\n');
            }
            std::fs::write(export.path().join(format!("{table}.jsonl")), body).unwrap();
        }
        manifest.data_hash = canonical_hash(&tables);
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let target = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        let err = import_mirror(&target_store, export.path(), &options(&ws_str)).unwrap_err();
        assert!(err.contains("namespace"), "unexpected: {err}");
        assert!(
            target_store.get_record("ctx::confirmed").unwrap().is_none(),
            "nothing may be written when validation fails"
        );
        assert!(target_store.get_record("ctx::inferred").unwrap().is_none());
    }

    #[test]
    fn legacy_manifest_without_format_version_imports_by_counts() {
        let source = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let ws_str = ws.path().display().to_string();
        let store = store_at(&source);
        seed(&store, &ws_str);
        let export = tempfile::tempdir().unwrap();
        export_of(&source, &export);

        let manifest_path = export.path().join(MANIFEST_FILE);
        let mut manifest: PortableManifest =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        manifest.format_version = None;
        // Simulate a foreign canonicalization: hash differs, counts agree.
        manifest.data_hash = "0".repeat(64);
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let target = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        let report = import_mirror(&target_store, export.path(), &options(&ws_str)).unwrap();
        assert_eq!(report.integrity, "counts_verified");
        assert!(target_store.get_record("ctx::confirmed").unwrap().is_some());

        // Counts disagree → refused.
        manifest.counts.insert("context_records".to_string(), 99);
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let target2 = tempfile::tempdir().unwrap();
        let err = import_mirror(&store_at(&target2), export.path(), &options(&ws_str)).unwrap_err();
        assert!(err.contains("integrity"), "unexpected: {err}");
    }

    #[test]
    fn workspace_remap_and_unmapped_skips() {
        let source = tempfile::tempdir().unwrap();
        let source_ws = tempfile::tempdir().unwrap();
        let source_ws_str = source_ws.path().display().to_string();
        let store = store_at(&source);
        seed(&store, &source_ws_str);
        let export = tempfile::tempdir().unwrap();
        export_of(&source, &export);

        // Explicit remap.
        let target = tempfile::tempdir().unwrap();
        let target_ws = tempfile::tempdir().unwrap();
        let target_ws_str = target_ws.path().display().to_string();
        let target_store = store_at(&target);
        let mut opts = options(&target_ws_str);
        opts.default_workspace = None;
        opts.workspace_map
            .insert(source_ws_str.clone(), target_ws_str.clone());
        let report = import_mirror(&target_store, export.path(), &opts).unwrap();
        assert_eq!(report.tables["context_records"].inserted, 2);
        let record = target_store.get_record("ctx::confirmed").unwrap().unwrap();
        assert_eq!(
            record.workspace_root.as_deref(),
            Some(crate::workspace::canonical_workspace_key(&target_ws_str).as_str())
        );

        // No mapping: project rows are skipped, not guessed.
        let target2 = tempfile::tempdir().unwrap();
        let target_store2 = store_at(&target2);
        let opts2 = ImportOptions {
            dry_run: false,
            workspace_map: BTreeMap::new(),
            default_workspace: None,
            import_origin: "test".to_string(),
            skills_root: None,
        };
        let report2 = import_mirror(&target_store2, export.path(), &opts2).unwrap();
        assert_eq!(report2.tables["context_records"].inserted, 0);
        assert_eq!(report2.tables["context_records"].skipped, 2);
        assert!(target_store2
            .get_record("ctx::confirmed")
            .unwrap()
            .is_none());
    }

    #[test]
    fn global_record_with_unmapped_evidence_still_imports() {
        let source = tempfile::tempdir().unwrap();
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        let store = store_at(&source);
        let mut input = HistoryInput::new(
            ws_b.path().display().to_string(),
            HistoryKind::Observation,
            "foreign observation",
        );
        input.created_at = Some(700);
        let (event_id, _) = store.record_history(&input, 700).unwrap();
        let mut global = ContextRecord::new(
            "ctx::global-learn",
            RecordKind::Experience,
            "learn.global.pattern",
            "cross-project pattern",
            Authority::AiInferred,
        );
        global.scope = RecordScope::Global;
        global.evidence = vec![event_id.to_string()];
        global.created_at = 800;
        global.updated_at = 800;
        store.put_record(&global, 800).unwrap();
        put(
            &store,
            "ctx::a",
            RecordKind::Preference,
            RecordScope::Project,
            "project A knowledge",
            Authority::UserConfirmed,
            Some(&ws_a.path().display().to_string()),
            900,
        );
        let export = tempfile::tempdir().unwrap();
        export_of(&source, &export);

        let target = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        let opts = ImportOptions {
            dry_run: false,
            workspace_map: BTreeMap::new(),
            default_workspace: None,
            import_origin: "test".to_string(),
            skills_root: None,
        };
        let report = import_mirror(&target_store, export.path(), &opts).unwrap();
        assert_eq!(report.tables["context_records"].inserted, 1);
        assert_eq!(report.tables["context_records"].skipped, 1);
        let imported = target_store
            .get_record("ctx::global-learn")
            .unwrap()
            .unwrap();
        assert_eq!(imported.authority, Authority::AiInferred);
        assert_eq!(imported.evidence.len(), 1);
        let local_event_id: i64 = imported.evidence[0].parse().unwrap();
        let event = target_store.get_event(local_event_id).unwrap().unwrap();
        assert_eq!(event.summary.as_deref(), Some("foreign observation"));
        assert_eq!(
            event.workspace_root,
            crate::workspace::canonical_workspace_key(&ws_b.path().display().to_string())
        );
        assert!(target_store.get_record("ctx::a").unwrap().is_none());
    }

    #[test]
    fn dry_run_writes_nothing_but_reports_plan() {
        let source = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let ws_str = ws.path().display().to_string();
        let store = store_at(&source);
        seed(&store, &ws_str);
        let export = tempfile::tempdir().unwrap();
        export_of(&source, &export);

        let target = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        let mut opts = options(&ws_str);
        opts.dry_run = true;
        let report = import_mirror(&target_store, export.path(), &opts).unwrap();
        assert!(report.dry_run);
        assert_eq!(report.tables["context_records"].inserted, 2);
        assert_eq!(report.tables["skills"].inserted, 1);
        assert!(target_store.get_record("ctx::confirmed").unwrap().is_none());
    }

    #[test]
    fn imported_active_skill_publishes_missing_artifact_only() {
        let source = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let ws_str = ws.path().display().to_string();
        let store = store_at(&source);
        let (skill, version) = skill("portable-publish", Some(&ws_str), "active", 1);
        store.import_skill_lineage(&skill, &[version]).unwrap();
        let export = tempfile::tempdir().unwrap();
        export_of(&source, &export);

        let target = tempfile::tempdir().unwrap();
        let target_store = store_at(&target);
        let skills_root = tempfile::tempdir().unwrap();
        let mut opts = options(&ws_str);
        opts.skills_root = Some(skills_root.path().to_path_buf());
        import_mirror(&target_store, export.path(), &opts).unwrap();
        let published = skills_root.path().join("portable-publish/SKILL.md");
        assert!(published.is_file(), "missing artifact must be published");
        assert!(std::fs::read_to_string(&published)
            .unwrap()
            .contains("Do the thing safely"));

        // A second import never clobbers an existing differing artifact.
        std::fs::write(&published, "# locally edited\n").unwrap();
        let target2 = tempfile::tempdir().unwrap();
        let target_store2 = store_at(&target2);
        let report = import_mirror(&target_store2, export.path(), &opts).unwrap();
        assert!(report
            .warnings
            .iter()
            .any(|w| w.contains("portable-publish") && w.contains("differ")));
        assert_eq!(
            std::fs::read_to_string(&published).unwrap(),
            "# locally edited\n"
        );
    }

    #[test]
    fn import_refuses_unknown_future_format() {
        let export = tempfile::tempdir().unwrap();
        let manifest = PortableManifest {
            format_version: Some(PORTABLE_FORMAT_VERSION + 1),
            generated_at: "0".to_string(),
            data_hash: "0".repeat(64),
            counts: BTreeMap::new(),
            source_db: None,
            tables: BTreeMap::new(),
        };
        std::fs::write(
            export.path().join(MANIFEST_FILE),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let target = tempfile::tempdir().unwrap();
        let err =
            import_mirror(&store_at(&target), export.path(), &options("/tmp/ws")).unwrap_err();
        assert!(err.contains("newer than supported"), "unexpected: {err}");
    }
}
