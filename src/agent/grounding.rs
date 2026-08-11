#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Grounded subagent context (Sprint 30B).
//!
//! Turns real repository data into a [`GroundedContext`] that the coordinator
//! hands to every subagent. The expensive discovery work — one index query or
//! one bounded workspace scan — happens exactly once per task and is then
//! reused by all five subagents.
//!
//! Two grounding sources are used, both built on existing infrastructure:
//!
//! * `.codebro/index.db` (when present): `IntelligentContextBuilder`
//!   (SemanticSearch over the existing `CodeIndexer`) resolves the task to
//!   actual indexed files and symbols.
//! * Workspace fallback (when the index is absent): `ProjectInfo::detect` +
//!   `WorkspaceRuntime` metadata + a single bounded source-file scan resolve
//!   relevant files, tests and manifest dependencies.
//!
//! Everything here is deterministic: no LLM calls, no network calls, no
//! randomness. The same task + workspace snapshot + index state always yields
//! the same grounded context.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::intelligence::context::IntelligentContextBuilder;
use crate::intelligence::index::CodeIndexer;
use crate::scanner::ProjectInfo;
use crate::workspace_runtime::{LocalFileSystem, WorkspaceRuntime};

use super::subagent::{SubAgentContext, SubAgentResult};

/// Upper bounds so a grounded context stays small and deterministic.
const MAX_RELEVANT_FILES: usize = 12;
const MAX_RELATED_SYMBOLS: usize = 15;
const MAX_CONTEXT_FRAGMENTS: usize = 8;

/// Deterministically accumulated repository context for one task.
#[derive(Debug, Clone, Default)]
pub struct GroundedContext {
    pub task_description: String,
    pub workspace_root: String,
    pub project_name: String,
    pub project_language: String,
    pub workspace_summary: String,
    pub git_state: String,
    pub build_info: String,
    pub relevant_files: Vec<String>,
    pub related_symbols: Vec<String>,
    pub dependencies: Vec<String>,
    pub test_files: Vec<String>,
    pub tool_observations: Vec<String>,
    pub memory_entries: Vec<String>,
    pub context_fragments: Vec<String>,
}

impl GroundedContext {
    /// Materialise a `SubAgentContext` for one subagent invocation.
    pub fn to_subagent_context(&self, previous_results: Vec<SubAgentResult>) -> SubAgentContext {
        SubAgentContext {
            task_description: self.task_description.clone(),
            project_root: self.workspace_root.clone(),
            relevant_files: self.relevant_files.clone(),
            related_symbols: self.related_symbols.clone(),
            dependencies: self.dependencies.clone(),
            previous_results,
            memory_entries: self.memory_entries.clone(),
            project_name: self.project_name.clone(),
            project_language: self.project_language.clone(),
            workspace_summary: self.workspace_summary.clone(),
            git_state: self.git_state.clone(),
            build_info: self.build_info.clone(),
            test_files: self.test_files.clone(),
            tool_observations: self.tool_observations.clone(),
            context_fragments: self.context_fragments.clone(),
        }
    }

    /// True when no repository-derived facts were discovered.
    pub fn is_empty(&self) -> bool {
        self.relevant_files.is_empty()
            && self.related_symbols.is_empty()
            && self.dependencies.is_empty()
            && self.test_files.is_empty()
    }
}

/// Assembles a [`GroundedContext`] for a task from existing repository
/// intelligence. Cheap discovery happens lazily; the expensive workspace
/// scan runs at most once per [`GroundingAssembler::assemble`] call.
pub struct GroundingAssembler {
    root: PathBuf,
}

