#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Integration tests for the Provider Runtime (P10.3).
//!
//! These exercise the coordinate facade end-to-end: selection, routing,
//! health, retry, failover, cost, diagnostics and concurrency.

use std::time::{Duration, Instant};

use crate::provider_runtime::{
    BackoffStrategy, Capability, CapabilitySet, CostObservation, HealthManager, HealthPolicyConfig,
    HealthState, Outcome, Priority, ProviderCost, ProviderId, ProviderRegistry, ProviderRouter,
    ProviderRuntime, RegisteredProvider, RetryPolicy, RetrySchedule, RouteRequest,
};

fn caps(xs: &[Capability]) -> CapabilitySet {
    CapabilitySet::new(xs.iter().copied())
}

fn provider(id: &str, xs: &[Capability], cost: f64) -> RegisteredProvider {
    RegisteredProvider::new(
        id,
        caps(xs),
        ProviderCost {
            input_per_million: cost,
            output_per_million: cost,
            cache_read_per_million: None,
        },
        Priority::Normal,
    )
}

#[test]
fn end_to_end_select() {
    let rt = ProviderRuntime::new();
    rt.register_value(provider("alpha", &[Capability::Streaming], 5.0)).unwrap();
    rt.register_value(provider("beta", &[Capability::Streaming, Capability::ToolCalling], 1.0)).unwrap();
    let decision = rt
        .select(
            &RouteRequest::new()
                .with_capabilities(vec![Capability::ToolCalling, Capability::Streaming]),
        )
        .unwrap();
    assert_eq!(decision.provider.id.as_str(), "beta");
}

#[test]
fn fail_no_capability_matches() {
    let rt = ProviderRuntime::new();
    rt.register_value(provider("alpha", &[Capability::Streaming], 1.0)).unwrap();
    let res = rt
        .select(&RouteRequest::new().with_capabilities(vec![Capability::Audio, Capability::Vision]));
    assert!(res.is_err());
}

#[test]
fn deterministic_selection_across_instances() {
    fn build() -> ProviderRuntime {
        let rt = ProviderRuntime::new();
        for (id, cost) in [("p1", 2.0f64), ("p2", 1.0), ("p3", 3.0)] {
            rt.register_value(provider(id, &[Capability::Streaming], cost)).unwrap();
        }
        rt
    }
    let a = build().select(&RouteRequest::new()).unwrap().provider.id;
    let b = build().select(&RouteRequest::new()).unwrap().provider.id;
    assert_eq!(a, b);
    assert_eq!(a.as_str(), "p2");
}

#[test]
fn health_is_observational_no_registry_mutation() {
    let rt = ProviderRuntime::new();
    rt.register_value(provider("a", &[Capability::Streaming], 1.0)).unwrap();
    let id = ProviderId::new("a");
    for i in 0..3 {
        rt.health().report_failure(&id, Instant::now() + Duration::from_secs(i));
    }
    assert!(!rt.health().is_available(&id));
    assert_eq!(rt.registry().get(&id).unwrap().cost.routing_cost(), 2.0);
}

#[test]
fn retry_exponential_deterministic_schedule() {
    let p = RetryPolicy::default()
        .with_initial(Duration::from_millis(100))
        .with_attempts(4);
    let s1 = RetrySchedule::from(p.clone(), 0);
    let s2 = RetrySchedule::from(p, 0);
    assert_eq!(s1.retry_delays, s2.retry_delays);
    assert_eq!(
        s1.retry_delays,
        vec![
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(400),
        ]
    );
}

#[test]
fn retry_immediate_strategy() {
    let p = RetryPolicy::immediate(3);
    assert_eq!(p.delay_for_attempt(1), Duration::ZERO);
    assert_eq!(p.max_attempts, 3);
}

#[test]
fn retry_budget_limits_fixed_backoff() {
    let p = RetryPolicy {
        strategy: BackoffStrategy::Fixed(Duration::from_millis(500)),
        max_attempts: 8,
        initial_backoff: Duration::ZERO,
        multiplier: 1.0,
        max_backoff: Duration::ZERO,
        budget: Duration::from_millis(1100),
    };
    let s = RetrySchedule::from(p, 0);
    assert_eq!(s.retry_delays.len(), 2);
}

#[test]
fn failover_plan_preserves_contract() {
    let rt = ProviderRuntime::new();
    rt.register_value(provider("a", &[Capability::Streaming], 1.0)).unwrap();
    rt.register_value(provider("b", &[Capability::Streaming, Capability::ToolCalling], 2.0)).unwrap();
    let plan = rt.failover_plan(
        &RouteRequest::new().with_capabilities(vec![Capability::ToolCalling, Capability::Streaming]),
    );
    assert_eq!(plan, vec![ProviderId::new("b")]);
}

#[test]
fn cost_tracking_rates_and_latency() {
    let rt = ProviderRuntime::new();
    let id = ProviderId::new("tok");
    rt.cost()
        .record(CostObservation {
            provider: id.clone(),
            input_tokens: 100,
            output_tokens: 50,
            estimated_cost: 0.01,
            actual_cost: Some(0.009),
            latency_ms: 40,
            success: true,
        });
    rt.cost()
        .record(CostObservation {
            provider: id.clone(),
            input_tokens: 100,
            output_tokens: 50,
            estimated_cost: 0.01,
            actual_cost: None,
            latency_ms: 60,
            success: false,
        });
    let stats = rt.cost().stats(&id);
    assert_eq!(stats.calls, 2);
    assert_eq!(stats.successes, 1);
    assert_eq!(stats.failures, 1);
    assert!((stats.success_rate() - 0.5).abs() < 1e-9);
    assert_eq!(stats.avg_latency_ms(), 50.0);
}

