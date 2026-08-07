#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Structured logging for the reliability layer.
///
/// Provides consistent logging with correlation IDs and structured output.
use std::sync::{Arc, Mutex};

/// Log levels supported by the structured logger.
#[derive(Debug, Clone, PartialEq, Eq)]
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

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "trace" => Some(LogLevel::Trace),
            "debug" => Some(LogLevel::Debug),
            "info" => Some(LogLevel::Info),
            "warn" => Some(LogLevel::Warn),
            "error" => Some(LogLevel::Error),
            _ => None,
        }
    }
}

/// A structured log entry.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: LogLevel,
    pub correlation_id: String,
    pub target: String,
    pub message: String,
    pub timestamp: String,
}

impl std::fmt::Display for LogEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {} {} [{}] {}",
            self.timestamp,
            self.level.as_str(),
            self.target,
            self.correlation_id,
            self.message
        )
    }
}

/// A sink that receives log entries.
pub trait LogSink: Send + Sync {
    fn emit(&self, entry: LogEntry);
}

/// A simple console log sink.
pub struct ConsoleLogSink;

impl LogSink for ConsoleLogSink {
    fn emit(&self, entry: LogEntry) {
        eprintln!("{}", entry);
    }
}

/// A collector that stores log entries in memory.
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
    fn emit(&self, entry: LogEntry) {
        let mut inner = self.entries.lock().unwrap();
        inner.push(entry);
        if inner.len() > self.max_entries {
            inner.remove(0);
        }
    }
}

/// Structured logger with correlation ID support.
///
/// Thread-safe: can be shared across tasks via `Clone`.
#[derive(Clone)]
pub struct StructuredLogger {
    pub correlation_id: String,
    pub target: String,
    sinks: Arc<Vec<Box<dyn LogSink>>>,
}

impl StructuredLogger {
    /// Creates a new structured logger with the given correlation ID and target.
    pub fn new(correlation_id: &str, target: &str) -> Self {
        StructuredLogger {
            correlation_id: correlation_id.to_string(),
            target: target.to_string(),
            sinks: Arc::new(Vec::new()),
        }
    }

    /// Creates a logger that delegates to an existing correlation ID (for child contexts).
    pub fn child(&self, target: &str) -> Self {
        StructuredLogger {
            correlation_id: self.correlation_id.clone(),
            target: target.to_string(),
            sinks: self.sinks.clone(),
        }
    }

    /// Adds a log sink.
    pub fn add_sink(&mut self, sink: Box<dyn LogSink>) {
        Arc::get_mut(&mut self.sinks).unwrap().push(sink);
    }

    /// Logs a trace message.
    pub fn trace(&self, message: &str) {
        self.log(LogLevel::Trace, message);
    }

    /// Logs a debug message.
    pub fn debug(&self, message: &str) {
        self.log(LogLevel::Debug, message);
    }

    /// Logs an info message.
    pub fn info(&self, message: &str) {
        self.log(LogLevel::Info, message);
    }

    /// Logs a warning message.
    pub fn warn(&self, message: &str) {
        self.log(LogLevel::Warn, message);
    }

    /// Logs an error message.
    pub fn error(&self, message: &str) {
        self.log(LogLevel::Error, message);
    }

    /// Core logging function.
    fn log(&self, level: LogLevel, message: &str) {
        let entry = LogEntry {
            level: level.clone(),
            correlation_id: self.correlation_id.clone(),
            target: self.target.clone(),
            message: message.to_string(),
            timestamp: chrono::Local::now().to_rfc3339(),
        };
        for sink in self.sinks.iter() {
            sink.emit(entry.clone());
        }
        // Always emit to stderr for visibility
        eprintln!("{}", entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_levels() {
        assert_eq!(LogLevel::Trace.as_str(), "TRACE");
        assert_eq!(LogLevel::Debug.as_str(), "DEBUG");
        assert_eq!(LogLevel::Info.as_str(), "INFO");
        assert_eq!(LogLevel::Warn.as_str(), "WARN");
        assert_eq!(LogLevel::Error.as_str(), "ERROR");
    }

    #[test]
    fn test_from_str() {
        assert_eq!(LogLevel::from_str("trace"), Some(LogLevel::Trace));
        assert_eq!(LogLevel::from_str("info"), Some(LogLevel::Info));
        assert_eq!(LogLevel::from_str("error"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str("unknown"), None);
    }

    #[test]
    fn test_logger_creation() {
        let logger = StructuredLogger::new("corr-1", "test-target");
        assert_eq!(logger.correlation_id, "corr-1");
        assert_eq!(logger.target, "test-target");
    }

    #[test]
    fn test_child_logger() {
        let parent = StructuredLogger::new("corr-1", "parent");
        let child = parent.child("child-target");
        assert_eq!(child.correlation_id, "corr-1");
        assert_eq!(child.target, "child-target");
    }

    #[test]
    fn test_memory_sink() {
        let sink = MemoryLogSink::new(100);
        let mut logger = StructuredLogger::new("corr-1", "test");
        logger.add_sink(Box::new(sink.clone()));

        logger.info("test message");
        assert_eq!(sink.count(), 1);

        let entries = sink.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, LogLevel::Info);
        assert_eq!(entries[0].message, "test message");
        assert_eq!(entries[0].correlation_id, "corr-1");
    }

    #[test]
    fn test_log_entry_display() {
        let entry = LogEntry {
            level: LogLevel::Error,
            correlation_id: "corr-1".to_string(),
            target: "test".to_string(),
            message: "something failed".to_string(),
            timestamp: "2026-08-05T00:00:00+00:00".to_string(),
        };
        let display = format!("{}", entry);
        assert!(display.contains("ERROR"));
        assert!(display.contains("test"));
        assert!(display.contains("corr-1"));
        assert!(display.contains("something failed"));
    }

    #[test]
    fn test_lru_eviction() {
        let sink = MemoryLogSink::new(5);
        let mut logger = StructuredLogger::new("corr-1", "test");
        logger.add_sink(Box::new(sink.clone()));

        for i in 0..10 {
            logger.info(&format!("message {}", i));
        }
        assert_eq!(sink.count(), 5);
    }

    #[test]
    fn test_all_log_levels() {
        let sink = MemoryLogSink::new(100);
        let mut logger = StructuredLogger::new("corr-1", "test");
        logger.add_sink(Box::new(sink.clone()));

        logger.trace("trace");
        logger.debug("debug");
        logger.info("info");
        logger.warn("warn");
        logger.error("error");

        assert_eq!(sink.count(), 5);
        let entries = sink.entries();
        assert_eq!(entries[0].level, LogLevel::Trace);
        assert_eq!(entries[1].level, LogLevel::Debug);
        assert_eq!(entries[2].level, LogLevel::Info);
        assert_eq!(entries[3].level, LogLevel::Warn);
        assert_eq!(entries[4].level, LogLevel::Error);
    }
}
