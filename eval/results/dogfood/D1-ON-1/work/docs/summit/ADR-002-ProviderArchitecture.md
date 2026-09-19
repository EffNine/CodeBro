# ADR-002: Provider Architecture

**ADR Number:** ADR-002
**Title:** Provider Architecture
**Author:** CodeBro Engineering
**Status:** Proposed
**Created:** 2026-08-07
**Updated:** 2026-08-07
**Part of:** Design Summit v2
**Supersedes:** None
**Related:** ADR-001, ADR-003, ADR-004

---

## 1. Context

### 1.1 Background

The v1.0 Provider trait (`src/providers/provider.rs`) defines the sole interface to LLM communication:

```rust
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn base_url(&self) -> &str;
    fn model(&self) -> &str;
    fn api_key(&self) -> Option<&str>;
    fn send_message(&self, message: &str) -> Pin<Box<dyn Future<Output = Result<String>>>>;
    fn stream_response(&self, message: &str) -> Pin<Box<dyn Future<Output = Result<UnboundedReceiver<String>>>>>;
}
```

Currently, only `OpenAiProvider` is implemented. Provider selection is hardcoded in the TUI.

### 1.2 Problem

The v1.0 provider architecture lacks:

1. **Dynamic discovery** — Providers must be manually registered
2. **Health monitoring** — No way to detect provider failures proactively
3. **Cost tracking** — No per-request cost accounting
4. **Routing** — No cost/latency/quality-aware provider selection
5. **Failover** — Manual intervention required on provider failure
6. **Plugin providers** — No extension mechanism for new providers

### 1.3 Constraints

- Provider trait is frozen — cannot modify the interface
- OpenAiProvider must continue to work
- New provider implementations must implement the same trait
- Cost tracking must not modify provider behavior

### 1.4 Stakeholders

- **AI Runtime** — Consumes provider via trait
- **Router** — Selects provider based on policy
- **Budget tracker** — Records costs per provider
- **Health monitor** — Probes provider availability
- **Plugin SDK** — Enables plugin-provided providers

---

## 2. Decision

### 2.1 Decision Statement

The Provider Runtime wraps the frozen Provider trait with management capabilities: discovery, health monitoring, cost tracking, routing, and failover. The Provider trait remains unchanged; the runtime adds infrastructure around it.

### 2.2 Rationale

1. **Wrap, don't replace** — Preserves v1.0 compatibility
2. **Trait-based** — Enables plugin providers
3. **Cost-aware** — Users control AI spending
4. **Resilient** — Automatic failover improves reliability
5. **Observable** — Health and metrics are visible

### 2.3 Principles Applied

- **Principle 4 (Model Agnostic)** — Provider is a detail, not a dependency
- **Principle 7 (Modular Architecture)** — Provider runtime is a distinct module
- **Principle 8 (Observable AI Actions)** — Provider events are emitted
- **Principle 9 (Performance Matters)** — Routing adds minimal overhead

---

## 3. Architecture

### 3.1 Provider Runtime Module

```
src/runtime/provider/
├── mod.rs              # Module assembly
├── discovery.rs        # Dynamic provider discovery
├── health.rs           # Health monitoring
├── metrics.rs          # Usage and cost metrics
└── failover.rs         # Automatic failover logic
```

### 3.2 Provider Runtime Trait

```rust
pub trait ProviderRuntime: Send + Sync {
    /// Discover and register available providers.
    async fn discover(&mut self) -> Result<Vec<ProviderId>>;

    /// Get the selected provider for a request.
    fn select_provider(&self, request: &AIRequest) -> ProviderId;

    /// Get provider health status.
    fn health(&self, provider: &ProviderId) -> ProviderHealth;

    /// Get usage metrics for a provider.
    fn metrics(&self, provider: &ProviderId) -> ProviderMetrics;

    /// Get current cost tracking.
    fn cost_tracking(&self) -> CostTracking;

    /// Failover to next available provider.
    async fn failover(&mut self, failed: &ProviderId) -> Option<ProviderId>;

    /// Register a new provider implementation.
    fn register_provider(&mut self, provider: Box<dyn Provider>);

    /// Get the current provider instance.
    fn get_provider(&self, id: &ProviderId) -> Option<Box<dyn Provider>>;
}
```

### 3.3 Routing Strategies

| Strategy | Selection Logic | Config Key |
|----------|-----------------|------------|
| Cost-optimal | Cheapest provider meeting quality floor | `routing.strategy = cost` |
| Latency-optimal | Fastest provider (lowest avg response time) | `routing.strategy = latency` |
| Quality-optimal | Highest quality provider (gpt-4o > gpt-3.5) | `routing.strategy = quality` |
| Balanced | Weighted combo of cost and quality | `routing.strategy = balanced` |
| Fallback | Primary, then secondary on error | `routing.strategy = fallback` |

