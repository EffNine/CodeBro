//! Feature-flag + connection configuration for the shadow sidecar.
//!
//! Everything is environment-driven; there is intentionally no config-file
//! support (no new persisted authority surface). The API key is read from
//! the environment **per call** by the adapter and is never stored on any
//! struct, never logged, and never serialized.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use std::path::PathBuf;
use std::time::Duration;

/// Default Jev evaluation endpoint (TypeSafe System One API).
pub const DEFAULT_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
/// Default model.
///
/// Phase 8 freezes the rollout on `jev-1.13.0` (the Phase-7 validated
/// snapshot). `JEV_SHADOW_MODEL` may still override for offline tests, but
/// every Phase-8 rollout artifact must record `jev-1.13.0` as requested
/// and resolved.
pub const DEFAULT_MODEL: &str = "jev-1.13.0";
/// Default bounded request timeout.
pub const DEFAULT_TIMEOUT_MS: u64 = 3000;
/// Clamp window for the timeout so a misconfigured env cannot hang the
/// (detached) shadow path or disable the bound entirely.
pub const MIN_TIMEOUT_MS: u64 = 250;
pub const MAX_TIMEOUT_MS: u64 = 10_000;

/// Shadow-sidecar configuration. `enabled` defaults to `false`.
#[derive(Debug, Clone)]
pub struct JevShadowConfig {
    /// Master switch. Default `false`: zero Jev network calls, zero impact.
    pub enabled: bool,
    /// Advisory switch (Phase 8). Default `false`. Advisory behavior is
    /// only possible when BOTH `enabled` and `advisory_enabled` are true
    /// (plus a key present): advisory ON without shadow MUST NOT activate.
    /// When shadow is on but advisory is off, the sidecar stays shadow-only
    /// (log/evidence, no human-visible advisory output).
    pub advisory_enabled: bool,
    /// Evaluation endpoint. Default [`DEFAULT_ENDPOINT`].
    pub endpoint: String,
    /// Model. Default [`DEFAULT_MODEL`].
    pub model: String,
    /// Bounded per-request timeout (before retry accounting).
    pub timeout: Duration,
    /// Maximum retries for 429/529 only. Fixed at 1; not configurable on
    /// purpose (never retry indefinitely).
    pub max_retries: u8,
    /// JSONL log path override (`JEV_SHADOW_LOG_PATH`); otherwise
    /// `<workspace>/.codebro/jev-shadow.jsonl`.
    pub log_path_override: Option<PathBuf>,
    /// Advisory JSONL log path override (`JEV_ADVISORY_LOG_PATH`); otherwise
    /// `<workspace>/.codebro/jev-advisory.jsonl`. Advisory events are kept
    /// in a separate file so shadow evidence stays untouched.
    pub advisory_log_path_override: Option<PathBuf>,
}

impl Default for JevShadowConfig {
    fn default() -> Self {
        JevShadowConfig {
            enabled: false,
            advisory_enabled: false,
            endpoint: DEFAULT_ENDPOINT.to_string(),
            model: DEFAULT_MODEL.to_string(),
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            max_retries: 1,
            log_path_override: None,
            advisory_log_path_override: None,
        }
    }
}

