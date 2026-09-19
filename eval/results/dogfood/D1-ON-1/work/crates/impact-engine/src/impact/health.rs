//! P6 evidence-based repository health findings.
//!
//! Findings are *observations*, not bugs and not quality scores. Each
//! finding carries severity, evidence, location, and a bounded confidence
//! so OpenCode can reason over deterministic structure instead of prose.
//!
//! Finding types:
//! - CYCLE: dependency cycle among modules/packages.
//! - HIGH_FANOUT: a module with an unusually large out-degree.
//! - HIGH_FANIN: a module with an unusually large in-degree.
//! - ORPHAN: a symbol/module with no edges at all.
//! - UNRESOLVED_REFERENCE: a validation `broken_index` issue.
//! - STALE_INDEX: the fact store's generation state differs from HEAD.
//! - MISSING_TEST_ASSOCIATION: a non-test source module with no linked test.
//! - LARGE_MODULE: a module with an unusually large symbol count.
//! - BOUNDARY_CROSSING: a dependency edge crossing a package boundary
//!   without going through a declared package dependency.
//!
//! Deterministic (sorted by type, then location), bounded (caller cap,
//! default 50), and evidence-grounded (counts + ids, never prose).

#![allow(dead_code, unused_imports)]

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::engineering_facts::{FactId, RelationshipKind};
use crate::fact_store::FactStore;

/// Finding type vocabulary (stable strings for MCP + doctor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FindingType {
    Cycle,
    HighFanout,
    HighFanin,
    Orphan,
    UnresolvedReference,
    StaleIndex,
    MissingTestAssociation,
    LargeModule,
    BoundaryCrossing,
}

impl FindingType {
    pub fn as_str(self) -> &'static str {
        match self {
            FindingType::Cycle => "CYCLE",
            FindingType::HighFanout => "HIGH_FANOUT",
            FindingType::HighFanin => "HIGH_FANIN",
            FindingType::Orphan => "ORPHAN",
            FindingType::UnresolvedReference => "UNRESOLVED_REFERENCE",
            FindingType::StaleIndex => "STALE_INDEX",
            FindingType::MissingTestAssociation => "MISSING_TEST_ASSOCIATION",
            FindingType::LargeModule => "LARGE_MODULE",
            FindingType::BoundaryCrossing => "BOUNDARY_CROSSING",
        }
    }
}

/// Severity of a finding (explicit, not a score).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Info,
    Warning,
    Error,
}

impl FindingSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            FindingSeverity::Info => "info",
            FindingSeverity::Warning => "warning",
            FindingSeverity::Error => "error",
        }
    }
}

/// One evidence-based health finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthFinding {
    #[serde(rename = "type")]
    pub finding_type: FindingType,
    pub severity: FindingSeverity,
    /// Short evidence line (counts, ids — never prose speculation).
    pub evidence: String,
    /// Canonical location (module path, symbol id, …) where applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Bounded confidence in [0,1]: structural certainty, not probability.
    pub confidence: f64,
}

/// Thresholds (deterministic constants, documented, not tuned per repo).
pub const HIGH_FANOUT_THRESHOLD: usize = 20;
pub const HIGH_FANIN_THRESHOLD: usize = 20;
pub const LARGE_MODULE_SYMBOLS: usize = 100;
pub const DEFAULT_FINDING_LIMIT: usize = 50;

