# Capability Negotiation Report

## Overview

Capability negotiation ensures that model requests are only routed to providers that support the required features. This prevents runtime errors from missing capabilities.

## Capability Enum

```rust
pub enum Capability {
    Streaming,       // Server-Sent Events support
    StructuredOutput, // JSON schema validation
    ToolCalling,     // Function/tool calling
    Vision,          // Image input processing
    Reasoning,       // Chain-of-thought support
    Embeddings,      // Text embedding generation
    Audio,           // Audio input/output
    ImageGeneration, // Image generation
}
```

## CapabilitySet

A set of capabilities supported by a model or provider:

```rust
pub struct CapabilitySet {
    pub capabilities: HashSet<Capability>,
}
```

### Key Methods

| Method | Description |
|--------|-------------|
| `new(caps)` | Create from iterator |
| `empty()` | Create empty set |
| `has(cap)` | Check if capability exists |
| `has_all(req)` | Check if all required capabilities exist |
| `has_any(req)` | Check if any required capability exists |
| `merge(other)` | Union with another set |
| `intersection(other)` | Intersection with another set |

## CapabilityNegotiation

Result of negotiating a request against a provider:

```rust
pub struct CapabilityNegotiation {
    pub request_capabilities: Vec<Capability>,
    pub provider_capabilities: Vec<Capability>,
    pub negotiated: Vec<Capability>,
    pub missing: Vec<Capability>,
    pub compatible: bool,
}
```

### Fields

- `request_capabilities`: What the request needs
- `provider_capabilities`: What the provider offers
- `negotiated`: Intersection of request and provider
- `missing`: Requested but not available
- `compatible`: True if all requirements are met

## Required Capabilities Detection

The runtime automatically detects required capabilities from ModelRequest:

```rust
pub fn required_capabilities(&self) -> Vec<Capability> {
    let mut caps = Vec::new();
    if self.stream {
        caps.push(Capability::Streaming);
    }
    if self.structured_output.is_some() {
        caps.push(Capability::StructuredOutput);
    }
    if !self.tools.is_empty() {
        caps.push(Capability::ToolCalling);
    }
    if self.reasoning_effort.is_some() {
        caps.push(Capability::Reasoning);
    }
    caps
}
```

## Negotiation Flow

1. **Extract Requirements**: Parse ModelRequest for required capabilities
2. **Check Provider**: Compare against provider's CapabilitySet
3. **Compute Intersection**: Find negotiated capabilities
4. **Identify Gaps**: Find missing capabilities
5. **Determine Compatibility**: All required must be present

## Example: Streaming Request

```rust
let request = ModelRequest::new("gpt-4o", vec![])
    .with_stream(true);

let provider_caps = CapabilitySet::new(vec![
    Capability::Streaming,
    Capability::ToolCalling,
]);

let negotiation = CapabilityNegotiation::new(&request, &provider_caps);
// negotiation.compatible = true
// negotiation.missing = []
```

## Example: Tool Calling Request

```rust
let request = ModelRequest::new("gpt-4o", vec![])
    .with_stream(true)
    .with_tools(vec![ToolDefinition::new("read_file", "...", ...)]);

let provider_caps = CapabilitySet::new(vec![Capability::Streaming]); // Missing ToolCalling

let negotiation = CapabilityNegotiation::new(&request, &provider_caps);
// negotiation.compatible = false
// negotiation.missing = [ToolCalling]
```

## SupportedCapabilities

Metadata about a model's capabilities:

```rust
pub struct SupportedCapabilities {
    pub model_id: String,
    pub provider_type: String,
    pub capabilities: CapabilitySet,
    pub confidence: f64,  // 0.0 to 1.0
}
```

Used for:
- Provider discovery
- Capability caching
- Confidence-based selection

## Test Coverage

11 capability tests covering:
- Display and parsing
- Set operations (has, has_all, has_any, merge, intersection)
- Negotiation compatibility
- Required capability detection
- SupportedCapabilities construction
