# Capability Matching Report

**Phase**: P10.3 — Provider Runtime Foundation

## Purpose

Capability negotiation must be **independent of provider identity**. A
request names the capabilities it needs; a provider either supports them
or not. The future must be additive.

## Capability Set

```rust
pub enum Capability {
    Streaming,          // streaming responses
    StructuredOutput,   // schema-validated JSON
    ToolCalling,        // tool / function calling
    Vision,             // image input
    Embeddings,         // text embeddings
    Reasoning,          // chain-of-thought
    Audio,              // audio input/output
    ImageGeneration,    // image generation
    LongContext,        // large context windows
    JsonMode,           // JSON-only mode
}
```

All capabilities serialize to stable codes (`streaming`, `structured_output`,
...) and parse from common aliases (`tool-calling`, `image-generation`, ...).

## CapabilitySet

A `HashSet<Capability>` wrapper with:

- `has`, `has_all`, `has_any`
- `insert`, `extend`, `remove`
- `iter()` — declaration order (stable, additive)
- `intersection`, `union`
- serde serialization / deserialization

`iter()` iterates `Capability::all()` (a fixed declaration-order array) and
filters, so iteration order never depends on `HashSet` hashing.

## Matching

```rust
pub struct CapabilityMatch {
    pub required: Vec<Capability>,
    pub provider: Vec<Capability>,
    pub satisfied: Vec<Capability>,
    pub missing: Vec<Capability>,
    pub compatible: bool,
}

impl CapabilityMatch {
    pub fn new(required: &[Capability], provider: &CapabilitySet) -> Self { ... }
}
```

`compatible == missing.is_empty()`. The match is a pure function of the
request and the provider descriptor — the provider id is never consulted.

## Additive Future

Adding a capability (e.g. `WebSearch`) is a single new variant:

- `Capability::all()` — add to the array (order defines iteration order).
- `code()` / `description()` — add arms.
- `FromStr` — add an alias arm.
- No changes to matching, routing, health, or any other component.

This satisfies the contract: **future capabilities MUST be additive**.

## Where Matching Is Used

1. `ProviderRouter::resolve` — Stage 1 hard filter; mismatches become
   `RejectionReason::CapabilityMismatch { missing }`.
2. `ProviderDiscovery::query` — capability filters in discovery.
3. `Failover::plan_satisfies_capabilities` — capability-based failover
   preserves the request contract.

## Tests (14)

Cover: additive count (10), has/has_all/any, empty set, insert/extend/
remove, intersection/union, compatible/incompatible/no-requirement match,
serialization round-trip, strict parsing, case-insensitive parsing,
descriptions, and declaration-order iteration.
