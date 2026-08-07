use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// The three logical memory tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemoryTier {
    /// Session-level memory: transient, tied to a single conversation
    Session,
    /// Project-level memory: persists across sessions for a project
    Project,
    /// Global memory: shared across all projects and sessions
    Global,
}

impl fmt::Display for MemoryTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryTier::Session => write!(f, "session"),
            MemoryTier::Project => write!(f, "project"),
            MemoryTier::Global => write!(f, "global"),
        }
    }
}

impl MemoryTier {
    /// Resolution order: Session -> Project -> Global
    pub fn resolution_order(&self) -> usize {
        match self {
            MemoryTier::Session => 0,
            MemoryTier::Project => 1,
            MemoryTier::Global => 2,
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "session" => Some(MemoryTier::Session),
            "project" => Some(MemoryTier::Project),
            "global" => Some(MemoryTier::Global),
            _ => None,
        }
    }
}

/// A memory entry is a piece of knowledge with metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub tier: MemoryTier,
    pub key: String,
    pub value: String,
    pub metadata: MemoryMetadata,
    pub created_at: u64,
    pub last_accessed: u64,
    pub access_count: u64,
}

impl MemoryEntry {
    pub fn new(
        id: impl Into<String>,
        tier: MemoryTier,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        MemoryEntry {
            id: id.into(),
            tier,
            key: key.into(),
            value: value.into(),
            metadata: MemoryMetadata::default(),
            created_at: now,
            last_accessed: now,
            access_count: 0,
        }
    }

    pub fn with_metadata(mut self, metadata: MemoryMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn record_access(&mut self) {
        self.last_accessed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.access_count += 1;
    }

    pub fn is_expired(&self, ttl: Duration) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_sub(self.last_accessed) > ttl.as_secs()
    }

    pub fn matches_key(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.key.to_lowercase().contains(&query)
            || self.value.to_lowercase().contains(&query)
    }
}

/// Metadata associated with a memory entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryMetadata {
    pub importance: f64,
    pub confidence: f64,
    pub tags: Vec<String>,
    pub source: Option<String>,
    pub context: Option<String>,
}

impl Default for MemoryMetadata {
    fn default() -> Self {
        MemoryMetadata {
            importance: 0.5,
            confidence: 0.5,
            tags: Vec::new(),
            source: None,
            context: None,
        }
    }
}

impl MemoryMetadata {
    pub fn new() -> Self {
        MemoryMetadata::default()
    }

    pub fn with_importance(mut self, importance: f64) -> Self {
        self.importance = importance.clamp(0.0, 1.0);
        self
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        let tag_str = tag.into();
        if !self.tags.contains(&tag_str) {
            self.tags.push(tag_str);
        }
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }
}

/// A query for memory resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryQuery {
    pub key: String,
    pub tier: Option<MemoryTier>,
    pub max_results: usize,
    pub require_confidence: Option<f64>,
    pub tags: Vec<String>,
}

impl MemoryQuery {
    pub fn new(key: impl Into<String>) -> Self {
        MemoryQuery {
            key: key.into(),
            tier: None,
            max_results: 10,
            require_confidence: None,
            tags: Vec::new(),
        }
    }

    pub fn in_tier(mut self, tier: MemoryTier) -> Self {
        self.tier = Some(tier);
        self
    }

    pub fn limit(mut self, max_results: usize) -> Self {
        self.max_results = max_results;
        self
    }

    pub fn require_confidence(mut self, confidence: f64) -> Self {
        self.require_confidence = Some(confidence);
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        let tag_str = tag.into();
        if !self.tags.contains(&tag_str) {
            self.tags.push(tag_str);
        }
        self
    }
}

/// Result of a memory resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryResolution {
    pub query: MemoryQuery,
    pub hits: Vec<MemoryEntry>,
    pub misses: Vec<String>,
    pub resolution_order: Vec<MemoryTier>,
    pub latency_ms: u64,
}

