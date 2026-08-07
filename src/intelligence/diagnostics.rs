#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Intelligence Platform Diagnostics
//!
//! Provides observability into the intelligence platform's health:
//! parse metrics, index health, graph integrity, search quality, and context quality.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Maximum number of records to retain per metric type.
const MAX_RECORDS: usize = 500;

// =========================================================================
// Parse Metrics
// =========================================================================

/// A single parse operation record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseMetric {
    pub file: String,
    pub language: String,
    pub duration_ms: f64,
    pub symbol_count: usize,
    pub error_count: usize,
    pub timestamp: String,
}

// =========================================================================
// Index Health
// =========================================================================

/// An event in the index lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexEvent {
    FileIndexed {
        file: String,
        symbol_count: usize,
    },
    FileRemoved {
        file: String,
    },
    DatabaseCorrupted {
        reason: String,
    },
    IndexRebuilt {
        file_count: usize,
        symbol_count: usize,
    },
}

/// Health status of the symbol index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexHealth {
    pub total_symbols: u32,
    pub total_files: u32,
    pub total_relationships: u32,
    pub languages: Vec<String>,
    pub last_index_event: Option<String>,
    pub health_status: IndexHealthStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IndexHealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

// =========================================================================
// Graph Integrity
// =========================================================================

/// An event in the graph lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GraphEvent {
    GraphBuilt {
        file_count: usize,
        edge_count: usize,
    },
    NodeAdded {
        file: String,
    },
    EdgeAdded {
        from: String,
        to: String,
    },
    CycleDetected {
        files: Vec<String>,
    },
    OrphanedNodeRemoved {
        file: String,
    },
}

/// Integrity status of the dependency graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphIntegrity {
    pub total_nodes: usize,
    pub total_edges: usize,
    pub orphaned_nodes: usize,
    pub cycles_detected: usize,
    pub health_status: GraphIntegrityStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum GraphIntegrityStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

// =========================================================================
// Search Metrics
// =========================================================================

/// A single search operation record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMetric {
    pub query: String,
    pub result_count: usize,
    pub duration_ms: f64,
    pub timestamp: String,
}

// =========================================================================
// Context Metrics
// =========================================================================

/// A single context build operation record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMetric {
    pub query: String,
    pub symbol_count: usize,
    pub file_count: usize,
    pub duration_ms: f64,
    pub timestamp: String,
}

// =========================================================================
// Diagnostics Collector
// =========================================================================

#[derive(Debug)]
struct IntelligenceDiagnosticsInner {
    parse_metrics: Vec<ParseMetric>,
    index_events: Vec<IndexEvent>,
    graph_events: Vec<GraphEvent>,
    search_metrics: Vec<SearchMetric>,
    context_metrics: Vec<ContextMetric>,
    index_health: IndexHealth,
    graph_integrity: GraphIntegrity,
}

/// Central diagnostics collector for the intelligence platform.
///
/// Thread-safe: can be shared across tasks via `Arc`.
#[derive(Debug, Clone)]
pub struct IntelligenceDiagnostics {
    inner: Arc<Mutex<IntelligenceDiagnosticsInner>>,
}

impl IntelligenceDiagnostics {
    /// Creates a new empty diagnostics collector.
    pub fn new() -> Self {
        IntelligenceDiagnostics {
            inner: Arc::new(Mutex::new(IntelligenceDiagnosticsInner {
                parse_metrics: Vec::new(),
                index_events: Vec::new(),
                graph_events: Vec::new(),
                search_metrics: Vec::new(),
                context_metrics: Vec::new(),
                index_health: IndexHealth {
                    total_symbols: 0,
                    total_files: 0,
                    total_relationships: 0,
                    languages: Vec::new(),
                    last_index_event: None,
                    health_status: IndexHealthStatus::Healthy,
                },
                graph_integrity: GraphIntegrity {
                    total_nodes: 0,
                    total_edges: 0,
                    orphaned_nodes: 0,
                    cycles_detected: 0,
                    health_status: GraphIntegrityStatus::Healthy,
                },
            })),
        }
    }

    // ------------------------------------------------------------------
    // Parse Metrics
    // ------------------------------------------------------------------

