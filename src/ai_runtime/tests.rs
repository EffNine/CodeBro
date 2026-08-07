use super::capabilities::{Capability, CapabilitySet, CapabilityNegotiation, SupportedCapabilities};
use super::diagnostics::{DiagnosticEvent, DiagnosticLevel, RuntimeDiagnostics, DiagnosticSummary};
use super::request::{Message, MessageRole, ModelRequest, ToolCall, FunctionCall};
use super::response::{
    Choice, FunctionCallDelta, ModelResponse, ResponseDelta, ResponseMessage, ResponseUsage,
    ToolCallDelta,
};
use super::stream::{StreamEvent, StreamPipeline, StreamSegment, StreamingOutput};
use super::structured_output::{
    JsonSchema, StructuredOutputBuilder, StructuredOutputSchema, StructuredOutputValidator,
};
use super::tool_contract::{FunctionDefinition, ToolArgument, ToolDefinition, ToolResult, ToolSchema};
use super::types::{
    AIRRuntimeError, AIRRuntimeResult, CostEstimate, HealthStatus, ModelId, Priority, ProviderType,
};
use super::router::{ModelCandidate, RuntimeRouter, RoutingConfig, RoutingDecision};
use super::AIRRuntime;
use std::str::FromStr;

// =============================================================================
// Capability tests
// =============================================================================

#[test]
fn test_capability_display() {
    assert_eq!(format!("{}", Capability::Streaming), "streaming");
    assert_eq!(format!("{}", Capability::ToolCalling), "tool_calling");
    assert_eq!(format!("{}", Capability::Vision), "vision");
    assert_eq!(format!("{}", Capability::Reasoning), "reasoning");
    assert_eq!(format!("{}", Capability::Embeddings), "embeddings");
    assert_eq!(format!("{}", Capability::Audio), "audio");
    assert_eq!(format!("{}", Capability::ImageGeneration), "image_generation");
    assert_eq!(format!("{}", Capability::StructuredOutput), "structured_output");
}

#[test]
fn test_capability_from_str() {
    assert_eq!(Capability::from_str("streaming").unwrap(), Capability::Streaming);
    assert_eq!(Capability::from_str("tool_calling").unwrap(), Capability::ToolCalling);
    assert_eq!(Capability::from_str("vision").unwrap(), Capability::Vision);
    assert_eq!(Capability::from_str("reasoning").unwrap(), Capability::Reasoning);
    assert_eq!(Capability::from_str("embeddings").unwrap(), Capability::Embeddings);
    assert_eq!(Capability::from_str("audio").unwrap(), Capability::Audio);
    assert_eq!(Capability::from_str("image_generation").unwrap(), Capability::ImageGeneration);
    assert_eq!(Capability::from_str("structured_output").unwrap(), Capability::StructuredOutput);
}

#[test]
fn test_capability_from_str_invalid() {
    assert!(Capability::from_str("nonexistent_capability").is_err());
}

#[test]
fn test_capability_set_empty() {
    let set = CapabilitySet::empty();
    assert!(set.capabilities.is_empty());
    assert!(!set.has(&Capability::Streaming));
}

#[test]
fn test_capability_set_has() {
    let set = CapabilitySet::new(vec![Capability::Streaming, Capability::ToolCalling]);
    assert!(set.has(&Capability::Streaming));
    assert!(set.has(&Capability::ToolCalling));
    assert!(!set.has(&Capability::Vision));
}

#[test]
fn test_capability_set_has_all() {
    let set = CapabilitySet::new(vec![
        Capability::Streaming,
        Capability::ToolCalling,
        Capability::Vision,
    ]);
    assert!(set.has_all(&[Capability::Streaming, Capability::ToolCalling]));
    assert!(!set.has_all(&[Capability::Streaming, Capability::Vision, Capability::Audio]));
}

#[test]
fn test_capability_set_has_any() {
    let set = CapabilitySet::new(vec![Capability::Streaming, Capability::ToolCalling]);
    assert!(set.has_any(&[Capability::ToolCalling, Capability::Vision]));
    assert!(!set.has_any(&[Capability::Vision, Capability::Audio]));
}

#[test]
fn test_capability_set_merge() {
    let mut set1 = CapabilitySet::new(vec![Capability::Streaming]);
    let set2 = CapabilitySet::new(vec![Capability::ToolCalling, Capability::Vision]);
    set1.merge(&set2);
    assert_eq!(set1.capabilities.len(), 3);
    assert!(set1.has(&Capability::Streaming));
    assert!(set1.has(&Capability::ToolCalling));
    assert!(set1.has(&Capability::Vision));
}

#[test]
fn test_capability_set_intersection() {
    let set1 = CapabilitySet::new(vec![Capability::Streaming, Capability::ToolCalling]);
    let set2 = CapabilitySet::new(vec![Capability::ToolCalling, Capability::Vision]);
    let intersection = set1.intersection(&set2);
    assert_eq!(intersection.capabilities.len(), 1);
    assert!(intersection.has(&Capability::ToolCalling));
}

#[test]
fn test_capability_negotiation_compatible() {
    let request = ModelRequest::new("gpt-4o", vec![]);
    let provider_caps = CapabilitySet::new(vec![Capability::Streaming, Capability::ToolCalling]);
    let negotiation = CapabilityNegotiation::new(&request, &provider_caps);
    assert!(negotiation.compatible);
    assert!(negotiation.missing.is_empty());
}

#[test]
fn test_capability_negotiation_incompatible() {
    let request = ModelRequest::new("gpt-4o", vec![])
        .with_stream(true)
        .with_tools(vec![ToolDefinition::new("test", "test", serde_json::json!({}))]);
    let provider_caps = CapabilitySet::new(vec![Capability::Streaming]); // missing ToolCalling
    let negotiation = CapabilityNegotiation::new(&request, &provider_caps);
    assert!(!negotiation.compatible);
    assert!(!negotiation.missing.is_empty());
}

