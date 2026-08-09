use super::budget::{budget, ContextBudget, TokenBudget};
use super::config::AssemblyConfig;
use super::intent::{IntentClassification, IntentType};
use super::ordering::{self, ContextSection};
use super::sources::*;
use super::statistics::AssemblyStatistics;
use crate::assembly::ContextFragment;
use crate::assembly::ContextPriority;
use crate::assembly::ContextSource;
use crate::error::Result;
use crate::intelligence::CodeIndexer;
use crate::memory_runtime::MemoryRuntime;
use crate::workspace_runtime::WorkspaceRuntime;

/// A request to the Context Assembler.
///
/// Carries the user prompt plus any optional runtime handles the caller
/// wishes to inject. All handles are optional so the assembler can be
/// tested in isolation.
pub struct ContextAssemblyRequest {
    pub user_request: String,
    pub conversation_history: Vec<ContextMessage>,
    pub tool_results: Vec<ContextFragment>,
    pub project_info: Option<crate::scanner::ProjectInfo>,
    pub workspace: Option<std::sync::Arc<WorkspaceRuntime>>,
    pub indexer: Option<std::sync::Arc<CodeIndexer>>,
    pub memory: Option<std::sync::Arc<MemoryRuntime>>,
}

/// A message in the conversation history.
#[derive(Debug, Clone)]
pub struct ContextMessage {
    pub role: String,
    pub content: String,
    pub tokens: usize,
}

impl ContextAssemblyRequest {
    pub fn new(user_request: impl Into<String>) -> Self {
        ContextAssemblyRequest {
            user_request: user_request.into(),
            conversation_history: Vec::new(),
            tool_results: Vec::new(),
            project_info: None,
            workspace: None,
            indexer: None,
            memory: None,
        }
    }

    pub fn with_conversation(mut self, history: Vec<ContextMessage>) -> Self {
        self.conversation_history = history;
        self
    }

    pub fn with_tool_results(mut self, results: Vec<ContextFragment>) -> Self {
        self.tool_results = results;
        self
    }

    pub fn with_project_info(mut self, info: crate::scanner::ProjectInfo) -> Self {
        self.project_info = Some(info);
        self
    }

    pub fn with_workspace(mut self, ws: WorkspaceRuntime) -> Self {
        self.workspace = Some(std::sync::Arc::new(ws));
        self
    }

    pub fn with_indexer(mut self, idx: CodeIndexer) -> Self {
        self.indexer = Some(std::sync::Arc::new(idx));
        self
    }

    pub fn with_memory(mut self, mem: MemoryRuntime) -> Self {
        self.memory = Some(std::sync::Arc::new(mem));
        self
    }
}

/// The result of a context assembly run.
#[derive(Debug, Clone)]
pub struct ContextAssemblyResult {
    pub fragments: Vec<ContextFragment>,
    pub intent: IntentClassification,
    pub statistics: AssemblyStatistics,
    pub config: AssemblyConfig,
}

impl ContextAssemblyResult {
    /// Render the assembled context as a single string ready for the prompt
    /// builder.
    pub fn render(&self) -> String {
        let mut out = String::new();

        for section in ContextSection::canonical_order() {
            let section_frags: Vec<&ContextFragment> = self
                .fragments
                .iter()
                .filter(|f| ordering::ContextSection::from_source(&f.source) == section)
                .collect();
            if section_frags.is_empty() {
                continue;
            }
            out.push_str(&format!("=== {} ===\n", section));
            for frag in section_frags {
                out.push_str(&frag.content);
                out.push('\n');
            }
            out.push('\n');
        }

        out
    }
}

/// The Context Assembly Engine.
///
/// The assembler is provider-agnostic: it only reads from runtime sources
/// and produces a ranked, deduplicated, budget-constrained fragment list.
/// The caller (Prompt Builder v2, Sprint 20) decides how to serialise the
/// result into an LLM prompt.
pub struct ContextAssembler {
    config: AssemblyConfig,
}

impl ContextAssembler {
    pub fn new(config: AssemblyConfig) -> Self {
        ContextAssembler { config }
    }

    pub fn with_config(mut self, config: AssemblyConfig) -> Self {
        self.config = config;
        self
    }

    pub fn config(&self) -> &AssemblyConfig {
        &self.config
    }

