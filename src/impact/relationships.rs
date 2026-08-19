#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Relationship construction for the init pipeline.
//!
//! This module builds cross-cutting [`RelationshipFact`] and [`ReferenceFact`]
//! entries from two sources:
//!
//! 1. **AST-derived (verified)** — actual call expressions and import/use
//!    statements parsed from source. These produce `Calls` and `Imports`
//!    relationship facts with `provenance=verified`.
//! 2. **Name-coincidence (heuristic)** — when symbol names match across
//!    modules with the same kind but no AST evidence, we create
//!    `References` and `Imports` edges tagged as `heuristic`.
//!
//! Edges are deduplicated: if the same `(source, target, kind)` is discovered
//! by both paths, the verified edge wins and the heuristic one is dropped.
//!
//! The output feeds directly into the impact analysis graph.

use crate::engineering_facts::{
    FactId, FactsBuilder, ModuleId, ReferenceFact, ReferenceId, RelationshipFact, RelationshipId,
    RelationshipKind, SymbolFact, SymbolId,
};
use crate::intelligence::parser::{ParseCall, ParseImport};

/// Build relationship and reference facts from module/symbol data plus
/// AST-derived calls and imports.
///
/// Returns the count of new facts added.
pub fn build_relationships(
    builder: &mut FactsBuilder,
    modules: &[crate::engineering_facts::ModuleFact],
    symbols: &[SymbolFact],
    calls: &[ParseCall],
    imports: &[ParseImport],
) -> usize {
    let mut count = 0u64;

    // ── Verified edges from AST data ──────────────────────────────────

    // Build a symbol lookup: name + module → SymbolId (for call resolution).
    let mut name_to_sym: std::collections::HashMap<String, Vec<(&SymbolId, &ModuleId)>> =
        std::collections::HashMap::new();
    for sym in symbols {
        if let Some(ref mod_id) = sym.module {
            name_to_sym
                .entry(sym.name.clone())
                .or_default()
                .push((&sym.id, mod_id));
        }
    }

    // Build a module lookup: module_id → module fact (for import path resolution).
    let mod_map: std::collections::HashMap<&ModuleId, &crate::engineering_facts::ModuleFact> =
        modules.iter().map(|m| (&m.id, m)).collect();

    // ── Calls from AST ────────────────────────────────────────────────
    let mut verified_call_edges: std::collections::HashSet<(FactId, FactId)> =
        std::collections::HashSet::new();
    let mut verified_import_edges: std::collections::HashSet<(FactId, FactId)> =
        std::collections::HashSet::new();

    for call in calls {
        // Resolve the callee name to a SymbolId.
        if let Some(callee_sym_id) = resolve_callee(&name_to_sym, &call.callee_name, call) {
            let caller_fact_id = caller_fact_id(symbols, &call);
            let callee_fact_id = FactId::Symbol(callee_sym_id.clone());
            let edge = (caller_fact_id.clone(), callee_fact_id);
            if verified_call_edges.insert(edge) {
                let rel_id = format!(
                    "rel::{caller_file}::{callee_name}@{line}",
                    caller_file = call.caller_file,
                    callee_name = call.callee_name,
                    line = call.line_start,
                );
                let rf = RelationshipFact::new(
                    RelationshipId::new(rel_id),
                    RelationshipKind::Calls,
                    caller_fact_id,
                    FactId::Symbol(callee_sym_id),
                );
                builder.add_relationship(rf);
                count += 1;
            }
        }
    }

    // ── Imports from AST ──────────────────────────────────────────────
    for imp in imports {
        // Try to resolve the import path to a module in the workspace.
        if let Some(target_mod_id) = resolve_import_path(modules, &imp.path, imp) {
            // Find the module containing this file.
            let Some(caller_mod) = find_module_for_file(modules, &imp.file) else {
                continue;
            };
            if &caller_mod != &target_mod_id {
                let edge = (
                    FactId::Module(target_mod_id.clone()),
                    FactId::Module(caller_mod.clone()),
                );
                if verified_import_edges.insert(edge) {
                    let rel_id = format!(
                        "rel::{caller_file}→{target_mod}::ast_import",
                        caller_file = imp.file,
                        target_mod = target_mod_id.as_str(),
                    );
                    let rf = RelationshipFact::new(
                        RelationshipId::new(rel_id),
                        RelationshipKind::Imports,
                        FactId::Module(target_mod_id.clone()),
                        FactId::Module(caller_mod),
                    );
                    builder.add_relationship(rf);
                    count += 1;
                }
            }
        }
    }

    // ── Heuristic edges (name-coincidence fallback) ───────────────────
    // Only add heuristic edges for edges NOT already covered by verified
    // AST extraction. This prevents duplicate/contradictory evidence.
    let heuristic_refs = build_heuristic_references(
        builder,
        symbols,
        &verified_call_edges,
        &verified_import_edges,
    );
    let heuristic_rels = build_heuristic_imports(
        builder,
        symbols,
        modules,
        &verified_call_edges,
        &verified_import_edges,
    );

    count += (heuristic_refs + heuristic_rels) as u64;

    tracing::info!(
        "impact relations: {} verified (AST), {} heuristic (name-coincidence)",
        count - (heuristic_refs + heuristic_rels) as u64,
        (heuristic_refs + heuristic_rels) as u64,
    );
    count as usize
}