#[test]
fn test_supported_capabilities() {
    let caps = CapabilitySet::new(vec![Capability::Streaming, Capability::ToolCalling]);
    let supported = SupportedCapabilities::new("gpt-4o", "openai", caps);
    assert_eq!(supported.model_id, "gpt-4o");
    assert_eq!(supported.provider_type, "openai");
    assert_eq!(supported.confidence, 1.0);

    let supported = supported.with_confidence(0.95);
    assert_eq!(supported.confidence, 0.95);
}

// =============================================================================
// Request tests
// =============================================================================

#[test]
fn test_message_creation() {
    let sys = Message::system("You are helpful");
    assert_eq!(sys.role, MessageRole::System);
    assert_eq!(sys.content, "You are helpful");

    let user = Message::user("Hello");
    assert_eq!(user.role, MessageRole::User);

    let assistant = Message::assistant("Hi there");
    assert_eq!(assistant.role, MessageRole::Assistant);
}

#[test]
fn test_message_role_from_str() {
    assert_eq!(MessageRole::from_str("system").unwrap(), MessageRole::System);
    assert_eq!(MessageRole::from_str("user").unwrap(), MessageRole::User);
    assert_eq!(MessageRole::from_str("assistant").unwrap(), MessageRole::Assistant);
    assert_eq!(MessageRole::from_str("tool").unwrap(), MessageRole::Tool);
    assert!(MessageRole::from_str("invalid").is_err());
}

#[test]
fn test_model_request_basic() {
    let request = ModelRequest::new("gpt-4o", vec![Message::user("Hello")]);
    assert_eq!(request.model_id.id, "gpt-4o");
    assert_eq!(request.messages.len(), 1);
    assert!(!request.stream);
    assert!(request.tools.is_empty());
}

#[test]
fn test_model_request_builder() {
    let request = ModelRequest::new("gpt-4o", vec![])
        .with_stream(true)
        .with_max_tokens(1024)
        .with_temperature(0.7)
        .with_top_p(0.9)
        .with_stop_sequences(vec!["\n".to_string()])
        .with_reasoning_effort("medium")
        .with_penalty(0.1, 0.2);

    assert!(request.stream);
    assert_eq!(request.max_tokens, Some(1024));
    assert_eq!(request.temperature, Some(0.7));
    assert_eq!(request.top_p, Some(0.9));
    assert_eq!(request.stop_sequences, vec!["\n"]);
    assert_eq!(request.reasoning_effort, Some("medium".to_string()));
    assert_eq!(request.presence_penalty, Some(0.1));
    assert_eq!(request.frequency_penalty, Some(0.2));
}

#[test]
fn test_model_request_required_capabilities() {
    let request = ModelRequest::new("gpt-4o", vec![])
        .with_stream(true);
    let caps = request.required_capabilities();
    assert!(caps.contains(&Capability::Streaming));
}

#[test]
fn test_model_request_to_json_and_from_json() {
    let request = ModelRequest::new("gpt-4o", vec![Message::user("Hello")])
        .with_stream(true)
        .with_max_tokens(512);
    let json = request.to_json().unwrap();
    assert_eq!(json["model_id"]["id"], "gpt-4o");
    assert_eq!(json["stream"], true);
    assert_eq!(json["max_tokens"], 512);

    let request2 = ModelRequest::from_json(&json.to_string()).unwrap();
    assert_eq!(request2.model_id.id, "gpt-4o");
    assert_eq!(request2.stream, true);
}

#[test]
fn test_model_request_display() {
    let request = ModelRequest::new("gpt-4o", vec![Message::user("Hello")]);
    let display = format!("{}", request);
    assert!(display.contains("gpt-4o"));
    assert!(display.contains("messages=1"));
}

// =============================================================================
// Response tests
// =============================================================================

#[test]
fn test_model_response_basic() {
    let response = ModelResponse::new(
        "resp-123",
        "gpt-4o",
        vec![Choice::new(0, ResponseMessage::new("assistant", Some("Hello!".to_string())), None)],
        ResponseUsage::new(10, 5, 15),
        1700000000,
        "openai",
    );
    assert_eq!(response.id, "resp-123");
    assert_eq!(response.content(), Some("Hello!"));
    assert_eq!(response.usage.total_tokens, 15);
}

#[test]
fn test_model_response_tool_calls() {
    let tool_call = ToolCall::new(
        "call_1",
        FunctionCall::new("read_file", serde_json::json!({"path": "src/main.rs"})),
    );
    let response = ModelResponse::new(
        "resp-123",
        "gpt-4o",
        vec![Choice::new(0, ResponseMessage::new("assistant", None).with_tool_calls(vec![tool_call.clone()]), None)],
        ResponseUsage::new(20, 10, 30),
        1700000000,
        "openai",
    );
    assert!(response.has_tool_calls());
    assert_eq!(response.tool_calls().len(), 1);
    assert_eq!(response.tool_calls()[0].function.name, "read_file");
}

#[test]
fn test_model_response_no_tool_calls() {
    let response = ModelResponse::new(
        "resp-123",
        "gpt-4o",
        vec![Choice::new(0, ResponseMessage::new("assistant", Some("Hello".to_string())), None)],
        ResponseUsage::new(10, 5, 15),
        1700000000,
        "openai",
    );
    assert!(!response.has_tool_calls());
}

#[test]
fn test_model_response_to_json() {
    let response = ModelResponse::new(
        "resp-123",
        "gpt-4o",
        vec![Choice::new(0, ResponseMessage::new("assistant", Some("Hi".to_string())), None)],
        ResponseUsage::new(5, 3, 8),
        1700000000,
        "openai",
    );
    let json = response.to_json().unwrap();
    assert_eq!(json["id"], "resp-123");
    assert_eq!(json["model_id"]["id"], "gpt-4o");
}