#[test]
fn diagnostics_summary_reflects_activity() {
    let rt = ProviderRuntime::new();
    rt.register_value(provider("audio", &[Capability::Audio], 1.0)).unwrap();
    rt.register_value(provider("both", &[Capability::Audio, Capability::Vision], 1.0)).unwrap();
    let _ = rt
        .select(
            &RouteRequest::new()
                .with_capabilities(vec![Capability::Audio, Capability::Vision]),
        )
        .unwrap();
    let s = rt.diagnostics_summary();
    assert!(s.selections >= 1);
    assert!(s.events >= 1);
}

#[test]
fn concurrent_registration_and_selection() {
    use std::thread;
    let rt = ProviderRuntime::new();
    for i in 0..16u64 {
        rt.register_value(provider(&format!("_{i}"), &[Capability::Streaming], i as f64))
            .unwrap();
    }
    let mut handles = Vec::new();
    for _ in 0..8 {
        let rt = rt.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..50 {
                let _ = rt.select(&RouteRequest::new());
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(rt.registry().len(), 16);
}

#[test]
fn concurrent_cost_recording() {
    use std::thread;
    let rt = ProviderRuntime::new();
    let id = ProviderId::new("p");
    let mut handles = Vec::new();
    for _ in 0..8 {
        let rt = rt.clone();
        let id = id.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..50 {
                rt.cost().record(CostObservation {
                    provider: id.clone(),
                    input_tokens: 10,
                    output_tokens: 10,
                    estimated_cost: 0.001,
                    actual_cost: None,
                    latency_ms: 5,
                    success: true,
                });
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(rt.cost().stats(&id).calls, 400);
}

#[test]
fn concurrent_health_observation() {
    use std::thread;
    let hm = HealthManager::new();
    let id = ProviderId::new("p");
    let mut handles = Vec::new();
    for i in 0..6 {
        let hm = hm.clone();
        let id = id.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..100 {
                if i % 2 == 0 {
                    hm.report_success(&id, Instant::now());
                } else {
                    hm.report_failure(&id, Instant::now());
                }
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let rec = hm.record(&id).unwrap();
    assert_eq!(rec.total_calls, 600);
}

#[test]
fn concurrent_failover_planning() {
    use std::thread;
    let rt = ProviderRuntime::new();
    for i in 0..8u64 {
        rt.register_value(provider(&format!("q{i}"), &[Capability::Streaming], i as f64))
            .unwrap();
    }
    let mut handles = Vec::new();
    for _ in 0..8 {
        let rt = rt.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..50 {
                let plan = rt.failover_plan(
                    &RouteRequest::new().with_capabilities(vec![Capability::Streaming]),
                );
                assert!(!plan.is_empty());
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn sequential_failures_move_to_cooldown() {
    let cfg = HealthPolicyConfig {
        min_samples: 1,
        cooldown_after: 2,
        ..Default::default()
    };
    let hm = HealthManager::with_config(cfg);
    let id = ProviderId::new("x");
    hm.report_failure(&id, Instant::now() + Duration::from_secs(1));
    hm.report_failure(&id, Instant::now() + Duration::from_secs(2));
    assert_eq!(hm.health(&id), HealthState::Cooldown);
}

#[test]
fn recovery_from_cooldown_over_time() {
    let cfg = HealthPolicyConfig {
        min_samples: 1,
        cooldown_after: 2,
        recovery_successes: 1,
        ..Default::default()
    };
    let hm = HealthManager::with_config(cfg);
    let id = ProviderId::new("x");
    for i in 1..=2 {
        hm.report_failure(&id, Instant::now() + Duration::from_secs(i));
    }
    hm.begin_recovery(&id).unwrap();
    hm.report_success(&id, Instant::now() + Duration::from_secs(9));
    assert_eq!(hm.health(&id), HealthState::Healthy);
}

#[test]
fn priority_orders_selection() {
    let rt = ProviderRuntime::new();
    rt.register_value(
        provider("normal", &[], 1.0).with_priority(Priority::Normal),
    )
    .unwrap();
    rt.register_value(
        provider("critical", &[], 1.0).with_priority(Priority::Critical),
    )
    .unwrap();
    let d = rt.select(&RouteRequest::new()).unwrap();
    assert_eq!(d.provider.id.as_str(), "critical");
}

#[test]
fn route_request_builders() {
    let req = RouteRequest::new()
        .with_capabilities(vec![Capability::JsonMode])
        .with_cost_ceiling(1.0)
        .allow_degraded(true)
        .with_priority(Priority::High);
    assert_eq!(req.required_capabilities, vec![Capability::JsonMode]);
    assert_eq!(req.max_cost, Some(1.0));
    assert!(req.allow_degraded);
    assert_eq!(req.priority, Priority::High);
}