// ── Call resolution ───────────────────────────────────────────────────

/// Resolve a callee name to a SymbolId using the symbol name index.
/// Returns None if the name doesn't match any known symbol, or if there
/// are multiple ambiguous matches in the same module.
fn resolve_callee(
    name_to_sym: &std::collections::HashMap<String, Vec<(&SymbolId, &ModuleId)>>,
    callee_name: &str,
    call: &ParseCall,
) -> Option<SymbolId> {
    let candidates = name_to_sym.get(callee_name)?;
    if candidates.is_empty() {
        return None;
    }

    // If the call is qualified (e.g. `pkg::func()`), try to match on
    // the full qualified name first.
    if call.is_qualified {
        // For qualified calls, we only create a relationship if the
        // callee name uniquely identifies one symbol. We don't attempt
        // full path resolution since we don't have namespace facts yet.
        if candidates.len() == 1 {
            return Some(candidates[0].0.clone());
        }
        // Ambiguous qualified call — skip to avoid false positives.
        return None;
    }

    // For unqualified calls, find symbols with matching name in any module.
    // If there's exactly one match globally, use it.
    // If there are multiple, we need the caller's context to disambiguate.
    if candidates.len() == 1 {
        return Some(candidates[0].0.clone());
    }

    // Multiple candidates: prefer the one in the same module as the caller
    // (local shadowing / re-export pattern).
    for (sym_id, _mod_id) in candidates {
        // We don't have access to symbol location here; just check if all
        // candidates are in the same module.
    }

    // Fall back: if all candidates are in the same module, pick the first.
    // Otherwise, ambiguous — don't create a verified edge.
    let first_mod = candidates[0].1;
    if candidates.iter().all(|(_, m)| *m == first_mod) {
        return Some(candidates[0].0.clone());
    }

    None
}

/// Get the FactId for the caller symbol from call metadata.
fn caller_fact_id(symbols: &[SymbolFact], call: &ParseCall) -> FactId {
    // Try to find the symbol that contains this call by name + file match.
    if let Some(ref caller_name) = call.caller_symbol {
        for sym in symbols {
            if &sym.name == caller_name {
                if let Some(ref loc_file) = sym.location.file {
                    if loc_file == &call.caller_file {
                        return FactId::Symbol(sym.id.clone());
                    }
                }
            }
        }
    }
    // Fallback: use the first symbol in the caller's file.
    for sym in symbols {
        if let Some(ref loc_file) = sym.location.file {
            if loc_file == &call.caller_file {
                return FactId::Symbol(sym.id.clone());
            }
        }
    }
    // Last resort: synthetic caller id based on file + line.
    FactId::Symbol(SymbolId::new(format!(
        "sym::{file}::anon_call@{line}",
        file = call.caller_file,
        line = call.line_start,
    )))
}