#[test]
fn test_model_response_from_json() {
    let json = r#"{
        "id": "resp-456",
        "model_id": {"id": "claude-3", "provider": {"Custom": "anthropic"}},
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "Ok", "tool_calls": []}, "finish_reason": null}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15, "cache_read_tokens": null, "cache_creation_tokens": null},
        "created_at": 1700000000,
        "provider_type": "anthropic",
        "raw_response": null
    }"#;
    let response = ModelResponse::from_json(json).unwrap();
    assert_eq!(response.id, "resp-456");
    assert_eq!(response.content(), Some("Ok"));
}

#[test]
fn test_model_response_display() {
    let response = ModelResponse::new(
        "resp-1",
        "gpt-4o",
        vec![Choice::new(0, ResponseMessage::new("assistant", Some("test".to_string())), None)],
        ResponseUsage::new(1, 1, 2),
        0,
        "openai",
    );
    let display = format!("{}", response);
    assert!(display.contains("resp-1"));
    assert!(display.contains("tokens=2"));
}

#[test]
fn test_response_usage_cost_estimation() {
    let usage = ResponseUsage::new(1000, 500, 1500);
    let cost = usage.estimated_cost(10.0);
    assert!((cost - 0.015).abs() < 0.0001);
}

#[test]
fn test_response_delta_serialization() {
    let delta = ResponseDelta {
        role: Some("assistant".to_string()),
        content: Some("Hello".to_string()),
        tool_calls: None,
    };
    let json = serde_json::to_string(&delta).unwrap();
    let parsed: ResponseDelta = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.content, Some("Hello".to_string()));
}

// =============================================================================
// Stream tests
// =============================================================================

#[test]
fn test_stream_segment() {
    let segment = StreamSegment::new(
        ResponseDelta {
            role: None,
            content: Some("Hello".to_string()),
            tool_calls: None,
        },
        0,
    );
    assert_eq!(segment.content_fragment(), Some("Hello"));
    assert!(!segment.is_finished());
}

#[test]
fn test_stream_segment_finish() {
    let segment = StreamSegment::new(ResponseDelta::default(), 0)
        .with_finish_reason("stop");
    assert!(segment.is_finished());
    assert_eq!(segment.finish_reason, Some("stop".to_string()));
}

#[test]
fn test_stream_event_display() {
    let event = StreamEvent::Segment(StreamSegment::new(
        ResponseDelta { content: Some("Hi".to_string()), ..Default::default() }, 0
    ));
    let display = format!("{}", event);
    assert!(display.contains("Segment"));
}

#[test]
fn test_stream_pipeline_basic() {
    let mut pipeline = StreamPipeline::new();
    pipeline.push(StreamEvent::Segment(StreamSegment::new(
        ResponseDelta { content: Some("Hello".to_string()), ..Default::default() }, 0
    )));
    pipeline.push(StreamEvent::Complete { total_tokens: 1, total_duration_ms: 100 });

    let output = pipeline.process().unwrap();
    assert_eq!(output.segments.len(), 1);
    assert_eq!(output.total_tokens, 1);
    assert!(output.finished);
}

#[test]
fn test_stream_pipeline_collect_content() {
    let mut pipeline = StreamPipeline::new();
    pipeline.push(StreamEvent::Segment(StreamSegment::new(
        ResponseDelta { content: Some("Hello".to_string()), ..Default::default() }, 0
    )));
    pipeline.push(StreamEvent::Segment(StreamSegment::new(
        ResponseDelta { content: Some(" World".to_string()), ..Default::default() }, 1
    )));
    pipeline.push(StreamEvent::Complete { total_tokens: 2, total_duration_ms: 200 });

    let output = pipeline.process().unwrap();
    assert_eq!(output.collect_content(), "Hello World");
}

#[test]
fn test_stream_pipeline_error() {
    let mut pipeline = StreamPipeline::new();
    pipeline.push(StreamEvent::Error {
        error: "connection lost".to_string(),
    });

    let result = pipeline.process();
    assert!(result.is_err());
}

#[test]
fn test_stream_pipeline_cancelled() {
    let mut pipeline = StreamPipeline::new();
    pipeline.push(StreamEvent::Segment(StreamSegment::new(
        ResponseDelta { content: Some("Partial".to_string()), ..Default::default() }, 0
    )));
    pipeline.push(StreamEvent::Cancelled);

    let output = pipeline.process().unwrap();
    assert!(output.finished);
    assert_eq!(output.collect_content(), "Partial");
}

#[test]
fn test_stream_pipeline_empty() {
    let mut pipeline = StreamPipeline::new();
    let output = pipeline.process().unwrap();
    assert!(output.finished);
    assert!(output.segments.is_empty());
}

#[test]
fn test_stream_pipeline_push_pop() {
    let mut pipeline = StreamPipeline::new();
    let event = StreamEvent::Segment(StreamSegment::new(ResponseDelta::default(), 0));
    pipeline.push(event.clone());
    assert_eq!(pipeline.pop().unwrap(), event);
    assert!(pipeline.is_empty());
}

#[test]
fn test_stream_pipeline_drain() {
    let mut pipeline = StreamPipeline::new();
    pipeline.push(StreamEvent::Segment(StreamSegment::new(ResponseDelta::default(), 0)));
    pipeline.push(StreamEvent::Segment(StreamSegment::new(ResponseDelta::default(), 1)));
    let events = pipeline.drain();
    assert_eq!(events.len(), 2);
    assert!(pipeline.is_empty());
}

#[test]
fn test_streaming_output_new() {
    let output = StreamingOutput::new();
    assert!(output.segments.is_empty());
    assert!(!output.finished);
}

#[test]
fn test_streaming_output_append() {
    let mut output = StreamingOutput::new();
    output.append(StreamSegment::new(ResponseDelta { content: Some("x".to_string()), ..Default::default() }, 0));
    assert_eq!(output.segments.len(), 1);
    assert_eq!(output.total_tokens, 1);
}

#[test]
fn test_streaming_output_finish() {
    let output = StreamingOutput::new().finish();
    assert!(output.finished);
}

