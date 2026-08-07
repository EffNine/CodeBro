#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Metrics — counters, gauges, and histograms for performance observability.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::observability::types::*;

const MAX_RECORDS: usize = 500;

#[derive(Debug)]
struct MetricInner {
    counters: HashMap<MetricName, u64>,
    gauges: HashMap<MetricName, f64>,
    histograms: HashMap<MetricName, Vec<f64>>,
    records: HashMap<MetricName, Vec<(String, MetricValue)>>,
}

#[derive(Debug, Clone)]
pub struct MetricRecorder {
    inner: Arc<Mutex<MetricInner>>,
}

impl MetricRecorder {
    pub fn new() -> Self {
        MetricRecorder {
            inner: Arc::new(Mutex::new(MetricInner {
                counters: HashMap::new(),
                gauges: HashMap::new(),
                histograms: HashMap::new(),
                records: HashMap::new(),
            })),
        }
    }

    pub fn increment(&self, name: MetricName, by: u64) {
        let mut inner = self.inner.lock().unwrap();
        *inner.counters.entry(name.clone()).or_insert(0) += by;
        let ts = chrono::Local::now().to_rfc3339();
        inner
            .records
            .entry(name)
            .or_default()
            .push((ts, MetricValue::Counter(by)));
    }

    pub fn set_gauge(&self, name: MetricName, value: f64) {
        let mut inner = self.inner.lock().unwrap();
        inner.gauges.insert(name.clone(), value);
        let ts = chrono::Local::now().to_rfc3339();
        inner
            .records
            .entry(name)
            .or_default()
            .push((ts, MetricValue::Gauge(value)));
    }

    pub fn record_histogram(&self, name: MetricName, value: f64) {
        let mut inner = self.inner.lock().unwrap();
        inner
            .histograms
            .entry(name.clone())
            .or_default()
            .push(value);
        if let Some(hist) = inner.histograms.get_mut(&name) {
            if hist.len() > MAX_RECORDS {
                hist.remove(0);
            }
        }
        let ts = chrono::Local::now().to_rfc3339();
        inner
            .records
            .entry(name)
            .or_default()
            .push((ts, MetricValue::Histogram(value)));
    }

    pub fn counter(&self, name: &MetricName) -> u64 {
        let inner = self.inner.lock().unwrap();
        inner.counters.get(name).copied().unwrap_or(0)
    }

    pub fn gauge(&self, name: &MetricName) -> f64 {
        let inner = self.inner.lock().unwrap();
        inner.gauges.get(name).copied().unwrap_or(0.0)
    }

    pub fn histogram(&self, name: &MetricName) -> Vec<f64> {
        let inner = self.inner.lock().unwrap();
        inner
            .histograms
            .get(name)
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    pub fn records(&self, name: &MetricName) -> Vec<(String, MetricValue)> {
        let inner = self.inner.lock().unwrap();
        inner
            .records
            .get(name)
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    pub fn all_counters(&self) -> Vec<(MetricName, u64)> {
        let inner = self.inner.lock().unwrap();
        inner
            .counters
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }

    pub fn all_gauges(&self) -> Vec<(MetricName, f64)> {
        let inner = self.inner.lock().unwrap();
        inner.gauges.iter().map(|(k, v)| (k.clone(), *v)).collect()
    }

    pub fn summary(&self) -> String {
        let inner = self.inner.lock().unwrap();
        let mut lines = Vec::new();
        lines.push("=== Metrics Summary ===".to_string());
        lines.push(format!("Counters: {}", inner.counters.len()));
        for (name, val) in &inner.counters {
            lines.push(format!("  {} = {}", name, val));
        }
        lines.push(format!("Gauges: {}", inner.gauges.len()));
        for (name, val) in &inner.gauges {
            lines.push(format!("  {} = {:.4}", name, val));
        }
        lines.push(format!("Histograms: {}", inner.histograms.len()));
        for (name, samples) in &inner.histograms {
            if !samples.is_empty() {
                let avg = samples.iter().sum::<f64>() / samples.len() as f64;
                lines.push(format!(
                    "  {} = avg {:.4}, samples={}",
                    name,
                    avg,
                    samples.len()
                ));
            }
        }
        lines.join("\n")
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.counters.clear();
        inner.gauges.clear();
        inner.histograms.clear();
        inner.records.clear();
    }
}

impl Default for MetricRecorder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counter_increment() {
        let m = MetricRecorder::new();
        m.increment(MetricName::ErrorCount, 1);
        m.increment(MetricName::ErrorCount, 1);
        assert_eq!(m.counter(&MetricName::ErrorCount), 2);
    }

    #[test]
    fn test_gauge_set() {
        let m = MetricRecorder::new();
        m.set_gauge(MetricName::ThreadUtilization, 0.75);
        assert!((m.gauge(&MetricName::ThreadUtilization) - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_histogram() {
        let m = MetricRecorder::new();
        m.record_histogram(MetricName::PipelineLatency, 100.0);
        m.record_histogram(MetricName::PipelineLatency, 200.0);
        m.record_histogram(MetricName::PipelineLatency, 150.0);
        let samples = m.histogram(&MetricName::PipelineLatency);
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0], 100.0);
        assert_eq!(samples[1], 200.0);
        assert_eq!(samples[2], 150.0);
    }

    #[test]
    fn test_summary() {
        let m = MetricRecorder::new();
        m.increment(MetricName::ErrorCount, 3);
        m.set_gauge(MetricName::ApprovalRate, 0.8);
        let summary = m.summary();
        assert!(summary.contains("error.count"));
        assert!(summary.contains("approval.rate"));
        assert!(summary.contains("3"));
    }

    #[test]
    fn test_clear() {
        let m = MetricRecorder::new();
        m.increment(MetricName::ErrorCount, 1);
        m.clear();
        assert_eq!(m.counter(&MetricName::ErrorCount), 0);
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let m = MetricRecorder::new();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let m = m.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        m.increment(MetricName::ErrorCount, 1);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(m.counter(&MetricName::ErrorCount), 1000);
    }

    #[test]
    fn test_all_counters() {
        let m = MetricRecorder::new();
        m.increment(MetricName::ErrorCount, 1);
        m.increment(MetricName::ValidationFailures, 2);
        let counters = m.all_counters();
        assert_eq!(counters.len(), 2);
    }
}
