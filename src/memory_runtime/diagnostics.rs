use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::types::{MemoryEntry, MemoryEvent, MemoryTier};

/// Memory diagnostics tracker.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct MemoryDiagnostics {
    hits: u64,
    misses: u64,
    evictions: u64,
    snapshot_creations: u64,
    snapshot_merges: u64,
    policy_violations: u64,
    resolution_latencies: Vec<u64>,
    events: Vec<MemoryEvent>,
    max_events: usize,
}

impl MemoryDiagnostics {
    pub fn new(max_events: usize) -> Self {
        MemoryDiagnostics {
            max_events,
            ..Default::default()
        }
    }

    /// Record a memory hit.
    pub fn record_hit(&mut self) {
        self.hits += 1;
    }

    /// Record a memory miss.
    pub fn record_miss(&mut self) {
        self.misses += 1;
    }

    /// Record an eviction.
    pub fn record_eviction(&mut self) {
        self.evictions += 1;
    }

    /// Record a snapshot creation.
    pub fn record_snapshot_creation(&mut self) {
        self.snapshot_creations += 1;
    }

    /// Record a snapshot merge.
    pub fn record_snapshot_merge(&mut self) {
        self.snapshot_merges += 1;
    }

    /// Record a policy violation.
    pub fn record_policy_violation(&mut self) {
        self.policy_violations += 1;
    }

    /// Record a resolution latency.
    pub fn record_resolution_latency(&mut self, latency_ms: u64) {
        self.resolution_latencies.push(latency_ms);
        if self.resolution_latencies.len() > 1000 {
            self.resolution_latencies.remove(0);
        }
    }

    /// Record a memory event.
    pub fn record_event(&mut self, event: MemoryEvent) {
        if self.events.len() >= self.max_events {
            self.events.remove(0);
        }
        self.events.push(event);
    }

    /// Get hit rate.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    /// Get average resolution latency.
    pub fn avg_resolution_latency(&self) -> u64 {
        if self.resolution_latencies.is_empty() {
            0
        } else {
            self.resolution_latencies.iter().sum::<u64>() / self.resolution_latencies.len() as u64
        }
    }

    /// Get p95 resolution latency.
    pub fn p95_resolution_latency(&self) -> u64 {
        if self.resolution_latencies.is_empty() {
            0
        } else {
            let mut sorted = self.resolution_latencies.clone();
            sorted.sort();
            let idx = (sorted.len() as f64 * 0.95) as usize;
            sorted[idx.min(sorted.len() - 1)]
        }
    }

    /// Get diagnostics summary.
    pub fn summary(&self) -> MemoryDiagnosticsSummary {
        MemoryDiagnosticsSummary {
            total_hits: self.hits,
            total_misses: self.misses,
            hit_rate: self.hit_rate(),
            total_evictions: self.evictions,
            total_snapshot_creations: self.snapshot_creations,
            total_snapshot_merges: self.snapshot_merges,
            total_policy_violations: self.policy_violations,
            avg_resolution_latency_ms: self.avg_resolution_latency(),
            p95_resolution_latency_ms: self.p95_resolution_latency(),
            recent_events: self.events.len(),
        }
    }

    /// Get recent events.
    pub fn events(&self) -> &[MemoryEvent] {
        &self.events
    }

    /// Clear diagnostics.
    pub fn clear(&mut self) {
        self.hits = 0;
        self.misses = 0;
        self.evictions = 0;
        self.snapshot_creations = 0;
        self.snapshot_merges = 0;
        self.policy_violations = 0;
        self.resolution_latencies.clear();
        self.events.clear();
    }
}

/// Summary of memory diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDiagnosticsSummary {
    pub total_hits: u64,
    pub total_misses: u64,
    pub hit_rate: f64,
    pub total_evictions: u64,
    pub total_snapshot_creations: u64,
    pub total_snapshot_merges: u64,
    pub total_policy_violations: u64,
    pub avg_resolution_latency_ms: u64,
    pub p95_resolution_latency_ms: u64,
    pub recent_events: usize,
}