#[test]
fn test_stream_pipeline_peek() {
    let mut pipeline = StreamPipeline::new();
    let event = StreamEvent::Segment(StreamSegment::new(ResponseDelta::default(), 0));
    pipeline.push(event.clone());
    assert_eq!(pipeline.peek().unwrap(), &event);
}

// =============================================================================
// Structured Output tests
// =============================================================================

#[test]
fn test_structured_output_schema_valid() {
    let schema = StructuredOutputSchema {
        name: "Person".to_string(),
        description: "A person".to_string(),
        properties: serde_json::json!({
            "name": {"type": "string"},
            "age": {"type": "integer"}
        }),
        required: vec!["name".to_string()],
        additional_properties: false,
    };
    assert!(schema.is_valid());
}

#[test]
fn test_structured_output_schema_invalid_empty_name() {
    let schema = StructuredOutputSchema {
        name: "".to_string(),
        description: "A person".to_string(),
        properties: serde_json::json!({}),
        required: vec![],
        additional_properties: false,
    };
    assert!(!schema.is_valid());
}

#[test]
fn test_structured_output_validate_valid_json() {
    let schema = StructuredOutputSchema {
        name: "Person".to_string(),
        description: "A person".to_string(),
        properties: serde_json::json!({
            "name": {"type": "string"},
            "age": {"type": "integer"}
        }),
        required: vec!["name".to_string()],
        additional_properties: false,
    };
    let json = serde_json::json!({"name": "Alice", "age": 30});
    let errors = schema.validate_json(&json);
    assert!(errors.is_empty());
}

#[test]
fn test_structured_output_validate_missing_required() {
    let schema = StructuredOutputSchema {
        name: "Person".to_string(),
        description: "A person".to_string(),
        properties: serde_json::json!({
            "name": {"type": "string"},
        }),
        required: vec!["name".to_string(), "age".to_string()],
        additional_properties: false,
    };
    let json = serde_json::json!({"name": "Alice"});
    let errors = schema.validate_json(&json);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("age"));
}

#[test]
fn test_structured_output_validate_not_object() {
    let schema = StructuredOutputSchema {
        name: "Person".to_string(),
        description: "A person".to_string(),
        properties: serde_json::json!({}),
        required: vec![],
        additional_properties: false,
    };
    let json = serde_json::json!("not an object");
    let errors = schema.validate_json(&json);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("object"));
}

#[test]
fn test_json_schema_string() {
    let schema = JsonSchema::string("A name");
    let value = schema.to_value();
    assert_eq!(value["type"], "string");
    assert_eq!(value["description"], "A name");
}

#[test]
fn test_json_schema_number() {
    let schema = JsonSchema::NumberSchema {
        description: "An age".to_string(),
        minimum: Some(0.0),
        maximum: Some(150.0),
    };
    let value = schema.to_value();
    assert_eq!(value["type"], "number");
    assert_eq!(value["minimum"], serde_json::json!(0.0));
    assert_eq!(value["maximum"], serde_json::json!(150.0));
}

#[test]
fn test_json_schema_integer() {
    let schema = JsonSchema::integer("Count");
    let value = schema.to_value();
    assert_eq!(value["type"], "integer");
}

#[test]
fn test_json_schema_boolean() {
    let schema = JsonSchema::boolean("Active");
    let value = schema.to_value();
    assert_eq!(value["type"], "boolean");
}

#[test]
fn test_json_schema_array() {
    let schema = JsonSchema::ArraySchema {
        description: "List of names".to_string(),
        items: Box::new(JsonSchema::string("Name")),
    };
    let value = schema.to_value();
    assert_eq!(value["type"], "array");
    assert_eq!(value["items"]["type"], "string");
}

#[test]
fn test_json_schema_object() {
    let schema = JsonSchema::ObjectSchema {
        description: "A person".to_string(),
        properties: vec![
            ("name".to_string(), JsonSchema::string("Name")),
            ("age".to_string(), JsonSchema::integer("Age")),
        ],
        required: vec!["name".to_string()],
    };
    let value = schema.to_value();
    assert_eq!(value["type"], "object");
    assert!(value["properties"]["name"].is_object());
}

#[test]
fn test_structured_output_builder() {
    let schema = StructuredOutputBuilder::new("Person", "A human being")
        .add_property("name", JsonSchema::string("Full name"))
        .add_property("age", JsonSchema::integer("Age in years"))
        .add_required("name")
        .with_additional_properties(false)
        .build();

    assert_eq!(schema.name, "Person");
    assert!(schema.validate_json(&serde_json::json!({"name": "Alice", "age": 30})).is_empty());
    assert!(!schema.validate_json(&serde_json::json!({"age": 30})).is_empty());
}

#[test]
fn test_structured_output_validator() {
    let validator = StructuredOutputValidator::new();
    let schema = StructuredOutputBuilder::new("Person", "A human")
        .add_property("name", JsonSchema::string("Name"))
        .add_required("name")
        .build();

    assert!(validator.validate_strict(&schema, &serde_json::json!({"name": "Alice"})).is_ok());
    assert!(validator.validate_strict(&schema, &serde_json::json!({})).is_err());
}

// =============================================================================
// Tool Contract tests
// =============================================================================

