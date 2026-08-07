use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Memory retention policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RetentionPolicy {
    /// Keep entries indefinitely
    Infinite,
    /// Keep entries for a fixed duration
    Duration(Duration),
    /// Keep entries up to a max count per tier
    MaxCount { per_tier: usize },
    /// Keep high-importance entries, evict low-value ones
    ImportanceThreshold { threshold: f64 },
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        RetentionPolicy::Duration(Duration::from_secs(30 * 24 * 3600)) // 30 days
    }
}

/// Memory eviction policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EvictionPolicy {
    /// Evict least recently used
    LRU,
    /// Evict least frequently used
    LFU,
    /// Evict lowest importance
    LowestImportance,
    /// Evict lowest confidence
    LowestConfidence,
    /// Evict oldest entries
    FIFO,
}

impl Default for EvictionPolicy {
    fn default() -> Self {
        EvictionPolicy::LRU
    }
}

/// Memory expiration policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExpirationPolicy {
    /// No expiration
    None,
    /// Expire after duration from last access
    IdleTimeout(Duration),
    /// Expire after duration from creation
    AbsoluteTimeout(Duration),
    /// Expire based on importance score
    ImportanceThreshold { threshold: f64 },
}

impl Default for ExpirationPolicy {
    fn default() -> Self {
        ExpirationPolicy::None
    }
}

/// Memory priority policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PriorityPolicy {
    /// Priority based on importance metadata
    Importance,
    /// Priority based on recency
    Recency,
    /// Priority based on access frequency
    Frequency,
}

impl Default for PriorityPolicy {
    fn default() -> Self {
        PriorityPolicy::Importance
    }
}

/// Conflict resolution policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictPolicy {
    /// First match wins (Session > Project > Global)
    FirstMatch,
    /// Highest importance wins
    HighestImportance,
    /// Highest confidence wins
    HighestConfidence,
    /// Most recent access wins
    MostRecent,
    /// Most accessed wins
    MostAccessed,
}

impl Default for ConflictPolicy {
    fn default() -> Self {
        ConflictPolicy::FirstMatch
    }
}

/// Access rule for memory entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessRule {
    pub tier: crate::memory_runtime::types::MemoryTier,
    pub allowed_keys: Vec<String>,
    pub denied_keys: Vec<String>,
    pub min_confidence: f64,
}

impl AccessRule {
    pub fn new(tier: crate::memory_runtime::types::MemoryTier) -> Self {
        AccessRule {
            tier,
            allowed_keys: Vec::new(),
            denied_keys: Vec::new(),
            min_confidence: 0.0,
        }
    }

    pub fn allow_key(mut self, key: impl Into<String>) -> Self {
        let key_str = key.into();
        if !self.allowed_keys.contains(&key_str) {
            self.allowed_keys.push(key_str);
        }
        self
    }

    pub fn deny_key(mut self, key: impl Into<String>) -> Self {
        let key_str = key.into();
        if !self.denied_keys.contains(&key_str) {
            self.denied_keys.push(key_str);
        }
        self
    }

    pub fn with_min_confidence(mut self, confidence: f64) -> Self {
        self.min_confidence = confidence.clamp(0.0, 1.0);
        self
    }

    pub fn matches(&self, key: &str, confidence: f64) -> bool {
        if confidence < self.min_confidence {
            return false;
        }
        if self.denied_keys.contains(&key.to_string()) {
            return false;
        }
        if self.allowed_keys.is_empty() {
            return true;
        }
        self.allowed_keys.contains(&key.to_string())
    }
}

/// Comprehensive memory policy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPolicy {
    pub retention: RetentionPolicy,
    pub eviction: EvictionPolicy,
    pub expiration: ExpirationPolicy,
    pub priority: PriorityPolicy,
    pub conflict_resolution: ConflictPolicy,
    pub access_rules: Vec<AccessRule>,
    pub max_entries_per_tier: usize,
    pub auto_consolidate: bool,
}

impl Default for MemoryPolicy {
    fn default() -> Self {
        MemoryPolicy {
            retention: RetentionPolicy::default(),
            eviction: EvictionPolicy::default(),
            expiration: ExpirationPolicy::default(),
            priority: PriorityPolicy::default(),
            conflict_resolution: ConflictPolicy::default(),
            access_rules: Vec::new(),
            max_entries_per_tier: 1000,
            auto_consolidate: false,
        }
    }
}

