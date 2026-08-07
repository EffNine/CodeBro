#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Core types for the Provider Runtime (P10.3 - Provider Runtime Foundation).
//!
//! The Provider Runtime coordinates providers. It does NOT implement any
//! provider. Provider implementations remain plugins.
//!
//! This module defines the primitives shared across the Provider Runtime:
//! provider identity, routing requests, health state, cost model, and
//! runtime errors. Provider identity is opaque — the runtime never
//! special-cases a vendor name.
//!
//! # Design Rules
//!
//! - **Opaque identity**: A provider is identified only by its unique
//!   `ProviderId`. Vendors (OpenAI, Anthropic, ...) are unknown to the
//!   runtime.
//! - **No networking**: the runtime never touches the network or vendor
//!   SDKs. All I/O belongs to provider plugins.
//! - **Observational**: cost and success tracking only observe outcomes.

use serde::{Deserialize, Serialize};
use std::fmt;

use super::capabilities::Capability;

/// A unique, opaque identifier for a provider.
///
/// Provider name MUST never influence routing. The id is used only for
/// registry bookkeeping and reporting.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderId(pub String);

impl ProviderId {
    /// Construct a new provider id from a string slice.
    pub fn new(id: impl Into<String>) -> Self {
        ProviderId(id.into())
    }

    /// Returns the underlying id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for ProviderId {
    fn from(s: &str) -> Self {
        ProviderId::new(s)
    }
}

impl From<String> for ProviderId {
    fn from(s: String) -> Self {
        ProviderId::new(s)
    }
}

/// Request for provider selection performed by the Provider Router.
///
/// The request is provider-agnostic. It describes what the caller needs,
/// not who should serve it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteRequest {
    /// Capabilities the chosen provider MUST support.
    pub required_capabilities: Vec<Capability>,
    /// Optional maximum per-invocation cost ceiling (fraction of a unit).
    /// Providers whose estimated cost exceeds this are rejected by policy.
    pub max_cost: Option<f64>,
    /// Whether degraded providers are acceptable. Defaults to false.
    pub allow_degraded: bool,
    /// Providers to exclude explicitly (e.g. already attempted).
    pub excluded: Vec<ProviderId>,
    /// Priority of this request; influences tie-breaking only.
    pub priority: Priority,
}

impl Default for RouteRequest {
    fn default() -> Self {
        RouteRequest {
            required_capabilities: Vec::new(),
            max_cost: None,
            allow_degraded: false,
            excluded: Vec::new(),
            priority: Priority::Normal,
        }
    }
}

impl RouteRequest {
    pub fn new() -> Self {
        RouteRequest::default()
    }

    pub fn with_capabilities(mut self, caps: Vec<Capability>) -> Self {
        self.required_capabilities = caps;
        self
    }

    pub fn with_cost_ceiling(mut self, ceiling: impl Into<Option<f64>>) -> Self {
        self.max_cost = ceiling.into();
        self
    }

    pub fn allow_degraded(mut self, allow: bool) -> Self {
        self.allow_degraded = allow;
        self
    }

    pub fn excluding(mut self, ids: Vec<ProviderId>) -> Self {
        self.excluded = ids;
        self
    }

    pub fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }
}

/// Priority applied to a request or a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}

impl Priority {
    /// Returns a numeric score — higher is more important.
    pub fn score(&self) -> u8 {
        match self {
            Priority::Low => 0,
            Priority::Normal => 1,
            Priority::High => 2,
            Priority::Critical => 3,
        }
    }
}

impl Default for Priority {
    fn default() -> Self {
        Priority::Normal
    }
}

/// Health state owned by the runtime's health manager.
///
/// Health evaluation MUST be observational — it never mutates provider
/// behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthState {
    /// Provider is fully available.
    Healthy,
    /// Provider is usable but impaired.
    Degraded,
    /// Provider is not available right now.
    Unavailable,
    /// Provider is cooling down after failures.
    Cooldown,
    /// Provider is being probed for recovery.
    Recovering,
}

impl Default for HealthState {
    fn default() -> Self {
        HealthState::Unavailable
    }
}

impl HealthState {
    /// A provider is selectable when healthy or recovering (the latter
    /// only when the caller explicitly allows degraded selection).
    pub fn is_selectable(&self) -> bool {
        matches!(self, HealthState::Healthy | HealthState::Recovering)
    }

