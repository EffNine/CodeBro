//! P6 deterministic risk signals for impact analysis.
//!
//! Risk signals are *evidence-based observations*, not guarantees and
//! not probabilities. OpenCode performs the actual reasoning; CodeBro
//! only reports which structural risk indicators a target touches.
//!
//! ```text
//! ImpactTarget + relationships + module paths
//!   └── assess_risk(...) → RiskSignal { level, indicators[] }
//! ```
//!
//! HIGH indicators (persistence, public MCP API, migrations, trust
//! boundaries, filesystem publishing, concurrency/fencing, auth):
//! - path contains `store`, `state.db`, `migration`, `migrate`
//! - path contains `mcp-server`, `tool_router`, `mcp/mod`
//! - path contains `auth`, `credential`, `secret`, `permission`
//! - path contains `skill` + `publish`, `lease`, `fencing`
//!
//! MEDIUM indicators (highly shared, many dependents, public API,
//! central utility):
//! - fan-out/fan-in above thresholds (caller supplies counts)
//! - path contains `core`, `lib`, `util`, `common`
//! - visibility is public (caller supplies flag)
//!
//! LOW: isolated leaf (no dependents, no indicators).
//!
//! Deterministic: same inputs → same level + same ordered indicators.
//! Bounded: at most 8 indicators, sorted.

#![allow(dead_code, unused_imports)]

use serde::{Deserialize, Serialize};

/// Blast-radius level. A signal, not a guarantee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskLevel {
    High,
    Medium,
    Low,
}

impl RiskLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            RiskLevel::High => "HIGH",
            RiskLevel::Medium => "MEDIUM",
            RiskLevel::Low => "LOW",
        }
    }
}

/// One deterministic risk indicator with evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskIndicator {
    /// Short code (`persistence_layer`, `public_mcp_api`, …).
    pub code: String,
    /// Why this indicator fired (path fragment, count, …).
    pub evidence: String,
}

/// Risk assessment for an impact target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskSignal {
    pub level: RiskLevel,
    /// Deterministic sorted indicators (≤8). Empty for LOW leaf modules.
    pub indicators: Vec<RiskIndicator>,
    /// Human-readable blast-radius summary (bounded, deterministic).
    pub blast_radius: String,
}

/// Inputs for risk assessment. All plain data — no I/O, no graph walk.
#[derive(Debug, Clone, Default)]
pub struct RiskInput {
    /// Workspace-relative target path (file/module path or symbol file).
    pub target_path: Option<String>,
    /// Target symbol/module/package name.
    pub target_name: Option<String>,
    /// Number of direct dependents (from impact traversal).
    pub direct_dependents: usize,
    /// Number of transitive dependents.
    pub transitive_dependents: usize,
    /// Whether the target is publicly visible (`pub` / exported).
    pub is_public: bool,
}

const MAX_INDICATORS: usize = 8;