    /// Records a parse operation.
    pub fn record_parse(
        &mut self,
        file: &str,
        language: &str,
        duration_ms: f64,
        symbol_count: usize,
        error_count: usize,
    ) {
        let metric = ParseMetric {
            file: file.to_string(),
            language: language.to_string(),
            duration_ms,
            symbol_count,
            error_count,
            timestamp: chrono::Local::now().to_rfc3339(),
        };
        let mut inner = self.inner.lock().unwrap();
        inner.parse_metrics.push(metric);
        if inner.parse_metrics.len() > MAX_RECORDS {
            inner.parse_metrics.remove(0);
        }
    }

    /// Returns all parse metrics.
    pub fn get_parse_metrics(&self) -> Vec<ParseMetric> {
        let inner = self.inner.lock().unwrap();
        inner.parse_metrics.clone()
    }

    /// Returns average parse duration for a language.
    pub fn avg_parse_duration(&self, language: &str) -> f64 {
        let inner = self.inner.lock().unwrap();
        let related: Vec<_> = inner
            .parse_metrics
            .iter()
            .filter(|m| m.language == language)
            .collect();
        if related.is_empty() {
            return 0.0;
        }
        related.iter().map(|m| m.duration_ms).sum::<f64>() / related.len() as f64
    }

    // ------------------------------------------------------------------
    // Index Health
    // ------------------------------------------------------------------

    /// Records an index event.
    pub fn record_index_event(&mut self, event: IndexEvent) {
        let mut inner = self.inner.lock().unwrap();
        inner.index_events.push(event);
        if inner.index_events.len() > MAX_RECORDS {
            inner.index_events.remove(0);
        }
        inner.index_health.last_index_event = Some(chrono::Local::now().to_rfc3339());
    }

    /// Updates index health from database stats.
    pub fn update_index_health(
        &mut self,
        total_symbols: u32,
        total_files: u32,
        total_relationships: u32,
        languages: Vec<String>,
    ) {
        let mut inner = self.inner.lock().unwrap();
        inner.index_health = IndexHealth {
            total_symbols,
            total_files,
            total_relationships,
            languages,
            last_index_event: inner.index_health.last_index_event.clone(),
            health_status: self.compute_index_health(total_symbols, total_files),
        };
    }

    fn compute_index_health(&self, symbols: u32, files: u32) -> IndexHealthStatus {
        if symbols == 0 {
            return IndexHealthStatus::Degraded;
        }
        if files == 0 {
            return IndexHealthStatus::Degraded;
        }
        IndexHealthStatus::Healthy
    }

    /// Returns current index health.
    pub fn get_index_health(&self) -> IndexHealth {
        let inner = self.inner.lock().unwrap();
        inner.index_health.clone()
    }

    // ------------------------------------------------------------------
    // Graph Integrity
    // ------------------------------------------------------------------

    /// Records a graph event.
    pub fn record_graph_event(&mut self, event: GraphEvent) {
        let mut inner = self.inner.lock().unwrap();
        inner.graph_events.push(event.clone());
        if inner.graph_events.len() > MAX_RECORDS {
            inner.graph_events.remove(0);
        }
        match &event {
            GraphEvent::CycleDetected { .. } => {
                inner.graph_integrity.cycles_detected += 1;
            }
            GraphEvent::OrphanedNodeRemoved { .. } => {
                inner.graph_integrity.orphaned_nodes += 1;
            }
            _ => {}
        }
        inner.graph_integrity.health_status = self.compute_graph_health(&inner.graph_integrity);
    }

    /// Updates graph integrity from graph stats.
    pub fn update_graph_integrity(&mut self, total_nodes: usize, total_edges: usize) {
        let mut inner = self.inner.lock().unwrap();
        inner.graph_integrity.total_nodes = total_nodes;
        inner.graph_integrity.total_edges = total_edges;
        inner.graph_integrity.health_status = self.compute_graph_health(&inner.graph_integrity);
    }

    fn compute_graph_health(&self, integrity: &GraphIntegrity) -> GraphIntegrityStatus {
        if integrity.cycles_detected > 10 {
            return GraphIntegrityStatus::Unhealthy;
        }
        if integrity.orphaned_nodes > 50 {
            return GraphIntegrityStatus::Degraded;
        }
        GraphIntegrityStatus::Healthy
    }

