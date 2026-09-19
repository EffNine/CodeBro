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

/// An `impl` block's textual extent, used to attribute method calls to
/// their self type (e.g. calls inside `impl User` belong to `User`).
#[derive(Debug, Clone)]
pub struct ImplScope {
    pub file: String,
    pub start: u32,
    pub end: u32,
    pub type_name: String,
}

/// The innermost impl type whose block contains `file:line`.
fn enclosing_impl<'a>(scopes: &'a [ImplScope], file: &str, line: u32) -> Option<&'a str> {
    scopes
        .iter()
        .filter(|s| s.file == file && s.start <= line && line <= s.end)
        .min_by_key(|s| s.end.saturating_sub(s.start))
        .map(|s| s.type_name.as_str())
}

/// The impl type owning a candidate symbol, via span containment.
fn symbol_impl<'a>(scopes: &'a [ImplScope], sym: &SymbolFact) -> Option<&'a str> {
    let file = sym.location.file.as_deref()?;
    let line = sym.location.line?;
    enclosing_impl(scopes, file, line)
}

/// Build relationship and reference facts from module/symbol data plus
/// AST-derived calls and imports.
///
/// Returns the count of new facts added plus the set of verified call
/// edges as `(caller, callee)` symbol-id pairs. Callers (e.g. the init
/// pipeline) use these to link tests to the symbols they exercise.
pub fn build_relationships(
    builder: &mut FactsBuilder,
    modules: &[crate::engineering_facts::ModuleFact],
    symbols: &[SymbolFact],
    calls: &[ParseCall],
    imports: &[ParseImport],
    impl_scopes: &[ImplScope],
) -> (usize, Vec<(FactId, FactId)>) {
    let mut count = 0u64;
    let mut verified_call_edges: Vec<(FactId, FactId)> = Vec::new();

    // ── Verified edges from AST data ──────────────────────────────────

    // Symbol lookup by name (for call resolution). Candidates keep the
    // full fact so receiver/impl matching can inspect location and kind.
    let mut name_to_sym: std::collections::HashMap<String, Vec<&SymbolFact>> =
        std::collections::HashMap::new();
    for sym in symbols {
        if sym.module.is_some() {
            name_to_sym.entry(sym.name.clone()).or_default().push(sym);
        }
    }

    // Build a module lookup: module_id → module fact (for import path resolution).
    let mod_map: std::collections::HashMap<&ModuleId, &crate::engineering_facts::ModuleFact> =
        modules.iter().map(|m| (&m.id, m)).collect();

    // ── Calls from AST ────────────────────────────────────────────────
    let mut verified_call_set: std::collections::HashSet<(FactId, FactId)> =
        std::collections::HashSet::new();
    let mut verified_import_edges: std::collections::HashSet<(FactId, FactId)> =
        std::collections::HashSet::new();

    for call in calls {
        // Resolve the callee name to a SymbolId, preferring receiver-type
        // and enclosing-impl matches over bare-name coincidence.
        let caller_impl = enclosing_impl(impl_scopes, &call.caller_file, call.line_start);
        if let Some(callee_sym_id) = resolve_callee(
            &name_to_sym,
            &call.callee_name,
            call,
            impl_scopes,
            caller_impl,
        ) {
            // Skip edges whose caller cannot be resolved to a known symbol
            // fact — dangling endpoints would break store validation and
            // pollute the impact graph.
            let Some(caller_fact_id) = caller_fact_id(symbols, &call) else {
                continue;
            };
            let callee_fact_id = FactId::Symbol(callee_sym_id.clone());
            let edge = (caller_fact_id.clone(), callee_fact_id);
            if verified_call_set.insert(edge.clone()) {
                verified_call_edges.push(edge.clone());
                // Disambiguator from the resolved target: two calls can
                // share file+line+name (`Ping::new(); Pong::new();`) yet
                // resolve to different symbols; the id must not collide.
                let mut h: u64 = 0x811c_9dc5;
                for b in callee_sym_id.as_str().bytes() {
                    h ^= b as u64;
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
                let rel_id = format!(
                    "rel::{caller_file}::{callee_name}@{line}#{h:016x}",
                    caller_file = call.caller_file,
                    callee_name = call.callee_name,
                    line = call.line_start,
                    h = h,
                );
                // Macro-text calls are syntactic recovery, not AST nodes —
                // they carry heuristic provenance so trust math stays honest.
                let provenance = if call.from_macro_text {
                    "heuristic"
                } else {
                    "verified"
                };
                let mut rf = RelationshipFact::new(
                    RelationshipId::new(rel_id),
                    RelationshipKind::Calls,
                    caller_fact_id,
                    FactId::Symbol(callee_sym_id),
                );
                rf.metadata = crate::engineering_facts::metadata::FactMetadata::builder()
                    .attr("provenance", provenance)
                    .build();
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
                // Direction convention: source imports target — the
                // importer module is the edge source, mirroring `Calls`
                // (caller → callee) and `dep::<a>-><b>` dependency ids.
                let edge = (
                    FactId::Module(caller_mod.clone()),
                    FactId::Module(target_mod_id.clone()),
                );
                if verified_import_edges.insert(edge) {
                    let rel_id = format!(
                        "rel::{caller_file}→{target_mod}::ast_import",
                        caller_file = imp.file,
                        target_mod = target_mod_id.as_str(),
                    );
                    let mut rf = RelationshipFact::new(
                        RelationshipId::new(rel_id),
                        RelationshipKind::Imports,
                        FactId::Module(caller_mod),
                        FactId::Module(target_mod_id.clone()),
                    );
                    rf.metadata = crate::engineering_facts::metadata::FactMetadata::builder()
                        .attr("provenance", "verified")
                        .build();
                    builder.add_relationship(rf);
                    count += 1;
                }
            }
        }
    }

    // ── Heuristic edges (name-coincidence fallback) ───────────────────
    // Only add heuristic edges for edges NOT already covered by verified
    // AST extraction. This prevents duplicate/contradictory evidence.
    let heuristic_refs =
        build_heuristic_references(builder, symbols, &verified_call_set, &verified_import_edges);
    let heuristic_rels = build_heuristic_imports(
        builder,
        symbols,
        modules,
        &verified_call_set,
        &verified_import_edges,
    );

    count += (heuristic_refs + heuristic_rels) as u64;

    tracing::info!(
        "impact relations: {} verified (AST), {} heuristic (name-coincidence)",
        count - (heuristic_refs + heuristic_rels) as u64,
        (heuristic_refs + heuristic_rels) as u64,
    );
    (count as usize, verified_call_edges)
}

// ── Call resolution ───────────────────────────────────────────────────

/// Resolve a callee name to a SymbolId.
///
/// Resolution order (most confident first; ambiguity always skips — the
/// graph must never invent evidence):
///
/// 1. **Receiver-typed** — the call names its type (`User::new()`,
///    `self.save()`, `Self::help()`): match candidates owned by that
///    impl type. A unique typed match wins even when other types define
///    the same method name.
/// 2. **Enclosing-impl preference** — unqualified calls inside
///    `impl X` prefer methods of `X`.
/// 3. **Bare-name fallback** — exactly one global candidate, or all
///    candidates in one module.
fn resolve_callee(
    name_to_sym: &std::collections::HashMap<String, Vec<&SymbolFact>>,
    callee_name: &str,
    call: &ParseCall,
    impl_scopes: &[ImplScope],
    caller_impl: Option<&str>,
) -> Option<SymbolId> {
    let candidates = name_to_sym.get(callee_name)?;
    if candidates.is_empty() {
        return None;
    }
    let owner_impl = |sym: &SymbolFact| symbol_impl(impl_scopes, sym);

    // The type this call is explicitly addressed to, if any.
    let wanted_impl: Option<String> = match call.receiver_type.as_deref() {
        Some("self") | Some("Self") => caller_impl.map(str::to_string),
        Some(other) => Some(other.to_string()),
        None => None,
    };

    if let Some(want) = &wanted_impl {
        let mut typed: Vec<SymbolId> = candidates
            .iter()
            .filter(|s| owner_impl(s) == Some(want.as_str()))
            .map(|s| s.id.clone())
            .collect();
        typed.sort();
        typed.dedup();
        if !typed.is_empty() {
            return typed.first().filter(|_| typed.len() == 1).cloned();
        }
        // The qualifier IS a known impl type but owns no such method —
        // genuinely unresolvable; never invent a cross-type edge.
        let known_type = impl_scopes.iter().any(|s| &s.type_name == want);
        if known_type {
            return None;
        }
        // Otherwise the qualifier is a namespace segment (e.g.
        // `crate::util::helper`) — fall through to bare-name rules.
    }

    if !call.is_qualified {
        if let Some(impl_type) = caller_impl {
            let mut typed: Vec<SymbolId> = candidates
                .iter()
                .filter(|s| owner_impl(s) == Some(impl_type))
                .map(|s| s.id.clone())
                .collect();
            typed.sort();
            typed.dedup();
            if typed.len() == 1 {
                return typed.into_iter().next();
            }
        }
    }

    if call.is_qualified {
        // Qualified with an untyped receiver (`obj.method()`,
        // `items.iter().any(..)`): the receiver's type is unknown, so a
        // bare-name match against an unrelated same-named function is
        // guesswork, not evidence. Skip — no invented edges.
        if call.receiver_type.is_none() {
            return None;
        }
        // Namespace-style qualifier that named no known impl (e.g.
        // `crate::util::helper`): only a unique global candidate is
        // trustworthy.
        return (candidates.len() == 1).then(|| candidates[0].id.clone());
    }

    // Unqualified bare-name fallback: unique globally, or all in one module.
    if candidates.len() == 1 {
        return Some(candidates[0].id.clone());
    }
    let first_mod = candidates[0].module.clone();
    candidates
        .iter()
        .all(|s| s.module == first_mod)
        .then(|| candidates[0].id.clone())
}

/// Get the FactId for the caller symbol from call metadata. Returns `None`
/// when the caller cannot be resolved to a known symbol fact; the edge is
/// then skipped so the store never holds a dangling endpoint.
fn caller_fact_id(symbols: &[SymbolFact], call: &ParseCall) -> Option<FactId> {
    // Try to find the symbol that contains this call by name + file match.
    if let Some(ref caller_name) = call.caller_symbol {
        for sym in symbols {
            if &sym.name == caller_name {
                if let Some(ref loc_file) = sym.location.file {
                    if loc_file == &call.caller_file {
                        return Some(FactId::Symbol(sym.id.clone()));
                    }
                }
            }
        }
    }
    // Fallback: use the first symbol in the caller's file.
    for sym in symbols {
        if let Some(ref loc_file) = sym.location.file {
            if loc_file == &call.caller_file {
                return Some(FactId::Symbol(sym.id.clone()));
            }
        }
    }
    // Unresolvable caller (e.g. a file with no extracted symbols): drop the
    // edge rather than emit a synthetic id that resolves to no fact.
    None
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

// ── Heuristic edges ───────────────────────────────────────────────────

/// Build a set of module-pair edges that have verified AST-derived
/// relationships (calls or imports). Heuristic references are only created
/// between symbols in modules that already have some verified connection,
/// preventing combinatorial explosion on common symbol names.
///
/// Verified calls connect *symbols*, so they are projected into module
/// space via each symbol's owning module before gating module-pair checks.
fn build_module_relationship_map(
    symbols: &[SymbolFact],
    verified_calls: &std::collections::HashSet<(FactId, FactId)>,
    verified_imports: &std::collections::HashSet<(FactId, FactId)>,
) -> std::collections::HashSet<(FactId, FactId)> {
    let module_of: std::collections::HashMap<&str, &ModuleId> = symbols
        .iter()
        .filter_map(|s| s.module.as_ref().map(|m| (s.id.as_str(), m)))
        .collect();
    let mut connected = std::collections::HashSet::new();
    for (src, tgt) in verified_calls {
        if let (FactId::Symbol(s), FactId::Symbol(t)) = (src, tgt) {
            if let (Some(sm), Some(tm)) = (module_of.get(s.as_str()), module_of.get(t.as_str())) {
                if sm != tm {
                    connected
                        .insert((FactId::Module((*sm).clone()), FactId::Module((*tm).clone())));
                    connected
                        .insert((FactId::Module((*tm).clone()), FactId::Module((*sm).clone())));
                }
            }
        }
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
    let module_connected = build_module_relationship_map(symbols, verified_calls, verified_imports);

    // Reference ids are keyed by (source module, name, target module,
    // name); several same-name symbol facts in one module would otherwise
    // emit identical ids and trip duplicate-facts validation.
    let mut seen_ref_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

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
                if !seen_ref_ids.insert(ref_id.clone()) {
                    continue;
                }
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
    let module_connected = build_module_relationship_map(symbols, verified_calls, verified_imports);

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
                // Name coincidence carries no true direction; orient the
                // edge deterministically (lexicographically smaller module
                // id first) so the id string matches the stored direction.
                let (src_mod, tgt_mod) = if cand_mod.as_str() <= sym_mod.as_str() {
                    (cand_mod.clone(), sym_mod.clone())
                } else {
                    (sym_mod.clone(), cand_mod.clone())
                };
                let edge = (src_mod.clone(), tgt_mod.clone());
                if seen_edges.contains(&edge) {
                    continue;
                }
                // Only create heuristic imports between modules that
                // already have a verified relationship.
                let src_fact = FactId::Module(src_mod.clone());
                let tgt_fact = FactId::Module(tgt_mod.clone());
                if !module_connected.contains(&(src_fact.clone(), tgt_fact.clone()))
                    && !module_connected.contains(&(tgt_fact.clone(), src_fact.clone()))
                {
                    continue;
                }
                // Check if this edge is already verified (either orientation).
                if verified_imports.contains(&(src_fact.clone(), tgt_fact.clone()))
                    || verified_imports.contains(&(tgt_fact.clone(), src_fact.clone()))
                {
                    continue;
                }
                seen_edges.insert(edge);
                let rel_id = format!(
                    "rel::{src_mod}→{tgt_mod}::heuristic_import",
                    src_mod = src_mod.as_str(),
                    tgt_mod = tgt_mod.as_str(),
                );
                let mut rf = RelationshipFact::new(
                    RelationshipId::new(rel_id),
                    RelationshipKind::Imports,
                    FactId::Module(src_mod),
                    FactId::Module(tgt_mod),
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
