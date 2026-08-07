# Provider Runtime Architecture Report

**Phase**: P10.3 — Provider Runtime Foundation
**Status**: APPROVED TO IMPLEMENT → IMPLEMENTED

## 1. Mission

Implement the Provider Runtime: the layer responsible for **coordinating
providers**. It is NOT responsible for implementing providers — provider
implementations remain plugins.

## 2. Ownership Contract

### Provider Runtime owns
- Provider Registry
- Provider Discovery
- Provider Resolution (capability matching)
- Health Management
- Routing Strategy
- Retry Policy
- Failover Policy
- Cost Tracking
- Provider Diagnostics

### Provider Runtime does NOT own
- HTTP client, REST/WebSocket, vendor SDKs
- Authentication / API keys
- OpenAI, Anthropic, Gemini, Ollama, LM Studio, Conductor logic

All of the above belong to Provider Plugins.

## 3. Routing Model

Selection is **deterministic** and follows this mandatory order:

```
Capability Match → Policy → Health → Cost → Priority → Registration Order
```

Provider name/id is **never** part of the ordering key. The final
tie-breaker is a monotonically increasing registration sequence, which
makes the result stable even though the registry stores providers in a
hash map internally.

## 4. Module Structure

```
src/provider_runtime/
  mod.rs          — ProviderRuntime facade (coordinator + re-exports)
  types.rs        — ProviderId, RouteRequest, Priority, HealthState,
                    ProviderCost, CostObservation, Outcome, errors
  capabilities.rs — Capability, CapabilitySet, CapabilityMatch (additive)
  provider.rs     — Provider plugin contract + RegisteredProvider record
  registry.rs     — register / unregister / lookup, deterministic order
  discovery.rs    — descriptive queries over registered providers
  health.rs       — HealthManager (Healthy/Degraded/Unavailable/Cooldown/Recovering)
  router.rs       — deterministic 6-stage selection + fallback chain
  retry.rs        — Immediate / Exponential / Fixed backoff, budget
  failover.rs     — Primary → Secondary → Fallback chain execution
  cost.rs         — CostTracker, TokenUsage, ProviderCostStats, dashboard
  diagnostics.rs  — ProviderEvent + selection/mismatch/retry/failover/stat records
  tests.rs        — integration & concurrency tests (18)
```

## 5. Capability Negotiation

Capabilities are additive descriptors, independent of provider identity:

`Streaming, StructuredOutput, ToolCalling, Vision, Embeddings, Reasoning,
Audio, ImageGeneration, LongContext, JsonMode`

Future capabilities are added as new enum variants without disturbing
matching logic. Matching is a pure function of the request and the
provider's descriptor.

## 6. Health Management

States: `Healthy, Degraded, Unavailable, Cooldown, Recovering`.

- Health evaluation is **observational only** — it never mutates provider
  behaviour.
- Unobserved providers are assumed `Healthy` until proven otherwise.
- Consecutive failures trigger `Cooldown`; ratio thresholds trigger
  `Degraded` / `Unavailable`; consecutive successes recover a provider.

## 7. Retry & Failover

- Retry belongs to the runtime, not providers: immediate, exponential
  backoff, fixed, max attempts, and a total retry budget. Deterministic.
- Failover supports Primary / Secondary / Fallback chains, health-based
  and capability-based failover, and preserves the request contract.

## 8. Cost Tracking

Tracks estimated cost, actual cost, token usage, latency, success rate and
failure rate. Reports metrics; never bills.

## 9. Observability

Emits `ProviderEvent` variants: `ProviderSelected`, `ProviderRejected`,
`ProviderUnavailable`, `RetryStarted`, `RetryCompleted`,
`FailoverTriggered`, `CostRecorded`, `ProviderRecovered`.

## 10. Acceptance Criteria Compliance

| Criterion | Status |
|-----------|--------|
| Existing Runtime unchanged | ✅ additive module only |
| AI Runtime unchanged | ✅ |
| Memory Runtime unchanged | ✅ |
| Provider-agnostic | ✅ opaque `ProviderId` |
| No networking / vendor SDK | ✅ none in module |
| Thread-safe | ✅ `Arc<RwLock>` throughout |
| Deterministic | ✅ 6-stage pipeline, seq tiebreak |
| Observable | ✅ `ProviderEvent` + diagnostics |
| Zero regressions | ✅ 2020/2020 pass |

## 11. Chief Architect Exit Criteria

1. **A provider plugin can be implemented without modifying the runtime** —
   implement `Provider` and `register()`.
2. **Routing decisions are deterministic** — verified by tests.
3. **Retry and failover are runtime responsibilities** — `retry.rs`,
   `failover.rs`.
4. **Capability negotiation is independent of provider identity** —
   `CapabilityMatch::new(required, provider_set)`.
5. **Health monitoring does not mutate provider behaviour** —
   `HealthManager` only reads registry, never writes it.
6. **Cost tracking is observational only** — `CostTracker` only records.
7. **No existing runtime requires redesign** — nothing outside
   `provider_runtime` was modified except one `mod` declaration.
