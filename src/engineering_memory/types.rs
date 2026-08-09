//! Core types for the engineering memory module.

use serde::{Deserialize, Serialize};

use crate::memory_runtime::MemoryTier;

/// Schema version for `engineering_memory.json`.
pub const CURRENT_SCHEMA_VERSION: &str = "1.0.0";

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
    pub fn new(
        id: impl Into<String>,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
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

    /// Returns true if the entry's key or value contains the given keyword
    /// (case-insensitive).
    pub fn matches_keyword(&self, keyword: &str) -> bool {
        let kw = keyword.to_lowercase();
        self.key.to_lowercase().contains(&kw)
            || self.value.to_lowercase().contains(&kw)
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
}

impl Default for EngineeringMemoryMetadata {
    fn default() -> Self {
        EngineeringMemoryMetadata {
            importance: 0.5,
            confidence: 0.5,
            tags: Vec::new(),
            source: None,
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
    pub fn from_entries(workspace_root: impl Into<String>, entries: Vec<EngineeringMemoryEntry>) -> Self {
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