impl GroundingAssembler {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        GroundingAssembler { root: root.into() }
    }

    /// Assemble grounded context from workspace/index data only.
    pub fn assemble(&self, task: &str) -> GroundedContext {
        self.assemble_with_extras(task, &[], &[])
    }

    /// Assemble grounded context including recent tool observations and
    /// engineering memory entries (both optional).
    pub fn assemble_with_extras(
        &self,
        task: &str,
        tool_observations: &[String],
        memory_entries: &[String],
    ) -> GroundedContext {
        let project = ProjectInfo::detect(self.root.clone()).unwrap_or_default();
        let workspace = WorkspaceRuntime::new(self.root.clone(), Arc::new(LocalFileSystem::new()));
        let meta = workspace.metadata();

        let mut grounded = GroundedContext {
            task_description: task.to_string(),
            workspace_root: self.root.to_string_lossy().to_string(),
            project_name: project.name.clone(),
            project_language: project.language.clone(),
            workspace_summary: format!(
                "Workspace root: {} | language: {} | build tool: {}",
                meta.root.name(),
                meta.language.as_deref().unwrap_or("unknown"),
                meta.build_tool.as_deref().unwrap_or("unknown"),
            ),
            git_state: if meta.has_git {
                format!(
                    "git: branch {}",
                    meta.branch.as_deref().unwrap_or("unknown")
                )
            } else {
                "git: not a repository".to_string()
            },
            build_info: project_build_info(&project),
            dependencies: manifest_dependencies(&self.root),
            tool_observations: tool_observations.to_vec(),
            memory_entries: memory_entries.to_vec(),
            ..GroundedContext::default()
        };

        // Index-backed grounding: SemanticSearch over the existing index.
        let index_db = self.root.join(".codebro").join("index.db");
        if index_db.exists() {
            if let Ok(indexer) = CodeIndexer::new(index_db) {
                if let Ok(intel) = IntelligentContextBuilder::new(indexer).build_context(task) {
                    grounded.relevant_files = intel
                        .related_files
                        .iter()
                        .map(|f| self.relativize(f))
                        .collect();
                    grounded.related_symbols = intel
                        .relevant_symbols
                        .iter()
                        .map(|r| r.symbol.name.clone())
                        .collect();
                }
            }
        }

        // Workspace fallback: one bounded source-file scan (ignores the index
        // locations) resolves files + tests deterministically.
        let source_files = collect_source_files(&self.root);
        if grounded.relevant_files.is_empty() {
            grounded.relevant_files = match_files(&source_files, task, &project.important_files);
        }
        if grounded.test_files.is_empty() {
            grounded.test_files = identify_test_files(&source_files);
        }

        grounded.related_symbols.truncate(MAX_RELATED_SYMBOLS);
        grounded.relevant_files.truncate(MAX_RELEVANT_FILES);
        sort_unique(&mut grounded.relevant_files);
        sort_unique(&mut grounded.related_symbols);
        sort_unique(&mut grounded.dependencies);
        sort_unique(&mut grounded.test_files);
        sort_unique(&mut grounded.memory_entries);

        grounded.context_fragments = self.context_fragments(&grounded, tool_observations);
        grounded
    }

    /// Render a small, deterministic set of summary context fragments.
    fn context_fragments(
        &self,
        grounded: &GroundedContext,
        tool_observations: &[String],
    ) -> Vec<String> {
        let mut fragments = Vec::new();
        fragments.push(format!(
            "Project {} ({})",
            grounded.project_name, grounded.project_language
        ));
        fragments.push(grounded.workspace_summary.clone());
        if !grounded.git_state.is_empty() {
            fragments.push(grounded.git_state.clone());
        }
        if !grounded.build_info.is_empty() {
            fragments.push(format!("Build info: {}", grounded.build_info));
        }
        for obs in tool_observations.iter().take(MAX_CONTEXT_FRAGMENTS) {
            fragments.push(truncate_for_summary(obs, 300));
        }
        fragments
    }

    /// Make an absolute indexed path relative to the workspace root.
    fn relativize(&self, path: &str) -> String {
        let p = Path::new(path);
        if p.is_absolute() {
            p.strip_prefix(&self.root)
                .map(|r| r.to_string_lossy().to_string())
                .unwrap_or_else(|_| path.to_string())
        } else {
            path.to_string()
        }
    }
}