impl MemoryPolicy {
    pub fn new() -> Self {
        MemoryPolicy::default()
    }

    pub fn with_retention(mut self, policy: RetentionPolicy) -> Self {
        self.retention = policy;
        self
    }

    pub fn with_eviction(mut self, policy: EvictionPolicy) -> Self {
        self.eviction = policy;
        self
    }

    pub fn with_expiration(mut self, policy: ExpirationPolicy) -> Self {
        self.expiration = policy;
        self
    }

    pub fn with_priority(mut self, policy: PriorityPolicy) -> Self {
        self.priority = policy;
        self
    }

    pub fn with_conflict_resolution(mut self, policy: ConflictPolicy) -> Self {
        self.conflict_resolution = policy;
        self
    }

    pub fn with_access_rule(mut self, rule: AccessRule) -> Self {
        self.access_rules.push(rule);
        self
    }

    pub fn with_max_entries(mut self, max: usize) -> Self {
        self.max_entries_per_tier = max;
        self
    }

    pub fn with_auto_consolidate(mut self, enabled: bool) -> Self {
        self.auto_consolidate = enabled;
        self
    }

    /// Check if an entry is subject to eviction.
    pub fn should_evict(&self, entry: &crate::memory_runtime::types::MemoryEntry) -> bool {
        match &self.retention {
            RetentionPolicy::Infinite => false,
            RetentionPolicy::Duration(ttl) => entry.is_expired(*ttl),
            RetentionPolicy::MaxCount { .. } => false, // Check count separately
            RetentionPolicy::ImportanceThreshold { threshold } => {
                entry.metadata.importance < *threshold
            }
        }
    }

