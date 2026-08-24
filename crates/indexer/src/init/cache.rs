//! Content-addressed parse cache.
//!
//! Tree-sitter parsing dominates indexing cost. This cache stores parsed
//! results keyed by **content digest**, so unchanged files are never
//! re-parsed across runs and reverted files restore their previous entry.
//! Entries are written atomically; any cache failure is non-fatal (the
//! pipeline falls back to parsing).

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::intelligence::parser::tree_sitter::ParseResult;

const SCHEMA: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct CachedParse {
    schema: u32,
    language: String,
    result: ParseResult,
}

/// Load a cached parse for a content digest. Returns `None` on any miss,
/// corruption or language mismatch — callers re-parse transparently.
pub fn load(dir: &Path, digest: &str, language: &str) -> Option<ParseResult> {
    let path = entry_path(dir, digest);
    let bytes = std::fs::read(path).ok()?;
    let cached: CachedParse = serde_json::from_slice(&bytes).ok()?;
    if cached.schema != SCHEMA || cached.language != language {
        return None;
    }
    Some(cached.result)
}

/// Store a parse result atomically. Best-effort: IO errors are swallowed so
/// indexing never fails because of the cache.
pub fn store(dir: &Path, digest: &str, language: &str, result: &ParseResult) {
    let path = entry_path(dir, digest);
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let cached = CachedParse {
        schema: SCHEMA,
        language: language.to_string(),
        result: Clone::clone(result),
    };
    if let Ok(bytes) = serde_json::to_vec(&cached) {
        // Re-serialising through the same schema guarantees a valid entry;
        // SymbolKind round-trips as part of ParseResult.
        let _ = codebro_core::persistence::write_atomic(&path, &bytes);
    }
}

fn entry_path(dir: &Path, digest: &str) -> PathBuf {
    dir.join(format!("{digest}.json"))
}