#[test]
fn test_tool_definition() {
    let tool = ToolDefinition::new("read_file", "Read a file", serde_json::json!({
        "type": "object",
        "properties": {"path": {"type": "string"}}
    }));
    assert_eq!(tool.r#type, "function");
    assert_eq!(tool.function.name, "read_file");
    assert_eq!(tool.function.description, "Read a file");
}

#[test]
fn test_tool_definition_with_strict() {
    let tool = ToolDefinition::new("read_file", "Read a file", serde_json::json!({
        "type": "object"
    })).with_strict(true);
    let params = tool.function.parameters.as_object().unwrap();
    assert_eq!(params.get("strict").unwrap(), true);
}

#[test]
fn test_tool_schema() {
    let schema = ToolSchema::new()
        .add_property("path", &serde_json::json!({"type": "string"}))
        .add_required("path");
    assert_eq!(schema.properties.len(), 1);
    assert_eq!(schema.required, vec!["path"]);
}

#[test]
fn test_tool_argument() {
    let arg = ToolArgument::new("path", serde_json::json!("src/main.rs"));
    assert_eq!(arg.name, "path");
    assert_eq!(arg.value, serde_json::json!("src/main.rs"));
}

#[test]
fn test_tool_result_success() {
    let result = ToolResult::success("file contents");
    assert!(result.is_success());
    assert_eq!(result.content(), Some("file contents"));
    assert!(result.error_msg().is_none());
}

#[test]
fn test_tool_result_error() {
    let result = ToolResult::error("not found");
    assert!(!result.is_success());
    assert!(result.content().is_none());
    assert_eq!(result.error_msg(), Some("not found"));
}

// =============================================================================
// Type tests
// =============================================================================

#[test]
fn test_provider_type_display() {
    assert_eq!(format!("{}", ProviderType::OpenAI), "openai");
    assert_eq!(format!("{}", ProviderType::Anthropic), "anthropic");
    assert_eq!(format!("{}", ProviderType::Ollama), "ollama");
    assert_eq!(format!("{}", ProviderType::Custom("my-provider".to_string())), "my-provider");
}

#[test]
fn test_provider_type_from_str() {
    assert_eq!(ProviderType::from_str("openai"), ProviderType::OpenAI);
    assert_eq!(ProviderType::from_str("anthropic"), ProviderType::Anthropic);
    assert_eq!(ProviderType::from_str("ollama"), ProviderType::Ollama);
    assert_eq!(ProviderType::from_str("my-provider"), ProviderType::Custom("my-provider".to_string()));
}

#[test]
fn test_airuntime_error_display() {
    let err = AIRRuntimeError::NoSuitableProvider("none found".to_string());
    let msg = format!("{}", err);
    assert!(msg.contains("No suitable provider"));
    assert!(msg.contains("none found"));

    let err = AIRRuntimeError::CapabilityMismatch {
        requested: vec!["streaming".to_string()],
        available: vec![],
    };
    let msg = format!("{}", err);
    assert!(msg.contains("Capability mismatch"));
}

#[test]
fn test_airuntime_result_type() {
    let ok: AIRRuntimeResult<String> = Ok("hello".to_string());
    assert_eq!(ok.unwrap(), "hello");

    let err: AIRRuntimeResult<String> = Err(AIRRuntimeError::Generic("fail".to_string()));
    assert!(err.is_err());
}

// =============================================================================
// Diagnostic tests
// =============================================================================

#[test]
fn test_runtime_diagnostics_record() {
    let mut diag = RuntimeDiagnostics::new(10);
    diag.record(DiagnosticEvent::DiagnosticLevel {
        event_id: "e1".to_string(),
        level: DiagnosticLevel::Info,
        message: "test".to_string(),
        timestamp: 1,
    });
    assert_eq!(diag.events().len(), 1);
}

#[test]
fn test_runtime_diagnostics_max_events() {
    let mut diag = RuntimeDiagnostics::new(2);
    diag.record(DiagnosticEvent::DiagnosticLevel {
        event_id: "e1".to_string(),
        level: DiagnosticLevel::Info,
        message: "first".to_string(),
        timestamp: 1,
    });
    diag.record(DiagnosticEvent::DiagnosticLevel {
        event_id: "e2".to_string(),
        level: DiagnosticLevel::Info,
        message: "second".to_string(),
        timestamp: 2,
    });
    diag.record(DiagnosticEvent::DiagnosticLevel {
        event_id: "e3".to_string(),
        level: DiagnosticLevel::Info,
        message: "third".to_string(),
        timestamp: 3,
    });
    assert_eq!(diag.events().len(), 2);
}

#[test]
fn test_runtime_diagnostics_clear() {
    let mut diag = RuntimeDiagnostics::new(10);
    diag.record(DiagnosticEvent::DiagnosticLevel {
        event_id: "e1".to_string(),
        level: DiagnosticLevel::Info,
        message: "test".to_string(),
        timestamp: 1,
    });
    diag.clear();
    assert!(diag.events().is_empty());
}

#[test]
fn test_runtime_diagnostics_summary() {
    let mut diag = RuntimeDiagnostics::new(100);
    diag.record(DiagnosticEvent::DiagnosticLevel {
        event_id: "e1".to_string(),
        level: DiagnosticLevel::Info,
        message: "info".to_string(),
        timestamp: 1,
    });
    diag.record(DiagnosticEvent::DiagnosticLevel {
        event_id: "e2".to_string(),
        level: DiagnosticLevel::Warning,
        message: "warn".to_string(),
        timestamp: 2,
    });
    diag.record(DiagnosticEvent::DiagnosticLevel {
        event_id: "e3".to_string(),
        level: DiagnosticLevel::Error,
        message: "error".to_string(),
        timestamp: 3,
    });
    let summary = diag.summary();
    assert_eq!(summary.total_events, 3);
    assert_eq!(summary.info, 1);
    assert_eq!(summary.warnings, 1);
    assert_eq!(summary.errors, 1);
}

#[test]
fn test_diagnostic_event_display() {
    let event = DiagnosticEvent::DiagnosticLevel {
        event_id: "e1".to_string(),
        level: DiagnosticLevel::Info,
        message: "test message".to_string(),
        timestamp: 1,
    };
    let display = format!("{}", event);
    assert!(display.contains("e1"));
    assert!(display.contains("INFO"));
    assert!(display.contains("test message"));
}

// =============================================================================
// DiagnosticLevel tests
// =============================================================================

#[test]
fn test_diagnostic_level_display() {
    assert_eq!(format!("{}", DiagnosticLevel::Info), "INFO");
    assert_eq!(format!("{}", DiagnosticLevel::Warning), "WARN");
    assert_eq!(format!("{}", DiagnosticLevel::Error), "ERROR");
    assert_eq!(format!("{}", DiagnosticLevel::Debug), "DEBUG");
}

// =============================================================================
// Router integration with capabilities
// =============================================================================

#[test]
fn test_router_routes_with_capability_match() {
    let router = RuntimeRouter::new(RoutingConfig::default());
    let candidate = ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming, Capability::ToolCalling]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    ).with_priority(Priority::High);
    router.register_candidate(candidate);

    let request = ModelRequest::new("gpt-4o", vec![Message::user("Hello")])
        .with_stream(true);
    let decision = router.route(&request).unwrap();
    assert_eq!(decision.selected_model.id, "gpt-4o");
}