    /// Returns current graph integrity.
    pub fn get_graph_integrity(&self) -> GraphIntegrity {
        let inner = self.inner.lock().unwrap();
        inner.graph_integrity.clone()
    }

    // ------------------------------------------------------------------
    // Search Metrics
    // ------------------------------------------------------------------

    /// Records a search operation.
    pub fn record_search(&mut self, query: &str, result_count: usize, duration_ms: f64) {
        let metric = SearchMetric {
            query: query.to_string(),
            result_count,
            duration_ms,
            timestamp: chrono::Local::now().to_rfc3339(),
        };
        let mut inner = self.inner.lock().unwrap();
        inner.search_metrics.push(metric);
        if inner.search_metrics.len() > MAX_RECORDS {
            inner.search_metrics.remove(0);
        }
    }

    /// Returns all search metrics.
    pub fn get_search_metrics(&self) -> Vec<SearchMetric> {
        let inner = self.inner.lock().unwrap();
        inner.search_metrics.clone()
    }

    /// Returns average search latency.
    pub fn avg_search_latency(&self) -> f64 {
        let inner = self.inner.lock().unwrap();
        if inner.search_metrics.is_empty() {
            return 0.0;
        }
        inner
            .search_metrics
            .iter()
            .map(|m| m.duration_ms)
            .sum::<f64>()
            / inner.search_metrics.len() as f64
    }

    // ------------------------------------------------------------------
    // Context Metrics
    // ------------------------------------------------------------------

    /// Records a context build operation.
    pub fn record_context_build(
        &mut self,
        query: &str,
        symbol_count: usize,
        file_count: usize,
        duration_ms: f64,
    ) {
        let metric = ContextMetric {
            query: query.to_string(),
            symbol_count,
            file_count,
            duration_ms,
            timestamp: chrono::Local::now().to_rfc3339(),
        };
        let mut inner = self.inner.lock().unwrap();
        inner.context_metrics.push(metric);
        if inner.context_metrics.len() > MAX_RECORDS {
            inner.context_metrics.remove(0);
        }
    }

    /// Returns all context metrics.
    pub fn get_context_metrics(&self) -> Vec<ContextMetric> {
        let inner = self.inner.lock().unwrap();
        inner.context_metrics.clone()
    }

    /// Returns average context build latency.
    pub fn avg_context_latency(&self) -> f64 {
        let inner = self.inner.lock().unwrap();
        if inner.context_metrics.is_empty() {
            return 0.0;
        }
        inner
            .context_metrics
            .iter()
            .map(|m| m.duration_ms)
            .sum::<f64>()
            / inner.context_metrics.len() as f64
    }

    // ------------------------------------------------------------------
    // Summary & Maintenance
    // ------------------------------------------------------------------

    /// Returns a human-readable summary of all diagnostics.
    pub fn summary(&self) -> String {
        let inner = self.inner.lock().unwrap();
        format!(
            "Intelligence Platform Diagnostics\n\
             ═════════════════════════════════\n\
             Parse Metrics:\n\
               Total operations: {}\n\
             Index Health:\n\
               Symbols: {} | Files: {} | Relationships: {}\n\
               Status: {:?}\n\
             Graph Integrity:\n\
               Nodes: {} | Edges: {} | Cycles: {} | Orphans: {}\n\
               Status: {:?}\n\
             Search Metrics:\n\
               Total operations: {}\n\
             Context Metrics:\n\
               Total operations: {}",
            inner.parse_metrics.len(),
            inner.index_health.total_symbols,
            inner.index_health.total_files,
            inner.index_health.total_relationships,
            inner.index_health.health_status,
            inner.graph_integrity.total_nodes,
            inner.graph_integrity.total_edges,
            inner.graph_integrity.cycles_detected,
            inner.graph_integrity.orphaned_nodes,
            inner.graph_integrity.health_status,
            inner.search_metrics.len(),
            inner.context_metrics.len(),
        )
    }

