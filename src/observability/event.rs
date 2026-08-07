#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Structured events for the observability platform.

use crate::observability::types::*;

pub fn intent_resolved(
    correlation_id: CorrelationId,
    intent_type: &str,
    detected_goal: &str,
    confidence: f64,
) -> Event {
    let mut event = Event::new(
        EventType::IntentResolved,
        correlation_id,
        "intent_engine",
        "Intent resolved",
    );
    event
        .attributes
        .push(Dimension::new("intent_type", intent_type));
    event
        .attributes
        .push(Dimension::new("detected_goal", detected_goal));
    event
}

pub fn recommendation_generated(
    correlation_id: CorrelationId,
    count: usize,
    top_kind: &str,
) -> Event {
    let mut event = Event::new(
        EventType::RecommendationGenerated,
        correlation_id,
        "recommendation_engine",
        "Recommendations generated",
    );
    event
        .attributes
        .push(Dimension::new("count", &count.to_string()));
    event.attributes.push(Dimension::new("top_kind", top_kind));
    event
}

pub fn workflow_created(
    correlation_id: CorrelationId,
    step_count: usize,
    strategy: &str,
    estimated_cost: f64,
) -> Event {
    let mut event = Event::new(
        EventType::WorkflowCreated,
        correlation_id,
        "workflow_engine",
        "Workflow created",
    );
    event
        .attributes
        .push(Dimension::new("step_count", &step_count.to_string()));
    event.attributes.push(Dimension::new("strategy", strategy));
    event.attributes.push(Dimension::new(
        "estimated_cost",
        &format!("{:.2}", estimated_cost),
    ));
    event
}

pub fn validation_completed(
    correlation_id: CorrelationId,
    result: &str,
    issue_count: usize,
    warning_count: usize,
) -> Event {
    let mut event = Event::new(
        EventType::ValidationCompleted,
        correlation_id,
        "adaptive_validation",
        "Validation completed",
    );
    event.attributes.push(Dimension::new("result", result));
    event
        .attributes
        .push(Dimension::new("issue_count", &issue_count.to_string()));
    event
        .attributes
        .push(Dimension::new("warning_count", &warning_count.to_string()));
    event
}

pub fn approval_granted(correlation_id: CorrelationId, workflow_id: &str, approver: &str) -> Event {
    let mut event = Event::new(
        EventType::ApprovalGranted,
        correlation_id,
        "approval_gate",
        "Approval granted",
    );
    event
        .attributes
        .push(Dimension::new("workflow_id", workflow_id));
    event.attributes.push(Dimension::new("approver", approver));
    event
}

pub fn preference_applied(correlation_id: CorrelationId, key: &str, new_value: &str) -> Event {
    let mut event = Event::new(
        EventType::PreferenceApplied,
        correlation_id,
        "preference_engine",
        "Preference applied",
    );
    event.attributes.push(Dimension::new("key", key));
    event
        .attributes
        .push(Dimension::new("new_value", new_value));
    event
}

pub fn pipeline_completed(
    correlation_id: CorrelationId,
    duration_ms: u64,
    status: &str,
    steps_executed: usize,
) -> Event {
    let mut event = Event::new(
        EventType::PipelineCompleted,
        correlation_id,
        "integration_pipeline",
        "Pipeline completed",
    );
    event
        .attributes
        .push(Dimension::new("duration_ms", &duration_ms.to_string()));
    event.attributes.push(Dimension::new("status", status));
    event.attributes.push(Dimension::new(
        "steps_executed",
        &steps_executed.to_string(),
    ));
    event
}

pub fn error_event(
    correlation_id: CorrelationId,
    error_type: &str,
    message: &str,
    recoverable: bool,
) -> Event {
    let mut event = Event::new(
        EventType::Error,
        correlation_id,
        "observability",
        "Error occurred",
    );
    event = event.with_severity(Severity::Error);
    event
        .attributes
        .push(Dimension::new("error_type", error_type));
    event.attributes.push(Dimension::new("message", message));
    event
        .attributes
        .push(Dimension::new("recoverable", &recoverable.to_string()));
    event
}