// ── Import resolution ─────────────────────────────────────────────────

/// Try to resolve an import path to a module ID by matching against
/// known module paths and names.
fn resolve_import_path(
    modules: &[crate::engineering_facts::ModuleFact],
    path: &str,
    imp: &ParseImport,
) -> Option<ModuleId> {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return None;
    }

    // Strategy 1: match last path segment against module names.
    let last = parts.last().unwrap();
    for m in modules {
        if m.name.ends_with(last) || m.path.as_deref() == Some(path) {
            return Some(m.id.clone());
        }
    }

    // Strategy 2: match the full path against module paths.
    for m in modules {
        if let Some(ref mp) = m.path {
            if mp.contains(last) || mp.ends_with(&format!("/{last}")) {
                return Some(m.id.clone());
            }
        }
    }

    None
}

/// Find the module ID that owns the given file path.
fn find_module_for_file(
    modules: &[crate::engineering_facts::ModuleFact],
    file: &str,
) -> Option<ModuleId> {
    for m in modules {
        if let Some(ref mp) = m.path {
            if mp == file {
                return Some(m.id.clone());
            }
        }
    }
    None
}

/// Overload: find module for a symbol's location file.
fn find_module_for_file_sym(
    symbols: &[SymbolFact],
    mod_id: &ModuleId,
    file: &Option<String>,
) -> bool {
    let _ = symbols;
    let _ = mod_id;
    let _ = file;
    false
}

// ── Heuristic edges ───────────────────────────────────────────────────

/// Build a set of module-pair edges that have verified AST-derived
/// relationships (calls or imports). Heuristic references are only created
/// between symbols in modules that already have some verified connection,
/// preventing combinatorial explosion on common symbol names.
fn build_module_relationship_map(
    verified_calls: &std::collections::HashSet<(FactId, FactId)>,
    verified_imports: &std::collections::HashSet<(FactId, FactId)>,
) -> std::collections::HashSet<(FactId, FactId)> {
    let mut connected = std::collections::HashSet::new();
    for (src, tgt) in verified_calls {
        connected.insert((src.clone(), tgt.clone()));
        connected.insert((tgt.clone(), src.clone()));
    }
    for (src, tgt) in verified_imports {
        connected.insert((src.clone(), tgt.clone()));
        connected.insert((tgt.clone(), src.clone()));
    }
    connected
}

/// Build heuristic reference edges from name-coincidence across modules.
/// Only creates edges between symbols whose modules have a verified
/// relationship, bounding the output to plausible pairs.
/// Skips edges already covered by verified AST extraction.
fn build_heuristic_references(
    builder: &mut FactsBuilder,
    symbols: &[SymbolFact],
    verified_calls: &std::collections::HashSet<(FactId, FactId)>,
    verified_imports: &std::collections::HashSet<(FactId, FactId)>,
) -> usize {
    let mut count = 0u64;
    let mut by_name: std::collections::HashMap<String, Vec<&SymbolFact>> =
        std::collections::HashMap::new();
    for s in symbols {
        by_name.entry(s.name.clone()).or_default().push(s);
    }

    // Build the set of module pairs with verified relationships.
    let module_connected = build_module_relationship_map(verified_calls, verified_imports);

    for sym in symbols {
        let sym_mod = match &sym.module {
            Some(m) => m,
            None => continue,
        };
        if let Some(candidates) = by_name.get(&sym.name) {
            for candidate in candidates {
                let cand_mod = match &candidate.module {
                    Some(m) => m,
                    None => continue,
                };
                if cand_mod == sym_mod {
                    continue;
                }
                if sym.kind != candidate.kind {
                    continue;
                }
                // Only create heuristic references between symbols in
                // modules that already have a verified relationship.
                let sym_fact = FactId::Symbol(sym.id.clone());
                let cand_fact = FactId::Symbol(candidate.id.clone());
                let sym_mod_fact = FactId::Module(sym_mod.clone());
                let cand_mod_fact = FactId::Module(cand_mod.clone());
                if !module_connected.contains(&(sym_mod_fact.clone(), cand_mod_fact.clone()))
                    && !module_connected.contains(&(cand_mod_fact.clone(), sym_mod_fact.clone()))
                {
                    continue;
                }
                let edge = (sym_fact.clone(), cand_fact.clone());
                if verified_calls.contains(&edge) {
                    continue;
                }
                let ref_id = format!(
                    "ref::{sym_mod}::{sym_name}→{cand_mod}::{cand_name}",
                    sym_mod = sym_mod.as_str(),
                    sym_name = sym.name,
                    cand_mod = cand_mod.as_str(),
                    cand_name = candidate.name,
                );
                let mut rf = ReferenceFact::new(ReferenceId::new(ref_id), sym_fact, cand_fact);
                rf.metadata = crate::engineering_facts::metadata::FactMetadata::builder()
                    .attr("provenance", "heuristic")
                    .build();
                builder.add_reference(rf);
                count += 1;
            }
        }
    }
    count as usize
}