/// Assess risk deterministically. Pure function over explicit inputs.
pub fn assess_risk(input: &RiskInput) -> RiskSignal {
    let mut indicators: Vec<RiskIndicator> = Vec::new();
    let path = input
        .target_path
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let name = input
        .target_name
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();

    let push = |indicators: &mut Vec<RiskIndicator>, code: &str, evidence: String| {
        if indicators.len() < MAX_INDICATORS {
            indicators.push(RiskIndicator {
                code: code.to_string(),
                evidence,
            });
        }
    };

    // ── HIGH indicators ──────────────────────────────────────────────
    if path.contains("store")
        || path.contains("state.db")
        || path.contains("persistence")
        || name.contains("contextstore")
    {
        push(
            &mut indicators,
            "persistence_layer",
            format!("path touches persistence: {}", clipped(&path)),
        );
    }
    if path.contains("migration")
        || path.contains("migrate")
        || path.contains("user_version")
        || name.contains("migrat")
    {
        push(
            &mut indicators,
            "database_migration",
            format!("path touches migration: {}", clipped(&path)),
        );
    }
    if path.contains("mcp")
        && (path.contains("mod") || path.contains("tool") || name.contains("tool"))
    {
        push(
            &mut indicators,
            "public_mcp_api",
            format!("path touches public MCP surface: {}", clipped(&path)),
        );
    }
    if path.contains("auth")
        || path.contains("credential")
        || path.contains("secret")
        || path.contains("permission")
        || path.contains("trust")
    {
        push(
            &mut indicators,
            "trust_boundary",
            format!("path touches trust boundary: {}", clipped(&path)),
        );
    }
    if (path.contains("skill") && path.contains("publish"))
        || path.contains("change_engine")
        || name.contains("apply_change")
    {
        push(
            &mut indicators,
            "filesystem_publishing",
            format!("path touches guarded mutation/publish: {}", clipped(&path)),
        );
    }
    if path.contains("lease")
        || path.contains("fenc")
        || path.contains("concurr")
        || name.contains("lease")
    {
        push(
            &mut indicators,
            "concurrency_fencing",
            format!("path touches concurrency/fencing: {}", clipped(&path)),
        );
    }

    // ── MEDIUM indicators ────────────────────────────────────────────
    let total_dependents = input.direct_dependents + input.transitive_dependents;
    if total_dependents >= 10 {
        push(
            &mut indicators,
            "many_dependents",
            format!(
                "{total_dependents} dependents (direct={} transitive={})",
                input.direct_dependents, input.transitive_dependents
            ),
        );
    } else if input.direct_dependents >= 5 {
        push(
            &mut indicators,
            "highly_shared_module",
            format!("{} direct dependents", input.direct_dependents),
        );
    }
    if input.is_public
        && (path.contains("core")
            || path.contains("lib")
            || path.contains("util")
            || path.contains("common"))
    {
        push(
            &mut indicators,
            "central_public_utility",
            format!("public item in shared area: {}", clipped(&path)),
        );
    } else if input.is_public && indicators.is_empty() {
        push(
            &mut indicators,
            "public_api",
            "public visibility".to_string(),
        );
    }

    indicators.sort_by(|a, b| {
        a.code
            .cmp(&b.code)
            .then_with(|| a.evidence.cmp(&b.evidence))
    });
    indicators.truncate(MAX_INDICATORS);

    let has_high = indicators.iter().any(|i| {
        matches!(
            i.code.as_str(),
            "persistence_layer"
                | "database_migration"
                | "public_mcp_api"
                | "trust_boundary"
                | "filesystem_publishing"
                | "concurrency_fencing"
        )
    });
    let level = if has_high {
        RiskLevel::High
    } else if !indicators.is_empty() {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    };

    let blast_radius = match level {
        RiskLevel::High => format!(
            "HIGH — {} indicator(s), {} direct + {} transitive dependents; review persistence/migration/trust effects",
            indicators.len(),
            input.direct_dependents,
            input.transitive_dependents
        ),
        RiskLevel::Medium => format!(
            "MEDIUM — {} indicator(s), {} direct + {} transitive dependents",
            indicators.len(),
            input.direct_dependents,
            input.transitive_dependents
        ),
        RiskLevel::Low => "LOW — isolated leaf; no risk indicators fired".to_string(),
    };

    RiskSignal {
        level,
        indicators,
        blast_radius,
    }
}

fn clipped(path: &str) -> String {
    if path.len() > 120 {
        format!("{}…", &path[..120])
    } else if path.is_empty() {
        "(unknown path)".to_string()
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_path_is_high() {
        let sig = assess_risk(&RiskInput {
            target_path: Some("crates/context-runtime/src/store.rs".to_string()),
            target_name: Some("ContextStore".to_string()),
            direct_dependents: 2,
            transitive_dependents: 7,
            is_public: true,
        });
        assert_eq!(sig.level, RiskLevel::High);
        assert!(sig.indicators.iter().any(|i| i.code == "persistence_layer"));
    }

    #[test]
    fn mcp_surface_is_high() {
        let sig = assess_risk(&RiskInput {
            target_path: Some("crates/mcp-server/src/mcp/mod.rs".to_string()),
            target_name: Some("tool_router".to_string()),
            ..Default::default()
        });
        assert_eq!(sig.level, RiskLevel::High);
    }

    #[test]
    fn migration_is_high() {
        let sig = assess_risk(&RiskInput {
            target_path: Some("crates/context-runtime/src/db.rs".to_string()),
            target_name: Some("migrate_v7".to_string()),
            ..Default::default()
        });
        assert_eq!(sig.level, RiskLevel::High);
    }

    #[test]
    fn many_dependents_is_medium() {
        let sig = assess_risk(&RiskInput {
            target_path: Some("src/utils.rs".to_string()),
            direct_dependents: 6,
            transitive_dependents: 0,
            ..Default::default()
        });
        assert_eq!(sig.level, RiskLevel::Medium);
    }

    #[test]
    fn isolated_leaf_is_low() {
        let sig = assess_risk(&RiskInput {
            target_path: Some("docs/notes.md".to_string()),
            ..Default::default()
        });
        assert_eq!(sig.level, RiskLevel::Low);
        assert!(sig.indicators.is_empty());
    }

    #[test]
    fn deterministic_ordering() {
        let input = RiskInput {
            target_path: Some("crates/mcp-server/src/store.rs".to_string()),
            target_name: Some("x".to_string()),
            direct_dependents: 12,
            transitive_dependents: 3,
            is_public: true,
        };
        let a = assess_risk(&input);
        let b = assess_risk(&input);
        assert_eq!(a, b);
        let mut sorted = a.indicators.clone();
        sorted.sort_by(|x, y| {
            x.code
                .cmp(&y.code)
                .then_with(|| x.evidence.cmp(&y.evidence))
        });
        assert_eq!(a.indicators, sorted);
    }
}