### 3.4 Health Monitoring

```rust
pub enum ProviderHealth {
    Healthy,
    Degraded { latency_ms: u64, error_rate: f32 },
    Unhealthy { last_error: String, consecutive_failures: u32 },
}

pub struct HealthMonitor {
    probe_interval: Duration,
    degradation_threshold: f32,
    failure_threshold: u32,
    cooldown_period: Duration,
}
```

### 3.5 Cost Tracking

```rust
pub struct CostTracking {
    pub daily: f64,
    pub session: f64,
    pub per_task: f64,
    pub providers: HashMap<ProviderId, ProviderCost>,
}

pub struct ProviderCost {
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost: f64,
}
```

### 3.6 Failover Logic

```
Provider fails
    ↓
Mark unhealthy
    ↓
Check cooldown period
    ↓
Select next available provider
    ↓
Retry request
    ↓
If all providers fail → return error to caller
```

---

## 4. Integration with v1.0

### 4.1 Provider Trait Compatibility

The Provider trait is unchanged. The runtime wraps existing implementations:

```rust
// v1.0: Direct usage
let provider = OpenAiProvider::new(config.clone());
let response = provider.stream_response(&prompt).await?;

// v2.0: Through runtime
runtime.register_provider(Box::new(OpenAiProvider::new(config.clone())));
let provider_id = runtime.select_provider(&request);
let provider = runtime.get_provider(&provider_id).unwrap();
let response = provider.stream_response(&prompt).await?;
```

### 4.2 Configuration

```toml
# ~/.codebro/config.toml

[routing]
strategy = "balanced"
quality_floor = "gpt-4o-mini"
cost_limit_daily = 5.00
cost_limit_session = 2.00

[providers]
[providers.openai]
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
api_key = "sk-..."

[health]
probe_interval = "30s"
degradation_threshold = 0.1
failure_threshold = 3
cooldown_period = "60s"
```

### 4.3 Event Emissions

| Event | Trigger |
|-------|---------|
| `ProviderDiscovered(id)` | Discovery finds new provider |
| `ProviderUnavailable(id)` | Health check fails |
| `ProviderFailed(id, error)` | Request fails |
| `ProviderSwitched { from, to }` | Failover triggers |
| `CostThresholdReached(level)` | Budget threshold hit |

---

## 5. Consequences

### 5.1 Positive Consequences

- Multiple providers supported without code changes
- Automatic failover improves reliability
- Cost tracking enables budget control
- Health monitoring enables proactive response
- Plugin providers can extend capability

### 5.2 Negative Consequences

- Additional abstraction layer adds complexity
- Health probing adds network overhead
- Cost tracking requires provider metadata

### 5.3 Trade-offs

| Aspect | Trade-off | Mitigation |
|--------|-----------|------------|
| Indirection | Trait + runtime wrapper | Negligible overhead |
| Probing | Network overhead | Configurable interval |
| Metadata | Requires provider to expose pricing | Default to zero if unknown |
| Failover | May switch to worse provider | Quality floor prevents downgrade |

---

## 6. Alternatives Considered

| Alternative | Description | Pros | Cons | Why Rejected |
|-------------|-------------|------|------|--------------|
| Modify Provider trait | Add routing methods to trait | Simpler | Breaks v1.0 compatibility | Frozen API |
| Singleton provider | Single provider instance | Simple | No failover, no routing | Violates Principle 4 |
| Hardcode providers | No runtime, just config | Simple | No plugin support | Violates extensibility |
| External provider service | Separate service for providers | Isolated | Adds deployment complexity | Overkill for desktop app |

---

## 7. Implementation Notes

### 7.1 Code Patterns

```rust
// Register provider
runtime.register_provider(Box::new(OpenAiProvider::new(config.clone())));

// Select provider
let provider_id = runtime.select_provider(&request);

// Execute with failover
let result = runtime.execute_with_failover(&request).await;
```

### 7.2 Anti-Patterns

```rust
// NEVER: Hardcode provider selection
let provider = OpenAiProvider::new(config.clone());

// ALWAYS: Use runtime selection
let provider_id = runtime.select_provider(&request);
let provider = runtime.get_provider(&provider_id).unwrap();
```

### 7.3 Plugin Integration

Plugins can provide providers by implementing the Provider trait and registering via:

```rust
// In plugin init()
runtime.register_provider(Box::new(MyProvider::new()));
```

---

## 8. References

- [ADR-001: Runtime Architecture](./ADR-001-RuntimeArchitecture.md)
- [Provider Capabilities Contract](../contracts/provider_capabilities.md)
- [Runtime Architecture](../summit/RuntimeArchitecture.md)
- [Provider Principles](../summit/RuntimePrinciples.md) §3

---

## 9. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-07 | Created | CodeBro Engineering |
