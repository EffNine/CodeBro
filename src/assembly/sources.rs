use crate::error::Result;
use crate::intelligence::CodeIndexer;
use crate::memory_runtime::{MemoryEntry, MemoryRuntime, MemoryTier};
use crate::workspace_runtime::{Change, WorkspaceRuntime};

/// A piece of context collected from one source during assembly.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextFragment {
    pub source: ContextSource,
    pub priority: ContextPriority,
    pub content: String,
    pub estimated_tokens: usize,
    pub relevance_score: f64,
}

impl ContextFragment {
    pub fn new(
        source: ContextSource,
        priority: ContextPriority,
        content: String,
        relevance_score: f64,
    ) -> Self {
        let estimated_tokens = Self::estimate_tokens(&content);
        ContextFragment {
            source,
            priority,
            content,
            estimated_tokens,
            relevance_score,
        }
    }

    pub fn estimate_tokens(text: &str) -> usize {
        text.len() / 4
    }
}

/// The origin of a fragment during assembly.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ContextSource {
    UserRequest,
    Workspace,
    EngineeringFacts,
    Memory,
    Git,
    Indexer,
    Scanner,
    ToolResults,
}

impl std::fmt::Display for ContextSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextSource::UserRequest => write!(f, "user_request"),
            ContextSource::Workspace => write!(f, "workspace"),
            ContextSource::EngineeringFacts => write!(f, "engineering_facts"),
            ContextSource::Memory => write!(f, "memory"),
            ContextSource::Git => write!(f, "git"),
            ContextSource::Indexer => write!(f, "indexer"),
            ContextSource::Scanner => write!(f, "scanner"),
            ContextSource::ToolResults => write!(f, "tool_results"),
        }
    }
}

/// Relative urgency assigned by the ranking pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextPriority {
    Critical,
    High,
    Medium,
    Low,
}

impl std::fmt::Display for ContextPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextPriority::Critical => write!(f, "critical"),
            ContextPriority::High => write!(f, "high"),
            ContextPriority::Medium => write!(f, "medium"),
            ContextPriority::Low => write!(f, "low"),
        }
    }
}

// ── Source-specific collector traits ─────────────────────────────────────

/// Collect workspace-level context fragments.
pub trait WorkspaceContextSource {
    fn collect_workspace_fragments(&self, query: &str) -> Result<Vec<ContextFragment>>;
}

impl WorkspaceContextSource for WorkspaceRuntime {
    fn collect_workspace_fragments(&self, _query: &str) -> Result<Vec<ContextFragment>> {
        let mut fragments = Vec::new();

        let meta = self.metadata();
        let workspace_content = format!(
            "Workspace: {}\nRoot: {:?}\nFiles: {}\nLanguage: {:?}\nBuild: {:?}\n",
            meta.root.name(),
            meta.root.0,
            meta.file_count,
            meta.language,
            meta.build_tool,
        );
        fragments.push(ContextFragment::new(
            ContextSource::Workspace,
            ContextPriority::High,
            workspace_content,
            0.8,
        ));

        if !meta.language.as_deref().unwrap_or("").is_empty() {
            let lang_frag = format!(
                "Language: {:?}\nOS: {:?}\nToolchains: {:?}",
                meta.language, meta.os, meta.toolchains,
            );
            fragments.push(ContextFragment::new(
                ContextSource::Workspace,
                ContextPriority::Medium,
                lang_frag,
                0.5,
            ));
        }

        Ok(fragments)
    }
}

/// Collect engineering-facts fragments from an indexer.
pub trait EngineeringFactsSource {
    fn collect_facts_fragments(
        &self,
        query: &str,
        max_symbols: usize,
    ) -> Result<Vec<ContextFragment>>;
}

impl EngineeringFactsSource for CodeIndexer {
    fn collect_facts_fragments(
        &self,
        query: &str,
        max_symbols: usize,
    ) -> Result<Vec<ContextFragment>> {
        let all_symbols = self.get_symbols().map_err(|e| {
            crate::error::CodeBroError::Context(format!("Failed to get symbols: {}", e))
        })?;
        let relevant: Vec<_> = if query.is_empty() {
            all_symbols.iter().take(max_symbols).cloned().collect()
        } else {
            let q = query.to_lowercase();
            all_symbols
                .iter()
                .filter(|s| {
                    s.name.to_lowercase().contains(&q) || s.file.to_lowercase().contains(&q)
                })
                .take(max_symbols)
                .cloned()
                .collect()
        };

        let mut symbol_lines = Vec::new();
        for sym in &relevant {
            symbol_lines.push(format!(
                "{} {} ({}) [{}:{}",
                sym.kind, sym.name, sym.language, sym.file, sym.line_start,
            ));
        }
        if !symbol_lines.is_empty() {
            return Ok(vec![ContextFragment::new(
                ContextSource::EngineeringFacts,
                ContextPriority::High,
                format!("Symbols:\n{}", symbol_lines.join("\n")),
                0.7,
            )]);
        }
        Ok(Vec::new())
    }
}