pub fn tool_executed(
    correlation_id: CorrelationId,
    tool_name: &str,
    success: bool,
    duration_ms: u64,
) -> Event {
    let mut event = Event::new(
        EventType::ToolExecuted,
        correlation_id,
        "dispatcher",
        "Tool executed",
    );
    event.attributes.push(Dimension::new("tool", tool_name));
    event
        .attributes
        .push(Dimension::new("success", &success.to_string()));
    event
        .attributes
        .push(Dimension::new("duration_ms", &duration_ms.to_string()));
    event
}

pub fn provider_called(
    correlation_id: CorrelationId,
    provider: &str,
    model: &str,
    success: bool,
    duration_ms: u64,
) -> Event {
    let mut event = Event::new(
        EventType::ProviderCalled,
        correlation_id,
        "provider_manager",
        "Provider call",
    );
    event.attributes.push(Dimension::new("provider", provider));
    event.attributes.push(Dimension::new("model", model));
    event
        .attributes
        .push(Dimension::new("success", &success.to_string()));
    event
        .attributes
        .push(Dimension::new("duration_ms", &duration_ms.to_string()));
    event
}

pub fn skill_activated(correlation_id: CorrelationId, skill_name: &str) -> Event {
    let mut event = Event::new(
        EventType::SkillActivated,
        correlation_id,
        "agent/skill",
        "Skill activated",
    );
    event.attributes.push(Dimension::new("skill", skill_name));
    event
}

pub fn sub_agent_completed(
    correlation_id: CorrelationId,
    agent_type: &str,
    task_id: &str,
    success: bool,
) -> Event {
    let mut event = Event::new(
        EventType::SubAgentCompleted,
        correlation_id,
        "agent/subagent",
        "Sub-agent completed",
    );
    event
        .attributes
        .push(Dimension::new("agent_type", agent_type));
    event.attributes.push(Dimension::new("task_id", task_id));
    event
        .attributes
        .push(Dimension::new("success", &success.to_string()));
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intent_resolved_event() {
        let corr = CorrelationId::new();
        let event = intent_resolved(corr.clone(), "preference", "change model", 0.95);
        assert_eq!(event.event_type, EventType::IntentResolved);
        assert_eq!(event.correlation_id, corr);
        assert_eq!(event.source, "intent_engine");
        assert_eq!(event.severity, Severity::Info);
    }

    #[test]
    fn test_pipeline_completed_event() {
        let corr = CorrelationId::new();
        let event = pipeline_completed(corr.clone(), 150, "success", 5);
        assert_eq!(event.event_type, EventType::PipelineCompleted);
        assert_eq!(event.source, "integration_pipeline");
    }

    #[test]
    fn test_error_event() {
        let corr = CorrelationId::new();
        let event = error_event(corr.clone(), "TimeoutError", "provider timed out", true);
        assert_eq!(event.event_type, EventType::Error);
        assert_eq!(event.severity, Severity::Error);
    }

    #[test]
    fn test_tool_executed_event() {
        let corr = CorrelationId::new();
        let event = tool_executed(corr.clone(), "edit_file", true, 42);
        assert_eq!(event.event_type, EventType::ToolExecuted);
        assert_eq!(event.attributes.len(), 3);
    }

    #[test]
    fn test_all_builders_produce_valid_events() {
        let corr = CorrelationId::new();

        let _e1 = intent_resolved(corr.clone(), "t", "g", 0.5);
        let _e2 = recommendation_generated(corr.clone(), 3, "general");
        let _e3 = workflow_created(corr.clone(), 2, "sequential", 1.5);
        let _e4 = validation_completed(corr.clone(), "pass", 0, 1);
        let _e5 = approval_granted(corr.clone(), "wf-1", "user");
        let _e6 = preference_applied(corr.clone(), "model", "gpt-4o");
        let _e7 = pipeline_completed(corr.clone(), 100, "ok", 3);
        let _e8 = tool_executed(corr.clone(), "read_file", true, 10);
        let _e9 = provider_called(corr.clone(), "openai", "gpt-4o", true, 500);
        let _e10 = skill_activated(corr.clone(), "rust_skills");
        let _e11 = sub_agent_completed(corr.clone(), "coding", "task-1", true);
        let _e12 = error_event(corr.clone(), "Err", "msg", false);

        for e in [
            _e1, _e2, _e3, _e4, _e5, _e6, _e7, _e8, _e9, _e10, _e11, _e12,
        ] {
            assert!(!e.event_id.is_empty());
            assert!(!e.wall_clock.is_empty());
        }
    }
}