    /// Run the full assembly pipeline for `request`.
    pub fn assemble(&self, request: &ContextAssemblyRequest) -> Result<ContextAssemblyResult> {
        use std::time::Instant;
        let start = Instant::now();

        // 1. Intent classification
        let intent = IntentClassification::classify(&request.user_request);

        // 2. Source selection (which sources to query)
        let source_prefs = intent.source_preferences();

        // 3. Fragment collection
        let mut all_fragments = Vec::new();

        // User request fragment
        all_fragments.extend(
            request
                .user_request
                .as_str()
                .collect_user_request_fragment(&request.user_request)?,
        );

        // Conversation history
        for msg in &request.conversation_history {
            all_fragments.push(ContextFragment::new(
                ContextSource::UserRequest,
                ContextPriority::Low,
                format!("[{}]: {}", msg.role, msg.content),
                0.4,
            ));
        }

        // Workspace
        if source_prefs.contains(&"workspace") {
            if let Some(ref ws) = request.workspace {
                all_fragments.extend(ws.collect_workspace_fragments(&request.user_request)?);
            }
        }

        // Engineering facts / symbols
        if source_prefs.contains(&"engineering_facts") {
            if let Some(ref idx) = request.indexer {
                all_fragments.extend(idx.collect_facts_fragments(
                    &request.user_request,
                    self.config.ranking_weights.normalised().symbol_proximity as usize + 10,
                )?);
            }
        }

        // Indexer (relevant files)
        if source_prefs.contains(&"indexer") {
            if let Some(ref idx) = request.indexer {
                all_fragments.extend(
                    idx.collect_indexer_fragments(&request.user_request, self.config.max_files)?,
                );
            }
        }

        // Git
        if source_prefs.contains(&"git") {
            if let Some(ref ws) = request.workspace {
                all_fragments.extend(ws.collect_git_fragments(&request.user_request)?);
            }
        }

        // Memory
        if source_prefs.contains(&"memory") {
            if let Some(ref mem) = request.memory {
                all_fragments.extend(
                    mem.collect_memory_fragments(&request.user_request, self.config.max_memories)?,
                );
            }
        }

        // Scanner / project info
        if let Some(ref proj) = request.project_info {
            all_fragments.extend(proj.collect_scanner_fragments()?);
        }

        // Tool results
        if !request.tool_results.is_empty() {
            all_fragments.extend(
                request
                    .tool_results
                    .collect_tool_results_fragments(self.config.max_files)?,
            );
        }

        let total_fragments = all_fragments.len();

        // 4. Ranking
        super::sources::rank_fragments(&mut all_fragments);

        // 5. Deduplication
        let dup_before = all_fragments.len();
        super::sources::dedup_fragments(&mut all_fragments);
        let duplicate_count = dup_before - all_fragments.len();

        // 6. Token budget
        let budget = self.config.default_budget.clone().into_budget();
        let discard_before = all_fragments.len();
        let discarded = budget::apply(&mut all_fragments, &budget);

        // 7. Ordering
        ordering::order_fragments(&mut all_fragments);

        let elapsed_ms = start.elapsed().as_millis() as u64;

        let estimated_tokens = budget::total_tokens(&all_fragments);
        let max_score = all_fragments
            .first()
            .map(|f| f.relevance_score)
            .unwrap_or(0.0);
        let min_score = all_fragments
            .last()
            .map(|f| f.relevance_score)
            .unwrap_or(0.0);

        let mut per_source = std::collections::HashMap::new();
        for f in &all_fragments {
            let key = f.source.to_string();
            *per_source.entry(key).or_insert(0) += 1;
        }

        let statistics = AssemblyStatistics {
            total_fragments,
            selected_fragments: all_fragments.len(),
            duplicate_count,
            discarded_fragments: discarded,
            estimated_tokens,
            max_score,
            min_score,
            per_source,
            elapsed_ms,
        };

        Ok(ContextAssemblyResult {
            fragments: all_fragments,
            intent,
            statistics,
            config: self.config.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembly::sources::ContextSource;
    use crate::assembly::ContextPriority;
    use crate::scanner::ProjectInfo;
    use crate::workspace_runtime::WorkspaceRuntime;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_request(user_request: &str) -> ContextAssemblyRequest {
        ContextAssemblyRequest::new(user_request)
    }

    #[test]
    fn test_assemble_basic_request() {
        let ws = WorkspaceRuntime::new(
            PathBuf::from("."),
            Arc::new(crate::workspace_runtime::LocalFileSystem::new()),
        );
        let req = make_request("add a new function");
        let assembler = ContextAssembler::new(AssemblyConfig::default());
        let result = assembler.assemble(&req).unwrap();

        assert!(!result.fragments.is_empty());
        assert_eq!(result.intent.intent, IntentType::Modification);
        assert!(result.statistics.selected_fragments > 0);
    }

    #[test]
    fn test_assemble_debug_request_prioritises_diagnostics() {
        let req = make_request("fix the auth bug");
        let assembler = ContextAssembler::new(AssemblyConfig::default());
        let result = assembler.assemble(&req).unwrap();
        assert_eq!(result.intent.intent, IntentType::Debugging);
        assert!(result.intent.prioritise_diagnostics);
    }

    #[test]
    fn test_assemble_with_project_info() {
        let info = ProjectInfo {
            name: "test-project".to_string(),
            path: PathBuf::from("."),
            language: "rust".to_string(),
            framework: None,
            build_system: Some("cargo".to_string()),
            package_manager: Some("cargo".to_string()),
            testing_framework: None,
            important_files: vec!["Cargo.toml".to_string()],
        };
        let req = make_request("what does this project do").with_project_info(info);
        let assembler = ContextAssembler::new(AssemblyConfig::default());
        let result = assembler.assemble(&req).unwrap();

        let has_scanner = result
            .fragments
            .iter()
            .any(|f| f.source == ContextSource::Scanner);
        assert!(has_scanner);
    }

    #[test]
    fn test_assemble_small_budget() {
        let mut cfg = AssemblyConfig::default();
        cfg.default_budget = TokenBudget::Small;
        let req = make_request("explain everything about this repo");
        let assembler = ContextAssembler::new(cfg);
        let result = assembler.assemble(&req).unwrap();

        assert!(result.statistics.estimated_tokens <= 2000);
    }

    #[test]
    fn test_render_sections() {
        let mut frags = vec![
            ContextFragment::new(
                ContextSource::UserRequest,
                ContextPriority::Critical,
                "user prompt".to_string(),
                1.0,
            ),
            ContextFragment::new(
                ContextSource::Memory,
                ContextPriority::Medium,
                "memory entry".to_string(),
                0.5,
            ),
        ];
        let result = ContextAssemblyResult {
            fragments: frags,
            intent: IntentClassification::classify("test"),
            statistics: AssemblyStatistics::default(),
            config: AssemblyConfig::default(),
        };
        let rendered = result.render();
        assert!(rendered.contains("user_intent"));
        assert!(rendered.contains("memory"));
        assert!(rendered.contains("user prompt"));
        assert!(rendered.contains("memory entry"));
    }

    #[test]
    fn test_assemble_flows_tool_results_into_fragments() {
        // Tool → ToolResult → Context Fragment → ContextAssembler →
        // EngineeringContext. Tool results must enter the canonical
        // fragment pipeline, not string concatenation.
        let req = make_request("run the tests").with_tool_results(vec![
            ContextFragment::new(
                ContextSource::ToolResults,
                ContextPriority::High,
                "test output: 12 passed, 0 failed".to_string(),
                0.9,
            ),
            ContextFragment::new(
                ContextSource::ToolResults,
                ContextPriority::Medium,
                "coverage: 81%".to_string(),
                0.6,
            ),
        ]);
        let assembler = ContextAssembler::new(AssemblyConfig::default());
        let result = assembler.assemble(&req).unwrap();

        let tool_frags: Vec<&ContextFragment> = result
            .fragments
            .iter()
            .filter(|f| f.source == ContextSource::ToolResults)
            .collect();
        assert_eq!(tool_frags.len(), 2);
        assert!(tool_frags.iter().any(|f| f.content.contains("12 passed")));
        assert!(result.statistics.total_fragments >= 3);
    }

    #[test]
    fn test_indexer_files_become_context_fragments() {
        // Indexed workspace files must become actual assembler fragments via
        // the existing indexer source (get_indexed_files → fragments).
        let tmp = tempfile::tempdir().unwrap();
        let source = "pub fn main() {}\n";
        let file = tmp.path().join("src").join("main.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, source).unwrap();

        let db_path = tmp.path().join("index.db");
        {
            let mut idx = crate::intelligence::CodeIndexer::new(db_path.clone()).unwrap();
            idx.index_file(&file, source).unwrap();
        }
        // A fresh indexer over the same DB reconstructs the file list from
        // the symbol store (get_indexed_files reads the DB, not memory).
        let idx = crate::intelligence::CodeIndexer::new(db_path).unwrap();

        let req = make_request("main").with_indexer(idx);
        let assembler = ContextAssembler::new(AssemblyConfig::default());
        let result = assembler.assemble(&req).unwrap();

        let indexer_frags: Vec<&ContextFragment> = result
            .fragments
            .iter()
            .filter(|f| f.source == ContextSource::Indexer)
            .collect();
        assert!(
            !indexer_frags.is_empty(),
            "indexed file should flow into assembler fragments"
        );
    }
}
