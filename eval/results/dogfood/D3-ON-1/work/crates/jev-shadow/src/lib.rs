//! Jev shadow sidecar — a completely non-authoritative decision observer.
//!
//! This crate is a **leaf**: it may use `codebro-core` (secret redaction)
//! and generic http/serde utilities, but nothing in the execution path
//! (sandbox policy, change engine, task lifecycle, evidence journal) may
//! depend on it. The only consumer is the MCP server's post-decision hook,
//! which calls [`ShadowObserver`] *after* the deterministic result is fully
//! computed, on a detached task whose outcome is discarded.
//!
//! What this crate can never do by construction: approve, deny, execute,
//! retry, delete, escalate, or otherwise control any action. There is no
//! API here that returns a decision to an execution path — [`ShadowObserver`]
//! returns log records only.
//!
//! Shadow mode is log/evidence-only and gated by [`JevShadowConfig::enabled`]
//! (`JEV_SHADOW_ENABLED`, default `false`). When disabled, zero network
//! calls are made.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

pub mod adapter;
pub mod advisory;
pub mod change_control;
pub mod config;
pub mod logging;
pub mod questions;
pub mod replay;
pub mod shadow;
pub mod types;

pub use adapter::JevClient;
pub use advisory::{AdvisoryEvent, AdvisoryState};
pub use config::JevShadowConfig;
pub use logging::{append_record, scan_text_for_secret};
pub use questions::QUESTION_SET_VERSION;
pub use questions::QUESTION_SET_VERSION_V2;
pub use questions::QUESTION_SET_VERSION_V3;
pub use shadow::{Agreement, ShadowObserver, ShadowRecord};
pub use types::{JevAnswer, ShadowDecision};