/// Compose a human-readable build/test command summary from `ProjectInfo`.
fn project_build_info(project: &ProjectInfo) -> String {
    let mut parts = Vec::new();
    if let Some(bs) = project.build_system.as_deref() {
        parts.push(format!("build: {}", bs));
    }
    if let Some(pm) = project.package_manager.as_deref() {
        parts.push(format!("package manager: {}", pm));
    }
    if let Some(tf) = project.testing_framework.as_deref() {
        parts.push(format!("testing: {}", tf));
    }
    if parts.is_empty() {
        "unknown build".to_string()
    } else {
        parts.join("; ")
    }
}

/// External dependencies from `Cargo.toml` / `package.json`, sorted and
/// deduplicated. Reads only build/project files (filesystem information).
fn manifest_dependencies(root: &Path) -> Vec<String> {
    let mut deps = Vec::new();

    let cargo_path = root.join("Cargo.toml");
    if cargo_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&cargo_path) {
            if let Ok(value) = content.parse::<toml::Value>() {
                for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                    if let Some(tbl) = value.get(section).and_then(|v| v.as_table()) {
                        for key in tbl.keys() {
                            deps.push(key.clone());
                        }
                    }
                }
                if let Some(ws) = value.get("workspace").and_then(|v| v.as_table()) {
                    if let Some(tbl) = ws.get("dependencies").and_then(|v| v.as_table()) {
                        for key in tbl.keys() {
                            deps.push(key.clone());
                        }
                    }
                }
            }
        }
    }

    let pkg_path = root.join("package.json");
    if pkg_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&pkg_path) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                for section in ["dependencies", "devDependencies"] {
                    if let Some(obj) = value.get(section).and_then(|v| v.as_object()) {
                        for key in obj.keys() {
                            deps.push(key.clone());
                        }
                    }
                }
            }
        }
    }

    sort_unique(&mut deps);
    deps
}

const SOURCE_EXTS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "rb", "kt", "swift", "c", "h", "cpp",
    "hpp", "cs", "php", "sh",
];

const IGNORED_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    ".codebro",
    "dist",
    "build",
    "vendor",
    "venv",
    ".venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
];

/// One bounded walk of the workspace collecting source files, sorted.
fn collect_source_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut it = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let rel = e.path().strip_prefix(root).unwrap_or(e.path());
            let rel_str = rel.to_string_lossy().to_string();
            !IGNORED_DIRS
                .iter()
                .any(|d| rel_str == *d || rel_str.starts_with(&format!("{}/", d)))
        });
    for entry in it.by_ref().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(path);
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if SOURCE_EXTS.contains(&ext) {
                files.push(rel.to_string_lossy().to_string());
            }
        }
    }
    sort_unique(&mut files);
    files
}

/// Words that carry no locality signal for file ranking.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "to", "in", "for", "with", "on", "at", "by", "from", "as",
    "is", "are", "was", "be", "it", "its", "this", "that", "these", "those", "do", "does",
    "should", "would", "how", "what", "why", "when", "where", "which", "please", "me",
];

/// Task keywords with locality signal, lowercased and deduplicated.
fn task_terms(task: &str) -> Vec<String> {
    let mut terms: Vec<String> = task
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .filter(|t| t.len() > 2)
        .filter(|t| !STOPWORDS.contains(t))
        .map(|t| t.to_string())
        .collect();
    sort_unique(&mut terms);
    terms
}

