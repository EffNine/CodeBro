//! Deterministic candidate generation from verified evidence.
//!
//! Every candidate must be anchored to something the runtime already
//! observed: a diagnostic location, a failing test's tested-symbol linkage,
//! or a recently edited path. Ambiguity is preserved, never resolved by
//! guessing.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use super::types::{Candidate, MAX_CANDIDATES, MAX_DIAG_LOCATIONS};
use crate::fact_store::FactStore;
use crate::sandbox::ParsedDiagnostic;

fn norm(p: &str) -> String {
    p.trim_start_matches("./").trim_end_matches('/').to_string()
}

/// A diagnostic distilled to the fields candidate generation needs.
#[derive(Debug, Clone)]
pub struct DiagnosticInput {
    pub file: Option<String>,
    pub line: Option<u32>,
    /// Failing test name when the runner identified one.
    pub test: Option<String>,
    pub severity: String,
}

/// A recent edit relevant to correlation (already converted to seconds).
#[derive(Debug, Clone)]
pub struct RecentEditInput {
    pub path: String,
    pub seconds_ago: u64,
    pub recommended_tests: Vec<String>,
}

/// Candidates with their generation anchors, deduplicated and capped.
#[derive(Debug, Default)]
pub struct CandidateSet {
    /// Ordered deterministically: insertion order (evidence priority), id asc.
    pub candidates: Vec<Candidate>,
    /// Names that map to more than one symbol id — ambiguity is surfaced,
    /// never silently resolved.
    pub ambiguous_names: std::collections::HashSet<String>,
    /// symbol_id → failing test names exercising it.
    pub exercised_by: std::collections::HashMap<String, Vec<String>>,
    /// Failing test name → its own function symbol id (when known).
    pub test_self: std::collections::HashMap<String, String>,
    /// Failing test name → symbols it exercises via `TestFact.tested`.
    pub test_exercises: std::collections::HashMap<String, Vec<String>>,
}

/// Build a candidate from a symbol fact, preserving its full span.
fn cand(sym: &crate::engineering_facts::SymbolFact) -> Candidate {
    let file = sym.location.file.clone().map(|f| norm(&f));
    Candidate {
        symbol_id: sym.id.as_str().to_string(),
        name: sym.name.clone(),
        line: sym.location.line,
        end_line: sym.location.span.as_ref().map(|sp| sp.end.line),
        file,
    }
}

impl CandidateSet {
    fn push(&mut self, c: Candidate) {
        if self.candidates.len() >= MAX_CANDIDATES {
            return;
        }
        if !self.candidates.iter().any(|x| x.symbol_id == c.symbol_id) {
            self.candidates.push(c);
        }
    }
}

