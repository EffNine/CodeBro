//! P6 file intelligence: normalized file model.
//!
//! A file is the smallest unit of repository structure CodeBro reasons
//! about deterministically. This module never reads file *contents* into
//! the index (hashes only) and never invents symbols: for languages
//! without tree-sitter support it preserves file-level intelligence
//! (path, language, size, hash, classification) and reports the parser
//! limitation explicitly.
//!
//! ```text
//! Repository
//!   └── FileRecord (stable id `file::<rel>`, hash, language, classes)
//!         ├── source / test / generated / config / doc / build / ci
//!         └── parser_supported: bool (+ limitation string when false)
//! ```
//!
//! Workspace isolation: records carry workspace-relative paths only.
//! Absolute paths never enter the model.

#![allow(dead_code, unused_imports)]

use serde::{Deserialize, Serialize};

use super::parser::languages::{
    file_language_from_extension, is_parser_supported, parser_limitation,
};

/// Stable file identity: `file::<workspace-relative-path>`.
pub fn file_id(rel: &str) -> String {
    format!("file::{rel}")
}

/// Normalized file classification. A file may carry several classes
/// (e.g. a Rust integration test is both `source` and `test`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileClass {
    Source,
    Test,
    Generated,
    Config,
    Documentation,
    Build,
    Ci,
}

impl FileClass {
    pub fn as_str(self) -> &'static str {
        match self {
            FileClass::Source => "source",
            FileClass::Test => "test",
            FileClass::Generated => "generated",
            FileClass::Config => "config",
            FileClass::Documentation => "documentation",
            FileClass::Build => "build",
            FileClass::Ci => "ci",
        }
    }
}

/// Normalized file intelligence record.
///
/// Content hashes (SHA-256 hex) drive incremental change detection.
/// Full contents are never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    /// Stable identity `file::<rel>`.
    pub id: String,
    /// Workspace-relative path (canonical, `/`-separated).
    pub path: String,
    /// File-level language label (`rust`, `c`, `shell`, `toml`, …).
    /// `None` for genuinely unknown extensions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// File size in bytes (metadata, no read required).
    pub size_bytes: u64,
    /// SHA-256 hex of file contents.
    pub hash: String,
    /// Line count (from the hashed read; 0 when unreadable).
    pub line_count: u64,
    /// Deterministic sorted classification set.
    pub classes: Vec<FileClass>,
    /// True when tree-sitter symbol extraction applies.
    pub parser_supported: bool,
    /// Human-readable limitation when `parser_supported` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parser_limitation: Option<String>,
    /// Unix seconds when this record was indexed.
    pub last_indexed: u64,
}

impl FileRecord {
    /// Deterministic ordering key: canonical path.
    pub fn sort_key(&self) -> &str {
        &self.path
    }

    pub fn is_test(&self) -> bool {
        self.classes.contains(&FileClass::Test)
    }

    pub fn is_generated(&self) -> bool {
        self.classes.contains(&FileClass::Generated)
    }

    pub fn is_source(&self) -> bool {
        self.classes.contains(&FileClass::Source)
    }
}

/// Classify a workspace-relative path + optional content probe.
///
/// `file_name` is the bare file name; `rel` the workspace-relative path;
/// `content_head` is an optional prefix of the file (first ~4 KiB) used
/// only for generated markers — never stored.
pub fn classify(rel: &str, file_name: &str, content_head: Option<&str>) -> Vec<FileClass> {
    let mut classes = Vec::new();
    let lower = rel.to_ascii_lowercase();
    let name_lower = file_name.to_ascii_lowercase();

    // Documentation.
    if is_doc_path(&lower) {
        classes.push(FileClass::Documentation);
    }
    // CI.
    if is_ci_path(&lower) {
        classes.push(FileClass::Ci);
    }
    // Build files.
    if is_build_file(&name_lower, &lower) {
        classes.push(FileClass::Build);
    }
    // Config files.
    if is_config_file(&name_lower, &lower) {
        classes.push(FileClass::Config);
    }
    // Generated markers: path segments + content head.
    if is_generated_path(&lower) || content_head.is_some_and(is_generated_content) {
        classes.push(FileClass::Generated);
    }
    // Tests: path or name heuristics (symbol-level `is_test` refines this).
    if is_test_path(&lower, &name_lower) {
        classes.push(FileClass::Test);
    }
    // Source: parsed languages + file-level code languages.
    if is_source_path(&lower) {
        classes.push(FileClass::Source);
    }

    classes.sort_by_key(|c| c.as_str());
    classes.dedup_by_key(|c| c.as_str());
    classes
}

/// Build a [`FileRecord`] from already-read content. The caller owns the
/// read (size-gated upstream); this function hashes + counts lines +
/// classifies deterministically.
pub fn record_for_content(rel: &str, size_bytes: u64, content: &str, now_secs: u64) -> FileRecord {
    let ext = rel.rsplit('.').next().unwrap_or("");
    // `rel` without extension == extension (e.g. `Makefile`): treat as no ext.
    let ext = if ext == rel { "" } else { ext };
    let language = file_language_from_extension(ext).map(str::to_string);
    let parser_supported = language.as_deref().is_some_and(is_parser_supported);
    let parser_limitation = language
        .as_deref()
        .and_then(parser_limitation)
        .map(str::to_string);
    let file_name = rel.rsplit('/').next().unwrap_or(rel);
    let head: String = content.chars().take(4096).collect();
    let classes = classify(rel, file_name, Some(&head));
    let hash = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(content.as_bytes());
        format!("{:x}", h.finalize())
    };
    FileRecord {
        id: file_id(rel),
        path: rel.to_string(),
        language,
        size_bytes,
        hash,
        line_count: content.lines().count() as u64,
        classes,
        parser_supported,
        parser_limitation,
        last_indexed: now_secs,
    }
}