    /// Check if an entry has expired.
    pub fn is_expired(&self, entry: &crate::memory_runtime::types::MemoryEntry) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        match &self.expiration {
            ExpirationPolicy::None => false,
            ExpirationPolicy::IdleTimeout(duration) => {
                now.saturating_sub(entry.last_accessed) > duration.as_secs()
            }
            ExpirationPolicy::AbsoluteTimeout(duration) => {
                now.saturating_sub(entry.created_at) > duration.as_secs()
            }
            ExpirationPolicy::ImportanceThreshold { threshold } => {
                entry.metadata.importance < *threshold
            }
        }
    }

    /// Get priority score for an entry.
    pub fn priority_score(&self, entry: &crate::memory_runtime::types::MemoryEntry) -> f64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        match &self.priority {
            PriorityPolicy::Importance => entry.metadata.importance,
            PriorityPolicy::Recency => {
                let elapsed = now.saturating_sub(entry.last_accessed);
                if elapsed < 3600 {
                    1.0
                } else if elapsed < 86400 {
                    0.8
                } else if elapsed < 604800 {
                    0.5
                } else {
                    0.2
                }
            }
            PriorityPolicy::Frequency => {
                (entry.access_count as f64).min(10.0) / 10.0
            }
        }
    }

    /// Check if access is allowed for a key in a tier.
    pub fn is_access_allowed(
        &self,
        tier: crate::memory_runtime::types::MemoryTier,
        key: &str,
        confidence: f64,
    ) -> bool {
        self.access_rules
            .iter()
            .filter(|r| r.tier == tier)
            .all(|r| r.matches(key, confidence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_runtime::types::{MemoryEntry, MemoryMetadata, MemoryTier};

    fn test_entry(id: &str, tier: MemoryTier, key: &str, value: &str) -> MemoryEntry {
        MemoryEntry::new(id, tier, key, value)
            .with_metadata(MemoryMetadata::new().with_importance(0.8).with_confidence(0.9))
    }

    #[test]
    fn test_retention_policy_infinite() {
        let policy = MemoryPolicy::new().with_retention(RetentionPolicy::Infinite);
        let entry = test_entry("e1", MemoryTier::Session, "key", "value");
        assert!(!policy.should_evict(&entry));
    }

    #[test]
    fn test_retention_policy_duration() {
        let policy = MemoryPolicy::new().with_retention(RetentionPolicy::Duration(
            Duration::from_secs(1),
        ));
        let entry = test_entry("e1", MemoryTier::Session, "key", "value");
        // Entry is fresh, shouldn't be evicted
        assert!(!policy.should_evict(&entry));
    }

    #[test]
    fn test_retention_policy_importance_threshold() {
        let policy = MemoryPolicy::new().with_retention(RetentionPolicy::ImportanceThreshold {
            threshold: 0.5,
        });
        let entry = test_entry("e1", MemoryTier::Session, "key", "value");
        assert!(!policy.should_evict(&entry));

        let low_importance = MemoryEntry::new("e2", MemoryTier::Session, "key", "value")
            .with_metadata(MemoryMetadata::new().with_importance(0.3));
        assert!(policy.should_evict(&low_importance));
    }

    #[test]
    fn test_expiration_policy_none() {
        let policy = MemoryPolicy::new().with_expiration(ExpirationPolicy::None);
        let entry = test_entry("e1", MemoryTier::Session, "key", "value");
        assert!(!policy.is_expired(&entry));
    }

    #[test]
    fn test_expiration_policy_idle_timeout() {
        let policy = MemoryPolicy::new().with_expiration(ExpirationPolicy::IdleTimeout(
            Duration::from_secs(1),
        ));
        let entry = test_entry("e1", MemoryTier::Session, "key", "value");
        // Entry is fresh
        assert!(!policy.is_expired(&entry));
    }

    #[test]
    fn test_priority_policy_importance() {
        let policy = MemoryPolicy::new().with_priority(PriorityPolicy::Importance);
        let entry = test_entry("e1", MemoryTier::Session, "key", "value");
        assert_eq!(policy.priority_score(&entry), 0.8);
    }

    #[test]
    fn test_priority_policy_recency() {
        let policy = MemoryPolicy::new().with_priority(PriorityPolicy::Recency);
        let entry = test_entry("e1", MemoryTier::Session, "key", "value");
        // Fresh entry should have high recency score
        let score = policy.priority_score(&entry);
        assert!(score > 0.9);
    }

    #[test]
    fn test_priority_policy_frequency() {
        let policy = MemoryPolicy::new().with_priority(PriorityPolicy::Frequency);
        let mut entry = test_entry("e1", MemoryTier::Session, "key", "value");
        entry.access_count = 5;
        assert_eq!(policy.priority_score(&entry), 0.5);
    }

    #[test]
    fn test_access_rule_allow() {
        let rule = AccessRule::new(MemoryTier::Session)
            .allow_key("language")
            .allow_key("framework");
        assert!(rule.matches("language", 0.9));
        assert!(rule.matches("framework", 0.9));
        assert!(!rule.matches("other", 0.9));
    }

    #[test]
    fn test_access_rule_deny() {
        let rule = AccessRule::new(MemoryTier::Session).deny_key("secret");
        assert!(!rule.matches("secret", 0.9));
        assert!(rule.matches("other", 0.9));
    }

    #[test]
    fn test_access_rule_confidence() {
        let rule = AccessRule::new(MemoryTier::Session)
            .with_min_confidence(0.5);
        assert!(!rule.matches("key", 0.3));
        assert!(rule.matches("key", 0.6));
    }

    #[test]
    fn test_conflict_policy_first_match() {
        assert_eq!(ConflictPolicy::FirstMatch, ConflictPolicy::default());
    }

    #[test]
    fn test_memory_policy_default() {
        let policy = MemoryPolicy::default();
        assert!(matches!(policy.retention, RetentionPolicy::Duration(_)));
        assert_eq!(policy.max_entries_per_tier, 1000);
        assert!(!policy.auto_consolidate);
    }

    #[test]
    fn test_memory_policy_builder() {
        let policy = MemoryPolicy::new()
            .with_retention(RetentionPolicy::Infinite)
            .with_eviction(EvictionPolicy::LRU)
            .with_expiration(ExpirationPolicy::None)
            .with_priority(PriorityPolicy::Importance)
            .with_conflict_resolution(ConflictPolicy::HighestImportance)
            .with_max_entries(500)
            .with_auto_consolidate(true);

        assert!(matches!(policy.retention, RetentionPolicy::Infinite));
        assert_eq!(policy.max_entries_per_tier, 500);
        assert!(policy.auto_consolidate);
    }
}
