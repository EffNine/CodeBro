# ADR-001: Provider Runtime Architecture

**Document:** `docs/ADR/adr-001-provider-runtime-architecture.md`
**Version:** 1.0.0
**Part of:** CodeBro P1 Core Runtime
**Status:** Proposed
**Created:** 2026-08-05
**Updated:** 2026-08-05
**Related RFC:** RFC-001

---

## 1. Context

### 1.1 Background

The `Provider` trait is defined in `src/providers/provider.rs` and implemented by `OpenAiProvider` in `src/providers/openai.rs`. Both are complete and correct. However, the production pipeline in `tui/ui.rs` does **not** use the trait. Instead, `call_ai_streaming()` (lines 885–946) makes raw `reqwest` calls directly, duplicating the HTTP logic that already exists in `OpenAiProvider::stream_response()`.

This violates:
- Architecture Manifest Section 4.2, Rule 2: "The `Provider` trait is the sole interface to LLM communication."
- Design Principle 4 (Model Agnostic): "The `Provider` trait is the single interface to LLM communication."

### 1.2 Constraints

- The `Provider` trait signature is frozen (Section 4.1 of Architecture Manifest).
- No new dependencies may be added without an RFC.
- The change must be backward-compatible — no session format or config changes.
- All existing tests must continue to pass.

### 1.3 Stakeholders

- **TUI module**: Must receive `Provider` instance and use it instead of raw HTTP
- **Provider module**: No changes needed (already correct)
- **Tools module**: No changes needed
- **Tests**: Must cover the new call path

---

## 2. Decision

### 2.1 Decision Statement

All LLM communication in the production pipeline must go through the `Provider` trait. The raw `reqwest` call in `tui/ui.rs::call_ai_streaming()` is replaced with a call to `provider.stream_response()`.

### 2.2 Rationale

1. **Architecture compliance**: The manifest explicitly forbids raw `reqwest` outside the provider module.
2. **Single source of truth**: The streaming HTTP logic lives in one place (`OpenAiProvider`), not duplicated in the TUI.
3. **Extensibility**: Adding a new provider requires only implementing the trait — no TUI changes.
4. **Testability**: The `Provider` trait can be mocked in tests.

### 2.3 Principles Applied

- **Principle 4 (Model Agnostic)**: Provider is a detail, not a dependency.
- **Principle 7 (Modular Architecture)**: Each module has one responsibility; providers handle LLM communication.
- **Principle 10 (Small, Composable Components)**: Removes a 60-line duplicated function.

---

## 3. Consequences

### 3.1 Positive Consequences

- Architecture manifest compliance achieved.
- Provider HTTP logic is now the single source of truth for LLM communication.
- Future providers (Anthropic, Gemini, etc.) can be added without TUI changes.
- Tests can mock the `Provider` trait instead of stubbing HTTP.

### 3.2 Negative Consequences

- `TuiApp` must hold a `Provider` reference (or the config must be passed to create one).
- `run_chat_pipeline` signature changes to accept a provider.
- The model picker currently fetches models via a standalone function — this remains unchanged (it's a metadata operation, not a chat call).

### 3.3 Trade-offs

| Aspect | Trade-off | Mitigation |
|--------|-----------|------------|
| TuiApp complexity | Must hold provider or recreate it per-task | Provider is cheap to create; recreate per-task from config |
| Test surface | New call path needs tests | Add integration test with mock provider |
| Error handling | Provider errors wrap differently | `CodeBroError::Provider` variant already exists |

### 3.4 Impact on Architecture

| Module | Impact |
|--------|--------|
| `tui/ui.rs` | Replace `call_ai_streaming` with provider call; pipeline signature gains `provider` param |
| `tui/app.rs` | No structural change; provider created from config at pipeline start |
| `providers/` | No changes |
| `error.rs` | No changes |

### 3.5 Impact on Future Work

- P2 multi-agent: Each subagent can receive its own provider instance.
- P4 intelligence layer: Context builder can use the same provider for embedding calls.

---

## 4. Alternatives Considered

| Alternative | Description | Pros | Cons | Why Rejected |
|-------------|-------------|------|------|--------------|
| Keep raw reqwest | Leave `call_ai_streaming` as-is | No changes needed | Violates architecture manifest | Architecture violation |
| Make Provider a concrete type | Use `OpenAiProvider` directly | Simpler types | Breaks model-agnostic principle | Principle 4 violation |
| Trait object vs generics | Use `dyn Provider` vs generic `P: Provider` | Trait object is simpler | Slight dynamic dispatch cost | Cost is negligible; trait object is more flexible |

---

## 5. Implementation Notes

### 5.1 Code Patterns

```rust
// In run_chat_pipeline, create provider from config:
let provider = OpenAiProvider::new(config.clone());

// Replace raw HTTP call:
// BEFORE:
let response = call_ai_streaming(config, &prompt, tx).await?;

// AFTER:
let response = call_ai_streaming(&provider, &prompt, tx).await?;
```

### 5.2 Anti-Patterns

```rust
// NEVER do this in tui/ or agent/:
let client = reqwest::Client::new();
let res = client.post(&url).bearer_auth(api_key).json(&body).send().await?;

// ALWAYS use the provider:
let mut rx = provider.stream_response(&prompt).await?;
```

### 5.3 Migration Steps

1. Add `OpenAiProvider` import to `tui/ui.rs`
2. Create provider instance at start of `run_chat_pipeline`
3. Replace `call_ai_streaming(config, ...)` with `call_ai_streaming(provider.as_ref(), ...)`
4. Rewrite `call_ai_streaming` to accept `&dyn Provider`
5. Remove raw `reqwest` code from `call_ai_streaming`
6. Run tests

---

## 6. References

- [Architecture Manifest Section 4](../../architecture/architecture_manifest_v1.md#4-provider-abstraction)
- [Design Principle 4](../../principles/design_principles.md#principle-4-model-agnostic)
- [RFC-001](../../RFC/rfc-001-react-runtime-loop.md)

---

## 7. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-05 | Created | CodeBro Engineering |
