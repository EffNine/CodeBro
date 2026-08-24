#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Language facts — detected repository languages.
//!
//! One aggregate record per distinct source language found in the
//! repository, with deterministic file/line totals. Pure engineering
//! knowledge: no parser internals.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::ids::LanguageId;
use crate::engineering_facts::metadata::FactMetadata;

/// A repository language surface — aggregated over all scanned source files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageFact {
    pub id: LanguageId,
    /// Canonical language name ("rust", "python", ...).
    pub name: String,
    /// Number of source files detected for this language.
    pub file_count: u64,
    /// Total line count across those files.
    pub line_count: u64,
    pub metadata: FactMetadata,
}

impl LanguageFact {
    pub fn new(id: LanguageId, name: impl Into<String>) -> Self {
        LanguageFact {
            id,
            name: name.into(),
            file_count: 0,
            line_count: 0,
            metadata: FactMetadata::new(),
        }
    }
}