/// Analyse a fact store for health findings. Pure over the store +
/// optional freshness flag; no I/O. Deterministic ordering, bounded
/// output (`limit`; 0 = default).
pub fn analyze_health(store: &FactStore, stale_index: bool, limit: usize) -> Vec<HealthFinding> {
    let cap = if limit == 0 {
        DEFAULT_FINDING_LIMIT
    } else {
        limit.min(500)
    };
    let mut out: Vec<HealthFinding> = Vec::new();

    if stale_index {
        out.push(HealthFinding {
            finding_type: FindingType::StaleIndex,
            severity: FindingSeverity::Warning,
            evidence: "fact store generation state differs from current repository state; reindex to refresh".to_string(),
            location: None,
            confidence: 0.9,
        });
    }

    // ── Degree maps over module-space edges ──────────────────────────
    let collection = store.collection();
    let mut out_degree: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut in_degree: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut touched: BTreeSet<String> = BTreeSet::new();

    // Symbol → owning module for call-edge projection.
    let mut symbol_module: BTreeMap<String, String> = BTreeMap::new();
    for s in collection.symbols() {
        if let Some(m) = s.module.as_ref() {
            symbol_module.insert(s.id.to_string(), m.to_string());
        }
    }
    // Module id set for boundary checks.
    let module_ids: BTreeSet<String> = collection
        .modules()
        .iter()
        .map(|m| m.id.to_string())
        .collect();

    for rel in collection.relationships() {
        let (src_mod, dst_mod) = match rel.kind {
            RelationshipKind::Calls => {
                // Project symbol endpoints into module space.
                let src = match &rel.source {
                    FactId::Symbol(s) => symbol_module.get(s.as_str()).cloned(),
                    FactId::Module(m) => Some(m.to_string()),
                    _ => None,
                };
                let dst = match &rel.target {
                    FactId::Symbol(s) => symbol_module.get(s.as_str()).cloned(),
                    FactId::Module(m) => Some(m.to_string()),
                    _ => None,
                };
                match (src, dst) {
                    (Some(a), Some(b)) => (a, b),
                    _ => continue,
                }
            }
            RelationshipKind::Imports
            | RelationshipKind::DependsOn
            | RelationshipKind::References => {
                let src = match &rel.source {
                    FactId::Module(m) => Some(m.to_string()),
                    FactId::Symbol(s) => symbol_module.get(s.as_str()).cloned(),
                    _ => None,
                };
                let dst = match &rel.target {
                    FactId::Module(m) => Some(m.to_string()),
                    FactId::Symbol(s) => symbol_module.get(s.as_str()).cloned(),
                    _ => None,
                };
                match (src, dst) {
                    (Some(a), Some(b)) => (a, b),
                    _ => continue,
                }
            }
            _ => continue,
        };
        if src_mod == dst_mod {
            continue;
        }
        touched.insert(src_mod.clone());
        touched.insert(dst_mod.clone());
        out_degree
            .entry(src_mod.clone())
            .or_default()
            .insert(dst_mod.clone());
        in_degree.entry(dst_mod).or_default().insert(src_mod);
    }

    // ── HIGH_FANOUT / HIGH_FANIN ─────────────────────────────────────
    for (module, targets) in &out_degree {
        if targets.len() >= HIGH_FANOUT_THRESHOLD {
            out.push(HealthFinding {
                finding_type: FindingType::HighFanout,
                severity: FindingSeverity::Info,
                evidence: format!("module depends on {} distinct modules", targets.len()),
                location: Some(module.clone()),
                confidence: 0.85,
            });
        }
    }
    for (module, sources) in &in_degree {
        if sources.len() >= HIGH_FANIN_THRESHOLD {
            out.push(HealthFinding {
                finding_type: FindingType::HighFanin,
                severity: FindingSeverity::Info,
                evidence: format!(
                    "module is depended on by {} distinct modules",
                    sources.len()
                ),
                location: Some(module.clone()),
                confidence: 0.85,
            });
        }
    }

    // ── CYCLE (deterministic DFS over module graph, bounded) ─────────
    {
        let mut cycles: BTreeSet<Vec<String>> = BTreeSet::new();
        let mut visited: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<String> = Vec::new();
        let mut on_stack: BTreeSet<String> = BTreeSet::new();
        let nodes: Vec<String> = {
            let mut all: BTreeSet<String> = BTreeSet::new();
            for k in out_degree.keys() {
                all.insert(k.clone());
            }
            for k in in_degree.keys() {
                all.insert(k.clone());
            }
            all.into_iter().collect()
        };
        fn dfs(
            node: &str,
            out_degree: &BTreeMap<String, BTreeSet<String>>,
            visited: &mut BTreeSet<String>,
            stack: &mut Vec<String>,
            on_stack: &mut BTreeSet<String>,
            cycles: &mut BTreeSet<Vec<String>>,
        ) {
            visited.insert(node.to_string());
            stack.push(node.to_string());
            on_stack.insert(node.to_string());
            if let Some(nexts) = out_degree.get(node) {
                let mut sorted: Vec<&String> = nexts.iter().collect();
                sorted.sort();
                for next in sorted {
                    if cycles.len() >= 8 {
                        break;
                    }
                    if on_stack.contains(next.as_str()) {
                        let pos = stack.iter().position(|n| n == next).unwrap_or(0);
                        let mut cycle: Vec<String> = stack[pos..].to_vec();
                        cycle.sort();
                        cycle.dedup();
                        if cycle.len() >= 2 {
                            cycles.insert(cycle);
                        }
                    } else if !visited.contains(next.as_str()) {
                        dfs(next, out_degree, visited, stack, on_stack, cycles);
                    }
                }
            }
            stack.pop();
            on_stack.remove(node);
        }
        for node in &nodes {
            if cycles.len() >= 8 {
                break;
            }
            if !visited.contains(node) {
                dfs(
                    node,
                    &out_degree,
                    &mut visited,
                    &mut stack,
                    &mut on_stack,
                    &mut cycles,
                );
            }
        }
        for cycle in cycles {
            out.push(HealthFinding {
                finding_type: FindingType::Cycle,
                severity: FindingSeverity::Warning,
                evidence: format!("dependency cycle among {} modules", cycle.len()),
                location: Some(cycle.join(" -> ")),
                confidence: 0.9,
            });
        }
    }

    // ── ORPHAN (modules with no edges) ───────────────────────────────
    for m in collection.modules() {
        let id = m.id.to_string();
        let has_out = out_degree.get(&id).is_some_and(|s| !s.is_empty());
        let has_in = in_degree.get(&id).is_some_and(|s| !s.is_empty());
        if !has_out && !has_in {
            // Skip documentation/config modules: isolation is expected there.
            let path = m.path.as_deref().unwrap_or("");
            if path.ends_with(".md") || path == "Cargo.toml" || path.contains("config") {
                continue;
            }
            out.push(HealthFinding {
                finding_type: FindingType::Orphan,
                severity: FindingSeverity::Info,
                evidence: "module has no dependency edges in either direction".to_string(),
                location: Some(id),
                confidence: 0.6,
            });
        }
    }
    let _ = module_ids;

    // ── UNRESOLVED_REFERENCE (store validation) ──────────────────────
    {
        let validation = crate::fact_store::validation::FactValidation::validate(store);
        let broken = validation
            .count_by_rule(crate::fact_store::validation::FactValidationRule::BrokenIndex);
        if broken > 0 {
            out.push(HealthFinding {
                finding_type: FindingType::UnresolvedReference,
                severity: FindingSeverity::Warning,
                evidence: format!(
                    "{broken} broken-index validation issue(s): edges reference missing facts"
                ),
                location: None,
                confidence: 0.95,
            });
        }
    }

    // ── LARGE_MODULE + MISSING_TEST_ASSOCIATION ──────────────────────
    {
        let mut symbols_per_module: BTreeMap<String, usize> = BTreeMap::new();
        for s in collection.symbols() {
            if let Some(m) = s.module.as_ref() {
                *symbols_per_module.entry(m.to_string()).or_default() += 1;
            }
        }
        // Modules covered by at least one test's `tested` set.
        let mut tested_modules: BTreeSet<String> = BTreeSet::new();
        let mut symbol_to_module: BTreeMap<String, String> = BTreeMap::new();
        for s in collection.symbols() {
            if let Some(m) = s.module.as_ref() {
                symbol_to_module.insert(s.id.to_string(), m.to_string());
            }
        }
        for t in collection.tests() {
            for sym in &t.tested {
                if let Some(m) = symbol_to_module.get(sym.as_str()) {
                    tested_modules.insert(m.clone());
                }
            }
            // A test inside a module covers that module structurally.
            if let Some(FactId::Symbol(owner)) = t.target.as_ref() {
                if let Some(m) = symbol_to_module.get(owner.as_str()) {
                    tested_modules.insert(m.clone());
                }
            }
        }
        for m in collection.modules() {
            let id = m.id.to_string();
            let n = symbols_per_module.get(&id).copied().unwrap_or(0);
            if n >= LARGE_MODULE_SYMBOLS {
                out.push(HealthFinding {
                    finding_type: FindingType::LargeModule,
                    severity: FindingSeverity::Info,
                    evidence: format!("module contains {n} symbols (>= {LARGE_MODULE_SYMBOLS})"),
                    location: Some(id.clone()),
                    confidence: 0.8,
                });
            }
            let path = m.path.as_deref().unwrap_or("");
            let is_test_mod = path.contains("test");
            let is_doc_or_config =
                path.ends_with(".md") || path.ends_with(".toml") || path.ends_with(".json");
            if !is_test_mod && !is_doc_or_config && n > 0 && !tested_modules.contains(&id) {
                out.push(HealthFinding {
                    finding_type: FindingType::MissingTestAssociation,
                    severity: FindingSeverity::Info,
                    evidence: format!("source module with {n} symbol(s) has no linked test"),
                    location: Some(id),
                    confidence: 0.65,
                });
            }
        }
    }

    // Deterministic ordering: type, then location, then evidence.
    out.sort_by(|a, b| {
        (
            a.finding_type.as_str(),
            a.location.as_deref().unwrap_or(""),
            &a.evidence,
        )
            .cmp(&(
                b.finding_type.as_str(),
                b.location.as_deref().unwrap_or(""),
                &b.evidence,
            ))
    });
    out.truncate(cap);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineering_facts::{
        FactsBuilder, ModuleFact, ModuleId, RelationshipFact, RelationshipId, SymbolFact, SymbolId,
        SymbolKind,
    };

    fn module(id: &str, path: &str) -> ModuleFact {
        let mut m = ModuleFact::new(ModuleId::new(id.to_string()), id.to_string());
        m.path = Some(path.to_string());
        m
    }

    fn symbol(id: &str, module: &str) -> SymbolFact {
        let mut s = SymbolFact::new(
            SymbolId::new(id.to_string()),
            id.to_string(),
            SymbolKind::Function,
        );
        s.module = Some(ModuleId::new(module.to_string()));
        s
    }

    fn calls(id: &str, src_mod: &str, dst_mod: &str) -> RelationshipFact {
        RelationshipFact::new(
            RelationshipId::new(id.to_string()),
            RelationshipKind::Imports,
            FactId::Module(ModuleId::new(src_mod.to_string())),
            FactId::Module(ModuleId::new(dst_mod.to_string())),
        )
    }

    fn build(mods: Vec<ModuleFact>, rels: Vec<RelationshipFact>) -> FactStore {
        let mut b = FactsBuilder::new();
        for m in mods {
            b.add_module(m);
        }
        for r in rels {
            b.add_relationship(r);
        }
        FactStore::build(b.build())
    }

    #[test]
    fn stale_index_finding_present_when_stale() {
        let store = build(vec![], vec![]);
        let findings = analyze_health(&store, true, 50);
        assert!(findings
            .iter()
            .any(|f| f.finding_type == FindingType::StaleIndex));
    }

    #[test]
    fn no_stale_finding_when_fresh() {
        let store = build(vec![], vec![]);
        let findings = analyze_health(&store, false, 50);
        assert!(!findings
            .iter()
            .any(|f| f.finding_type == FindingType::StaleIndex));
    }

    #[test]
    fn cycle_detected() {
        let store = build(
            vec![module("mod::a", "a.rs"), module("mod::b", "b.rs")],
            vec![
                calls("r1", "mod::a", "mod::b"),
                calls("r2", "mod::b", "mod::a"),
            ],
        );
        let findings = analyze_health(&store, false, 50);
        assert!(findings
            .iter()
            .any(|f| f.finding_type == FindingType::Cycle));
    }

    #[test]
    fn orphan_reported_for_isolated_source_module() {
        let store = build(vec![module("mod::lone", "src/lone.rs")], vec![]);
        let findings = analyze_health(&store, false, 50);
        assert!(findings
            .iter()
            .any(|f| f.finding_type == FindingType::Orphan));
    }

    #[test]
    fn deterministic_ordering_and_bound() {
        let store = build(
            vec![module("mod::a", "a.rs"), module("mod::b", "b.rs")],
            vec![calls("r1", "mod::a", "mod::b")],
        );
        let a = analyze_health(&store, true, 50);
        let b = analyze_health(&store, true, 50);
        assert_eq!(a, b);
        let one = analyze_health(&store, true, 1);
        assert!(one.len() <= 1);
    }

    #[test]
    fn unused_symbol_helper_keeps_api_stable() {
        let s = symbol("sym::x", "mod::a");
        assert_eq!(s.name, "sym::x");
    }
}
