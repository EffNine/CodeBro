#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Core types for the observability platform.
//!
//! All types are immutable, serializable, and deterministic.
//! No wall-clock time is embedded in business-logic types;
//! timestamps are observational-only.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

// =========================================================================
// Event Types
// =========================================================================

/// Categories of observability events emitted by the platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    IntentResolved,
    RecommendationGenerated,
    WorkflowCreated,
    ValidationCompleted,
    ApprovalGranted,
    PreferenceApplied,
    PipelineCompleted,
    ToolExecuted,
    ProviderCalled,
    SkillActivated,
    SubAgentCompleted,
    Error,
    Custom(String),
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventType::IntentResolved => write!(f, "intent_resolved"),
            EventType::RecommendationGenerated => write!(f, "recommendation_generated"),
            EventType::WorkflowCreated => write!(f, "workflow_created"),
            EventType::ValidationCompleted => write!(f, "validation_completed"),
            EventType::ApprovalGranted => write!(f, "approval_granted"),
            EventType::PreferenceApplied => write!(f, "preference_applied"),
            EventType::PipelineCompleted => write!(f, "pipeline_completed"),
            EventType::ToolExecuted => write!(f, "tool_executed"),
            EventType::ProviderCalled => write!(f, "provider_called"),
            EventType::SkillActivated => write!(f, "skill_activated"),
            EventType::SubAgentCompleted => write!(f, "sub_agent_completed"),
            EventType::Error => write!(f, "error"),
            EventType::Custom(s) => write!(f, "{s}"),
        }
    }
}

// =========================================================================
// Pipeline Stage
// =========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipelineStage {
    Classification,
    AmbiguityDetection,
    ConfidenceEstimation,
    Recommendation,
    WorkflowPlanning,
    Validation,
    Approval,
    Execution,
    Reflection,
    Custom(String),
}

impl fmt::Display for PipelineStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PipelineStage::Classification => write!(f, "classification"),
            PipelineStage::AmbiguityDetection => write!(f, "ambiguity_detection"),
            PipelineStage::ConfidenceEstimation => write!(f, "confidence_estimation"),
            PipelineStage::Recommendation => write!(f, "recommendation"),
            PipelineStage::WorkflowPlanning => write!(f, "workflow_planning"),
            PipelineStage::Validation => write!(f, "validation"),
            PipelineStage::Approval => write!(f, "approval"),
            PipelineStage::Execution => write!(f, "execution"),
            PipelineStage::Reflection => write!(f, "reflection"),
            PipelineStage::Custom(s) => write!(f, "{s}"),
        }
    }
}

// =========================================================================
// Metric Types
// =========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MetricName {
    PipelineLatency,
    ModuleLatency,
    ValidationFailures,
    RecommendationCount,
    WorkflowSize,
    ApprovalRate,
    ErrorCount,
    ThreadUtilization,
    TokenCount,
    CostUsd,
    Custom(String),
}

impl fmt::Display for MetricName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetricName::PipelineLatency => write!(f, "pipeline.latency"),
            MetricName::ModuleLatency => write!(f, "module.latency"),
            MetricName::ValidationFailures => write!(f, "validation.failures"),
            MetricName::RecommendationCount => write!(f, "recommendation.count"),
            MetricName::WorkflowSize => write!(f, "workflow.size"),
            MetricName::ApprovalRate => write!(f, "approval.rate"),
            MetricName::ErrorCount => write!(f, "error.count"),
            MetricName::ThreadUtilization => write!(f, "thread.utilization"),
            MetricName::TokenCount => write!(f, "token.count"),
            MetricName::CostUsd => write!(f, "cost.usd"),
            MetricName::Custom(s) => write!(f, "{s}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MetricValue {
    Counter(u64),
    Gauge(f64),
    Histogram(f64),
}

impl MetricValue {
    pub fn as_f64(&self) -> f64 {
        match self {
            MetricValue::Counter(v) => *v as f64,
            MetricValue::Gauge(v) => *v,
            MetricValue::Histogram(v) => *v,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricKind {
    Increment,
    Decrement,
    Set,
    Measure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricUnit {
    Count,
    Seconds,
    Millis,
    Bytes,
    USD,
    Percent,
    Custom(String),
}

impl fmt::Display for MetricUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetricUnit::Count => write!(f, "count"),
            MetricUnit::Seconds => write!(f, "s"),
            MetricUnit::Millis => write!(f, "ms"),
            MetricUnit::Bytes => write!(f, "B"),
            MetricUnit::USD => write!(f, "$"),
            MetricUnit::Percent => write!(f, "%"),
            MetricUnit::Custom(s) => write!(f, "{s}"),
        }
    }
}

// =========================================================================
// Tracing Types
// =========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TraceId(pub String);

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TraceId {
    pub fn new() -> Self {
        TraceId(uuid::Uuid::new_v4().to_string())
    }
}

impl Default for TraceId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpanId(pub String);

impl fmt::Display for SpanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl SpanId {
    pub fn new() -> Self {
        SpanId(uuid::Uuid::new_v4().to_string())
    }
}

impl Default for SpanId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TracePhase {
    Start,
    Active,
    End,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub phase: TracePhase,
    pub event_type: EventType,
    pub description: String,
    pub attributes: Vec<(String, String)>,
}

// =========================================================================
// Correlation
// =========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CorrelationId(pub String);

impl fmt::Display for CorrelationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl CorrelationId {
    pub fn new() -> Self {
        CorrelationId(uuid::Uuid::new_v4().to_string())
    }
}

impl Default for CorrelationId {
    fn default() -> Self {
        Self::new()
    }
}

// =========================================================================
// Severity
// =========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Debug => write!(f, "DEBUG"),
            Severity::Info => write!(f, "INFO"),
            Severity::Warn => write!(f, "WARN"),
            Severity::Error => write!(f, "ERROR"),
            Severity::Fatal => write!(f, "FATAL"),
        }
    }
}

