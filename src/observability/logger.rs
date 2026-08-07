#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Logger — structured logging with correlation IDs and pluggable sinks.

use std::sync::{Arc, Mutex};

use crate::observability::types::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub level: LogLevel,
    pub correlation_id: CorrelationId,
    pub trace_id: Option<TraceId>,
    pub target: String,
    pub message: String,
    pub wall_clock: String,
    pub attributes: Vec<Dimension>,
}

impl std::fmt::Display for LogEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {} {} [corr={} trace={:?}] {}",
            self.wall_clock,
            self.level.as_str(),
            self.target,
            self.correlation_id,
            self.trace_id.as_ref().map(|t| t.to_string()),
            self.message
        )
    }
}

pub trait LogSink: Send + Sync {
    fn emit(&self, entry: &LogEntry);
}

#[derive(Debug, Default)]
pub struct MemoryLogSink {
    entries: Arc<Mutex<Vec<LogEntry>>>,
    max_entries: usize,
}

impl Clone for MemoryLogSink {
    fn clone(&self) -> Self {
        MemoryLogSink {
            entries: self.entries.clone(),
            max_entries: self.max_entries,
        }
    }
}

impl MemoryLogSink {
    pub fn new(max_entries: usize) -> Self {
        MemoryLogSink {
            entries: Arc::new(Mutex::new(Vec::new())),
            max_entries,
        }
    }

    pub fn entries(&self) -> Vec<LogEntry> {
        let inner = self.entries.lock().unwrap();
        inner.clone()
    }

    pub fn count(&self) -> usize {
        let inner = self.entries.lock().unwrap();
        inner.len()
    }
}

impl LogSink for MemoryLogSink {
    fn emit(&self, entry: &LogEntry) {
        let mut inner = self.entries.lock().unwrap();
        inner.push(entry.clone());
        if inner.len() > self.max_entries {
            inner.remove(0);
        }
    }
}

pub struct ConsoleLogSink;

impl LogSink for ConsoleLogSink {
    fn emit(&self, entry: &LogEntry) {
        eprintln!("{}", entry);
    }
}

#[derive(Clone)]
pub struct Logger {
    correlation_id: CorrelationId,
    trace_id: Option<TraceId>,
    target: String,
    sinks: Arc<Vec<Box<dyn LogSink>>>,
}

impl Logger {
    pub fn new(correlation_id: CorrelationId, target: &str) -> Self {
        Logger {
            correlation_id,
            trace_id: None,
            target: target.to_string(),
            sinks: Arc::new(Vec::new()),
        }
    }

    pub fn child(&self, target: &str) -> Self {
        Logger {
            correlation_id: self.correlation_id.clone(),
            trace_id: self.trace_id.clone(),
            target: target.to_string(),
            sinks: self.sinks.clone(),
        }
    }

    pub fn with_trace(mut self, trace_id: TraceId) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    pub fn add_sink(&mut self, sink: Box<dyn LogSink>) {
        Arc::get_mut(&mut self.sinks).unwrap().push(sink);
    }

    pub fn trace(&self, message: &str) {
        self.log(LogLevel::Trace, message);
    }

    pub fn debug(&self, message: &str) {
        self.log(LogLevel::Debug, message);
    }

    pub fn info(&self, message: &str) {
        self.log(LogLevel::Info, message);
    }

    pub fn warn(&self, message: &str) {
        self.log(LogLevel::Warn, message);
    }

    pub fn error(&self, message: &str) {
        self.log(LogLevel::Error, message);
    }

    fn log(&self, level: LogLevel, message: &str) {
        let entry = LogEntry {
            level,
            correlation_id: self.correlation_id.clone(),
            trace_id: self.trace_id.clone(),
            target: self.target.clone(),
            message: message.to_string(),
            wall_clock: chrono::Local::now().to_rfc3339(),
            attributes: Vec::new(),
        };
        for sink in self.sinks.iter() {
            sink.emit(&entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logger_creation() {
        let logger = Logger::new(CorrelationId::new(), "test-target");
        assert_eq!(logger.target, "test-target");
    }

    #[test]
    fn test_child_logger() {
        let parent = Logger::new(CorrelationId::new(), "parent");
        let child = parent.child("child");
        assert_eq!(child.correlation_id, parent.correlation_id);
        assert_eq!(child.target, "child");
    }

    #[test]
    fn test_memory_sink() {
        let sink = MemoryLogSink::new(100);
        let mut logger = Logger::new(CorrelationId::new(), "test");
        logger.add_sink(Box::new(sink.clone()));
        logger.info("hello");
        assert_eq!(sink.count(), 1);
        let entries = sink.entries();
        assert_eq!(entries[0].level, LogLevel::Info);
        assert_eq!(entries[0].message, "hello");
    }

    #[test]
    fn test_lru_eviction() {
        let sink = MemoryLogSink::new(5);
        let mut logger = Logger::new(CorrelationId::new(), "test");
        logger.add_sink(Box::new(sink.clone()));
        for i in 0..10 {
            logger.info(&format!("msg {i}"));
        }
        assert_eq!(sink.count(), 5);
    }

    #[test]
    fn test_all_log_levels() {
        let sink = MemoryLogSink::new(100);
        let mut logger = Logger::new(CorrelationId::new(), "test");
        logger.add_sink(Box::new(sink.clone()));
        logger.trace("t");
        logger.debug("d");
        logger.info("i");
        logger.warn("w");
        logger.error("e");
        assert_eq!(sink.count(), 5);
        let entries = sink.entries();
        assert_eq!(entries[0].level, LogLevel::Trace);
        assert_eq!(entries[4].level, LogLevel::Error);
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let sink = MemoryLogSink::new(10_000);
        let logger = Logger::new(CorrelationId::new(), "test");
        let mut l = logger;
        l.add_sink(Box::new(sink.clone()));
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let l = l.child(&format!("t{i}"));
                thread::spawn(move || {
                    for _ in 0..100 {
                        l.info("log");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(sink.count(), 1000);
    }
}
