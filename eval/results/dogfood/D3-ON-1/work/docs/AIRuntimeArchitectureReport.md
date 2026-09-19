# AI Runtime Architecture Report

## Overview

The AI Runtime is a provider-agnostic abstraction layer that sits between the CodeBro agent and underlying LLM providers. It owns the core abstractions for model requests, responses, routing, streaming, structured output, and tool contracts—without implementing any specific provider.

## Architecture

```
codebro/src/ai_runtime/
├── mod.rs              # Module root, re-exports, AIRuntime wrapper
├── types.rs            # Core types: ModelId, ProviderType, Priority, CostEstimate, HealthStatus, AIRRuntimeError
├── capabilities.rs     # Capability negotiation: Capability enum, CapabilitySet, CapabilityNegotiation
├── request.rs          # ModelRequest, Message, MessageRole, ToolCall, FunctionCall
├── response.rs         # ModelResponse, Choice, ResponseMessage, ResponseUsage, streaming deltas
├── stream.rs           # StreamPipeline, StreamSegment, StreamEvent, StreamingOutput
├── structured_output.rs # StructuredOutputSchema, JsonSchema, StructuredOutputBuilder, Validator
├── tool_contract.rs    # ToolDefinition, FunctionDefinition, ToolSchema, ToolArgument, ToolResult
├── diagnostics.rs      # RuntimeDiagnostics, DiagnosticEvent, DiagnosticLevel
├── router.rs           # RuntimeRouter, ModelCandidate, RoutingConfig, RoutingDecision
└── tests.rs            # 132 comprehensive tests
```

## Core Responsibilities

### AI Runtime Owns
- **ModelRequest**: Provider-agnostic request structure with messages, tools, streaming flags, and parameters
- **ModelResponse**: Provider-agnostic response with choices, usage stats, and tool calls
- **RuntimeRouter**: Provider-agnostic model selection based on capabilities, cost, health, and priority
- **Capability Negotiation**: Matching request requirements against provider capabilities
- **Streaming Pipeline**: Processing stream events into structured output
- **Structured Output**: JSON schema definition, validation, and builder pattern
- **Tool Invocation Contract**: Tool definitions, function calls, and results

### AI Runtime Does NOT Own
- Provider implementations (OpenAI, Anthropic, Ollama, etc.)
- API keys and authentication
- HTTP clients and networking
- Vendor SDKs

## Routing Philosophy

Routing is **capability-first, provider-agnostic**:

1. **Selection Criteria** (in order of priority):
   - Capabilities: Does the model support required features?
   - Health: Is the provider operational?
   - Cost: What's the estimated cost per million tokens?
   - Priority: Request priority level (Low/Normal/High/Critical)
   - Latency: Historical latency measurements
   - Success Rate: Track record of successful completions

2. **Never Route By**:
   - Provider name
   - Vendor lock-in considerations
   - Hardcoded preferences

## Capability System

Supports 8 core capabilities:

| Capability | Description |
|------------|-------------|
| Streaming | Server-Sent Events support |
| StructuredOutput | JSON schema validation |
| ToolCalling | Function/tool calling |
| Vision | Image input processing |
| Reasoning | Chain-of-thought support |
| Embeddings | Text embedding generation |
| Audio | Audio input/output |
| ImageGeneration | Image generation |

## Testing

- **132 tests** covering all modules
- Zero regressions
- Tests for: capabilities, routing, requests, responses, streaming, structured output, tool contracts, diagnostics

## Key Design Decisions

1. **No Provider Dependencies**: Zero imports of provider SDKs
2. **Serde Serialization**: All types serialize/deserialize for logging and persistence
3. **Builder Pattern**: Fluent APIs for request construction
4. **Arc-based Concurrency**: Thread-safe router with read/write locks
5. **Diagnostic Tracking**: Event-based diagnostics for observability

## Integration Points

The AI Runtime integrates with:
- **Agent Layer**: Sends ModelRequest, receives ModelResponse
- **Provider Layer**: Providers implement a future trait (not part of this module)
- **Tool System**: Uses ToolDefinition from tool_contract module
- **Observability**: Emits DiagnosticEvent for tracing

## Future Extensions

The abstraction layer is designed to support:
- Multi-provider failover
- Cost-aware routing
- Capability-based fallback chains
- Streaming with backpressure
- Structured output validation
- Tool calling with schema validation