    /// A `Healthy`/`Degraded` provider is considered up for health
    /// accounting.
    pub fn is_up(&self) -> bool {
        matches!(self, HealthState::Healthy | HealthState::Degraded)
    }

    pub fn is_healthy(&self) -> bool {
        matches!(self, HealthState::Healthy)
    }
}

/// A descriptive cost model supplied by a provider plugin.
///
/// The Provider Runtime uses this only to estimate and track cost. It
/// does NOT perform billing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderCost {
    /// Estimated cost per 1M input tokens.
    pub input_per_million: f64,
    /// Estimated cost per 1M output tokens.
    pub output_per_million: f64,
    /// Optional cache-read cost per 1M tokens.
    pub cache_read_per_million: Option<f64>,
}

impl Default for ProviderCost {
    fn default() -> Self {
        ProviderCost {
            input_per_million: 0.0,
            output_per_million: 0.0,
            cache_read_per_million: None,
        }
    }
}

impl ProviderCost {
    /// Estimate the cost of an invocation given token usage. Observational.
    pub fn estimate(&self, input_tokens: usize, output_tokens: usize) -> f64 {
        let input_cost = (input_tokens as f64 / 1_000_000.0) * self.input_per_million;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * self.output_per_million;
        input_cost + output_cost
    }

    /// Per-request cost used purely for relative comparison in routing.
    /// Uses nominal pricing so ordering is stable regardless of live usage.
    pub fn routing_cost(&self) -> f64 {
        self.input_per_million + self.output_per_million
    }
}

/// A recorded cost observation (never a bill).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostObservation {
    pub provider: ProviderId,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub estimated_cost: f64,
    pub actual_cost: Option<f64>,
    pub latency_ms: u64,
    pub success: bool,
}

/// Outcome of an invocation as reported to the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Success,
    Failure,
    Timeout,
}

/// Errors raised by the Provider Runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProviderRuntimeError {
    /// No provider is registered under the given id.
    NotFound(ProviderId),
    /// A provider with the id is already registered.
    Duplicate(ProviderId),
    /// No registered provider matches the request.
    NoSuitableProvider(String),
    /// The required capabilities exceed every registered provider.
    CapabilityMismatch {
        requested: Vec<Capability>,
        available: Vec<Capability>,
    },
    /// Retry budget was exhausted.
    RetryExhausted {
        provider: ProviderId,
        attempts: usize,
    },
    /// The failover chain is empty or exhausted.
    FailoverExhausted {
        attempted: usize,
        total: usize,
    },
    /// A cost ceiling prevented selection.
    CostCeilingExceeded { provider: ProviderId },
    /// Runtime was in an invalid/illegal state.
    InvalidState(String),
    /// Capability string could not be parsed.
    UnknownCapability(String),
    /// A generic runtime error.
    Generic(String),
}

impl fmt::Display for ProviderRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderRuntimeError::NotFound(id) => write!(f, "Provider not found: {id}"),
            ProviderRuntimeError::Duplicate(id) => write!(f, "Duplicate provider: {id}"),
            ProviderRuntimeError::NoSuitableProvider(msg) => {
                write!(f, "No suitable provider: {msg}")
            }
            ProviderRuntimeError::CapabilityMismatch { requested, available } => write!(
                f,
                "Capability mismatch: requested {requested:?}, available {available:?}"
            ),
            ProviderRuntimeError::RetryExhausted { provider, attempts } => {
                write!(f, "Retry exhausted for {provider} after {attempts} attempts")
            }
            ProviderRuntimeError::FailoverExhausted { total, attempted } => {
                write!(f, "Failover exhausted: {attempted} of {total} attempted")
            }
            ProviderRuntimeError::CostCeilingExceeded { provider } => {
                write!(f, "Cost ceiling exceeded for {provider}")
            }
            ProviderRuntimeError::InvalidState(msg) => write!(f, "Invalid state: {msg}"),
            ProviderRuntimeError::UnknownCapability(cap) => {
                write!(f, "Unknown capability: {cap}")
            }
            ProviderRuntimeError::Generic(msg) => write!(f, "Provider runtime error: {msg}"),
        }
    }
}

impl std::error::Error for ProviderRuntimeError {}

pub type ProviderRuntimeResult<T> = Result<T, ProviderRuntimeError>;