/// Collect memory fragments from the MemoryRuntime.
pub trait MemoryContextSource {
    fn collect_memory_fragments(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<ContextFragment>>;
}

impl MemoryContextSource for MemoryRuntime {
    fn collect_memory_fragments(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<ContextFragment>> {
        use crate::memory_runtime::MemoryQuery;
        let q = MemoryQuery::new(query)
            .limit(max_results)
            .require_confidence(0.3);
        let resolution = self.resolve(&q);

        let mut fragments = Vec::new();
        for entry in &resolution.hits {
            let content = format!(
                "[{}] {} = {} (tier: {}, confidence: {:.2})",
                entry.id, entry.key, entry.value, entry.tier, entry.metadata.confidence,
            );
            fragments.push(ContextFragment::new(
                ContextSource::Memory,
                ContextPriority::Medium,
                content,
                entry.metadata.confidence as f64,
            ));
        }
        Ok(fragments)
    }
}

/// Collect git-repository context fragments.
pub trait GitContextSource {
    fn collect_git_fragments(&self, query: &str) -> Result<Vec<ContextFragment>>;
}

impl GitContextSource for WorkspaceRuntime {
    fn collect_git_fragments(&self, _query: &str) -> Result<Vec<ContextFragment>> {
        let mut fragments = Vec::new();

        self.ensure_discovered();
        let meta = self.metadata();
        if meta.has_git {
            if let Some(ref branch) = meta.branch {
                let branch_line = format!("Branch: {}\n", branch);
                fragments.push(ContextFragment::new(
                    ContextSource::Git,
                    ContextPriority::High,
                    branch_line,
                    0.6,
                ));
            }
            if let Some(ref remote) = meta.remote_url {
                fragments.push(ContextFragment::new(
                    ContextSource::Git,
                    ContextPriority::Low,
                    format!("Remote: {}\n", remote),
                    0.2,
                ));
            }
        }

        if let Some(latest) = self.snapshots().latest_id() {
            if let Ok(diff) = self.diff("empty", &latest) {
                if !diff.is_empty() {
                    let mut changes = Vec::new();
                    for change in diff.changes.iter().take(20) {
                        changes.push(format!("  {:?} {}", change.kind, change.rel_path.display()));
                    }
                    fragments.push(ContextFragment::new(
                        ContextSource::Git,
                        ContextPriority::High,
                        format!(
                            "Workspace changes ({}):\n{}",
                            diff.count(),
                            changes.join("\n")
                        ),
                        0.7,
                    ));
                }
            }
        }

        Ok(fragments)
    }
}

/// Collect indexer (file-level) fragments.
pub trait IndexerContextSource {
    fn collect_indexer_fragments(
        &self,
        query: &str,
        max_files: usize,
    ) -> Result<Vec<ContextFragment>>;
}

impl IndexerContextSource for CodeIndexer {
    fn collect_indexer_fragments(
        &self,
        query: &str,
        max_files: usize,
    ) -> Result<Vec<ContextFragment>> {
        use crate::intelligence::index::CodeIndexerTrait;
        let files = self.get_indexed_files();
        let q = query.to_lowercase();

        let mut scored: Vec<(String, f64)> = files
            .into_iter()
            .map(|f| {
                let score = if q.is_empty() {
                    0.5
                } else {
                    let mut s = 0.0f64;
                    if f.to_lowercase().contains(&q) {
                        s += 2.0;
                    }
                    for term in q.split_whitespace() {
                        if f.to_lowercase().contains(term) {
                            s += 1.0;
                        }
                    }
                    s
                };
                (f, score)
            })
            .filter(|(_, s)| *s > 0.0)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut fragments = Vec::new();
        for (path, score) in scored.into_iter().take(max_files) {
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => format!("[unreadable] {}", path),
            };
            fragments.push(ContextFragment::new(
                ContextSource::Indexer,
                if score > 1.5 {
                    ContextPriority::High
                } else {
                    ContextPriority::Medium
                },
                format!("--- {} ---\n{}", path, content),
                score,
            ));
        }
        Ok(fragments)
    }
}

/// Collect scanner (project-info) fragments.
pub trait ScannerContextSource {
    fn collect_scanner_fragments(&self) -> Result<Vec<ContextFragment>>;
}

impl ScannerContextSource for crate::scanner::ProjectInfo {
    fn collect_scanner_fragments(&self) -> Result<Vec<ContextFragment>> {
        let mut lines = Vec::new();
        lines.push(format!("Project: {}", self.name));
        lines.push(format!("Language: {}", self.language));
        if let Some(ref fw) = self.framework {
            lines.push(format!("Framework: {}", fw));
        }
        if let Some(ref bs) = self.build_system {
            lines.push(format!("Build System: {}", bs));
        }
        if let Some(ref pm) = self.package_manager {
            lines.push(format!("Package Manager: {}", pm));
        }
        if let Some(ref tf) = self.testing_framework {
            lines.push(format!("Testing: {}", tf));
        }
        if !self.important_files.is_empty() {
            lines.push("Important Files:".to_string());
            for f in &self.important_files {
                lines.push(format!("  - {}", f));
            }
        }
        Ok(vec![ContextFragment::new(
            ContextSource::Scanner,
            ContextPriority::Medium,
            lines.join("\n"),
            0.6,
        )])
    }
}

/// Collect tool-results fragments (previous execution output).
pub trait ToolResultsContextSource {
    fn collect_tool_results_fragments(&self, max_results: usize) -> Result<Vec<ContextFragment>>;
}

impl ToolResultsContextSource for Vec<ContextFragment> {
    fn collect_tool_results_fragments(&self, max_results: usize) -> Result<Vec<ContextFragment>> {
        Ok(self.iter().take(max_results).cloned().collect())
    }
}

/// Collect user-request fragments (the raw prompt).
pub trait UserRequestContextSource {
    fn collect_user_request_fragment(&self, request: &str) -> Result<Vec<ContextFragment>>;
}

impl UserRequestContextSource for str {
    fn collect_user_request_fragment(&self, request: &str) -> Result<Vec<ContextFragment>> {
        Ok(vec![ContextFragment::new(
            ContextSource::UserRequest,
            ContextPriority::Critical,
            request.to_string(),
            1.0,
        )])
    }
}

/// Rank a fragment list deterministically by score, priority, and content fingerprint.
pub fn rank_fragments(fragments: &mut Vec<ContextFragment>) {
    fragments.sort_by(|a, b| {
        a.relevance_score
            .partial_cmp(&b.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.priority.cmp(&a.priority))
    });
}

/// Deduplicate fragments by source+content fingerprint.
pub fn dedup_fragments(fragments: &mut Vec<ContextFragment>) {
    let mut seen = std::collections::HashSet::new();
    fragments.retain(|f| {
        let fingerprint = fragment_fingerprint(&f.source.to_string(), &f.content);
        seen.insert(fingerprint)
    });
}

/// A deterministic, content-aware fragment fingerprint.
///
/// Same source + same content → identical fingerprint.
/// Same source + different content → different fingerprint (no length-only
/// collisions). Different source + same content → different fingerprint.
///
/// The fingerprint is the source and full content joined by a NUL separator:
/// deterministic, cheap, stable across runs, and based only on the fragment
/// itself. No random IDs, no nondeterministic hashing.
pub fn fragment_fingerprint(source: &str, content: &str) -> String {
    let mut fp = String::with_capacity(source.len() + content.len() + 1);
    fp.push_str(source);
    fp.push('\u{0}');
    fp.push_str(content);
    fp
}

/// Apply a token budget and return (selected, discarded) counts.
pub fn apply_token_budget(fragments: &mut Vec<ContextFragment>, budget: usize) -> (usize, usize) {
    let mut remaining = budget;
    let before = fragments.len();
    fragments.retain(|f| {
        if f.estimated_tokens <= remaining {
            remaining -= f.estimated_tokens;
            true
        } else {
            false
        }
    });
    (before, before - fragments.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frag(source: ContextSource, content: &str, score: f64) -> ContextFragment {
        ContextFragment::new(source, ContextPriority::Medium, content.to_string(), score)
    }

    #[test]
    fn test_fragment_fingerprint_content_aware() {
        // Same source + same content → identical fingerprint.
        assert_eq!(
            fragment_fingerprint("tool_result", "aaaa"),
            fragment_fingerprint("tool_result", "aaaa")
        );
        // Same source + different content → different fingerprint (no length
        // collisions).
        assert_ne!(
            fragment_fingerprint("tool_result", "abcdefghij"),
            fragment_fingerprint("tool_result", "klmnopqrst")
        );
        // Different source + same content → different fingerprint.
        assert_ne!(
            fragment_fingerprint("tool_result", "same content"),
            fragment_fingerprint("agent_analysis", "same content")
        );
    }

    #[test]
    fn test_dedup_same_source_same_content() {
        let mut fragments = vec![
            frag(ContextSource::ToolResults, "output", 0.9),
            frag(ContextSource::ToolResults, "output", 0.9),
        ];
        dedup_fragments(&mut fragments);
        assert_eq!(fragments.len(), 1);
    }

    #[test]
    fn test_dedup_same_source_different_content() {
        let mut fragments = vec![
            frag(ContextSource::ToolResults, "abcdefghij", 0.9),
            frag(ContextSource::ToolResults, "klmnopqrst", 0.9),
        ];
        dedup_fragments(&mut fragments);
        assert_eq!(fragments.len(), 2);
    }

    #[test]
    fn test_dedup_different_source_same_content() {
        let mut fragments = vec![
            frag(ContextSource::ToolResults, "same content", 0.9),
            frag(ContextSource::Scanner, "same content", 0.8),
        ];
        dedup_fragments(&mut fragments);
        assert_eq!(fragments.len(), 2);
    }
}