impl JevShadowConfig {
    /// Read configuration from the environment.
    ///
    /// - `JEV_SHADOW_ENABLED`: truthy (`1`/`true`/`yes`/`on`, case-insensitive)
    ///   enables; anything else (including unset) is `false`.
    /// - `JEV_ADVISORY_ENABLED`: truthy enables advisory output, but ONLY
    ///   when `JEV_SHADOW_ENABLED` is also on (see [`Self::is_advisory_live`]).
    ///   Both default `false`.
    /// - `JEV_SHADOW_ENDPOINT` / `JEV_SHADOW_MODEL` / `JEV_SHADOW_TIMEOUT_MS`
    ///   override the defaults. Timeout is clamped to
    ///   [`MIN_TIMEOUT_MS`]..=[`MAX_TIMEOUT_MS`].
    /// - `JEV_SHADOW_LOG_PATH` overrides the shadow log file location.
    /// - `JEV_ADVISORY_LOG_PATH` overrides the advisory log file location.
    pub fn from_env() -> Self {
        let mut cfg = JevShadowConfig::default();
        cfg.enabled = matches!(
            std::env::var("JEV_SHADOW_ENABLED")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes" | "on"
        );
        cfg.advisory_enabled = matches!(
            std::env::var("JEV_ADVISORY_ENABLED")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes" | "on"
        );
        if let Ok(ep) = std::env::var("JEV_SHADOW_ENDPOINT") {
            let ep = ep.trim().to_string();
            if !ep.is_empty() {
                cfg.endpoint = ep;
            }
        }
        if let Ok(m) = std::env::var("JEV_SHADOW_MODEL") {
            let m = m.trim().to_string();
            if !m.is_empty() {
                cfg.model = m;
            }
        }
        if let Ok(t) = std::env::var("JEV_SHADOW_TIMEOUT_MS") {
            if let Ok(ms) = t.trim().parse::<u64>() {
                cfg.timeout = Duration::from_millis(ms.clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS));
            }
        }
        if let Ok(p) = std::env::var("JEV_SHADOW_LOG_PATH") {
            let p = p.trim().to_string();
            if !p.is_empty() {
                cfg.log_path_override = Some(PathBuf::from(p));
            }
        }
        if let Ok(p) = std::env::var("JEV_ADVISORY_LOG_PATH") {
            let p = p.trim().to_string();
            if !p.is_empty() {
                cfg.advisory_log_path_override = Some(PathBuf::from(p));
            }
        }
        cfg
    }

    /// Resolve the JSONL log path for a workspace root.
    pub fn log_path_for(&self, workspace_root: &std::path::Path) -> PathBuf {
        if let Some(p) = &self.log_path_override {
            return p.clone();
        }
        workspace_root.join(".codebro").join("jev-shadow.jsonl")
    }

    /// Resolve the advisory JSONL log path for a workspace root.
    pub fn advisory_log_path_for(&self, workspace_root: &std::path::Path) -> PathBuf {
        if let Some(p) = &self.advisory_log_path_override {
            return p.clone();
        }
        workspace_root.join(".codebro").join("jev-advisory.jsonl")
    }

    /// Live means: flag on AND a key present. Only then may any network
    /// call happen. (The key itself is read per call by the adapter.)
    pub fn is_live(&self) -> bool {
        self.enabled && !api_key_from_env().unwrap_or_default().is_empty()
    }

    /// Advisory-live means: BOTH flags on AND a key present. Advisory ON
    /// without shadow MUST NOT activate advisory behavior: when `enabled`
    /// is false this returns false regardless of `advisory_enabled`.
    /// Advisory mode never bypasses the shadow integration boundary — it
    /// only adds an informational projection on top of a live shadow call.
    pub fn is_advisory_live(&self) -> bool {
        self.enabled && self.advisory_enabled && !api_key_from_env().unwrap_or_default().is_empty()
    }
}

/// Read the API key from the environment only. Never logged, never stored.
pub fn api_key_from_env() -> Option<String> {
    std::env::var("TYPESAFE_API_KEY")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled() {
        let cfg = JevShadowConfig::default();
        assert!(!cfg.enabled, "feature flag MUST default OFF");
        assert!(!cfg.advisory_enabled, "advisory flag MUST default OFF");
        assert_eq!(cfg.endpoint, DEFAULT_ENDPOINT);
        assert_eq!(cfg.model, DEFAULT_MODEL);
        assert_eq!(DEFAULT_MODEL, "jev-1.13.0");
        assert!(!cfg.is_live());
        assert!(!cfg.is_advisory_live());
    }
}
