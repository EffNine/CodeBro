//! CodeBro context runtime — the durable user-context foundation.
//!
//! CodeBro v1.x owns engineering truth for a workspace: verified facts
//! (`facts.json`), agent-recorded engineering memory
//! (`engineering_memory.json`), and project identity
//! (`project_identity.json`), all pure-JSON files inside `.codebro/`.
//!
//! This crate adds the *user-context* domain that those stores deliberately
//! do not cover: durable context records (preferences, intents, patterns,
//! experiences), an append-only observation event log, and session
//! clustering — persisted in a single SQLite database (`state.db`) with an
//! FTS5 content index.
//!
//! # Scope and invariants
//!
//! - **One canonical home per domain.** Facts / decisions / constraints
//!   continue to live in their JSON homes. Context records never duplicate
//!   them; they may *reference* them via `related_ids`. There is no second
//!   source of truth.
//! - **Provenance is structural.** Every record carries an
//!   [`Authority`]; AI-inferred and observed records must cite evidence
//!   (event ids) or the store refuses them. Agent inference is never
//!   stored as user-confirmed truth.
//! - **Knowledge may go stale.** Records carry an explicit lifecycle
//!   (active → superseded/expired/rejected) and confidence decays at
//!   retrieval time when evidence does not refresh it.
//! - **Never promoted.** Nothing in this crate can write the JSON fact,
//!   memory, or identity files. Trust separation is enforced by
//!   `crates/mcp-server/tests/trust_separation.rs`-style integration tests.
//! - **Reversible.** Records can be superseded (audit trail preserved) or
//!   removed.
//!
//! # Layout
//!
//! | Module | Role |
//! |--------|------|
//! | [`types`] | Domain model: authority, record kinds, records, events |
//! | [`db`] | SQLite bootstrap: schema, migrations, corruption quarantine |
//! | [`store`] | [`ContextStore`]: transactional record/event operations |
//! | [`retrieval`] | Deterministic retrieval: filters + FTS5 + confidence decay |
//! | [`history`] | P2 sessions + history: lifecycle, taxonomy, append-only log |
//! | [`recall`] | P2 recall: scoped FTS search, ranking, session grouping |
//! | [`learning`] | P3 learning + inference: candidates, evaluation, confidence |
//! | [`workspace`] | Workspace identity canonicalization |
//! | [`fingerprint`] | User fingerprint: lanes, precedence, namespace resolution |
//! | [`intent`] | Intent semantics: rationale/priority/status over `extra_json` |
//!
//! Later phases (learning, skills, tasks) build on this foundation without redesigning
//! it: fingerprint/intent records are context records, recall queries this
//! store, the learning loop consumes the event log, and skill/task tables
//! are added by new `user_version` migrations.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

pub mod db;
pub mod fingerprint;
pub mod history;
pub mod intent;
pub mod learning;
pub mod recall;
pub mod repo_index;
pub mod retrieval;
pub mod skills;
pub mod store;
pub mod tasks;
pub mod types;
pub mod workspace;

#[cfg(test)]
mod learning_tests;

/// Path-compatibility facade mirroring the other runtime crates (e.g.
/// `memory_runtime` in codebro-memory-runtime): `context_runtime::<sub>`.
pub mod context_runtime {
    pub use crate::{
        authority_rank, db, decay_rate_per_month, decayed_confidence,
        fingerprint::{self, Lane, ResolutionScope, ResolvedContext},
        history::{
            self, HistoryInput, HistoryKind, OpenSession, SessionFilter, SessionRecord,
            SessionStatus,
        },
        intent::{self, IntentMetadata, IntentPriority, IntentStatus},
        learning::{
            self, CandidateKind, CandidateStatus, LearnScope, LearningCandidate, LearningRunOutcome,
        },
        lifecycle_for_authority,
        recall::{self, RecallGroup, RecallHit, RecallOutcome, RecallQuery, RecallScope},
        repo_index::{self, RepoIndexRecord, RepoIndexStatus, RepoIndexUpsert},
        retrieval, skills,
        skills::{
            content_hash, is_valid_skill_name, mint_skill_candidate_id, mint_skill_id,
            mint_version_id, validate_skill_content, Skill, SkillApplicability, SkillCandidate,
            SkillCandidateStatus, SkillError, SkillHealth, SkillScope, SkillStatus,
            SkillValidation, SkillVersion, SkillVersionStatus, SKILL_APPROVAL_MIN_CONFIDENCE,
            SKILL_CANDIDATE_TTL_SECS,
        },
        store, tasks,
        tasks::{
            mint_worker_id, task_transition_allowed, OutcomeClassification, ResolvedSkillRef,
            SkillRefResolution, TaskCheckpoint, TaskOutcome, TaskOutcomeInput, TaskOutcomeRecord,
            TaskPriority, TaskRecord, TaskResumeEvent, TaskResumeSnapshot, TaskStatus,
            TaskValidation, TaskValidationResult, TASK_LEASE_TTL_SECS,
        },
        types,
        workspace::{self, canonical_workspace_key},
        Authority, ContextRecord, ContextRetriever, ContextStore, EventRecord, LifecycleStage,
        RankedRecord, RecordKind, RecordQuery, RecordScope, RecordStatus, SCHEMA_VERSION,
        STATE_DB_FILE,
    };
}

pub use db::{SCHEMA_VERSION, STATE_DB_FILE};
pub use fingerprint::{Lane, ResolutionScope, ResolvedContext};
pub use history::{
    HistoryInput, HistoryKind, OpenSession, SessionFilter, SessionRecord, SessionStatus,
    STALE_AFTER_SECS,
};
pub use intent::{IntentMetadata, IntentPriority, IntentStatus};
pub use learning::{
    CandidateKind, CandidateStatus, LearnScope, LearningCandidate, LearningRunOutcome,
};
pub use recall::{RecallGroup, RecallHit, RecallOutcome, RecallQuery, RecallScope};
pub use repo_index::{RepoIndexRecord, RepoIndexStatus, RepoIndexUpsert};
pub use retrieval::{
    decay_rate_per_month, decayed_confidence, ContextRetriever, RankedRecord, RecordQuery,
};
pub use skills::{
    Skill, SkillApplicability, SkillCandidate, SkillCandidateStatus, SkillError, SkillHealth,
    SkillScope, SkillStatus, SkillValidation, SkillVersion, SkillVersionStatus,
};
pub use store::ContextStore;
pub use tasks::{
    mint_worker_id, task_transition_allowed, NewTask, OutcomeClassification, ResolvedSkillRef,
    SkillRefResolution, TaskCheckpoint, TaskOutcome, TaskOutcomeInput, TaskOutcomeRecord,
    TaskPriority, TaskRecord, TaskResumeEvent, TaskResumeSnapshot, TaskStatus, TaskValidation,
    TaskValidationResult, TASK_LEASE_TTL_SECS,
};
pub use types::{
    authority_rank, lifecycle_for_authority, Authority, ContextRecord, EventRecord, LifecycleStage,
    RecordKind, RecordScope, RecordStatus,
};
pub use workspace::canonical_workspace_key;