#[test]
fn test_router_picks_lower_cost_among_equal() {
    let router = RuntimeRouter::new(RoutingConfig::default());
    let cheap = ModelCandidate::new(
        ModelId::openai("gpt-4o-mini"),
        CapabilitySet::new(vec![Capability::Streaming]),
        HealthStatus::Healthy,
        CostEstimate {
            input_cost_per_million: 0.15,
            output_cost_per_million: 0.60,
            cache_read_cost_per_million: None,
            cache_creation_cost_per_million: None,
        },
    ).with_priority(Priority::Normal);
    let expensive = ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming]),
        HealthStatus::Healthy,
        CostEstimate {
            input_cost_per_million: 2.50,
            output_cost_per_million: 10.0,
            cache_read_cost_per_million: None,
            cache_creation_cost_per_million: None,
        },
    ).with_priority(Priority::Normal);
    router.register_candidate(cheap);
    router.register_candidate(expensive);

    let request = ModelRequest::new("gpt-4o", vec![]);
    let decision = router.route(&request).unwrap();
    assert_eq!(decision.selected_model.id, "gpt-4o-mini");
}

#[test]
fn test_router_skips_unhealthy_candidate() {
    let router = RuntimeRouter::new(RoutingConfig::default());
    let healthy = ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    );
    let unhealthy = ModelCandidate::new(
        ModelId::anthropic("claude-3"),
        CapabilitySet::new(vec![Capability::Streaming]),
        HealthStatus::Unhealthy,
        CostEstimate::default(),
    );
    router.register_candidate(healthy);
    router.register_candidate(unhealthy);

    let request = ModelRequest::new("gpt-4o", vec![]);
    let decision = router.route(&request).unwrap();
    assert_eq!(decision.selected_model.id, "gpt-4o");
}

#[test]
fn test_router_alternatives_in_decision() {
    let router = RuntimeRouter::new(RoutingConfig::default());
    let c1 = ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    ).with_priority(Priority::High);
    let c2 = ModelCandidate::new(
        ModelId::openai("gpt-4o-mini"),
        CapabilitySet::new(vec![Capability::Streaming]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    ).with_priority(Priority::Low);
    router.register_candidate(c1);
    router.register_candidate(c2);

    let request = ModelRequest::new("gpt-4o", vec![]);
    let decision = router.route(&request).unwrap();
    assert_eq!(decision.alternatives.len(), 1);
    assert_eq!(decision.alternatives[0].id, "gpt-4o-mini");
}

// =============================================================================
// ModelRequest edge cases
// =============================================================================

#[test]
fn test_model_request_empty_messages() {
    let request = ModelRequest::new("gpt-4o", vec![]);
    assert_eq!(request.messages.len(), 0);
}

#[test]
fn test_model_request_with_structured_output() {
    let schema = StructuredOutputBuilder::new("Response", "A response")
        .add_property("answer", JsonSchema::string("The answer"))
        .add_required("answer")
        .build();
    let request = ModelRequest::new("gpt-4o", vec![])
        .with_structured_output(schema);
    assert!(request.structured_output.is_some());
    assert_eq!(request.required_capabilities().len(), 1);
    assert!(request.required_capabilities().contains(&Capability::StructuredOutput));
}

#[test]
fn test_model_request_with_tools() {
    let tools = vec![
        ToolDefinition::new("read_file", "Read a file", serde_json::json!({})),
        ToolDefinition::new("write_file", "Write a file", serde_json::json!({})),
    ];
    let request = ModelRequest::new("gpt-4o", vec![])
        .with_tools(tools.clone());
    assert_eq!(request.tools.len(), 2);
    assert!(request.required_capabilities().contains(&Capability::ToolCalling));
}

#[test]
fn test_model_request_function_call_from_str() {
    let fc = FunctionCall::new_with_args("read_file", r#"{"path": "src/main.rs"}"#).unwrap();
    assert_eq!(fc.name, "read_file");
    assert_eq!(fc.arguments["path"], "src/main.rs");
}

#[test]
fn test_model_request_function_call_invalid_json() {
    let result = FunctionCall::new_with_args("read_file", "not json");
    assert!(result.is_err());
}

// =============================================================================
// ToolDefinition and ToolSchema roundtrip
// =============================================================================

#[test]
fn test_tool_definition_roundtrip() {
    let tool = ToolDefinition::new("read_file", "Read a file", serde_json::json!({
        "type": "object",
        "properties": {"path": {"type": "string"}}
    }));
    let json = serde_json::to_string(&tool).unwrap();
    let parsed: ToolDefinition = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.function.name, "read_file");
    assert_eq!(parsed.function.description, "Read a file");
}

#[test]
fn test_tool_schema_roundtrip() {
    let schema = ToolSchema::new()
        .add_property("path", &serde_json::json!({"type": "string"}))
        .add_required("path");
    let json = serde_json::to_string(&schema).unwrap();
    let parsed: ToolSchema = serde_json::from_str(&json).unwrap();
    assert_eq!(schema.properties.len(), 1);
    assert_eq!(parsed.required, vec!["path"]);
}

// =============================================================================
// StreamingOutput tests
// =============================================================================

#[test]
fn test_streaming_output_default() {
    let output = StreamingOutput::default();
    assert!(!output.finished);
    assert_eq!(output.total_tokens, 0);
}