    /// Clears all diagnostic records.
    pub fn clear(&mut self) {
        let mut inner = self.inner.lock().unwrap();
        inner.parse_metrics.clear();
        inner.index_events.clear();
        inner.graph_events.clear();
        inner.search_metrics.clear();
        inner.context_metrics.clear();
        inner.index_health = IndexHealth {
            total_symbols: 0,
            total_files: 0,
            total_relationships: 0,
            languages: Vec::new(),
            last_index_event: None,
            health_status: IndexHealthStatus::Healthy,
        };
        inner.graph_integrity = GraphIntegrity {
            total_nodes: 0,
            total_edges: 0,
            orphaned_nodes: 0,
            cycles_detected: 0,
            health_status: GraphIntegrityStatus::Healthy,
        };
    }
}

impl Default for IntelligenceDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}

// =========================================================================
// Trait Definition
// =========================================================================

/// Trait for intelligence platform diagnostics.
///
/// Implementations provide observability into parse, index, graph,
/// search, and context operations.
pub trait IntelligenceDiagnosticsTrait: Send + Sync {
    fn record_parse(
        &mut self,
        file: &str,
        language: &str,
        duration_ms: f64,
        symbol_count: usize,
        error_count: usize,
    );
    fn get_parse_metrics(&self) -> Vec<ParseMetric>;
    fn record_index_event(&mut self, event: IndexEvent);
    fn get_index_health(&self) -> IndexHealth;
    fn record_graph_event(&mut self, event: GraphEvent);
    fn get_graph_integrity(&self) -> GraphIntegrity;
    fn record_search(&mut self, query: &str, result_count: usize, duration_ms: f64);
    fn get_search_metrics(&self) -> Vec<SearchMetric>;
    fn record_context_build(
        &mut self,
        query: &str,
        symbol_count: usize,
        file_count: usize,
        duration_ms: f64,
    );
    fn get_context_metrics(&self) -> Vec<ContextMetric>;
    fn summary(&self) -> String;
    fn clear(&mut self);
}

impl IntelligenceDiagnosticsTrait for IntelligenceDiagnostics {
    fn record_parse(
        &mut self,
        file: &str,
        language: &str,
        duration_ms: f64,
        symbol_count: usize,
        error_count: usize,
    ) {
        IntelligenceDiagnostics::record_parse(
            self,
            file,
            language,
            duration_ms,
            symbol_count,
            error_count,
        );
    }

    fn get_parse_metrics(&self) -> Vec<ParseMetric> {
        IntelligenceDiagnostics::get_parse_metrics(self)
    }

    fn record_index_event(&mut self, event: IndexEvent) {
        IntelligenceDiagnostics::record_index_event(self, event);
    }

    fn get_index_health(&self) -> IndexHealth {
        IntelligenceDiagnostics::get_index_health(self)
    }

    fn record_graph_event(&mut self, event: GraphEvent) {
        IntelligenceDiagnostics::record_graph_event(self, event);
    }

    fn get_graph_integrity(&self) -> GraphIntegrity {
        IntelligenceDiagnostics::get_graph_integrity(self)
    }

    fn record_search(&mut self, query: &str, result_count: usize, duration_ms: f64) {
        IntelligenceDiagnostics::record_search(self, query, result_count, duration_ms);
    }

    fn get_search_metrics(&self) -> Vec<SearchMetric> {
        IntelligenceDiagnostics::get_search_metrics(self)
    }

    fn record_context_build(
        &mut self,
        query: &str,
        symbol_count: usize,
        file_count: usize,
        duration_ms: f64,
    ) {
        IntelligenceDiagnostics::record_context_build(
            self,
            query,
            symbol_count,
            file_count,
            duration_ms,
        );
    }

    fn get_context_metrics(&self) -> Vec<ContextMetric> {
        IntelligenceDiagnostics::get_context_metrics(self)
    }

    fn summary(&self) -> String {
        IntelligenceDiagnostics::summary(self)
    }

