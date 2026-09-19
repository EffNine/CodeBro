# Health Management Report

**Phase**: P10.3 — Provider Runtime Foundation

## Purpose

Health evaluation MUST be observational. It observes invocation outcomes
and derives state, but it **never mutates provider behaviour** — the
health manager holds no reference to, and cannot influence, provider
plugins.

## States

| State | Meaning | Selectable |
|-------|---------|-----------|
| `Healthy` | Fully available | Yes |
| `Recovering` | Probe in progress | Yes (degraded selection) |
| `Degraded` | Usable but impaired | Only when allowed |
| `Unavailable` | Not available | No |
| `Cooldown` | Cooling down after failures | No |

## Default Policy

```rust
pub struct HealthPolicyConfig {
    min_samples: 3,            // min calls before ratio thresholds apply
    degrade_threshold: 0.4,    // failure ratio → Degraded
    unavailable_threshold: 0.8,// failure ratio → Unavailable
    cooldown_after: 3,         // consecutive failures → Cooldown
    cooldown_duration: 60s,
    recovery_successes: 2,     // consecutive successes → Healthy
}
```

## Observations (Observational Only)

- `report_success(provider, at)` — increments successes; resets
  consecutive-failure counter; removes active cooldown; recovers after
  `recovery_successes` consecutive successes.
- `report_failure(provider, at)` — increments failures; triggers
  `Cooldown` after `cooldown_after` consecutive failures; otherwise
  applies `Degraded` / `Unavailable` once `min_samples` is reached.
- `begin_recovery(provider)` — transitions `Unavailable`/`Cooldown` →
  `Recovering` (for probes).

A provider that has never been observed is **assumed `Healthy`** until
proven otherwise.

## Interaction With the Registry

`HealthManager` only reads the registry through the router/discovery. It
holds its own per-provider records keyed by `ProviderId`. It never writes
to the registry and never alters a provider descriptor.

## Thread Safety

All state lives behind `Arc<RwLock<HealthInner>>`; `HealthManager` is
`Clone` and safe to share across threads (verified by concurrency tests).

## Tests (13)

Cover: initial health, unknown-assumed-healthy, success observations,
heavy-failure → Cooldown, degrade-at-threshold, cooldown trigger,
cooldown non-selectable, recovery to Healthy, recovery requires
consecutive successes, unknown recovery error, failure-rate math,
degraded selectable only when allowed, all-records/count.
