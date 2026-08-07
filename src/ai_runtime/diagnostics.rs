use serde::{Deserialize, Serialize};
use std::fmt;

/// Diagnostic events produced during runtime operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticEvent {
    /// Router selected a model for a request
    ModelSelected {
        event_id: String,
        model_id: String,
        reason: String,
        timestamp: u64,
    },
    /// Capability negotiation failed for a model
    CapabilityNegotiationFailed {
        event_id: String,
        model_id: String,
        missing_capabilities: Vec<String>,
        timestamp: u64,
    },
    /// Streaming pipeline started
    StreamingPipelineStarted {
        event_id: String,
        model_id: String,
        timestamp: u64,
    },
    /// Streaming pipeline stopped
    StreamingPipelineStopped {
        event_id: String,
        model_id: String,
        tokens_emitted: usize,
        timestamp: u64,
    },
    /// Structured output validation failed
    StructuredOutputValidationFailed {
        event_id: String,
        model_id: String,
        errors: Vec<String>,
        timestamp: u64,
    },
    /// Tool call was created
    ToolCallCreated {
        event_id: String,
        tool_name: String,
        timestamp: u64,
    },
    /// Generic diagnostic event
    DiagnosticLevel {
        event_id: String,
        level: DiagnosticLevel,
        message: String,
        timestamp: u64,
    },
}

impl fmt::Display for DiagnosticEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiagnosticEvent::ModelSelected { event_id, model_id, reason, .. } => {
                write!(f, "[{}] Router selected model {}: {}", event_id, model_id, reason)
            }
            DiagnosticEvent::CapabilityNegotiationFailed { event_id, model_id, missing_capabilities, .. } => {
                write!(f, "[{}] Capability negotiation failed for {}: missing {:?}", event_id, model_id, missing_capabilities)
            }
            DiagnosticEvent::StreamingPipelineStarted { event_id, model_id, .. } => {
                write!(f, "[{}] Streaming pipeline started for {}", event_id, model_id)
            }
            DiagnosticEvent::StreamingPipelineStopped { event_id, model_id, tokens_emitted, .. } => {
                write!(f, "[{}] Streaming pipeline stopped for {} (tokens: {})", event_id, model_id, tokens_emitted)
            }
            DiagnosticEvent::StructuredOutputValidationFailed { event_id, model_id, errors, .. } => {
                write!(f, "[{}] Structured output validation failed for {}: {:?}", event_id, model_id, errors)
            }
            DiagnosticEvent::ToolCallCreated { event_id, tool_name, .. } => {
                write!(f, "[{}] Tool call created: {}", event_id, tool_name)
            }
            DiagnosticEvent::DiagnosticLevel { event_id, level, message, .. } => {
                write!(f, "[{}] [{}] {}", event_id, level, message)
            }
        }
    }
}

/// Severity level for diagnostic events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticLevel {
    Info,
    Warning,
    Error,
    Debug,
}

impl fmt::Display for DiagnosticLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiagnosticLevel::Info => write!(f, "INFO"),
            DiagnosticLevel::Warning => write!(f, "WARN"),
            DiagnosticLevel::Error => write!(f, "ERROR"),
            DiagnosticLevel::Debug => write!(f, "DEBUG"),
        }
    }
}

/// Runtime diagnostics collector that tracks events over time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeDiagnostics {
    events: Vec<DiagnosticEvent>,
    max_events: usize,
}

impl RuntimeDiagnostics {
    pub fn new(max_events: usize) -> Self {
        RuntimeDiagnostics {
            events: Vec::new(),
            max_events,
        }
    }

    pub fn record(&mut self, event: DiagnosticEvent) {
        if self.events.len() >= self.max_events {
            self.events.remove(0);
        }
        self.events.push(event);
    }

    pub fn events(&self) -> &[DiagnosticEvent] {
        &self.events
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn summary(&self) -> DiagnosticSummary {
        let info_count = self.events.iter().filter(|e| matches!(e, DiagnosticEvent::DiagnosticLevel { level: DiagnosticLevel::Info, .. })).count();
        let warn_count = self.events.iter().filter(|e| matches!(e, DiagnosticEvent::DiagnosticLevel { level: DiagnosticLevel::Warning, .. })).count();
        let error_count = self.events.iter().filter(|e| matches!(e, DiagnosticEvent::DiagnosticLevel { level: DiagnosticLevel::Error, .. })).count();
        DiagnosticSummary {
            total_events: self.events.len(),
            info: info_count,
            warnings: warn_count,
            errors: error_count,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiagnosticSummary {
    pub total_events: usize,
    pub info: usize,
    pub warnings: usize,
    pub errors: usize,
}
