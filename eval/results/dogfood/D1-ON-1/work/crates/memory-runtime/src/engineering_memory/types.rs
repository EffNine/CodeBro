//! Core types for the engineering memory module.

use serde::{Deserialize, Serialize};

use crate::memory_runtime::MemoryTier;

/// Schema version for `engineering_memory.json`.
///
/// 1.1.0 adds optional lifecycle/provenance fields to entry metadata
/// (`expires_at`, `status`, `provenance`, `supersedes`, `adjustments`).
/// All new fields carry serde defaults, so stores written by 1.0.0 load
/// unchanged and vice versa.
pub const CURRENT_SCHEMA_VERSION: &str = "1.1.0";

/// Lifecycle status of a memory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    /// In normal use; eligible for resolution.
    #[default]
    Active,
    /// Past its expiry; excluded from resolution, retained for audit.
    Expired,
    /// Replaced by a newer entry under the same key.
    Superseded,
}

impl MemoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryStatus::Active => "active",
            MemoryStatus::Expired => "expired",
            MemoryStatus::Superseded => "superseded",
        }
    }
}

/// Structured provenance: who/what produced this memory and how.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MemoryProvenance {
    /// Logical origin class of the memory.
    #[serde(default)]
    pub origin: String,
    /// Identifier of the producing agent/session when known.
    #[serde(default)]
    pub session: Option<String>,
    /// Tool or path used to record it.
    #[serde(default)]
    pub created_via: Option<String>,
}

impl MemoryProvenance {
    pub fn agent(session: Option<String>) -> Self {
        MemoryProvenance {
            origin: "agent".to_string(),
            session,
            created_via: Some("record_memory".to_string()),
        }
    }
}

/// One confidence adjustment event (audit trail).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceAdjustment {
    /// Epoch seconds when the adjustment happened.
    pub at: u64,
    pub from: f64,
    pub to: f64,
    pub reason: Option<String>,
}

/// A single engineering memory entry persisted at project tier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineeringMemoryEntry {
    /// Unique identifier within the project.
    pub id: String,
    /// Short descriptive key.
    pub key: String,
    /// Full memory value.
    pub value: String,
    /// Importance and confidence metadata.
    pub metadata: EngineeringMemoryMetadata,
    /// Epoch seconds when the entry was created.
    pub created_at: u64,
    /// Epoch seconds of the last access.
    pub last_accessed: u64,
    /// Number of times the entry was accessed.
    pub access_count: u64,
}

impl EngineeringMemoryEntry {
    /// Create a new entry with the given id, key, and value.
    pub fn new(id: impl Into<String>, key: impl Into<String>, value: impl Into<String>) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        EngineeringMemoryEntry {
            id: id.into(),
            key: key.into(),
            value: value.into(),
            metadata: EngineeringMemoryMetadata::default(),
            created_at: now,
            last_accessed: now,
            access_count: 0,
        }
    }

    /// Attach metadata to this entry.
    pub fn with_metadata(mut self, metadata: EngineeringMemoryMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Record an access event.
    pub fn record_access(&mut self) {
        self.last_accessed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.access_count += 1;
    }

    /// True when the entry has an expiry in the past relative to `now`
    /// (epoch seconds).
    pub fn is_expired_at(&self, now: u64) -> bool {
        matches!(self.metadata.expires_at, Some(at) if at <= now)
    }

    /// True when the entry is eligible for resolution: active lifecycle
    /// status and not past its expiry.
    pub fn is_resolvable_at(&self, now: u64) -> bool {
        self.metadata.status == MemoryStatus::Active && !self.is_expired_at(now)
    }

    /// Returns true if the entry's key or value contains the given keyword
    /// (case-insensitive).
    pub fn matches_keyword(&self, keyword: &str) -> bool {
        let kw = keyword.to_lowercase();
        self.key.to_lowercase().contains(&kw) || self.value.to_lowercase().contains(&kw)
    }

    /// Returns true if the entry carries at least one of the given tags.
    pub fn matches_tags(&self, tags: &[String]) -> bool {
        if tags.is_empty() {
            return true;
        }
        tags.iter().any(|t| self.metadata.tags.contains(t))
    }
}