    fn clear(&mut self) {
        IntelligenceDiagnostics::clear(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostics_creation() {
        let diag = IntelligenceDiagnostics::new();
        let summary = diag.summary();
        assert!(summary.contains("Intelligence Platform Diagnostics"));
        assert!(summary.contains("Symbols: 0"));
    }

    #[test]
    fn test_parse_metric_recording() {
        let mut diag = IntelligenceDiagnostics::new();
        diag.record_parse("test.rs", "rust", 15.0, 5, 0);
        diag.record_parse("main.rs", "rust", 25.0, 10, 0);

        let metrics = diag.get_parse_metrics();
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].file, "test.rs");
        assert_eq!(metrics[1].symbol_count, 10);

        let avg = diag.avg_parse_duration("rust");
        assert!((avg - 20.0).abs() < 0.01);
    }

    #[test]
    fn test_index_health_update() {
        let mut diag = IntelligenceDiagnostics::new();
        diag.update_index_health(100, 10, 50, vec!["rust".to_string(), "go".to_string()]);

        let health = diag.get_index_health();
        assert_eq!(health.total_symbols, 100);
        assert_eq!(health.total_files, 10);
        assert_eq!(health.languages.len(), 2);
        assert_eq!(health.health_status, IndexHealthStatus::Healthy);
    }

    #[test]
    fn test_graph_integrity_update() {
        let mut diag = IntelligenceDiagnostics::new();
        diag.update_graph_integrity(50, 80);

        let integrity = diag.get_graph_integrity();
        assert_eq!(integrity.total_nodes, 50);
        assert_eq!(integrity.total_edges, 80);
        assert_eq!(integrity.health_status, GraphIntegrityStatus::Healthy);
    }

    #[test]
    fn test_search_metric_recording() {
        let mut diag = IntelligenceDiagnostics::new();
        diag.record_search("auth", 5, 8.0);
        diag.record_search("user", 3, 5.0);

        let metrics = diag.get_search_metrics();
        assert_eq!(metrics.len(), 2);
        assert!((diag.avg_search_latency() - 6.5).abs() < 0.01);
    }

    #[test]
    fn test_context_metric_recording() {
        let mut diag = IntelligenceDiagnostics::new();
        diag.record_context_build("test", 10, 3, 50.0);
        diag.record_context_build("query", 5, 2, 30.0);

        let metrics = diag.get_context_metrics();
        assert_eq!(metrics.len(), 2);
        assert!((diag.avg_context_latency() - 40.0).abs() < 0.01);
    }

    #[test]
    fn test_index_event_cycle_detection() {
        let mut diag = IntelligenceDiagnostics::new();
        diag.record_graph_event(GraphEvent::CycleDetected {
            files: vec!["a.rs".to_string(), "b.rs".to_string()],
        });
        diag.record_graph_event(GraphEvent::CycleDetected {
            files: vec!["c.rs".to_string(), "d.rs".to_string()],
        });

        let integrity = diag.get_graph_integrity();
        assert_eq!(integrity.cycles_detected, 2);
    }

    #[test]
    fn test_clear() {
        let mut diag = IntelligenceDiagnostics::new();
        diag.record_parse("test.rs", "rust", 10.0, 5, 0);
        diag.record_search("test", 3, 5.0);
        diag.clear();

        assert_eq!(diag.get_parse_metrics().len(), 0);
        assert_eq!(diag.get_search_metrics().len(), 0);
    }

    #[test]
    fn test_lru_eviction() {
        let mut diag = IntelligenceDiagnostics::new();
        for i in 0..=MAX_RECORDS {
            diag.record_parse(&format!("file_{}.rs", i), "rust", 1.0, 1, 0);
        }
        assert_eq!(diag.get_parse_metrics().len(), MAX_RECORDS);
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let diag = IntelligenceDiagnostics::new();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let mut diag = diag.clone();
                thread::spawn(move || {
                    for j in 0..100 {
                        diag.record_parse(&format!("file_{}_{}.rs", i, j), "rust", 1.0, 1, 0);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let count = diag.get_parse_metrics().len();
        assert!(
            count > 0,
            "should have recorded parse metrics, got {}",
            count
        );
        assert!(
            count <= MAX_RECORDS,
            "should not exceed MAX_RECORDS, got {}",
            count
        );
    }

    #[test]
    fn test_trait_impl() {
        let diag = IntelligenceDiagnostics::new();
        // Verify it implements the trait
        fn assert_trait<T: IntelligenceDiagnosticsTrait>(_t: &T) {}
        assert_trait(&diag);
    }
}