impl MemoryDiagnosticsSummary {
    pub fn is_healthy(&self) -> bool {
        self.total_policy_violations == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostics_record_hit() {
        let mut diag = MemoryDiagnostics::new(100);
        diag.record_hit();
        assert_eq!(diag.summary().total_hits, 1);
    }

    #[test]
    fn test_diagnostics_record_miss() {
        let mut diag = MemoryDiagnostics::new(100);
        diag.record_miss();
        assert_eq!(diag.summary().total_misses, 1);
    }

    #[test]
    fn test_diagnostics_hit_rate() {
        let mut diag = MemoryDiagnostics::new(100);
        diag.record_hit();
        diag.record_hit();
        diag.record_hit();
        diag.record_miss();
        assert_eq!(diag.hit_rate(), 0.75);
    }

    #[test]
    fn test_diagnostics_hit_rate_empty() {
        let diag = MemoryDiagnostics::new(100);
        assert_eq!(diag.hit_rate(), 0.0);
    }

    #[test]
    fn test_diagnostics_eviction() {
        let mut diag = MemoryDiagnostics::new(100);
        diag.record_eviction();
        assert_eq!(diag.summary().total_evictions, 1);
    }

    #[test]
    fn test_diagnostics_snapshot_creation() {
        let mut diag = MemoryDiagnostics::new(100);
        diag.record_snapshot_creation();
        assert_eq!(diag.summary().total_snapshot_creations, 1);
    }

    #[test]
    fn test_diagnostics_snapshot_merge() {
        let mut diag = MemoryDiagnostics::new(100);
        diag.record_snapshot_merge();
        assert_eq!(diag.summary().total_snapshot_merges, 1);
    }

    #[test]
    fn test_diagnostics_policy_violation() {
        let mut diag = MemoryDiagnostics::new(100);
        diag.record_policy_violation();
        assert_eq!(diag.summary().total_policy_violations, 1);
    }

    #[test]
    fn test_diagnostics_resolution_latency() {
        let mut diag = MemoryDiagnostics::new(100);
        diag.record_resolution_latency(10);
        diag.record_resolution_latency(20);
        diag.record_resolution_latency(30);
        assert_eq!(diag.avg_resolution_latency(), 20);
    }

    #[test]
    fn test_diagnostics_avg_latency_empty() {
        let diag = MemoryDiagnostics::new(100);
        assert_eq!(diag.avg_resolution_latency(), 0);
    }

    #[test]
    fn test_diagnostics_p95_latency() {
        let mut diag = MemoryDiagnostics::new(100);
        for i in 1..=100 {
            diag.record_resolution_latency(i);
        }
        let p95 = diag.p95_resolution_latency();
        assert!(p95 >= 90 && p95 <= 100);
    }

    #[test]
    fn test_diagnostics_summary_healthy() {
        let diag = MemoryDiagnostics::new(100);
        assert!(diag.summary().is_healthy());
    }

    #[test]
    fn test_diagnostics_summary_unhealthy() {
        let mut diag = MemoryDiagnostics::new(100);
        diag.record_policy_violation();
        assert!(!diag.summary().is_healthy());
    }

    #[test]
    fn test_diagnostics_clear() {
        let mut diag = MemoryDiagnostics::new(100);
        diag.record_hit();
        diag.record_miss();
        diag.clear();
        assert_eq!(diag.summary().total_hits, 0);
        assert_eq!(diag.summary().total_misses, 0);
    }

    #[test]
    fn test_diagnostics_record_event() {
        let mut diag = MemoryDiagnostics::new(100);
        diag.record_event(MemoryEvent::MemoryResolved {
            event_id: "e1".to_string(),
            query_key: "key".to_string(),
            tier: MemoryTier::Session,
            hit_count: 1,
            timestamp: 0,
        });
        assert_eq!(diag.events().len(), 1);
    }

    #[test]
    fn test_diagnostics_event_limit() {
        let mut diag = MemoryDiagnostics::new(5);
        for i in 0..10 {
            diag.record_event(MemoryEvent::MemoryResolved {
                event_id: format!("e{}", i),
                query_key: "key".to_string(),
                tier: MemoryTier::Session,
                hit_count: 1,
                timestamp: 0,
            });
        }
        assert_eq!(diag.events().len(), 5);
    }
}