/// Deterministic, bounded file ranking: task-term overlap first (relevance
/// order), then important files, then a small top-of-scan fallback so
/// research is never empty. Relevant source files always take precedence over
/// generic important files.
fn match_files(files: &[String], task: &str, important: &[String]) -> Vec<String> {
    let terms = task_terms(task);
    let mut scored: Vec<(usize, &String)> = files
        .iter()
        .map(|f| (score_file(f, &terms), f))
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));

    let mut out: Vec<String> = scored
        .iter()
        .take(MAX_RELEVANT_FILES)
        .map(|(_, f)| f.to_string())
        .collect();

    // Important files fill remaining slots; they never displace a relevant
    // source file that matched the task terms.
    for imp in important {
        if !imp.is_empty() && !out.contains(imp) {
            out.push(imp.clone());
        }
    }

    // When keyword matching found too little, surface the first few source
    // files so the research output stays grounded in real repository data.
    if out.iter().filter(|f| f.starts_with("src/")).count() < 3 {
        for f in files.iter().take(MAX_RELEVANT_FILES) {
            if !out.contains(f) {
                out.push(f.clone());
            }
            if out.iter().filter(|x| x.starts_with("src/")).count() >= 3 {
                break;
            }
        }
    }

    dedup_preserve_order(&mut out);
    out.truncate(MAX_RELEVANT_FILES);
    out
}

/// Number of task terms that appear in a relative file path.
fn score_file(path: &str, terms: &[String]) -> usize {
    let lower = path.to_lowercase();
    terms.iter().filter(|t| lower.contains(t.as_str())).count()
}

/// Source files that look like tests, sorted.
fn identify_test_files(files: &[String]) -> Vec<String> {
    let mut tests: Vec<String> = files.iter().filter(|f| is_test_path(f)).cloned().collect();
    sort_unique(&mut tests);
    tests
}

/// A path is test-like when it is under a tests dir or matches common test
/// suffixes/prefixes.
fn is_test_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    if lower.contains("/tests/") || lower.starts_with("tests/") {
        return true;
    }
    if lower.contains("/test/") || lower.starts_with("test/") {
        return true;
    }
    let name = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    name.ends_with("_test") || name.ends_with(".test") || name == "test"
}

fn sort_unique(v: &mut Vec<String>) {
    v.sort();
    v.dedup();
}

/// Deduplicate while preserving the existing order (used for relevance-ranked
/// file lists where alphabetical re-sorting would destroy the ranking).
fn dedup_preserve_order(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|s| seen.insert(s.clone()));
}