#[test]
fn test_streaming_output_multiple_segments() {
    let mut output = StreamingOutput::new();
    output.append(StreamSegment::new(ResponseDelta { content: Some("a".to_string()), ..Default::default() }, 0));
    output.append(StreamSegment::new(ResponseDelta { content: Some("b".to_string()), ..Default::default() }, 1));
    output.append(StreamSegment::new(ResponseDelta { content: Some("c".to_string()), ..Default::default() }, 2));
    assert_eq!(output.total_tokens, 3);
    assert_eq!(output.collect_content(), "abc");
}

// =============================================================================
// Message edge cases
// =============================================================================

#[test]
fn test_message_tool_result() {
    let msg = Message::tool("result content", "call_123");
    assert_eq!(msg.role, MessageRole::Tool);
    assert_eq!(msg.content, "result content");
    let json = serde_json::to_value(&msg).unwrap();
    // Tool message serializes with capitalized variant
    assert!(json["role"].is_string());
}

#[test]
fn test_choice_creation() {
    let choice = Choice::new(0, ResponseMessage::new("assistant", Some("Hi".to_string())), Some("stop".to_string()));
    assert_eq!(choice.index, 0);
    assert_eq!(choice.finish_reason, Some("stop".to_string()));
}

// =============================================================================
// FunctionCallDelta and ToolCallDelta tests
// =============================================================================

#[test]
fn test_tool_call_delta() {
    let delta = ToolCallDelta {
        index: 0,
        id: Some("call_1".to_string()),
        r#type: Some("function".to_string()),
        function: Some(FunctionCallDelta {
            name: Some("read_file".to_string()),
            arguments: Some(r#"{"path":"x"}"#.to_string()),
        }),
    };
    assert_eq!(delta.index, 0);
    assert_eq!(delta.function.as_ref().unwrap().name, Some("read_file".to_string()));
}

// =============================================================================
// RoutingConfig defaults
// =============================================================================

#[test]
fn test_routing_config_defaults() {
    let config = RoutingConfig::default();
    assert_eq!(config.max_candidates, 5);
    assert_eq!(config.cost_weight, 0.25);
    assert_eq!(config.latency_weight, 0.25);
    assert_eq!(config.quality_weight, 0.3);
    assert_eq!(config.health_weight, 0.2);
    assert!(config.failover_enabled);
    assert!(config.cache_enabled);
}

// =============================================================================
// AIRRuntime wrapper
// =============================================================================

#[test]
fn test_airuntime_creation() {
    let runtime = AIRRuntime::new(RoutingConfig::default());
    assert_eq!(runtime.router().candidates().len(), 0);
}

#[test]
fn test_airuntime_debug() {
    let runtime = AIRRuntime::new(RoutingConfig::default());
    let debug = format!("{:?}", runtime);
    assert!(debug.contains("AIRRuntime"));
}

// =============================================================================
// Router candidate management
// =============================================================================

#[test]
fn test_router_registers_candidate() {
    let router = RuntimeRouter::new(RoutingConfig::default());
    let candidate = ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming, Capability::ToolCalling]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    );
    router.register_candidate(candidate);
    assert_eq!(router.candidates().len(), 1);
}

#[test]
fn test_router_replaces_same_model() {
    let router = RuntimeRouter::new(RoutingConfig::default());
    let c1 = ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    );
    let c2 = ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming, Capability::ToolCalling]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    );
    router.register_candidate(c1);
    router.register_candidate(c2);
    assert_eq!(router.candidates().len(), 1);
    assert_eq!(router.candidates()[0].capabilities.capabilities.len(), 2);
}

#[test]
fn test_router_unregisters_candidate() {
    let router = RuntimeRouter::new(RoutingConfig::default());
    let candidate = ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    );
    router.register_candidate(candidate);
    router.unregister_candidate(&ModelId::openai("gpt-4o"));
    assert_eq!(router.candidates().len(), 0);
}

#[test]
fn test_router_health_update() {
    let router = RuntimeRouter::new(RoutingConfig::default());
    let candidate = ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    );
    router.register_candidate(candidate);
    router.update_health(&ModelId::openai("gpt-4o"), HealthStatus::Unhealthy);
    assert_eq!(router.candidates()[0].health, HealthStatus::Unhealthy);
}

#[test]
fn test_router_no_candidates() {
    let router = RuntimeRouter::new(RoutingConfig::default());
    let request = ModelRequest::new("gpt-4o", vec![]);
    let result = router.route(&request);
    assert!(result.is_err());
    match result.unwrap_err() {
        AIRRuntimeError::NoSuitableProvider(_) => {},
        e => panic!("Expected NoSuitableProvider, got: {:?}", e),
    }
}

#[test]
fn test_router_selects_best_candidate() {
    let router = RuntimeRouter::new(RoutingConfig::default());
    let low_priority = ModelCandidate::new(
        ModelId::openai("gpt-4o-mini"),
        CapabilitySet::new(vec![Capability::Streaming]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    ).with_priority(Priority::Low);
    let high_priority = ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming, Capability::ToolCalling]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    ).with_priority(Priority::High);
    router.register_candidate(low_priority);
    router.register_candidate(high_priority);

    let request = ModelRequest::new("gpt-4o", vec![]);
    let decision = router.route(&request).unwrap();
    assert_eq!(decision.selected_model.id, "gpt-4o");
}

#[test]
fn test_router_diagnostic_events() {
    let router = RuntimeRouter::new(RoutingConfig::default());
    let candidate = ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    );
    router.register_candidate(candidate);

    let request = ModelRequest::new("gpt-4o", vec![]);
    router.route(&request).unwrap();

    let diag = router.diagnostics();
    assert!(diag.summary().total_events > 0);
}

#[test]
fn test_router_request_history() {
    let router = RuntimeRouter::new(RoutingConfig::default());
    let candidate = ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    );
    router.register_candidate(candidate);

    let request = ModelRequest::new("gpt-4o", vec![]);
    router.route(&request).unwrap();
    router.route(&request).unwrap();

    assert_eq!(router.request_history().len(), 2);
}

