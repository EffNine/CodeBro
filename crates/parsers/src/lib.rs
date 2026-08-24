//! CodeBro parsers — tree-sitter parser platform.
//!
//! Provides language parsing and symbol/index extraction for the languages
//! CodeBro understands. The indexer crate drives this platform to produce
//! engineering facts; nothing else in the workspace parses source directly.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

pub mod intelligence;