/// First non-empty line, capped for compact summaries.
pub fn truncate_for_summary(text: &str, max_chars: usize) -> String {
    let first_line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string();
    if first_line.chars().count() <= max_chars {
        first_line
    } else {
        first_line.chars().take(max_chars).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn fixture_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n[dependencies]\nanyhow = \"1\"\nserde = { version = \"1\", features = [\"derive\"] }\ntokio = { version = \"1\", features = [\"full\"] }\n",
        );
        write(
            &dir.path().join("src/canonical_runtime.rs"),
            "pub struct CanonicalRuntime {}\nimpl CanonicalRuntime {\n    pub fn run_execution_loop(&self) {}\n}\n",
        );
        write(
            &dir.path().join("src/agent/tool_parser.rs"),
            "pub fn parse_tool_calls(text: &str) {}\npub fn trace_runtime_parsing() {}\n",
        );
        write(
            &dir.path().join("src/agent/tool_parser_test.rs"),
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn test_parse_tool_calls() {}\n}\n",
        );
        dir
    }

    #[test]
    fn test_empty_index_fallback_is_grounded() {
        let dir = fixture_project();
        let assembler = GroundingAssembler::new(dir.path());

        // No `.codebro/index.db` exists in the fixture: the workspace scan
        // must still produce useful, real repository data.
        let grounded = assembler.assemble("trace the parser module execution");

        assert!(!grounded.relevant_files.is_empty(), "files from scan");
        assert!(
            grounded
                .relevant_files
                .iter()
                .any(|f| f.contains("tool_parser")),
            "parser file matched by task terms: {:?}",
            grounded.relevant_files
        );
        assert!(
            grounded.dependencies.contains(&"anyhow".to_string()),
            "manifest dependency anyhow present: {:?}",
            grounded.dependencies
        );
        assert!(
            grounded.dependencies.contains(&"tokio".to_string()),
            "manifest dependency tokio present"
        );
        assert!(
            grounded.test_files.iter().any(|f| f.contains("test")),
            "test files identified: {:?}",
            grounded.test_files
        );
        assert!(!grounded.project_name.is_empty());
        assert!(grounded.project_language == "rust");
        assert!(grounded.build_info.contains("cargo"));
        assert!(grounded
            .context_fragments
            .iter()
            .any(|f| f.contains("rust")));
        assert!(!grounded.is_empty());
    }

    #[test]
    fn test_indexed_grounding_resolves_files_and_symbols() {
        let dir = fixture_project();
        let codebro_dir = dir.path().join(".codebro");
        std::fs::create_dir_all(&codebro_dir).unwrap();
        let db_path = codebro_dir.join("index.db");
        {
            let mut indexer = CodeIndexer::new(db_path.clone()).unwrap();
            let source =
                std::fs::read_to_string(dir.path().join("src/canonical_runtime.rs")).unwrap();
            indexer
                .index_file(&dir.path().join("src/canonical_runtime.rs"), &source)
                .unwrap();
            let source =
                std::fs::read_to_string(dir.path().join("src/agent/tool_parser.rs")).unwrap();
            indexer
                .index_file(&dir.path().join("src/agent/tool_parser.rs"), &source)
                .unwrap();
        }

        let assembler = GroundingAssembler::new(dir.path());
        let grounded = assembler.assemble("trace canonical runtime execution");

        assert!(
            grounded
                .relevant_files
                .iter()
                .any(|f| f == "src/canonical_runtime.rs"),
            "indexed file resolved: {:?}",
            grounded.relevant_files
        );
        assert!(
            grounded
                .related_symbols
                .iter()
                .any(|s| s == "run_execution_loop"),
            "indexed symbol resolved: {:?}",
            grounded.related_symbols
        );
        assert!(
            grounded
                .related_symbols
                .iter()
                .any(|s| s == "trace_runtime_parsing"),
            "indexed symbol from second file resolved: {:?}",
            grounded.related_symbols
        );
        assert!(
            grounded.dependencies.contains(&"tokio".to_string()),
            "dependencies still grounded from manifest"
        );
    }

    #[test]
    fn test_assemble_is_deterministic() {
        let dir = fixture_project();
        let assembler = GroundingAssembler::new(dir.path());
        let a = assembler.assemble("trace the parser module execution");
        let b = assembler.assemble("trace the parser module execution");
        assert_eq!(a.relevant_files, b.relevant_files);
        assert_eq!(a.related_symbols, b.related_symbols);
        assert_eq!(a.dependencies, b.dependencies);
        assert_eq!(a.test_files, b.test_files);
    }

    #[test]
    fn test_manifest_dependencies_parsed() {
        let dir = fixture_project();
        let deps = manifest_dependencies(dir.path());
        for expected in ["anyhow", "serde", "tokio"] {
            assert!(deps.contains(&expected.to_string()), "missing {}", expected);
        }
    }

    #[test]
    fn test_relativize_absolute_paths() {
        let dir = fixture_project();
        let assembler = GroundingAssembler::new(dir.path());
        let abs = dir.path().join("src/main.rs");
        assert_eq!(assembler.relativize(&abs.to_string_lossy()), "src/main.rs");
        assert_eq!(assembler.relativize("relative/only.rs"), "relative/only.rs");
    }

    #[test]
    fn test_match_files_bounded_and_relevance_ordered() {
        let files = vec![
            "src/zz.rs".to_string(),
            "src/aa.rs".to_string(),
            "src/parser.rs".to_string(),
            "src/parser_test.rs".to_string(),
        ];
        let matched = match_files(&files, "fix the parser", &["Cargo.toml".to_string()]);
        assert!(
            matched.iter().any(|f| f.contains("parser")),
            "parser files match: {:?}",
            matched
        );
        assert!(matched.iter().any(|f| f == "Cargo.toml"));
        // A relevant source file outranks a generic important file.
        assert_eq!(matched[0], "src/parser.rs");
        assert!(matched.len() <= MAX_RELEVANT_FILES);
        assert_eq!(
            matched.len(),
            matched
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
        );
    }
}