#[test]
fn test_router_clear_history() {
    let router = RuntimeRouter::new(RoutingConfig::default());
    let candidate = ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    );
    router.register_candidate(candidate);

    let request = ModelRequest::new("gpt-4o", vec![]);
    router.route(&request).unwrap();
    router.clear_history();
    assert_eq!(router.request_history().len(), 0);
}

#[test]
fn test_model_candidate_score_unhealthy() {
    let candidate = ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming]),
        HealthStatus::Unhealthy,
        CostEstimate::default(),
    );
    let request = ModelRequest::new("gpt-4o", vec![]);
    let score = candidate.score_for_request(&request);
    assert!(score < 0.0);
}

#[test]
fn test_capability_set_new_with_iter() {
    let set = CapabilitySet::new(vec![Capability::Streaming]);
    assert!(set.has(&Capability::Streaming));
}

#[test]
fn test_message_role_serialization() {
    let role = MessageRole::User;
    let json = serde_json::to_string(&role).unwrap();
    // MessageRole serializes with capitalized variant name
    assert!(json.contains("User") || json.contains("user"));
    let parsed: MessageRole = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, MessageRole::User);
}

#[test]
fn test_response_message_tool_calls_empty() {
    let msg = ResponseMessage::new("assistant", Some("Hi".to_string()));
    assert!(!msg.is_tool_call());
}

#[test]
fn test_tool_result_display() {
    let success = ToolResult::success("ok");
    let error = ToolResult::error("fail");
    assert!(format!("{}", success).contains("Success"));
    assert!(format!("{}", error).contains("Error"));
}

#[test]
fn test_json_schema_enum() {
    let schema = JsonSchema::StringSchema {
        description: "Status".to_string(),
        pattern: None,
        enum_values: Some(vec!["active".to_string(), "inactive".to_string()]),
    };
    let value = schema.to_value();
    let enums = value["enum"].as_array().unwrap();
    assert_eq!(enums.len(), 2);
    assert_eq!(enums[0], "active");
}

#[test]
fn test_json_schema_with_pattern() {
    let schema = JsonSchema::StringSchema {
        description: "Email".to_string(),
        pattern: Some(r"^[^@]+@[^@]+$".to_string()),
        enum_values: None,
    };
    let value = schema.to_value();
    assert_eq!(value["pattern"], r"^[^@]+@[^@]+$");
}

#[test]
fn test_structured_output_builder_additional_properties() {
    let schema = StructuredOutputBuilder::new("Person", "A human")
        .add_property("name", JsonSchema::string("Name"))
        .with_additional_properties(true)
        .build();
    assert!(schema.additional_properties);
}

#[test]
fn test_routing_decision_serialization() {
    let decision = RoutingDecision {
        selected_model: ModelId::openai("gpt-4o"),
        selected_candidate: ModelCandidate::new(
            ModelId::openai("gpt-4o"),
            CapabilitySet::new(vec![Capability::Streaming]),
            HealthStatus::Healthy,
            CostEstimate::default(),
        ),
        score: 85.0,
        reason: "best score".to_string(),
        alternatives: vec![],
        capability_negotiation: CapabilityNegotiation {
            request_capabilities: vec![],
            provider_capabilities: vec![Capability::Streaming],
            negotiated: vec![],
            missing: vec![],
            compatible: true,
        },
    };
    let json = serde_json::to_string(&decision).unwrap();
    let parsed: RoutingDecision = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.selected_model.id, "gpt-4o");
}

#[test]
fn test_cost_estimate_with_cache() {
    let estimate = CostEstimate {
        input_cost_per_million: 2.5,
        output_cost_per_million: 10.0,
        cache_read_cost_per_million: Some(0.5),
        cache_creation_cost_per_million: Some(0.1),
    };
    let cost = estimate.estimate(1000000, 1000000, Some(500000));
    assert!(cost > 0.0);
    // Cache read should reduce cost
    let cost_no_cache = estimate.estimate(1000000, 1000000, None);
    assert!(cost < cost_no_cache);
}

#[test]
fn test_model_id_from_provider() {
    let id = ModelId::new("gpt-4o", ProviderType::OpenAI);
    assert_eq!(id.id, "gpt-4o");
    assert_eq!(id.provider, ProviderType::OpenAI);
}

#[test]
fn test_model_id_as_ref() {
    let id = ModelId::openai("gpt-4o");
    assert_eq!(id.as_ref(), "gpt-4o");
}

#[test]
fn test_stream_segment_is_finished_false_by_default() {
    let segment = StreamSegment::new(ResponseDelta::default(), 0);
    assert!(!segment.is_finished());
}

#[test]
fn test_stream_event_is_methods() {
    let seg = StreamEvent::Segment(StreamSegment::new(ResponseDelta::default(), 0));
    let comp = StreamEvent::Complete { total_tokens: 1, total_duration_ms: 100 };
    let err = StreamEvent::Error { error: "test".to_string() };
    let cancel = StreamEvent::Cancelled;

    assert!(seg.is_segment());
    assert!(!seg.is_complete());
    assert!(!seg.is_error());
    assert_eq!(seg.as_segment().is_some(), true);

    assert!(!comp.is_segment());
    assert!(comp.is_complete());

    assert!(!err.is_segment());
    assert!(err.is_error());
}

#[test]
fn test_tool_definition_default_type() {
    let tool = ToolDefinition::new("test", "test desc", serde_json::json!({}));
    assert_eq!(tool.r#type, "function");
}

#[test]
fn test_diagnostic_summary_all_zero() {
    let diag = RuntimeDiagnostics::default();
    let summary = diag.summary();
    assert_eq!(summary.total_events, 0);
    assert_eq!(summary.info, 0);
    assert_eq!(summary.warnings, 0);
    assert_eq!(summary.errors, 0);
}

#[test]
fn test_capability_description() {
    assert_eq!(Capability::Streaming.description(), "Streaming responses via Server-Sent Events");
    assert_eq!(Capability::ToolCalling.description(), "Tool/function calling capabilities");
    assert_eq!(Capability::Vision.description(), "Vision input — ability to process images");
}