impl MemoryResolution {
    pub fn new(query: MemoryQuery, hits: Vec<MemoryEntry>, latency_ms: u64) -> Self {
        let resolution_order = if query.tier.is_some() {
            vec![query.tier.unwrap()]
        } else {
            vec![
                MemoryTier::Session,
                MemoryTier::Project,
                MemoryTier::Global,
            ]
        };

        let misses = resolution_order
            .iter()
            .filter(|tier| !hits.iter().any(|h| h.tier == **tier))
            .map(|t| t.to_string())
            .collect();

        MemoryResolution {
            query,
            hits,
            misses,
            resolution_order,
            latency_ms,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    pub fn first_hit(&self) -> Option<&MemoryEntry> {
        self.hits.first()
    }
}

/// Memory events for diagnostics and observability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryEvent {
    MemoryResolved {
        event_id: String,
        query_key: String,
        tier: MemoryTier,
        hit_count: usize,
        timestamp: u64,
    },
    MemoryEvicted {
        event_id: String,
        entry_id: String,
        tier: MemoryTier,
        reason: String,
        timestamp: u64,
    },
    SnapshotCreated {
        event_id: String,
        snapshot_id: String,
        entry_count: usize,
        timestamp: u64,
    },
    SnapshotMerged {
        event_id: String,
        source_snapshot: String,
        target_snapshot: String,
        entries_merged: usize,
        timestamp: u64,
    },
    PolicyApplied {
        event_id: String,
        policy_name: String,
        action: String,
        affected_count: usize,
        timestamp: u64,
    },
}

impl fmt::Display for MemoryEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryEvent::MemoryResolved {
                event_id,
                query_key,
                tier,
                hit_count,
                ..
            } => {
                write!(
                    f,
                    "[{}] Memory resolved for '{}' in {}: {} hits",
                    event_id, query_key, tier, hit_count
                )
            }
            MemoryEvent::MemoryEvicted {
                event_id,
                entry_id,
                tier,
                reason,
                ..
            } => {
                write!(
                    f,
                    "[{}] Memory evicted from {}: {} - {}",
                    event_id, tier, entry_id, reason
                )
            }
            MemoryEvent::SnapshotCreated {
                event_id,
                snapshot_id,
                entry_count,
                ..
            } => {
                write!(
                    f,
                    "[{}] Snapshot created: {} with {} entries",
                    event_id, snapshot_id, entry_count
                )
            }
            MemoryEvent::SnapshotMerged {
                event_id,
                source_snapshot,
                target_snapshot,
                entries_merged,
                ..
            } => {
                write!(
                    f,
                    "[{}] Snapshots merged: {} -> {} ({} entries)",
                    event_id, source_snapshot, target_snapshot, entries_merged
                )
            }
            MemoryEvent::PolicyApplied {
                event_id,
                policy_name,
                action,
                affected_count,
                ..
            } => {
                write!(
                    f,
                    "[{}] Policy '{}': {} ({} entries affected)",
                    event_id, policy_name, action, affected_count
                )
            }
        }
    }
}

/// Errors specific to memory runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryRuntimeError {
    EntryNotFound(String),
    InvalidTier(String),
    SnapshotError(String),
    PolicyViolation(String),
    ResolutionError(String),
    Conflict(String),
    Generic(String),
}

impl fmt::Display for MemoryRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryRuntimeError::EntryNotFound(id) => write!(f, "Entry not found: {}", id),
            MemoryRuntimeError::InvalidTier(tier) => write!(f, "Invalid tier: {}", tier),
            MemoryRuntimeError::SnapshotError(msg) => write!(f, "Snapshot error: {}", msg),
            MemoryRuntimeError::PolicyViolation(msg) => write!(f, "Policy violation: {}", msg),
            MemoryRuntimeError::ResolutionError(msg) => write!(f, "Resolution error: {}", msg),
            MemoryRuntimeError::Conflict(msg) => write!(f, "Conflict: {}", msg),
            MemoryRuntimeError::Generic(msg) => write!(f, "Memory runtime error: {}", msg),
        }
    }
}

impl std::error::Error for MemoryRuntimeError {}

pub type MemoryRuntimeResult<T> = Result<T, MemoryRuntimeError>;