/// Metadata attached to an engineering memory entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineeringMemoryMetadata {
    /// Importance score in [0.0, 1.0].
    pub importance: f64,
    /// Confidence score in [0.0, 1.0].
    pub confidence: f64,
    /// Associative tags for filtering.
    pub tags: Vec<String>,
    /// Source of the memory (e.g. "sprint-23-review").
    pub source: Option<String>,
    /// Epoch seconds after which the entry is expired (None = never).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// Lifecycle status (defaults to Active for pre-1.1 stores).
    #[serde(default)]
    pub status: MemoryStatus,
    /// Structured provenance record.
    #[serde(default)]
    pub provenance: MemoryProvenance,
    /// Id of the entry this one supersedes (key-conflict lineage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// Bounded audit trail of confidence adjustments (most recent last).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adjustments: Vec<ConfidenceAdjustment>,
}

impl Default for EngineeringMemoryMetadata {
    fn default() -> Self {
        EngineeringMemoryMetadata {
            importance: 0.5,
            confidence: 0.5,
            tags: Vec::new(),
            source: None,
            expires_at: None,
            status: MemoryStatus::Active,
            provenance: MemoryProvenance::default(),
            supersedes: None,
            adjustments: Vec::new(),
        }
    }
}

impl EngineeringMemoryMetadata {
    /// Create empty metadata.
    pub fn new() -> Self {
        EngineeringMemoryMetadata::default()
    }

    /// Set importance (clamped to [0.0, 1.0]).
    pub fn with_importance(mut self, importance: f64) -> Self {
        self.importance = importance.clamp(0.0, 1.0);
        self
    }

    /// Set confidence (clamped to [0.0, 1.0]).
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    /// Add a tag (duplicates are ignored).
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        let tag_str = tag.into();
        if !self.tags.contains(&tag_str) {
            self.tags.push(tag_str);
        }
        self
    }

    /// Set the source label.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Set expiry relative to now (seconds). Zero means no expiry.
    pub fn with_ttl(mut self, seconds: u64) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.expires_at = if seconds == 0 {
            None
        } else {
            Some(now + seconds)
        };
        self
    }

    /// Set absolute expiry (epoch seconds).
    pub fn with_expires_at(mut self, at: u64) -> Self {
        self.expires_at = Some(at);
        self
    }
}

/// The canonical on-disk format for `.codebro/engineering_memory.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineeringMemoryFile {
    /// Schema version.
    pub schema_version: String,
    /// Workspace root this file belongs to.
    pub workspace_root: String,
    /// All persisted entries.
    pub entries: Vec<EngineeringMemoryEntry>,
    /// Epoch seconds of last write.
    pub updated_at: u64,
}

impl EngineeringMemoryFile {
    /// Create a fresh file wrapper.
    pub fn new(workspace_root: impl Into<String>) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        EngineeringMemoryFile {
            schema_version: CURRENT_SCHEMA_VERSION.to_string(),
            workspace_root: workspace_root.into(),
            entries: Vec::new(),
            updated_at: now,
        }
    }

    /// Create from existing entries.
    pub fn from_entries(
        workspace_root: impl Into<String>,
        entries: Vec<EngineeringMemoryEntry>,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        EngineeringMemoryFile {
            schema_version: CURRENT_SCHEMA_VERSION.to_string(),
            workspace_root: workspace_root.into(),
            entries,
            updated_at: now,
        }
    }
}

/// Errors that can occur during memory resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineeringMemoryResolveError {
    /// The resolved context exceeded the token budget.
    TokenBudgetExceeded(usize),
    /// No entries matched the query.
    NoMatches,
    /// Generic resolution error.
    Generic(String),
}

impl std::fmt::Display for EngineeringMemoryResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineeringMemoryResolveError::TokenBudgetExceeded(tokens) => {
                write!(f, "token budget exceeded: {} tokens", tokens)
            }
            EngineeringMemoryResolveError::NoMatches => {
                write!(f, "no memory entries matched the query")
            }
            EngineeringMemoryResolveError::Generic(msg) => {
                write!(f, "resolution error: {}", msg)
            }
        }
    }
}

impl std::error::Error for EngineeringMemoryResolveError {}

pub type EngineeringMemoryResolveResult<T> = Result<T, EngineeringMemoryResolveError>;