fn is_doc_path(lower_rel: &str) -> bool {
    lower_rel.ends_with(".md")
        || lower_rel.ends_with(".markdown")
        || lower_rel.ends_with(".rst")
        || lower_rel.ends_with(".txt") && lower_rel.contains("readme")
        || lower_rel.starts_with("docs/")
        || lower_rel.contains("/docs/")
}

fn is_ci_path(lower_rel: &str) -> bool {
    lower_rel.starts_with(".github/workflows/")
        || lower_rel.starts_with(".gitlab-ci")
        || lower_rel == ".github/workflows"
        || lower_rel.contains("ci/")
            && (lower_rel.ends_with(".yml") || lower_rel.ends_with(".yaml"))
}

fn is_build_file(name_lower: &str, lower_rel: &str) -> bool {
    matches!(
        name_lower,
        "cargo.toml"
            | "go.mod"
            | "package.json"
            | "pyproject.toml"
            | "setup.py"
            | "setup.cfg"
            | "requirements.txt"
            | "makefile"
            | "cmakelists.txt"
            | "build.gradle"
            | "pom.xml"
    ) || lower_rel.ends_with(".mk")
        || name_lower == "dockerfile"
        || lower_rel.contains("dockerfile")
}

fn is_config_file(name_lower: &str, lower_rel: &str) -> bool {
    lower_rel.ends_with(".toml")
        || lower_rel.ends_with(".yaml")
        || lower_rel.ends_with(".yml")
        || lower_rel.ends_with(".json")
        || lower_rel.ends_with(".ini")
        || lower_rel.ends_with(".cfg")
        || lower_rel.ends_with(".conf")
        || name_lower.starts_with('.') && !lower_rel.contains('/')
        || lower_rel.contains(".config/")
        || lower_rel.starts_with(".config/")
}

fn is_generated_path(lower_rel: &str) -> bool {
    lower_rel.contains("/target/")
        || lower_rel.starts_with("target/")
        || lower_rel.contains("/node_modules/")
        || lower_rel.contains("/dist/")
        || lower_rel.starts_with("dist/")
        || lower_rel.contains("/build/")
        || lower_rel.ends_with(".min.js")
        || lower_rel.ends_with(".bundle.js")
        || lower_rel.contains("generated")
        || lower_rel.contains("codegen")
}

fn is_generated_content(head: &str) -> bool {
    head.contains("@generated")
        || head.contains("DO NOT EDIT")
        || head.contains("auto-generated")
        || head.contains("Code generated by")
}

fn is_test_path(lower_rel: &str, name_lower: &str) -> bool {
    lower_rel.contains("/tests/")
        || lower_rel.starts_with("tests/")
        || lower_rel.contains("/test/")
        || lower_rel.starts_with("test/")
        || name_lower.starts_with("test_")
        || name_lower.ends_with("_test.rs")
        || name_lower.ends_with("_test.py")
        || name_lower.ends_with("_test.go")
        || name_lower.ends_with(".test.ts")
        || name_lower.ends_with(".test.js")
        || name_lower.ends_with(".spec.ts")
        || name_lower.ends_with(".spec.js")
        || lower_rel.contains("test")
            && (name_lower.starts_with("test") || name_lower.contains("test"))
}

fn is_source_path(lower_rel: &str) -> bool {
    let ext = lower_rel.rsplit('.').next().unwrap_or("");
    matches!(
        ext,
        "rs" | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "go"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "cc"
            | "cxx"
            | "hh"
            | "sh"
            | "bash"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_id_is_path_derived() {
        assert_eq!(file_id("src/main.rs"), "file::src/main.rs");
    }

    #[test]
    fn rust_source_is_parser_supported() {
        let r = record_for_content("src/main.rs", 100, "fn main() {}", 1);
        assert_eq!(r.language.as_deref(), Some("rust"));
        assert!(r.parser_supported);
        assert!(r.parser_limitation.is_none());
        assert!(r.is_source());
    }

    #[test]
    fn c_file_preserves_file_level_without_symbols() {
        let r = record_for_content("src/util.c", 50, "int x;\n", 1);
        assert_eq!(r.language.as_deref(), Some("c"));
        assert!(!r.parser_supported);
        assert!(r.parser_limitation.is_some());
        assert!(r.is_source());
    }

    #[test]
    fn shell_file_is_file_level_only() {
        let r = record_for_content("scripts/ci.sh", 20, "#!/bin/sh\n", 1);
        assert_eq!(r.language.as_deref(), Some("shell"));
        assert!(!r.parser_supported);
    }

    #[test]
    fn test_path_detected() {
        let r = record_for_content("tests/integration.rs", 10, "// t\n", 1);
        assert!(r.is_test());
    }

    #[test]
    fn generated_marker_detected() {
        let r = record_for_content(
            "src/gen.rs",
            30,
            "// Code generated by foo. DO NOT EDIT.\n",
            1,
        );
        assert!(r.is_generated());
    }

    #[test]
    fn config_file_classified() {
        let r = record_for_content("Cargo.toml", 10, "[package]\n", 1);
        assert!(r.classes.contains(&FileClass::Config));
        assert!(r.classes.contains(&FileClass::Build));
    }

    #[test]
    fn hash_changes_with_content() {
        let a = record_for_content("a.rs", 3, "aaa", 1);
        let b = record_for_content("a.rs", 3, "aab", 1);
        assert_ne!(a.hash, b.hash);
    }

    #[test]
    fn unknown_extension_has_no_language_but_stable_id() {
        let r = record_for_content("assets/logo.bin", 4, "xxxx", 1);
        assert!(r.language.is_none());
        assert!(!r.parser_supported);
        assert_eq!(r.id, "file::assets/logo.bin");
    }
}
