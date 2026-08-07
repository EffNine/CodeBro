#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
// This whole module is the legacy execution path and is intentionally marked
// `#[deprecated]`; it only emits deprecation noise about itself.
#![allow(deprecated)]
#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use crate::agent::memory_manager::MemoryConsolidationEngine;
use crate::agent::Memory;
use crate::agent::Planner;
use crate::agent::SkillManager;
use crate::agent::TraceStore;
use crate::config::Config;
use crate::intelligence::index::CodeIndexer;
use crate::intelligence::reasoning::AgentReasoningEngine;
use crate::providers::Provider;
use crate::tools::Tool;
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

/// LEGACY - DO NOT WIRE UP IN PRODUCTION.
///
/// This is the original monolithic agent (planner -> permission -> tools ->
/// provider). It has been SUPERSEDED by the production execution pipeline:
///
///   `src/tools/executor.rs::run_tool_pipeline`  (workspace detect -> project
///      scan -> tool routing -> real filesystem/shell/git tool execution)
///   followed by the LLM synthesis in `src/tui/ui.rs::run_chat_pipeline` /
///   `call_ai_streaming`.
///
/// `Agent` is only kept alive by unit tests. Having a second, silent tool-execution
/// engine invites duplicate path bugs, so it must NOT be constructed from the chat
/// path. If this struct ever becomes unreachable even in tests, delete it.
#[deprecated(note = "Superseded by crate::tools::run_tool_pipeline + call_ai_streaming")]
pub struct Agent {
    pub config: Config,
    pub provider: Box<dyn Provider>,
    pub tools: HashMap<String, Box<dyn Tool>>,
    pub memory: Memory,
    pub planner: Planner,
    pub skill_manager: Option<SkillManager>,
    pub trace_store: Option<TraceStore>,
    pub memory_manager: Option<MemoryConsolidationEngine>,
    pub workspace_manager: Option<crate::agent::workspace::WorkspaceManager>,
    pub permission_manager: Option<crate::agent::permissions::PermissionManager>,
    pub indexer: Option<CodeIndexer>,
    pub task_counter: u64,
}

impl Agent {
    pub fn new(config: Config, provider: Box<dyn Provider>) -> Result<Self> {
        let mut tools = HashMap::new();
        tools.insert(
            "list_files".to_string(),
            Box::new(crate::tools::ListFiles) as Box<dyn Tool>,
        );
        tools.insert(
            "read_file".to_string(),
            Box::new(crate::tools::ReadFile) as Box<dyn Tool>,
        );
        tools.insert(
            "create_file".to_string(),
            Box::new(crate::tools::CreateFile) as Box<dyn Tool>,
        );
        tools.insert(
            "edit_file".to_string(),
            Box::new(crate::tools::EditFile) as Box<dyn Tool>,
        );
        tools.insert(
            "run_command".to_string(),
            Box::new(crate::tools::RunCommand::new()) as Box<dyn Tool>,
        );
        tools.insert(
            "git_status".to_string(),
            Box::new(crate::tools::GitStatus) as Box<dyn Tool>,
        );
        tools.insert(
            "git_diff".to_string(),
            Box::new(crate::tools::GitDiff) as Box<dyn Tool>,
        );

        let memory = Memory::load()?;
        let planner = Planner::new().with_memory(memory.clone());

        let config_dir = Config::config_dir();

        let skill_manager = if config_dir.join("skills").exists() {
            Some(SkillManager::new(config_dir.join("skills"))?)
        } else {
            None
        };

        let trace_store =
            if config_dir.join("traces").exists() || config_dir.join("traces").join(".").exists() {
                Some(TraceStore::new(config_dir.join("traces"))?)
            } else {
                TraceStore::new(config_dir.join("traces"))?;
                Some(TraceStore::new(config_dir.join("traces"))?)
            };

        let memory_manager = Some(MemoryConsolidationEngine::new(config_dir.clone()));

        let workspace_manager = Some(crate::agent::workspace::WorkspaceManager::new(
            config_dir.join("workspace.json"),
        )?);

        let permission_manager = Some(crate::agent::permissions::PermissionManager::new(
            config_dir,
        )?);

        let indexer = Some(CodeIndexer::new(
            Config::config_dir().join("code_index.db"),
        )?);

        let mut planner = Planner::new().with_memory(memory.clone());

        if let Some(ref idx) = indexer {
            let reasoning_engine = AgentReasoningEngine::new(idx.clone());
            planner = planner.with_reasoning_engine(reasoning_engine);
        }

        Ok(Agent {
            config,
            provider,
            tools,
            memory,
            planner,
            skill_manager,
            trace_store,
            memory_manager,
            workspace_manager,
            permission_manager,
            indexer,
            task_counter: 0,
        })
    }

    pub async fn run(&mut self, user_input: &str) -> Result<String> {
        self.task_counter += 1;
        let task_id = format!("task-{}", self.task_counter);

        let tool_names: Vec<&str> = self.tools.keys().map(|s| s.as_str()).collect();
        let project_root = self.workspace_manager.as_ref().and_then(|w| {
            if w.info().root.is_empty() {
                None
            } else {
                Some(w.info().root.as_str())
            }
        });

        let plan = self
            .planner
            .create_plan(user_input, &tool_names, project_root);

        let mut context = String::new();
        context.push_str(&format!("User request: {}\n", user_input));
        context.push_str(&format!("Plan: {}\n", plan));

        let mut tools_executed = Vec::new();
        let files_changed = Vec::new();
        let mut commands_executed = Vec::new();
        let mut result = "success".to_string();

        for tool_name in &plan.tools {
            if let Some(tool) = self.tools.get(tool_name) {
                if let Some(ref pm) = self.permission_manager {
                    let decision = pm.check_permission(tool_name, "");
                    if decision.is_denied() {
                        context.push_str(&format!(
                            "Tool {} denied by permission system.\n",
                            tool_name
                        ));
                        continue;
                    }
                }

                let tool_result =
                    tool.execute(&plan.args.get(tool_name).cloned().unwrap_or_default());

                match tool_result {
                    Ok(output) => {
                        context.push_str(&format!("Tool {} result: {}\n", tool_name, output));
                        tools_executed.push(tool_name.clone());

                        if tool_name == "run_command" {
                            commands_executed
                                .push(plan.args.get(tool_name).cloned().unwrap_or_default());
                        }
                    }
                    Err(e) => {
                        context.push_str(&format!("Tool {} error: {}\n", tool_name, e));
                        result = "partial_failure".to_string();
                    }
                }
            }
        }

        let response = self.provider.send_message(&context).await?;

        self.memory
            .add_entry(user_input.to_string(), response.clone());

        if let Some(ref mut mm) = self.memory_manager {
            mm.consolidate(&mut self.memory);
        }

        self.memory.save()?;

        if let Some(ref mut ws) = self.workspace_manager {
            ws.track_command(user_input)?;
        }

        if let Some(ref ts) = self.trace_store {
            let lesson = if result == "success" {
                None
            } else {
                Some(format!("Task completed with issues: {}", result))
            };

            let trace = crate::agent::trace::create_trace(
                &task_id,
                user_input,
                &plan.summary,
                &tools_executed,
                &files_changed,
                &commands_executed,
                &result,
                lesson,
                &plan.memory_influence,
                plan.skill_used.as_deref(),
            );
            ts.record(&trace)?;
        }

        if let Some(ref mut sm) = self.skill_manager {
            sm.record_usage(
                &plan.skill_used.clone().unwrap_or_default(),
                result == "success",
            )?;
        }

        Ok(response)
    }
}