// =========================================================================
// Dimensions
// =========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dimension {
    pub key: String,
    pub value: String,
}

impl Dimension {
    pub fn new(key: &str, value: &str) -> Self {
        Dimension {
            key: key.to_string(),
            value: value.to_string(),
        }
    }
}

// =========================================================================
// Event
// =========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub event_id: String,
    pub event_type: EventType,
    pub correlation_id: CorrelationId,
    pub trace_id: Option<TraceId>,
    pub span_id: Option<SpanId>,
    pub monotonic_timestamp: Duration,
    pub wall_clock: String,
    pub severity: Severity,
    pub source: String,
    pub description: String,
    pub attributes: Vec<Dimension>,
}

impl Event {
    pub fn new(
        event_type: EventType,
        correlation_id: CorrelationId,
        source: &str,
        description: &str,
    ) -> Self {
        Event {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_type,
            correlation_id,
            trace_id: None,
            span_id: None,
            monotonic_timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or(Duration::ZERO),
            wall_clock: chrono::Local::now().to_rfc3339(),
            severity: Severity::Info,
            source: source.to_string(),
            description: description.to_string(),
            attributes: Vec::new(),
        }
    }

    pub fn with_trace(mut self, trace_id: TraceId, span_id: SpanId) -> Self {
        self.trace_id = Some(trace_id);
        self.span_id = Some(span_id);
        self
    }

    pub fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_attribute(mut self, key: &str, value: &str) -> Self {
        self.attributes.push(Dimension::new(key, value));
        self
    }

    pub fn with_attributes(mut self, attrs: Vec<(&str, &str)>) -> Self {
        for (k, v) in attrs {
            self.attributes.push(Dimension::new(k, v));
        }
        self
    }
}

// =========================================================================
// Event Payload
// =========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventPayload {
    IntentResolved {
        intent_type: String,
        detected_goal: String,
        confidence: f64,
    },
    RecommendationGenerated {
        count: usize,
        top_kind: String,
    },
    WorkflowCreated {
        step_count: usize,
        strategy: String,
        estimated_cost: f64,
    },
    ValidationCompleted {
        result: String,
        issue_count: usize,
        warning_count: usize,
    },
    ApprovalGranted {
        workflow_id: String,
        approver: String,
    },
    PreferenceApplied {
        key: String,
        new_value: String,
    },
    PipelineCompleted {
        duration_ms: u64,
        status: String,
        steps_executed: usize,
    },
    Error {
        error_type: String,
        message: String,
        recoverable: bool,
    },
    None,
}

impl fmt::Display for EventPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventPayload::IntentResolved {
                intent_type,
                detected_goal,
                confidence,
            } => write!(
                f,
                "IntentResolved(type={intent_type}, goal={detected_goal}, conf={confidence:.2})"
            ),
            EventPayload::RecommendationGenerated { count, top_kind } => {
                write!(f, "RecommendationGenerated(count={count}, top={top_kind})")
            }
            EventPayload::WorkflowCreated {
                step_count,
                strategy,
                estimated_cost,
            } => write!(
                f,
                "WorkflowCreated(steps={step_count}, strategy={strategy}, cost={estimated_cost:.2})"
            ),
            EventPayload::ValidationCompleted {
                result,
                issue_count,
                warning_count,
            } => write!(
                f,
                "ValidationCompleted(result={result}, issues={issue_count}, warnings={warning_count})"
            ),
            EventPayload::ApprovalGranted {
                workflow_id,
                approver,
            } => write!(f, "ApprovalGranted(workflow={workflow_id}, approver={approver})"),
            EventPayload::PreferenceApplied { key, new_value } => {
                write!(f, "PreferenceApplied(key={key}, value={new_value})")
            }
            EventPayload::PipelineCompleted {
                duration_ms,
                status,
                steps_executed,
            } => write!(
                f,
                "PipelineCompleted(duration={duration_ms}ms, status={status}, steps={steps_executed})"
            ),
            EventPayload::Error {
                error_type,
                message,
                recoverable,
            } => write!(
                f,
                "Error(type={error_type}, msg={message}, recoverable={recoverable})"
            ),
            EventPayload::None => write!(f, "None"),
        }
    }
}
