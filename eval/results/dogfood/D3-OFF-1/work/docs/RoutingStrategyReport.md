# Routing Strategy Report

**Phase**: P10.3 — Provider Runtime Foundation

## Purpose

Define and verify the deterministic provider selection strategy. Selection
must never depend on provider name.

## Deterministic Selection Pipeline

`ProviderRouter::resolve(request)` applies, in order:

1. **Capability Match** (hard filter) — provider must satisfy every
   required capability.
2. **Policy** — exclusions from the request + optional cost ceiling.
3. **Health** — skip non-selectable providers (Unavailable/Cooldown);
   Degraded selectable only when allowed.
4. **Cost** — ascending `routing_cost()` (nominal pricing).
5. **Priority** — descending priority score.
6. **Registration Order** — ascending monotonically-increasing seq.

## Ordering Key

```
(health_rank asc, cost asc, priority desc, registration_seq asc)
```

`health_rank`: Healthy=0, Recovering=1, Degraded=2, Unavailable=3,
Cooldown=4.

The provider id/name is **never read** as part of the ordering key.
Because `registration_seq` is unique per provider, the total order is
stable regardless of the registry's internal hash map iteration order.

## Public API

- `ProviderRouter::resolve(&RouteRequest) -> RoutingDecision`
- `ProviderRouter::chain(&RouteRequest) -> Vec<RouterDecision>`
- `ProviderRouter::provider_capabilities(&ProviderId) -> Option<CapabilitySet>`
- `ProviderRouter::with_policy(RoutingPolicy)`
- `RoutingPolicy { allow_degraded_fallback, skip_unhealthy }`

## RoutingDecision

```
provider, applied: Vec<String>, rejected: Vec<Rejection>, considered: usize
```

`Rejection` records a rejected candidate and its `RejectionReason`
(CapabilityMismatch, ExcludedByRequest, Unhealthy, CostCeiling) for
diagnostics and full auditability of every decision.

## Determinism Tests

- `test_resolve_deterministic_across_calls` — identical results on repeat.
- `test_deterministic_selection_across_instances` — same input, same
  output across separate runtimes.
- `test_name_never_influences_routing` — providers differing only in
  name/pricing still order by the routing key, not by label.

## Verification

```
cargo test --bin codebro provider_runtime::router
# passed (17 resolved + 2 failover-adjacent tests)
```