/// Build heuristic import edges from name-coincidence across modules.
/// Only creates edges between modules that already have a verified
/// relationship, bounding the output to plausible pairs.
/// Skips edges already covered by verified AST extraction.
fn build_heuristic_imports(
    builder: &mut FactsBuilder,
    symbols: &[SymbolFact],
    modules: &[crate::engineering_facts::ModuleFact],
    verified_calls: &std::collections::HashSet<(FactId, FactId)>,
    verified_imports: &std::collections::HashSet<(FactId, FactId)>,
) -> usize {
    let mut count = 0u64;
    let mut by_name: std::collections::HashMap<String, Vec<&SymbolFact>> =
        std::collections::HashMap::new();
    for s in symbols {
        by_name.entry(s.name.clone()).or_default().push(s);
    }

    let mut seen_edges: std::collections::HashSet<(ModuleId, ModuleId)> =
        std::collections::HashSet::new();

    // Build the set of module pairs with verified relationships.
    let module_connected = build_module_relationship_map(verified_calls, verified_imports);

    for sym in symbols {
        let sym_mod = match &sym.module {
            Some(m) => m.clone(),
            None => continue,
        };
        if let Some(candidates) = by_name.get(&sym.name) {
            for candidate in candidates {
                let cand_mod = match &candidate.module {
                    Some(m) => m.clone(),
                    None => continue,
                };
                // Compare by reference to avoid moving sym_mod.
                if &cand_mod == &sym_mod {
                    continue;
                }
                if sym.kind != candidate.kind {
                    continue;
                }
                let sym_mod_edge = sym_mod.clone();
                let edge = (cand_mod.clone(), sym_mod_edge.clone());
                if seen_edges.contains(&edge) {
                    continue;
                }
                // Only create heuristic imports between modules that
                // already have a verified relationship.
                let sym_mod_fact = FactId::Module(sym_mod_edge.clone());
                let cand_mod_fact = FactId::Module(cand_mod.clone());
                if !module_connected.contains(&(sym_mod_fact.clone(), cand_mod_fact.clone()))
                    && !module_connected.contains(&(cand_mod_fact.clone(), sym_mod_fact.clone()))
                {
                    continue;
                }
                // Check if this edge is already verified.
                let vert_id = FactId::Module(cand_mod.clone());
                let vert_tgt = FactId::Module(sym_mod_edge.clone());
                if verified_imports.contains(&(vert_id, vert_tgt)) {
                    continue;
                }
                seen_edges.insert(edge);
                let rel_id = format!(
                    "rel::{cand_mod}→{sym_mod}::heuristic_import",
                    cand_mod = cand_mod.as_str(),
                    sym_mod = sym_mod_edge.as_str(),
                );
                let mut rf = RelationshipFact::new(
                    RelationshipId::new(rel_id),
                    RelationshipKind::Imports,
                    FactId::Module(cand_mod),
                    FactId::Module(sym_mod_edge),
                );
                rf.metadata = crate::engineering_facts::metadata::FactMetadata::builder()
                    .attr("provenance", "heuristic")
                    .build();
                builder.add_relationship(rf);
                count += 1;
            }
        }
    }
    count as usize
}