/// Generate candidates in fixed priority order:
/// 1. exact diagnostic locations (file+line inside a symbol span)
/// 2. failing tests' own symbols and exercised symbols (`tested`)
/// 3. symbols inside recently edited files
pub fn generate(
    store: &FactStore,
    diagnostics: &[DiagnosticInput],
    recent_edits: &[RecentEditInput],
) -> CandidateSet {
    let mut set = CandidateSet::default();
    let collection = store.collection();
    let norm = |p: &str| p.trim_start_matches("./").to_string();

    // --- 1. exact diagnostic locations ------------------------------------
    for diag in diagnostics.iter().take(MAX_DIAG_LOCATIONS) {
        let Some(file) = diag.file.as_deref() else {
            continue;
        };
        let file_n = norm(file);
        let line = diag.line;
        for sym in collection.symbols() {
            let Some(sym_file) = sym.location.file.as_deref() else {
                continue;
            };
            if norm(sym_file) != file_n {
                continue;
            }
            let start = sym.location.line.unwrap_or(0);
            let end = sym
                .location
                .span
                .as_ref()
                .map(|sp| sp.end.line)
                .unwrap_or(start);
            let hits_span = line.is_some_and(|l| l >= start && l <= end);
            if hits_span || line.is_none() {
                let mut cd = cand(sym);
                cd.file = Some(file_n.clone());
                set.push(cd);
            }
        }
        // Panic messages carry a thread/test name that may match a test fact.
        if let Some(test_name) = &diag.test {
            register_failing_test(&mut set, store, test_name);
        }
    }

    // --- 2. failing tests → tested symbols ---------------------------------
    for diag in diagnostics.iter().take(MAX_DIAG_LOCATIONS) {
        if let Some(test_name) = &diag.test {
            register_failing_test(&mut set, store, test_name);
        }
    }

    // --- 3. recently edited files ------------------------------------------
    for edit in recent_edits.iter().take(MAX_DIAG_LOCATIONS) {
        let file_n = norm(&edit.path);
        for sym in collection.symbols() {
            let sym_file_norm = sym.location.file.as_deref().map(norm);
            let matches_file = sym_file_norm.as_deref() == Some(file_n.as_str());
            if matches_file {
                let mut cd = cand(sym);
                cd.file = Some(file_n.clone());
                set.push(cd);
            }
        }
    }

    // Ambiguity marking: identical names across distinct symbol ids.
    let mut seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for c in &set.candidates {
        match seen.get(c.name.as_str()) {
            Some(first_id) if *first_id != c.symbol_id => {
                set.ambiguous_names.insert(c.name.clone());
            }
            _ => {
                seen.insert(c.name.as_str(), c.symbol_id.as_str());
            }
        }
    }

    set.candidates.sort_by(|a, b| a.symbol_id.cmp(&b.symbol_id));
    set
}

impl CandidateSet {
    /// Map a runner-reported failing test name (which may carry a module
    /// path, e.g. `tests::adds`) onto the canonical `TestFact` names.
    pub fn canonical_failing_names(&self, runner_name: &str) -> Vec<String> {
        let matches =
            |n: &str| n == runner_name || n.ends_with(runner_name) || runner_name.ends_with(n);
        let mut v: Vec<String> = self
            .test_self
            .keys()
            .chain(self.test_exercises.keys())
            .filter(|k| matches(k))
            .cloned()
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

/// Register a failing test's own symbol plus everything it exercises.
fn register_failing_test(set: &mut CandidateSet, store: &FactStore, test_name: &str) {
    // Suffix matching is vacuous for blank names (`ends_with("")` is always
    // true), which would register EVERY test fact as failing — fabricated
    // evidence plus an unbounded registration cost. Ignore them outright.
    let test_name = test_name.trim();
    if test_name.is_empty() {
        return;
    }
    let collection = store.collection();
    let matches_name = |n: &str| n == test_name || n.ends_with(test_name) || test_name.ends_with(n);
    for tf in collection.tests() {
        if !matches_name(&tf.name) {
            continue;
        }
        if let Some(crate::engineering_facts::FactId::Symbol(own)) = &tf.target {
            set.test_self
                .insert(tf.name.clone(), own.as_str().to_string());
            if let Some(sym) = collection
                .symbols()
                .iter()
                .find(|s| s.id.as_str() == own.as_str())
            {
                set.push(cand(sym));
            }
        }
        for sid in &tf.tested {
            set.test_exercises
                .entry(tf.name.clone())
                .or_default()
                .push(sid.as_str().to_string());
            set.exercised_by
                .entry(sid.as_str().to_string())
                .or_default()
                .push(tf.name.clone());
            if let Some(sym) = collection
                .symbols()
                .iter()
                .find(|s| s.id.as_str() == sid.as_str())
            {
                set.push(cand(sym));
            }
        }
    }
}

/// Convenience: build `DiagnosticInput`s from sandbox-runtime parsed
/// diagnostics without reparsing raw output.
pub fn diagnostics_from(diagnostics: &[ParsedDiagnostic]) -> Vec<DiagnosticInput> {
    diagnostics
        .iter()
        .map(|d| DiagnosticInput {
            file: d.file.clone(),
            line: d.line,
            test: d.test.clone(),
            severity: d.severity.clone(),
        })
        .collect()
}
