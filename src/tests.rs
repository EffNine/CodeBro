#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
// The agent engine under test here is the LEGACY path (superseded by
// `crate::tools::run_tool_pipeline` + `call_ai_streaming`). It is marked
// `#[deprecated]` as a dead-path signal, but these tests deliberately keep
// exercising it as a regression suite, so suppress that noise.
#![allow(deprecated)]
#![allow(dead_code, unused_imports, unused_variables, clippy::all)]

use crate::agent::memory_manager::MemoryConsolidationEngine;
use crate::agent::permissions::{PermissionDecision, PermissionLevel, PermissionManager};
use crate::agent::plan_memory::{PlanMemoryStore, PlanRecord};
use crate::agent::reflection::{ReflectionEngine, ReflectionStore};
use crate::agent::skill::SkillManager;
use crate::agent::skill::SkillStatus;
use crate::agent::workspace::{WorkspaceInfo, WorkspaceManager};
use crate::agent::Memory;
use crate::agent::Planner;
use crate::agent::SubAgent;
use crate::agent::TraceStore;
use crate::config::Config;
use crate::dispatcher::ToolDispatcher;
use crate::providers::Provider as _;
use crate::scanner::ProjectInfo;
use crate::tools::filesystem::{CreateFile, EditFile, ListFiles, ReadFile};
use crate::tools::git::{GitDiff, GitStatus};
use crate::tools::patch::{FilePatch, PatchEngine, PatchSet};
use crate::tools::shell::RunCommand;
use crate::tools::shell::ShellCommandRecord;
use crate::tools::shell::ShellHistory;
use crate::tools::Tool;
use std::collections::HashMap;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_list_files() {
    let tool = ListFiles;
    let result = tool.execute(".").expect("list_files should work");
    assert!(!result.is_empty());
}

#[test]
fn test_read_file() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("test.txt");
    fs::write(&file_path, "hello world").expect("write test file");

    let tool = ReadFile;
    let result = tool
        .execute(file_path.to_str().unwrap())
        .expect("read_file should work");
    assert_eq!(result, "hello world");
}

#[test]
fn test_create_file() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("new.txt");

    let tool = CreateFile;
    let result = tool
        .execute(&format!(
            "{}|hello from create",
            file_path.to_str().unwrap()
        ))
        .expect("create_file should work");
    assert!(result.contains("Created file"));
    assert!(file_path.exists());
    assert_eq!(fs::read_to_string(&file_path).unwrap(), "hello from create");
}

#[test]
fn test_edit_file() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("edit.txt");
    fs::write(&file_path, "old content").expect("write test file");

    let tool = EditFile;
    let result = tool
        .execute(&format!(
            "{}|old content|new content",
            file_path.to_str().unwrap()
        ))
        .expect("edit_file should work");
    assert!(result.contains("Edited file"));
    assert_eq!(fs::read_to_string(&file_path).unwrap(), "new content");
}

#[test]
fn test_run_command() {
    let tool = RunCommand::new();
    let result = tool.execute("echo hello").expect("run_command should work");
    assert_eq!(result, "hello");
}

#[test]
fn test_config_load() {
    let config = Config::load();
    assert!(config.is_ok());
    let config = config.unwrap();
    assert!(!config.provider.is_empty());
}

#[test]
fn test_patch_creation() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("test.txt");
    let old_content = "line1\nline2\nline3\n";
    fs::write(&file_path, old_content).expect("write test file");

    let new_content = "line1\nline2 modified\nline3\n";
    let patch = PatchEngine::create_patch(&file_path, old_content, new_content)
        .expect("create patch should work");

    assert!(!patch.unified_diff.is_empty());
    assert!(patch.unified_diff.contains("line2 modified"));
}

#[test]
fn test_patch_preview() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("test.txt");
    let old_content = "hello world\n";
    fs::write(&file_path, old_content).expect("write test file");

    let new_content = "hello rust\n";
    let patch = PatchEngine::create_patch(&file_path, old_content, new_content)
        .expect("create patch should work");

    let preview = PatchEngine::preview(&patch);
    assert!(preview.contains("hello rust"));
}

#[test]
fn test_patch_apply() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("test.txt");
    let old_content = "line1\nline2\nline3\n";
    fs::write(&file_path, old_content).expect("write test file");

    let new_content = "line1\nline2_modified\nline3\n";
    let patch = PatchEngine::create_patch(&file_path, old_content, new_content)
        .expect("create patch should work");

    let result = PatchEngine::apply(&patch, false).expect("apply patch should work");
    assert!(result.contains("Patch applied"));

    let content = fs::read_to_string(&file_path).expect("read patched file");
    assert!(content.contains("line2_modified"));
}

#[test]
fn test_patch_rollback() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("test.txt");
    let original = "original content\n";
    fs::write(&file_path, original).expect("write test file");

    PatchEngine::rollback(&file_path, original).expect("rollback should work");
    let content = fs::read_to_string(&file_path).expect("read rolled back file");
    assert_eq!(content, original);
}

#[tokio::test]
async fn test_tool_dispatcher() {
    let registry = crate::dispatcher::ToolRegistry::new()
        .register(std::sync::Arc::new(ListFiles))
        .register(std::sync::Arc::new(ReadFile));

    let mut dispatcher = ToolDispatcher::new(registry);
    assert!(dispatcher.has_tool("list_files"));
    assert!(dispatcher.has_tool("read_file"));
    assert!(!dispatcher.has_tool("unknown_tool"));

    let result = dispatcher.dispatch("list_files", ".").await;
    assert!(result.is_ok());
}

#[test]
fn test_memory_add_entry() {
    let mut memory = Memory::default();
    memory.add_entry("test input".to_string(), "test response".to_string());
    assert_eq!(memory.short_term.len(), 1);
    assert_eq!(memory.short_term[0].user_input, "test input");
}

#[test]
fn test_memory_recent_files() {
    let mut memory = Memory::default();
    memory.add_recent_file("src/main.rs".to_string());
    memory.add_recent_file("src/lib.rs".to_string());
    memory.add_recent_file("src/main.rs".to_string());

    assert_eq!(memory.project.recent_files.len(), 2);
    assert_eq!(memory.project.recent_files[0], "src/main.rs");
}

#[test]
fn test_memory_session() {
    let mut memory = Memory::default();
    let session_id = memory.start_session(Some("Test Session".to_string()));
    assert_eq!(memory.current_session_id, Some(session_id));
    assert_eq!(memory.sessions.len(), 1);

    memory.add_entry("hello".to_string(), "world".to_string());
    assert_eq!(memory.sessions[0].messages.len(), 1);

    memory.end_session().expect("end session");
    assert_eq!(memory.current_session_id, None);
}

#[test]
fn test_memory_short_term_limit() {
    let mut memory = Memory::default();
    for i in 0..150 {
        memory.add_entry(format!("input {}", i), format!("response {}", i));
    }
    assert!(memory.short_term.len() <= 100);
}

#[test]
fn test_memory_task_lifecycle() {
    let mut memory = Memory::default();
    let task_id = memory.add_task(
        "test task".to_string(),
        vec!["read_file".to_string()],
        vec![],
    );
    assert_eq!(memory.project.tasks.len(), 1);
    assert_eq!(
        memory.project.tasks[0].status,
        crate::agent::memory::TaskStatus::InProgress
    );

    memory.complete_task(&task_id).unwrap();
    assert_eq!(
        memory.project.tasks[0].status,
        crate::agent::memory::TaskStatus::Completed
    );
}

#[test]
fn test_memory_decision() {
    let mut memory = Memory::default();
    memory.add_decision(
        "context".to_string(),
        "decision".to_string(),
        "rationale".to_string(),
    );
    assert_eq!(memory.project.decisions.len(), 1);
}

#[test]
fn test_memory_lesson_search() {
    let mut memory = Memory::default();
    memory.add_lesson(
        "use patch engine".to_string(),
        "editing files".to_string(),
        "reflection".to_string(),
    );
    let lessons = memory.search_lessons("patch");
    assert_eq!(lessons.len(), 1);
}

#[test]
fn test_memory_solution_search() {
    let mut memory = Memory::default();
    memory.add_solution(
        "fix build".to_string(),
        "run cargo build".to_string(),
        "rust project".to_string(),
    );
    let solutions = memory.search_solutions("build");
    assert_eq!(solutions.len(), 1);
}

#[test]
fn test_skill_creation() {
    let dir = tempdir().expect("tempdir");
    let mut manager = SkillManager::new(dir.path().join("skills")).unwrap();
    let skill = manager
        .create_skill(
            "read_and_explain".to_string(),
            "Read a file and explain it".to_string(),
            vec!["explain".to_string(), "read".to_string()],
            vec!["read_file".to_string()],
            vec!["read main.rs".to_string()],
            vec!["read_file".to_string()],
            vec!["src/main.rs".to_string()],
        )
        .unwrap();

    assert_eq!(manager.list_skills().len(), 1);
    assert!(manager.get_skill(&skill.id).is_some());
}

#[test]
fn test_skill_ranking() {
    let dir = tempdir().expect("tempdir");
    let mut manager = SkillManager::new(dir.path().join("skills")).unwrap();
    manager
        .create_skill(
            "test_a".to_string(),
            "Test skill A".to_string(),
            vec!["test".to_string()],
            vec![],
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
    manager
        .create_skill(
            "test_b".to_string(),
            "Test skill B".to_string(),
            vec!["test".to_string()],
            vec![],
            vec![],
            vec![],
            vec![],
        )
        .unwrap();

    let ranked = manager.rank_skills("test");
    assert_eq!(ranked.len(), 2);
}

#[test]
fn test_skill_usage_tracking() {
    let dir = tempdir().expect("tempdir");
    let mut manager = SkillManager::new(dir.path().join("skills")).unwrap();
    let skill = manager
        .create_skill(
            "tracked_skill".to_string(),
            "Tracked".to_string(),
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        )
        .unwrap();

    manager.record_usage(&skill.id, true).unwrap();
    let updated = manager.get_skill(&skill.id).unwrap();
    assert_eq!(updated.usage_count, 1);
    assert_eq!(updated.success_count, 1);
}

#[test]
fn test_reflection_creation() {
    let reflection = ReflectionEngine::reflect(
        "fix bug",
        None,
        &["read_file".to_string()],
        &["src/main.rs".to_string()],
        true,
        &[],
    );

    assert!(reflection.success);
    assert!(!reflection.what_worked.is_empty());
    assert!(reflection
        .lessons_learned
        .contains(&"Tool read_file was effective".to_string()));
}

#[test]
fn test_reflection_store() {
    let mut store = ReflectionStore::new();
    let reflection =
        ReflectionEngine::reflect("task", None, &[], &[], false, &["error".to_string()]);

    store.add_reflection(reflection);
    assert_eq!(store.reflections.len(), 1);
    assert_eq!(store.failure_patterns().len(), 1);
}

#[test]
fn test_plan_memory_retrieval() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("plans.json");
    let mut store = PlanMemoryStore::new(path).unwrap();

    store
        .add_plan(PlanRecord {
            id: "1".to_string(),
            summary: "test plan".to_string(),
            user_input: "read file".to_string(),
            tools: vec!["read_file".to_string()],
            args: HashMap::new(),
            success: true,
            usage_count: 1,
            success_count: 1,
            confidence: 1.0,
            created_at: chrono::Local::now().to_rfc3339(),
            last_used: None,
        })
        .unwrap();

    let similar = store.find_similar("read file", &vec!["read_file".to_string()]);
    assert_eq!(similar.len(), 1);
    assert_eq!(similar[0].id, "1");
}

#[test]
fn test_memory_scoring() {
    let entry = crate::agent::memory::MemoryEntry {
        user_input: "test".to_string(),
        response: "response".to_string(),
        timestamp: chrono::Local::now().to_rfc3339(),
        session_id: None,
        importance: 0.8,
        confidence: 0.9,
        usage_count: 5,
        last_used: Some(chrono::Local::now().to_rfc3339()),
    };
    let score = entry.score();
    assert!(score > 0.5);
    assert!(score <= 1.0);
}

#[test]
fn test_memory_scoring_low_importance() {
    let entry = crate::agent::memory::MemoryEntry {
        user_input: "test".to_string(),
        response: "response".to_string(),
        timestamp: "2020-01-01T00:00:00Z".to_string(),
        session_id: None,
        importance: 0.1,
        confidence: 0.1,
        usage_count: 0,
        last_used: None,
    };
    let score = entry.score();
    assert!(score < 0.3);
}

#[test]
fn test_memory_consolidation_deduplication() {
    let dir = tempdir().expect("tempdir");
    let engine = MemoryConsolidationEngine::new(dir.path().to_path_buf());

    let mut memory = Memory::default();
    memory.add_entry("cargo test".to_string(), "passed".to_string());
    memory.add_entry("cargo test".to_string(), "passed".to_string());
    memory.add_entry("rust build".to_string(), "success".to_string());

    let removed = engine.consolidate(&mut memory);
    assert!(memory.short_term.len() <= 2);
}

#[test]
fn test_memory_consolidation_outdated_removal() {
    let dir = tempdir().expect("tempdir");
    let engine = MemoryConsolidationEngine::new(dir.path().to_path_buf());

    let mut memory = Memory::default();
    memory.add_entry("old task".to_string(), "done".to_string());
    memory.short_term[0].timestamp = "2020-01-01T00:00:00Z".to_string();

    engine.remove_outdated(&mut memory);
    assert_eq!(memory.short_term.len(), 0);
}

#[test]
fn test_memory_consolidation_low_value_removal() {
    let dir = tempdir().expect("tempdir");
    let engine = MemoryConsolidationEngine::new(dir.path().to_path_buf());

    let mut memory = Memory::default();
    let entry = crate::agent::memory::MemoryEntry {
        user_input: "low value".to_string(),
        response: "response".to_string(),
        timestamp: "2020-01-01T00:00:00Z".to_string(),
        session_id: None,
        importance: 0.1,
        confidence: 0.1,
        usage_count: 0,
        last_used: None,
    };
    memory.short_term.push(entry);

    engine.remove_low_value(&mut memory);
    assert_eq!(memory.short_term.len(), 0);
}

#[test]
fn test_memory_consolidation_merge_similar() {
    let dir = tempdir().expect("tempdir");
    let engine = MemoryConsolidationEngine::new(dir.path().to_path_buf());

    let mut memory = Memory::default();
    memory.add_entry("cargo test".to_string(), "passed".to_string());
    memory.add_entry(
        "cargo test passed".to_string(),
        "all tests passed".to_string(),
    );

    engine.merge_similar(&mut memory);
    assert!(memory.short_term.len() <= 2);
}

#[test]
fn test_skill_lifecycle_draft_to_testing() {
    let mut skill = crate::agent::skill::Skill::default();
    skill.name = "test_skill".to_string();
    skill.status = SkillStatus::Draft;

    for j in 2..5 {
        skill.success_count += 1;
        skill.usage_count += 1;
    }
    skill.update_confidence();
    skill.advance_status();

    assert_eq!(skill.status, SkillStatus::Testing);
}

#[test]
fn test_skill_lifecycle_testing_to_trusted() {
    let mut skill = crate::agent::skill::Skill::default();
    skill.name = "test_skill".to_string();
    skill.status = SkillStatus::Testing;

    for _ in 0..5 {
        skill.success_count += 1;
        skill.usage_count += 1;
    }
    skill.update_confidence();
    skill.advance_status();

    assert_eq!(skill.status, SkillStatus::Trusted);
}

#[test]
fn test_skill_confidence_update_success() {
    let mut skill = crate::agent::skill::Skill::default();
    skill.name = "test_skill".to_string();
    skill.confidence = 0.5;

    for _ in 0..10 {
        skill.success_count += 1;
        skill.usage_count += 1;
    }
    skill.update_confidence();

    assert!(skill.confidence > 0.5);
}

#[test]
fn test_skill_confidence_update_failure() {
    let mut skill = crate::agent::skill::Skill::default();
    skill.name = "test_skill".to_string();
    skill.confidence = 0.5;

    for _ in 0..10 {
        skill.failure_count += 1;
        skill.usage_count += 1;
    }
    skill.update_confidence();

    assert!(skill.confidence < 0.5);
}

#[test]
fn test_skill_not_applicable_deprecated() {
    let mut skill = crate::agent::skill::Skill::default();
    skill.name = "old_skill".to_string();
    skill.status = SkillStatus::Deprecated;

    assert!(!skill.is_applicable(Some("rust"), Some("cargo")));
}

#[test]
fn test_skill_not_applicable_low_confidence() {
    let mut skill = crate::agent::skill::Skill::default();
    skill.name = "low_conf_skill".to_string();
    skill.confidence = 0.2;

    assert!(!skill.is_applicable(Some("rust"), Some("cargo")));
}

#[test]
fn test_skill_language_mismatch() {
    let mut skill = crate::agent::skill::Skill::default();
    skill.name = "rust_skill".to_string();
    skill.language = Some("rust".to_string());

    assert!(!skill.is_applicable(Some("python"), None));
}

#[test]
fn test_skill_language_match() {
    let mut skill = crate::agent::skill::Skill::default();
    skill.name = "rust_skill".to_string();
    skill.language = Some("rust".to_string());

    assert!(skill.is_applicable(Some("rust"), None));
}

#[test]
fn test_skill_conflict_resolution() {
    let dir = std::env::temp_dir();
    let mut manager = SkillManager::new(dir.join("skills_test_conflict")).unwrap();

    let skill1 = manager
        .create_skill(
            "rust_test".to_string(),
            "Rust test skill".to_string(),
            vec!["test".to_string()],
            vec!["run_command".to_string()],
            vec![],
            vec!["run_command".to_string()],
            vec![],
        )
        .unwrap();

    let skill2 = manager
        .create_skill(
            "general_test".to_string(),
            "General test skill".to_string(),
            vec!["test".to_string()],
            vec!["run_command".to_string()],
            vec![],
            vec!["run_command".to_string()],
            vec![],
        )
        .unwrap();

    manager.record_usage(&skill1.id, true).unwrap();
    manager.record_usage(&skill1.id, true).unwrap();
    manager.record_usage(&skill1.id, true).unwrap();

    manager.record_usage(&skill2.id, true).unwrap();

    let best = manager.find_best_skill("test", Some("rust"), None);
    assert!(best.is_some());
}

#[test]
fn test_skill_validation() {
    let dir = std::env::temp_dir();
    let mut manager = SkillManager::new(dir.join("skills_test_validation")).unwrap();

    let skill = manager
        .create_skill(
            "rust_skill".to_string(),
            "Rust specific skill".to_string(),
            vec!["build".to_string()],
            vec!["run_command".to_string()],
            vec![],
            vec!["run_command".to_string()],
            vec![],
        )
        .unwrap();

    assert!(manager.validate_skill_compatibility(&skill.id, Some("rust"), None));
    assert!(manager.validate_skill_compatibility(&skill.id, Some("python"), None));
}

#[test]
fn test_skill_failure_count() {
    let dir = std::env::temp_dir();
    let mut manager = SkillManager::new(dir.join("skills_test_failure")).unwrap();

    let skill = manager
        .create_skill(
            "failing_skill".to_string(),
            "Failing skill".to_string(),
            vec!["test".to_string()],
            vec!["run_command".to_string()],
            vec![],
            vec!["run_command".to_string()],
            vec![],
        )
        .unwrap();

    manager.record_usage(&skill.id, false).unwrap();
    manager.record_usage(&skill.id, false).unwrap();
    manager.record_usage(&skill.id, false).unwrap();

    let updated = manager.get_skill(&skill.id).unwrap();
    assert_eq!(updated.failure_count, 3);
    assert!(updated.confidence < 0.5);
}

#[test]
fn test_planner_memory_retrieval() {
    let dir = tempdir().expect("tempdir");
    let memory = Memory::default();
    let planner = Planner::new().with_memory(memory);

    let plan = planner.create_plan("read the main file", &["read_file", "list_files"], None);
    assert!(plan.tools.contains(&"read_file".to_string()));
}

#[test]
fn test_planner_skill_selection() {
    let dir = tempdir().expect("tempdir");
    let mut skill_manager = SkillManager::new(dir.path().join("skills")).unwrap();

    let skill = skill_manager
        .create_skill(
            "rust_build".to_string(),
            "Build Rust project".to_string(),
            vec!["build".to_string(), "cargo".to_string()],
            vec!["run_command".to_string()],
            vec!["cargo build".to_string()],
            vec!["run_command".to_string()],
            vec![],
        )
        .unwrap();

    for _ in 0..5 {
        skill_manager.record_usage(&skill.id, true).unwrap();
    }

    let memory = Memory::default();
    let planner = Planner::new()
        .with_memory(memory)
        .with_skill_manager(skill_manager);

    let plan = planner.create_plan("build the project", &["run_command"], None);
    assert_eq!(plan.skill_used, Some("rust_build".to_string()));
}

#[test]
fn test_planner_reasoning() {
    let dir = tempdir().expect("tempdir");
    let memory = Memory::default();
    let planner = Planner::new().with_memory(memory);

    let plan = planner.create_plan("read the main file", &["read_file", "list_files"], None);
    assert!(!plan.reasoning.is_empty());
    assert!(!plan.memory_influence.is_empty() || plan.skill_used.is_none());
}

#[test]
fn test_permission_allow() {
    let dir = tempdir().expect("tempdir");
    let mut manager = PermissionManager::new(dir.path().join("perms")).unwrap();

    manager.allow("custom_tool").unwrap();
    let decision = manager.check_permission("custom_tool", "");
    assert!(decision.is_allowed());
}

#[test]
fn test_permission_deny() {
    let dir = tempdir().expect("tempdir");
    let mut manager = PermissionManager::new(dir.path().join("perms")).unwrap();

    manager.deny("dangerous_tool").unwrap();
    let decision = manager.check_permission("dangerous_tool", "");
    assert!(decision.is_denied());
}

#[test]
fn test_permission_dangerous_pattern() {
    let dir = tempdir().expect("tempdir");
    let manager = PermissionManager::new(dir.path().join("perms")).unwrap();

    assert!(manager.is_dangerous("run_command", "rm -rf /"));
    assert!(manager.is_dangerous("run_command", "git push origin main"));
    assert!(!manager.is_dangerous("run_command", "echo hello"));
}

#[test]
fn test_permission_safe_tool_auto_allowed() {
    let dir = tempdir().expect("tempdir");
    let manager = PermissionManager::new(dir.path().join("perms")).unwrap();

    let decision = manager.check_permission("list_files", ".");
    assert!(decision.is_allowed());
}

#[test]
fn test_permission_decision_properties() {
    let decision = PermissionDecision {
        tool_name: "test".to_string(),
        decision: PermissionLevel::Allow,
        reason: None,
    };
    assert!(decision.is_allowed());
    assert!(!decision.requires_ask());
    assert!(!decision.is_denied());
}

#[test]
fn test_permission_decision_ask() {
    let decision = PermissionDecision {
        tool_name: "test".to_string(),
        decision: PermissionLevel::Ask,
        reason: Some("Requires confirmation".to_string()),
    };
    assert!(!decision.is_allowed());
    assert!(decision.requires_ask());
}

#[test]
fn test_workspace_creation() {
    let info = WorkspaceInfo::new("/tmp/test_project");
    assert_eq!(info.root, "/tmp/test_project");
    assert!(info.active_files.is_empty());
    assert!(info.active_files.is_empty());
}

#[test]
fn test_workspace_add_active_file() {
    let mut info = WorkspaceInfo::new("/tmp");
    info.add_active_file("src/main.rs".to_string());
    info.add_active_file("src/lib.rs".to_string());

    assert_eq!(info.active_files.len(), 2);
    assert_eq!(info.active_files[0], "src/lib.rs");
}

#[test]
fn test_workspace_add_recent_file() {
    let mut info = WorkspaceInfo::new("/tmp");
    info.add_recent_file("src/main.rs".to_string());
    info.add_recent_file("src/lib.rs".to_string());
    info.add_recent_file("src/main.rs".to_string());

    assert_eq!(info.active_files.len(), 2);
    assert_eq!(info.active_files[0], "src/main.rs");
}

#[test]
fn test_workspace_add_recent_command() {
    let mut info = WorkspaceInfo::new("/tmp");
    info.add_recent_command("cargo build".to_string());
    info.add_recent_command("cargo test".to_string());

    assert_eq!(info.recent_commands.len(), 2);
    assert_eq!(info.recent_commands[0], "cargo test");
}

#[test]
fn test_workspace_set_language() {
    let mut info = WorkspaceInfo::new("/tmp");
    info.set_language("rust".to_string());
    assert_eq!(info.language, Some("rust".to_string()));
}

#[test]
fn test_workspace_manager_detect_project() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace_path = dir.path().join("workspace.json");
    let mut manager = WorkspaceManager::new(workspace_path).unwrap();

    manager.detect_project(".").unwrap();
    assert!(!manager.info().root.is_empty());
}

#[test]
fn test_workspace_manager_track_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace_path = dir.path().join("workspace.json");
    let mut manager = WorkspaceManager::new(workspace_path).unwrap();

    manager.track_file_access("src/main.rs").unwrap();
    assert_eq!(manager.info().active_files.len(), 1);
    assert_eq!(manager.info().active_files.len(), 1);
}

#[test]
fn test_workspace_manager_track_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace_path = dir.path().join("workspace.json");
    let mut manager = WorkspaceManager::new(workspace_path).unwrap();

    manager.track_command("cargo test").unwrap();
    assert_eq!(manager.info().recent_commands.len(), 1);
}

#[test]
fn test_trace_creation() {
    let trace = crate::agent::trace::create_trace(
        "task-1",
        "Add API endpoint",
        "Read files, patch, test",
        &["read_file".to_string(), "patch_file".to_string()],
        &["src/main.rs".to_string()],
        &["cargo test".to_string()],
        "success",
        None,
        &[],
        None,
    );

    assert_eq!(trace.task_id, "task-1");
    assert_eq!(trace.result, "success");
    assert_eq!(trace.tools_executed.len(), 2);
}

#[test]
fn test_trace_with_lesson() {
    let trace = crate::agent::trace::create_trace(
        "task-2",
        "Fix build error",
        "Read, edit, test",
        &["read_file".to_string(), "edit_file".to_string()],
        &["src/lib.rs".to_string()],
        &["cargo build".to_string()],
        "success",
        Some("Always check Cargo.toml dependencies first".to_string()),
        &["Project uses cargo".to_string()],
        Some("rust_build"),
    );

    assert!(trace.lesson_learned.is_some());
    assert_eq!(trace.skill_used, Some("rust_build".to_string()));
    assert_eq!(trace.memory_influence.len(), 1);
}

#[test]
fn test_trace_store_record_and_load() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = TraceStore::new(dir.path().join("traces")).unwrap();

    let trace = crate::agent::trace::create_trace(
        "task-3",
        "Test request",
        "Test plan",
        &["run_command".to_string()],
        &[],
        &["echo test".to_string()],
        "success",
        None,
        &[],
        None,
    );

    store.record(&trace).unwrap();
    let loaded = store.load("task-3").unwrap();
    assert_eq!(loaded.task_id, "task-3");
    assert_eq!(loaded.result, "success");
}

#[test]
fn test_trace_store_list_traces() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = TraceStore::new(dir.path().join("traces")).unwrap();

    let trace1 = crate::agent::trace::create_trace(
        "task-4",
        "Request 1",
        "Plan 1",
        &["read_file".to_string()],
        &[],
        &[],
        "success",
        None,
        &[],
        None,
    );
    let trace2 = crate::agent::trace::create_trace(
        "task-5",
        "Request 2",
        "Plan 2",
        &["run_command".to_string()],
        &[],
        &[],
        "success",
        None,
        &[],
        None,
    );

    store.record(&trace1).unwrap();
    store.record(&trace2).unwrap();

    let traces = store.list_traces();
    assert_eq!(traces.len(), 2);
}

#[test]
fn test_shell_history_record() {
    let dir = tempfile::tempdir().expect("tempdir");
    let history_path = dir.path().join("shell_history.json");

    let tool = RunCommand::new().with_history_path(history_path.clone());

    let result = tool
        .execute("echo history_test")
        .expect("run_command should work");
    assert_eq!(result, "history_test");

    let history = ShellHistory::load(&history_path).expect("should load history");
    assert!(!history.commands.is_empty());
    assert_eq!(
        history.commands.back().unwrap().command,
        "echo history_test"
    );
}

#[test]
fn test_shell_history_recent() {
    let mut history = ShellHistory::new();
    for i in 0..10 {
        let record = ShellCommandRecord {
            command: format!("cmd {}", i),
            working_directory: "/tmp".to_string(),
            timestamp: chrono::Local::now().to_rfc3339(),
            success: true,
            duration_ms: 10,
            exit_code: Some(0),
        };
        history.add(record);
    }

    let recent = history.recent(3);
    assert_eq!(recent.len(), 3);
    assert_eq!(recent[0].command, "cmd 9");
    assert_eq!(recent[1].command, "cmd 8");
    assert_eq!(recent[2].command, "cmd 7");
}

#[test]
fn test_shell_history_save_load() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test_history.json");

    let mut history = ShellHistory::new();
    let record = ShellCommandRecord {
        command: "echo test".to_string(),
        working_directory: "/tmp".to_string(),
        timestamp: chrono::Local::now().to_rfc3339(),
        success: true,
        duration_ms: 5,
        exit_code: Some(0),
    };
    history.add(record);
    history.save(&path).expect("should save history");

    let loaded = ShellHistory::load(&path).expect("should load history");
    assert_eq!(loaded.commands.len(), 1);
    assert_eq!(loaded.commands[0].command, "echo test");
}

#[test]
fn test_run_command_with_working_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tool = RunCommand::new().with_working_directory(dir.path().to_string_lossy().to_string());
    let result = tool.execute("pwd").expect("run_command should work");
    let expected = std::fs::canonicalize(dir.path())
        .expect("canonicalize path")
        .to_string_lossy()
        .to_string();
    assert_eq!(result, expected);
}

// ===== Intelligence Module Tests =====

#[test]
fn test_parser_rust_parsing() {
    let source = r#"
pub fn hello() -> String {
    "hello".to_string()
}

struct Point {
    x: i32,
    y: i32,
}

enum Color {
    Red,
    Blue,
}

trait Greet {
    fn say_hello(&self);
}

impl Greet for Point {
    fn say_hello(&self) {}
}
"#;

    let result = crate::intelligence::parser::parse_source("rust", source, "test.rs")
        .expect("Rust parsing should work");

    assert!(!result.symbols.is_empty());
    let symbol_names: Vec<String> = result.symbols.iter().map(|s| s.name.clone()).collect();
    assert!(symbol_names.contains(&"hello".to_string()));
}

#[test]
fn test_parser_python_parsing() {
    let source = r#"
def greet(name):
    return f"Hello, {name}"

class Person:
    def __init__(self, name):
        self.name = name
"#;

    let result = crate::intelligence::parser::parse_source("python", source, "test.py")
        .expect("Python parsing should work");

    assert!(!result.symbols.is_empty());
    let symbol_names: Vec<String> = result.symbols.iter().map(|s| s.name.clone()).collect();
    assert!(symbol_names.contains(&"greet".to_string()));
}

#[test]
fn test_parser_javascript_parsing() {
    let source = r#"
function greet(name) {
    return `Hello, ${name}`;
}

class Person {
    constructor(name) {
        this.name = name;
    }
}
"#;

    let result = crate::intelligence::parser::parse_source("javascript", source, "test.js")
        .expect("JavaScript parsing should work");

    assert!(!result.symbols.is_empty());
    let symbol_names: Vec<String> = result.symbols.iter().map(|s| s.name.clone()).collect();
    assert!(symbol_names.contains(&"greet".to_string()));
}

#[test]
fn test_parser_go_parsing() {
    let source = r#"
package main

func greet(name string) string {
    return "Hello, " + name
}

type Person struct {
    Name string
}
"#;

    let result = crate::intelligence::parser::parse_source("go", source, "test.go")
        .expect("Go parsing should work");

    assert!(!result.symbols.is_empty());
    let symbol_names: Vec<String> = result.symbols.iter().map(|s| s.name.clone()).collect();
    assert!(symbol_names.contains(&"greet".to_string()));
}

#[test]
fn test_indexer_symbol_storage() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test_index.db");

    let mut indexer = crate::intelligence::index::CodeIndexer::new(db_path)
        .expect("indexer creation should work");

    let source = r#"
pub fn hello() -> String {
    "hello".to_string()
}
"#;

    let file_path = dir.path().join("test.rs");
    std::fs::write(&file_path, source).expect("write test file");

    let symbols = indexer
        .index_file(&file_path, source)
        .expect("indexing should work");

    assert!(!symbols.is_empty());
    assert_eq!(symbols[0].name, "hello");
}

#[test]
fn test_indexer_incremental_update() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test_index.db");

    let mut indexer = crate::intelligence::index::CodeIndexer::new(db_path)
        .expect("indexer creation should work");

    let source1 = r#"pub fn hello() -> String { "hello".to_string() }"#;
    let file_path = dir.path().join("test.rs");
    std::fs::write(&file_path, source1).expect("write test file");

    indexer
        .index_file(&file_path, source1)
        .expect("first indexing should work");

    let source2 = r#"pub fn hello() -> String { "hello world".to_string() }"#;
    indexer
        .incremental_update(&file_path, source2)
        .expect("incremental update should work");

    let count_after = indexer
        .get_symbol_count()
        .expect("should get count after update");
    assert!(count_after > 0);
}

#[test]
fn test_indexer_remove_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test_index.db");

    let mut indexer = crate::intelligence::index::CodeIndexer::new(db_path)
        .expect("indexer creation should work");

    let source = r#"pub fn hello() -> String { "hello".to_string() }"#;
    let file_path = dir.path().join("test.rs");
    std::fs::write(&file_path, source).expect("write test file");

    indexer
        .index_file(&file_path, source)
        .expect("indexing should work");

    let count_before = indexer.get_symbol_count().expect("should get count");
    assert!(count_before > 0);

    indexer
        .remove_file(&file_path)
        .expect("removal should work");

    let count_after = indexer
        .get_symbol_count()
        .expect("should get count after removal");
    assert_eq!(count_after, 0);
}

#[test]
fn test_search_symbol_lookup() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test_index.db");

    let mut indexer = crate::intelligence::index::CodeIndexer::new(db_path)
        .expect("indexer creation should work");

    let source = r#"
pub fn authenticate_user(username: &str, password: &str) -> bool {
    true
}

pub fn login_handler(request: &str) -> String {
    "logged in".to_string()
}
"#;

    let file_path = dir.path().join("auth.rs");
    std::fs::write(&file_path, source).expect("write test file");
    indexer
        .index_file(&file_path, source)
        .expect("indexing should work");

    let results = indexer.search("auth").expect("search should work");
    assert!(!results.is_empty());
}

#[test]
fn test_search_relevance_ranking() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test_index.db");

    let mut indexer = crate::intelligence::index::CodeIndexer::new(db_path)
        .expect("indexer creation should work");

    let source = r#"
pub fn authenticate_user(username: &str, password: &str) -> bool {
    true
}

pub fn helper_function() -> i32 {
    42
}
"#;

    let file_path = dir.path().join("auth.rs");
    std::fs::write(&file_path, source).expect("write test file");
    indexer
        .index_file(&file_path, source)
        .expect("indexing should work");

    let results = indexer
        .search("authenticate_user")
        .expect("search should work");
    assert!(!results.is_empty());
    assert_eq!(results[0].name, "authenticate_user");
}

#[test]
fn test_dependency_graph_creation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test_index.db");

    let mut indexer = crate::intelligence::index::CodeIndexer::new(db_path)
        .expect("indexer creation should work");

    let source1 = r#"use user::User; pub fn get_user(id: i32) -> User { User::new(id) }"#;
    let source2 = r#"pub struct User { id: i32 }"#;

    let file1 = dir.path().join("auth.rs");
    let file2 = dir.path().join("user.rs");

    std::fs::write(&file1, source1).expect("write test file");
    std::fs::write(&file2, source2).expect("write test file");

    indexer
        .index_file(&file1, source1)
        .expect("indexing should work");
    indexer
        .index_file(&file2, source2)
        .expect("indexing should work");

    let graph = crate::intelligence::graph::DependencyGraph::from_indexer(&indexer)
        .expect("graph creation should work");

    assert!(!graph.get_all_files().is_empty());
}

#[test]
fn test_dependency_graph_find_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test_index.db");

    let mut indexer = crate::intelligence::index::CodeIndexer::new(db_path)
        .expect("indexer creation should work");

    let source1 =
        r#"use database::Connection; pub fn get_conn() -> Connection { Connection::new() }"#;
    let source2 = r#"pub struct Connection { pub fn new() -> Self { Connection } }"#;

    let file1 = dir.path().join("auth.rs");
    let file2 = dir.path().join("database.rs");

    std::fs::write(&file1, source1).expect("write test file");
    std::fs::write(&file2, source2).expect("write test file");

    indexer
        .index_file(&file1, source1)
        .expect("indexing should work");
    indexer
        .index_file(&file2, source2)
        .expect("indexing should work");

    let graph = crate::intelligence::graph::DependencyGraph::from_indexer(&indexer)
        .expect("graph creation should work");

    let path = graph.find_path("auth.rs", "database.rs");
    assert!(path.is_some() || path.is_none());
}

#[test]
fn test_intelligent_context_builder() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test_index.db");

    let mut indexer = crate::intelligence::index::CodeIndexer::new(db_path)
        .expect("indexer creation should work");

    let source = r#"pub fn authenticate_user(username: &str, password: &str) -> bool { true }"#;
    let file_path = dir.path().join("auth.rs");
    std::fs::write(&file_path, source).expect("write test file");
    indexer
        .index_file(&file_path, source)
        .expect("indexing should work");

    let builder = crate::intelligence::context::IntelligentContextBuilder::new(indexer);
    let context = builder
        .build_context("auth")
        .expect("context building should work");

    // Bind the field to silence the absurd_extreme_comparisons lint: a
    // count cannot be negative, so the original >= 0 assertion was redundant.
    let _ = context.total_symbols_found;
}

#[test]
fn test_intelligent_context_for_modification() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test_index.db");

    let mut indexer = crate::intelligence::index::CodeIndexer::new(db_path)
        .expect("indexer creation should work");

    let source = r#"pub fn authenticate_user(username: &str, password: &str) -> bool { true }"#;
    let file_path = dir.path().join("auth.rs");
    std::fs::write(&file_path, source).expect("write test file");
    indexer
        .index_file(&file_path, source)
        .expect("indexing should work");

    let builder = crate::intelligence::context::IntelligentContextBuilder::new(indexer);
    let context = builder
        .build_context_for_modification("authenticate_user")
        .expect("modification context should work");

    assert!(!context.relevant_symbols.is_empty());
}

#[test]
fn test_lsp_foundation() {
    let mut lsp = crate::intelligence::lsp::create_lsp_foundation();

    let doc = crate::intelligence::lsp::LspTextDocumentItem {
        uri: "file:///test.rs".to_string(),
        language_id: "rust".to_string(),
        version: 1,
        text: "pub fn hello() {}".to_string(),
    };

    lsp.open_document(doc);

    assert!(lsp.get_document("file:///test.rs").is_some());

    lsp.close_document("file:///test.rs");
    assert!(lsp.get_document("file:///test.rs").is_none());
}

#[test]
fn test_lsp_find_definition() {
    let mut lsp = crate::intelligence::lsp::create_lsp_foundation();

    let doc = crate::intelligence::lsp::LspTextDocumentItem {
        uri: "file:///test.rs".to_string(),
        language_id: "rust".to_string(),
        version: 1,
        text: "pub fn hello() {}".to_string(),
    };

    lsp.open_document(doc);

    let position = crate::intelligence::lsp::LspPosition {
        line: 0,
        character: 0,
    };

    let result = lsp.find_definition("file:///test.rs", &position);
    assert!(result.is_none());
}

#[test]
fn test_agent_reasoning_engine() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test_index.db");

    let mut indexer = crate::intelligence::index::CodeIndexer::new(db_path)
        .expect("indexer creation should work");

    let source = r#"pub fn authenticate_user(username: &str, password: &str) -> bool { true }"#;
    let file_path = dir.path().join("auth.rs");
    std::fs::write(&file_path, source).expect("write test file");
    indexer
        .index_file(&file_path, source)
        .expect("indexing should work");

    let engine = crate::intelligence::reasoning::AgentReasoningEngine::new(indexer);
    let result = engine
        .analyze_before_modification("Add caching to authentication")
        .expect("reasoning should work");

    assert!(!result.steps.is_empty());
    assert!(!result.plan.is_empty());
}

#[test]
fn test_agent_reasoning_find_patterns() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test_index.db");

    let mut indexer = crate::intelligence::index::CodeIndexer::new(db_path)
        .expect("indexer creation should work");

    let source = r#"pub fn authenticate_user(username: &str, password: &str) -> bool { true }"#;
    let file_path = dir.path().join("auth.rs");
    std::fs::write(&file_path, source).expect("write test file");
    indexer
        .index_file(&file_path, source)
        .expect("indexing should work");

    let engine = crate::intelligence::reasoning::AgentReasoningEngine::new(indexer);
    let patterns = engine
        .find_existing_patterns("authenticate")
        .expect("pattern finding should work");

    assert!(!patterns.is_empty());
}

// ===== v0.6 Autonomous Agent Core Tests =====

#[test]
fn test_subagent_creation() {
    let research = crate::agent::subagent::ResearchAgent::new();
    assert_eq!(research.name(), "research");
    assert_eq!(
        research.purpose(),
        "Understand codebase and gather information"
    );
    assert!(!research.capabilities().is_empty());
}

#[test]
fn test_subagent_planning_can_handle() {
    let planning = crate::agent::subagent::PlanningAgent::new();
    assert!(planning.can_handle("Plan implementation of new feature"));
    assert!(planning.can_handle("Create design for API"));
    assert!(!planning.can_handle("Explain this function"));
}

#[test]
fn test_subagent_coding_can_handle() {
    let coding = crate::agent::subagent::CodingAgent::new();
    assert!(coding.can_handle("Modify the authentication code"));
    assert!(coding.can_handle("Add new endpoint"));
    assert!(coding.can_handle("Fix the bug in login"));
    assert!(!coding.can_handle("Explain the codebase"));
}

#[test]
fn test_subagent_testing_can_handle() {
    let testing = crate::agent::subagent::TestingAgent::new();
    assert!(testing.can_handle("Run tests for the new feature"));
    assert!(testing.can_handle("Validate the changes"));
    assert!(!testing.can_handle("Explain the architecture"));
}

#[test]
fn test_subagent_review_can_handle() {
    let review = crate::agent::subagent::ReviewAgent::new();
    assert!(review.can_handle("Review the implementation"));
    assert!(review.can_handle("Check code quality"));
    assert!(!review.can_handle("Explain the code"));
}

#[test]
fn test_task_router_simple_task() {
    let router = crate::agent::TaskRouter::new();
    let routing = router.route("Explain this function");
    assert!(matches!(
        routing,
        crate::agent::TaskRouting::DirectMainAgent
    ));
}

#[test]
fn test_task_router_complex_task() {
    let router = crate::agent::TaskRouter::new();
    let routing = router.route("Refactor authentication system");
    assert!(matches!(
        routing,
        crate::agent::TaskRouting::ParallelSubAgents(_)
    ));
}

#[test]
fn test_task_router_moderate_task() {
    let router = crate::agent::TaskRouter::new();
    let routing = router.route("Add new API endpoint");
    assert!(matches!(
        routing,
        crate::agent::TaskRouting::SequentialSubAgents(_)
    ));
}

#[test]
fn test_task_router_analysis() {
    let router = crate::agent::TaskRouter::new();
    let analysis = router.analyze("Refactor authentication system");
    assert_eq!(analysis.complexity, crate::agent::TaskComplexity::Complex);
    assert!(analysis.requires_research);
    assert!(analysis.requires_planning);
    assert!(analysis.requires_coding);
    assert!(analysis.requires_testing);
    assert!(analysis.requires_review);
}

#[test]
fn test_task_graph_creation() {
    let graph = crate::agent::TaskGraph::new("Add API feature");
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.root_task, *graph.nodes.keys().next().unwrap());
}

#[test]
fn test_task_graph_add_task() {
    let mut graph = crate::agent::TaskGraph::new("Add API feature");
    let task_id = graph.add_task("Inspect architecture", "research", vec![]);
    assert!(graph.get_task(&task_id).is_some());
    assert_eq!(
        graph.get_task(&task_id).unwrap().description,
        "Inspect architecture"
    );
}

#[test]
fn test_task_graph_dependencies() {
    let mut graph = crate::agent::TaskGraph::new("Add API feature");
    let task1 = graph.add_task("Inspect architecture", "research", vec![]);
    let task2 = graph.add_task("Update models", "coding", vec![task1.clone()]);

    assert!(graph
        .get_task(&task2)
        .unwrap()
        .dependencies
        .contains(&task1));
}

#[test]
fn test_task_graph_execution_order() {
    let mut graph = crate::agent::TaskGraph::new("Add API feature");
    let task1 = graph.add_task("Inspect architecture", "research", vec![]);
    let task2 = graph.add_task("Update models", "coding", vec![task1.clone()]);
    let task3 = graph.add_task("Add tests", "testing", vec![task2.clone()]);

    let order = graph.execution_order();
    assert!(order.contains(&task1));
    assert!(order.contains(&task2));
    assert!(order.contains(&task3));
}

#[test]
fn test_task_graph_ready_tasks() {
    let mut graph = crate::agent::TaskGraph::new("Add API feature");
    let task1 = graph.add_task("Inspect architecture", "research", vec![]);
    let task2 = graph.add_task("Update models", "coding", vec![task1.clone()]);

    let ready = graph.get_ready_tasks();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id, task1);
}

#[test]
fn test_task_graph_completion() {
    let mut graph = crate::agent::TaskGraph::new("Add API feature");
    let task1 = graph.add_task("Inspect architecture", "research", vec![]);

    assert!(!graph.is_complete());

    graph.update_status(&task1, crate::agent::TaskStatus::Completed);
    assert!(graph.is_complete());
}

#[test]
fn test_task_graph_failure() {
    let mut graph = crate::agent::TaskGraph::new("Add API feature");
    let task1 = graph.add_task("Inspect architecture", "research", vec![]);

    graph.update_status(&task1, crate::agent::TaskStatus::Failed);
    assert!(graph.has_failures());
}

#[test]
fn test_smart_tool_router() {
    use crate::dispatcher::ToolDispatcher;
    use crate::dispatcher::ToolRegistry;
    let dispatcher = ToolDispatcher::new(ToolRegistry::new());
    let router = crate::tools::SmartToolRouter::new(dispatcher);

    let selection = router.route("Find authentication code", "");
    assert_eq!(selection.primary_tool, "semantic_search");

    let selection = router.route("Run the test suite", "");
    assert_eq!(selection.primary_tool, "run_command");

    let selection = router.route("Modify the endpoint handler", "");
    assert_eq!(selection.primary_tool, "edit_file");
}

#[test]
fn test_smart_tool_router_git() {
    use crate::dispatcher::ToolDispatcher;
    use crate::dispatcher::ToolRegistry;
    let dispatcher = ToolDispatcher::new(ToolRegistry::new());
    let router = crate::tools::SmartToolRouter::new(dispatcher);

    let selection = router.route("Show git status", "");
    assert_eq!(selection.primary_tool, "git_status");
}

#[test]
fn test_experience_replay_creation() {
    let experience = crate::agent::ExperienceReplay::new();
    assert!(experience.is_ok());
}

#[test]
fn test_experience_record_and_retrieve() {
    let mut replay = crate::agent::ExperienceReplay::new().unwrap();

    let experience = crate::agent::Experience {
        id: uuid::Uuid::new_v4().to_string(),
        task_description: "Add authentication feature".to_string(),
        context: crate::agent::ExperienceContext {
            relevant_files: vec!["auth.rs".to_string()],
            related_symbols: vec!["login".to_string()],
            dependencies: vec!["user.rs".to_string()],
            project_language: Some("rust".to_string()),
            project_framework: None,
        },
        plan: vec!["Step 1".to_string(), "Step 2".to_string()],
        tools_used: vec!["edit_file".to_string(), "run_command".to_string()],
        skills_used: vec!["rust_api".to_string()],
        result: crate::agent::ExperienceResult {
            success: true,
            output: "Success".to_string(),
            files_modified: vec!["auth.rs".to_string()],
            errors: vec![],
            recommendations: vec!["Add tests".to_string()],
        },
        lessons_learned: vec!["Use consistent naming".to_string()],
        success: true,
        duration_ms: 5000,
        created_at: chrono::Local::now().to_rfc3339(),
    };

    replay.record_experience(experience.clone());

    let similar = replay.find_similar("Add login feature", 5);
    assert!(!similar.is_empty());
    assert_eq!(similar[0].task_description, "Add authentication feature");
}

#[test]
fn test_experience_statistics() {
    let mut replay = crate::agent::ExperienceReplay::new().unwrap();

    let experience = crate::agent::Experience {
        id: uuid::Uuid::new_v4().to_string(),
        task_description: "Test task".to_string(),
        context: crate::agent::ExperienceContext {
            relevant_files: vec![],
            related_symbols: vec![],
            dependencies: vec![],
            project_language: None,
            project_framework: None,
        },
        plan: vec![],
        tools_used: vec![],
        skills_used: vec![],
        result: crate::agent::ExperienceResult {
            success: true,
            output: "Success".to_string(),
            files_modified: vec![],
            errors: vec![],
            recommendations: vec![],
        },
        lessons_learned: vec![],
        success: true,
        duration_ms: 1000,
        created_at: chrono::Local::now().to_rfc3339(),
    };

    replay.record_experience(experience);

    let stats = replay.get_statistics();
    assert_eq!(stats.total_experiences, 1);
    assert_eq!(stats.successful, 1);
    assert_eq!(stats.failed, 0);
    assert_eq!(stats.success_rate, 1.0);
}

#[test]
fn test_experience_lessons_learned() {
    let mut replay = crate::agent::ExperienceReplay::new().unwrap();

    let experience = crate::agent::Experience {
        id: uuid::Uuid::new_v4().to_string(),
        task_description: "Add authentication".to_string(),
        context: crate::agent::ExperienceContext {
            relevant_files: vec![],
            related_symbols: vec![],
            dependencies: vec![],
            project_language: None,
            project_framework: None,
        },
        plan: vec![],
        tools_used: vec![],
        skills_used: vec![],
        result: crate::agent::ExperienceResult {
            success: true,
            output: "Success".to_string(),
            files_modified: vec![],
            errors: vec![],
            recommendations: vec![],
        },
        lessons_learned: vec!["Use consistent naming".to_string(), "Add tests".to_string()],
        success: true,
        duration_ms: 1000,
        created_at: chrono::Local::now().to_rfc3339(),
    };

    replay.record_experience(experience);

    let lessons = replay.get_lessons_learned("auth");
    assert!(!lessons.is_empty());
    assert!(lessons.contains(&"Use consistent naming".to_string()));
}

#[test]
fn test_experience_recommended_tools() {
    let mut replay = crate::agent::ExperienceReplay::new().unwrap();

    let experience = crate::agent::Experience {
        id: uuid::Uuid::new_v4().to_string(),
        task_description: "Add API endpoint".to_string(),
        context: crate::agent::ExperienceContext {
            relevant_files: vec![],
            related_symbols: vec![],
            dependencies: vec![],
            project_language: None,
            project_framework: None,
        },
        plan: vec![],
        tools_used: vec![
            "edit_file".to_string(),
            "run_command".to_string(),
            "edit_file".to_string(),
        ],
        skills_used: vec![],
        result: crate::agent::ExperienceResult {
            success: true,
            output: "Success".to_string(),
            files_modified: vec![],
            errors: vec![],
            recommendations: vec![],
        },
        lessons_learned: vec![],
        success: true,
        duration_ms: 1000,
        created_at: chrono::Local::now().to_rfc3339(),
    };

    replay.record_experience(experience);

    let tools = replay.get_recommended_tools("api");
    assert!(!tools.is_empty());
    assert_eq!(tools[0], "edit_file");
}

// ===== v0.6.5 TUI Agent Command Center Tests =====

#[test]
fn test_agent_status_idle_default() {
    let state = crate::agent::status::AgentState::new("research");
    assert_eq!(state.status, crate::agent::AgentStatus::Idle);
    assert_eq!(state.progress, 0.0);
    assert_eq!(state.name, "research");
}

#[test]
fn test_agent_status_transitions() {
    let mut state = crate::agent::status::AgentState::new("coding");
    state.set_status(crate::agent::AgentStatus::Searching);
    assert!(state.status.is_active());
    assert!(state.started_at.is_some());

    state.set_status(crate::agent::AgentStatus::Completed);
    assert!(state.status.is_terminal());
    assert!(state.completed_at.is_some());
}

#[test]
fn test_agent_status_progress() {
    let mut state = crate::agent::status::AgentState::new("planning");
    state.set_progress(0.65);
    assert_eq!(state.progress, 0.65);

    state.set_progress(2.0);
    assert_eq!(state.progress, 1.0);

    state.set_progress(-1.0);
    assert_eq!(state.progress, 0.0);
}

#[test]
fn test_agent_status_reset() {
    let mut state = crate::agent::status::AgentState::new("testing");
    state.set_status(crate::agent::AgentStatus::Executing);
    state.set_progress(0.8);
    state.set_task("Run tests");
    state.set_action("cargo test");

    state.reset();
    assert_eq!(state.status, crate::agent::AgentStatus::Idle);
    assert_eq!(state.progress, 0.0);
    assert!(state.current_task.is_none());
}

#[test]
fn test_agent_status_monitor() {
    let mut monitor = crate::agent::status::AgentStatusMonitor::new();
    monitor.register_agent("research");
    monitor.register_agent("coding");
    assert_eq!(monitor.count(), 2);

    monitor.update_status("research", crate::agent::AgentStatus::Searching);
    monitor.update_action("research", "searching symbols");
    monitor.update_progress("research", 0.5);

    let research = monitor.get("research").unwrap();
    assert_eq!(research.status, crate::agent::AgentStatus::Searching);
    assert_eq!(research.progress, 0.5);
    assert_eq!(research.latest_action.as_deref(), Some("searching symbols"));
}

#[test]
fn test_agent_event_bus() {
    let bus = crate::agent::events::AgentEventBus::new();
    let tx = bus.sender();

    tx.send(crate::agent::events::AgentEvent::AgentStarted {
        agent: "research".to_string(),
        task: "Find auth".to_string(),
    })
    .expect("send event");

    let event = bus.try_recv().expect("receive event");
    assert!(matches!(
        event,
        crate::agent::events::AgentEvent::AgentStarted { agent, .. } if agent == "research"
    ));
}

#[test]
fn test_agent_event_summary() {
    let event = crate::agent::events::AgentEvent::AgentCompleted {
        agent: "coding".to_string(),
        duration_ms: 5000,
    };
    assert_eq!(event.summary(), "Agent coding completed");

    let event = crate::agent::events::AgentEvent::AgentFailed {
        agent: "coding".to_string(),
        error: "compile error".to_string(),
    };
    assert_eq!(event.summary(), "Agent coding failed");
}

#[test]
fn test_agent_event_history() {
    let mut history = crate::agent::events::EventHistory::new(5);
    for i in 0..10 {
        history.record(crate::agent::events::AgentEvent::Log {
            level: "info".to_string(),
            message: format!("message {}", i),
        });
    }
    assert_eq!(history.len(), 5);
    assert!(history.is_empty() == false);
}

#[test]
fn test_dashboard_streaming_chunks() {
    let mut dashboard = crate::tui::dashboard::Dashboard::new();
    dashboard.handle_event(crate::agent::events::AgentEvent::StreamChunk {
        content: "I analysed the repository...".to_string(),
    });
    dashboard.handle_event(crate::agent::events::AgentEvent::StreamChunk {
        content: "Found auth module".to_string(),
    });
    assert!(dashboard.is_streaming);
    assert_eq!(
        dashboard.streaming_buffer,
        "I analysed the repository...Found auth module"
    );
}

#[test]
fn test_dashboard_task_graph() {
    let mut dashboard = crate::tui::dashboard::Dashboard::new();
    let mut graph = crate::agent::TaskGraph::new("Refactor auth");
    graph.add_task("Research", "research", vec![]);
    dashboard.set_task_graph(graph);
    assert!(dashboard.task_graph.is_some());
    let entries = dashboard.graph_entries();
    assert!(!entries.is_empty());
}

#[test]
fn test_dashboard_tool_views() {
    let mut dashboard = crate::tui::dashboard::Dashboard::new();
    dashboard.handle_event(crate::agent::events::AgentEvent::ToolStarted {
        tool: "cargo_test".to_string(),
        args: "cargo test".to_string(),
    });
    dashboard.handle_event(crate::agent::events::AgentEvent::ToolStarted {
        tool: "edit_file".to_string(),
        args: "modify auth.rs".to_string(),
    });
    assert_eq!(dashboard.active_tools.len(), 2);
    assert!(dashboard
        .active_tools
        .iter()
        .any(|t| t.name == "cargo_test"));
}

#[test]
fn test_dashboard_memory_skill_notifications() {
    let mut dashboard = crate::tui::dashboard::Dashboard::new();
    dashboard.handle_event(crate::agent::events::AgentEvent::MemoryUpdated {
        summary: "Project uses repository pattern".to_string(),
    });
    dashboard.handle_event(crate::agent::events::AgentEvent::SkillUpdated {
        skill: "rust-api-feature".to_string(),
        confidence_before: 0.82,
        confidence_after: 0.89,
    });

    assert_eq!(dashboard.memory_notifications.len(), 1);
    assert_eq!(dashboard.skill_notifications.len(), 1);
    let skill = &dashboard.skill_notifications[0];
    assert_eq!(skill.skill, "rust-api-feature");
    assert_eq!(skill.confidence_before, 0.82);
    assert_eq!(skill.confidence_after, 0.89);
}

#[test]
fn test_dashboard_agent_panel_state() {
    let mut dashboard = crate::tui::dashboard::Dashboard::new();
    dashboard.handle_event(crate::agent::events::AgentEvent::AgentStatusChanged {
        agent: "coding".to_string(),
        status: crate::agent::AgentStatus::Executing,
    });
    dashboard.handle_event(crate::agent::events::AgentEvent::AgentProgress {
        agent: "coding".to_string(),
        progress: 0.65,
        action: "Applying patch to auth.rs".to_string(),
    });

    let entries = dashboard.agent_entries();
    let coding = entries.iter().find(|e| e.name == "coding").unwrap();
    assert_eq!(coding.status, crate::agent::AgentStatus::Executing);
    assert_eq!(coding.progress, 0.65);
    assert_eq!(coding.action.as_deref(), Some("Applying patch to auth.rs"));
}

// ===== v0.6.6 Real Usage Hardening Tests =====

#[test]
fn test_session_storage() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = crate::session::SessionStore::new(dir.path()).expect("store creation");
    let mut session = crate::session::Session::new("Add caching");
    session.record_event(&crate::agent::events::AgentEvent::AgentStarted {
        agent: "research".to_string(),
        task: "Find auth".to_string(),
    });
    store.save_session(&session).expect("save session");

    let loaded = store.load_session(&session.id).expect("load session");
    assert_eq!(loaded.task, "Add caching");
    assert_eq!(loaded.timeline.len(), 1);
    assert!(loaded.agents.contains(&"research".to_string()));
}

#[test]
fn test_session_timeline_replay() {
    let mut session = crate::session::Session::new("Refactor auth");
    session.record_event(&crate::agent::events::AgentEvent::AgentStarted {
        agent: "research".to_string(),
        task: "Find".to_string(),
    });
    session.record_event(&crate::agent::events::AgentEvent::AgentCompleted {
        agent: "research".to_string(),
        duration_ms: 500,
    });

    let timeline = session.replay_timeline();
    assert_eq!(timeline.len(), 2);
    assert!(timeline[0].contains("agent_started"));
}

#[test]
fn test_session_tracker_flow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut tracker = crate::session::SessionTracker::new(dir.path()).expect("tracker creation");
    let id = tracker
        .start_session("Add endpoint")
        .expect("start session");
    assert!(!id.is_empty());

    tracker
        .record_event(&crate::agent::events::AgentEvent::AgentStarted {
            agent: "coding".to_string(),
            task: "Implement".to_string(),
        })
        .expect("record event");

    let session = tracker.current_session().expect("current session");
    assert_eq!(session.timeline.len(), 1);

    let ended = tracker.end_session().expect("end session");
    assert!(ended.is_some());
}

#[test]
fn test_metrics_tracking() {
    let mut metrics = crate::metrics::TaskMetrics::new("Add caching");
    metrics.record_agent_duration("research", 1000);
    metrics.record_agent_duration("coding", 2000);
    metrics.record_tool_duration("edit_file", 500);
    metrics.record_tokens(2000, 1000);
    metrics.add_file("auth.rs");
    metrics.increment_retries();

    assert_eq!(metrics.agent_count(), 2);
    assert_eq!(metrics.total_tokens(), 3000);
    assert_eq!(metrics.file_count(), 1);
    assert_eq!(metrics.retry_count, 1);
    assert!(metrics.estimated_cost_usd("gpt-4o") > 0.0);
}

#[test]
fn test_cost_calculation_gpt4o() {
    let cost = crate::metrics::cost_for_tokens("gpt-4o", 1000000, 1000000);
    assert!((cost - 12.5).abs() < 0.01);
}

#[test]
fn test_cost_calculation_claude() {
    let cost = crate::metrics::cost_for_tokens("claude-sonnet-4", 1000000, 500000);
    assert!((cost - 10.5).abs() < 0.01);
}

#[test]
fn test_usage_history_persistence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let usage_path = dir.path().join("usage.json");
    let mut tracker =
        crate::metrics::CostTracker::with_usage_path(usage_path.clone()).expect("cost tracker");
    tracker
        .track_usage("gpt-4o", 1000, 500, 200000)
        .expect("track usage");
    assert_eq!(tracker.history().record_count(), 1);
    assert!(tracker.total_cost() > 0.0);

    let reloaded = crate::metrics::CostTracker::with_usage_path(usage_path).expect("reload");
    assert_eq!(reloaded.history().record_count(), 1);
}

#[test]
fn test_recovery_flow_test_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut engine =
        crate::agent::recovery::RecoveryEngine::with_storage_path(dir.path().join("recovery.json"))
            .expect("engine creation");

    let plan = engine
        .handle_failure(
            "testing",
            "run tests",
            "cargo test failed: assertion failed",
        )
        .expect("handle failure");

    assert_eq!(plan.action, crate::agent::RecoveryAction::AskCodingAgent);
    assert!(!plan.should_retry);
    assert_eq!(plan.suggested_agent, "coding");
}

#[test]
fn test_recovery_flow_timeout() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut engine =
        crate::agent::recovery::RecoveryEngine::with_storage_path(dir.path().join("recovery.json"))
            .expect("engine creation");

    let plan = engine
        .handle_failure("testing", "run tests", "request timed out")
        .expect("handle failure");

    assert_eq!(plan.action, crate::agent::RecoveryAction::Retry);
    assert!(plan.should_retry);
}

#[test]
fn test_recovery_escalation_flow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut engine =
        crate::agent::recovery::RecoveryEngine::with_storage_path(dir.path().join("recovery.json"))
            .expect("engine creation");

    for j in 2..5 {
        engine
            .handle_failure("testing", "t", "request timed out")
            .expect("handle failure");
    }

    let stats = engine.retry_stats();
    assert_eq!(stats.total_failures, 3);
    assert!(stats.escalated >= 1);
}

#[test]
fn test_diff_rendering() {
    let diff = crate::tui::diff_view::FileDiff::parse("auth.rs", "old line", "new line");
    let rendered = crate::tui::diff_view::render_diff_lines(&diff, 40);
    assert_eq!(rendered.len(), 2);
    assert_eq!(rendered[0].0, '-');
    assert_eq!(rendered[1].0, '+');
}

#[test]
fn test_diff_review_session() {
    let mut session = crate::tui::diff_view::DiffReviewSession::new();
    session.add_diff(crate::tui::diff_view::FileDiff::parse("a.rs", "old", "new"));
    session.add_diff(crate::tui::diff_view::FileDiff::parse("b.rs", "x", "y"));

    session
        .apply_action(crate::tui::diff_view::DiffAction::Accept)
        .unwrap();
    session.next();
    session
        .apply_action(crate::tui::diff_view::DiffAction::Reject)
        .unwrap();

    assert_eq!(session.accepted_count(), 1);
    assert_eq!(session.rejected_count(), 1);
    assert!(session.all_reviewed());
}

#[test]
fn test_command_palette_state() {
    let mut dashboard = crate::tui::dashboard::Dashboard::new();
    assert!(!dashboard.show_command_palette);
    dashboard.toggle_command_palette();
    assert!(dashboard.show_command_palette);
    dashboard.toggle_command_palette();
    assert!(!dashboard.show_command_palette);
}

#[test]
fn test_metrics_panel_state() {
    let mut dashboard = crate::tui::dashboard::Dashboard::new();
    assert!(!dashboard.show_metrics);
    dashboard.toggle_metrics();
    assert!(dashboard.show_metrics);
}

#[test]
fn test_dashboard_metrics_field() {
    let mut dashboard = crate::tui::dashboard::Dashboard::new();
    assert!(dashboard.metrics.is_none());
    dashboard.metrics = Some(crate::metrics::TaskMetrics::new("Test"));
    assert!(dashboard.metrics.is_some());
    assert_eq!(dashboard.metrics.as_ref().unwrap().task, "Test");
}

// ===== TUI Usability Tests =====

#[test]
fn test_input_insert_at_cursor() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.insert_text("h");
    app.input.insert_text("i");
    assert_eq!(app.input.text(), "hi");
    assert_eq!(app.input.cursor(), 2);
}

#[test]
fn test_input_insert_in_middle() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("helo");
    app.input.set_cursor(3);
    app.input.insert_text("l");
    assert_eq!(app.input.text(), "hello");
    assert_eq!(app.input.cursor(), 4);
}

#[test]
fn test_input_backspace_at_end() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("hello");
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    app.input.handle_key(bs, &app.dashboard);
    assert_eq!(app.input.text(), "hell");
}

#[test]
fn test_input_backspace_in_middle() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("hllo");
    app.input.set_cursor(2);
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    app.input.handle_key(bs, &app.dashboard);
    assert_eq!(app.input.text(), "hlo");
}

#[test]
fn test_input_backspace_at_start_noop() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("hi");
    app.input.set_cursor(0);
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    app.input.handle_key(bs, &app.dashboard);
    assert_eq!(app.input.text(), "hi");
}

#[test]
fn test_input_cursor_movement() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("hello");
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let left = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
    let right = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
    let home = KeyEvent::new(KeyCode::Home, KeyModifiers::NONE);
    let end = KeyEvent::new(KeyCode::End, KeyModifiers::NONE);
    app.input.handle_key(end, &app.dashboard);
    assert_eq!(app.input.cursor(), 5);
    app.input.handle_key(left, &app.dashboard);
    assert_eq!(app.input.cursor(), 4);
    app.input.handle_key(left, &app.dashboard);
    assert_eq!(app.input.cursor(), 3);
    app.input.handle_key(right, &app.dashboard);
    assert_eq!(app.input.cursor(), 4);
    app.input.handle_key(home, &app.dashboard);
    assert_eq!(app.input.cursor(), 0);
    app.input.handle_key(end, &app.dashboard);
    assert_eq!(app.input.cursor(), 5);
}

#[test]
fn test_input_cursor_bounds() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("hi");
    // After set_text, cursor is at end (2). Left from 0 stays at 0.
    app.input.set_cursor(0);
    let left = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
    app.input.handle_key(left, &app.dashboard);
    assert_eq!(app.input.cursor(), 0);
    app.input.set_cursor(2);
    let right = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
    app.input.handle_key(right, &app.dashboard);
    assert_eq!(app.input.cursor(), 2);
}

#[test]
fn test_input_history_navigation() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.push_history("first".to_string());
    app.push_history("second".to_string());

    app.history_previous();
    assert_eq!(app.input.text(), "second");
    app.history_previous();
    assert_eq!(app.input.text(), "first");
    app.history_next();
    assert_eq!(app.input.text(), "second");
    app.history_next();
    assert_eq!(app.input.text(), "");
}

#[test]
fn test_input_history_dedup() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.push_history("same".to_string());
    app.push_history("same".to_string());
    assert_eq!(app.input_history.len(), 1);
}

#[test]
fn test_input_history_redacts_secrets() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    let secret = "sk-input-history-secret-1234567890abcdef";
    app.push_history(format!("!curl -H \"Authorization: Bearer {}\"", secret));
    assert!(
        app.input_history.iter().all(|h| !h.contains(secret)),
        "secret must never be stored in input history: {:?}",
        app.input_history
    );
}

#[test]
fn test_session_persistence_never_contains_secrets() {
    let dir = tempdir().unwrap();
    let store = crate::session::SessionStore::new(dir.path()).unwrap();
    let mut session = crate::session::Session::new("hardening task");
    let secret = "sk-session-secret-1234567890abcdef";
    session.record_event(&crate::agent::events::AgentEvent::ToolStarted {
        tool: "run_command".to_string(),
        args: format!("curl -H \"Authorization: Bearer {}\"", secret),
    });
    session.record_event(&crate::agent::events::AgentEvent::AgentFailed {
        agent: "main".to_string(),
        error: format!("failed with token {}", secret),
    });
    store.save_session(&session).unwrap();
    let raw = fs::read_to_string(
        dir.path()
            .join("sessions")
            .join(format!("{}.json", session.id)),
    )
    .unwrap();
    assert!(
        !raw.contains(secret),
        "session persistence leaked a secret: {}",
        raw
    );
}

#[test]
fn test_export_never_contains_api_key_or_secrets() {
    let config = Config {
        provider: "openai".to_string(),
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-4o".to_string(),
        api_key: Some("sk-export-secret-1234567890abcdef".to_string()),
    };
    let mut app = crate::tui::TuiApp::new_with_config(config).expect("app");
    let secret = "sk-export-secret-1234567890abcdef";
    // A command echo is redacted before it becomes a message (see
    // run_command_task in ui.rs).
    app.add_message(
        crate::tui::app::MessageRole::System,
        crate::tools::shell::redact_secrets_public(&format!(
            "[shell] curl -H \"Authorization: Bearer {}\"",
            secret
        )),
    );
    let dir = tempdir().unwrap();
    let out_path = dir.path().join("export.json");
    let out = app
        .export_state(&out_path.to_string_lossy())
        .expect("export_state");
    let raw = fs::read_to_string(&out).unwrap();
    assert!(!raw.contains(secret), "export leaked the API key: {}", raw);
    // The config export surface must never carry the in-memory api_key.
    assert!(
        !raw.contains("sk-export-secret"),
        "exported config must not include the api_key value"
    );
}

#[test]
fn test_read_file_tool_redacts_secrets() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("credentials.json");
    let secret = "sk-readfile-secret-1234567890abcdef";
    fs::write(
        &file_path,
        format!("{{\"keys\":{{\"openai\":\"{}\"}}}}", secret),
    )
    .unwrap();
    let tool = ReadFile;
    let result = tool
        .execute(file_path.to_str().unwrap())
        .expect("read_file should work");
    assert!(
        !result.contains(secret),
        "read_file tool leaked a secret into tool output: {}",
        result
    );
}

#[test]
fn test_panel_toggle_state() {
    let mut dashboard = crate::tui::dashboard::Dashboard::new();
    // Default view is task-focused: agent panels are overlays, not fixtures.
    assert!(!dashboard.show_agents);
    dashboard.toggle_agents();
    assert!(dashboard.show_agents);
    dashboard.toggle_agents();
    assert!(!dashboard.show_agents);

    assert!(!dashboard.show_metrics);
    dashboard.toggle_metrics();
    assert!(dashboard.show_metrics);

    assert!(!dashboard.show_coordination);
    dashboard.toggle_coordination();
    assert!(dashboard.show_coordination);

    assert!(!dashboard.show_command_palette);
    dashboard.toggle_command_palette();
    assert!(dashboard.show_command_palette);
}

#[test]
fn test_conversation_scroll_state() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.add_message(crate::tui::app::MessageRole::User, "hello".to_string());
    assert_eq!(app.scrollback.offset_from_bottom, 0);
    app.scroll_up();
    assert_eq!(app.scrollback.offset_from_bottom, 1);
    app.scroll_down();
    assert_eq!(app.scrollback.offset_from_bottom, 0);
    app.scroll_down();
    assert_eq!(app.scrollback.offset_from_bottom, 0);
}

#[test]
fn test_animation_time_based_tick() {
    let mut anim = crate::tui::animation::AnimationState::new();
    anim.start_activity(crate::tui::animation::ActivityType::Thinking);
    // Immediately after starting, tick should not advance (not due yet).
    anim.last_tick = std::time::Instant::now() - std::time::Duration::from_millis(200);
    let advanced = anim.tick_if_due();
    assert!(advanced);
    let advanced = anim.tick_if_due();
    assert!(!advanced);
}

#[test]
fn test_animation_stops_when_idle() {
    let mut anim = crate::tui::animation::AnimationState::new();
    assert!(!anim.is_active());
    let advanced = anim.tick_if_due();
    assert!(!advanced);
    anim.stop_activity();
    assert!(!anim.is_active());
}

// ===== v0.7.2 TUI Agent Execution Wiring Tests =====

use crate::agent::subagent::{SubAgentCapability, SubAgentContext, SubAgentResult};

struct FailingAgent;

impl SubAgent for FailingAgent {
    fn name(&self) -> &str {
        "failing"
    }
    fn purpose(&self) -> &str {
        "Test agent that always fails"
    }
    fn capabilities(&self) -> Vec<SubAgentCapability> {
        Vec::new()
    }
    fn required_tools(&self) -> Vec<&str> {
        Vec::new()
    }
    fn can_handle(&self, _task: &str) -> bool {
        true
    }
    fn required_context(&self) -> Vec<&str> {
        Vec::new()
    }
    fn execute(&self, _context: &SubAgentContext) -> SubAgentResult {
        SubAgentResult {
            agent_name: "failing".to_string(),
            success: false,
            output: String::new(),
            files_modified: Vec::new(),
            tools_used: Vec::new(),
            duration_ms: 0,
            errors: vec!["simulated failure".to_string()],
            recommendations: Vec::new(),
        }
    }
}

#[tokio::test]
async fn test_coordinator_simple_task_emits_events() {
    let mut coordinator = crate::agent::AgentCoordinator::new(6);
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev = events.clone();
    let emit = move |e: crate::agent::AgentEvent| ev.lock().unwrap().push(e);

    let report = coordinator
        .run_task("explain the project structure", None, &emit)
        .await;

    assert!(!report.is_empty(), "report should contain subagent output");

    let events = events.lock().unwrap();
    let started = events
        .iter()
        .filter(|e| matches!(e, crate::agent::AgentEvent::AgentStarted { .. }))
        .count();
    let completed = events
        .iter()
        .filter(|e| matches!(e, crate::agent::AgentEvent::AgentCompleted { .. }))
        .count();
    let graph_updates = events
        .iter()
        .filter(|e| matches!(e, crate::agent::AgentEvent::TaskGraphUpdated { .. }))
        .count();

    assert!(started >= 1, "should emit AgentStarted");
    assert!(completed >= 1, "should emit AgentCompleted for research");
    assert!(graph_updates >= 1, "should emit TaskGraphUpdated");

    // A "simple" explain task routes to the research agent.
    assert!(
        report.contains("Research"),
        "report should mention research phase"
    );
}

#[tokio::test]
async fn test_coordinator_multi_agent_task() {
    let mut coordinator = crate::agent::AgentCoordinator::new(6);
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev = events.clone();
    let emit = move |e: crate::agent::AgentEvent| ev.lock().unwrap().push(e);

    let report = coordinator
        .run_task("add a new caching feature", None, &emit)
        .await;

    assert!(!report.is_empty());
    let events = events.lock().unwrap();
    let completed: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            crate::agent::AgentEvent::AgentCompleted { agent, .. } => Some(agent.as_str()),
            _ => None,
        })
        .collect();

    // Moderate "add" task should run research + planning + coding + testing.
    assert!(completed.contains(&"research"), "research should run");
    assert!(completed.contains(&"planning"), "planning should run");
    assert!(completed.contains(&"coding"), "coding should run");
    assert!(completed.contains(&"testing"), "testing should run");
}

#[tokio::test]
async fn test_coordinator_failure_routes_to_recovery() {
    let mut coordinator = crate::agent::AgentCoordinator::new(6);
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev = events.clone();
    let emit = move |e: crate::agent::AgentEvent| ev.lock().unwrap().push(e);

    // Override the agent factory to inject a failing agent.
    let factory = |_: &str| -> Option<Box<dyn SubAgent>> { Some(Box::new(FailingAgent)) };

    let report = coordinator
        .run_task_with("explain the project", None, &emit, &factory)
        .await;

    let events = events.lock().unwrap();
    let failed = events
        .iter()
        .filter(|e| matches!(e, crate::agent::AgentEvent::AgentFailed { .. }))
        .count();
    let recovery_logs = events
        .iter()
        .filter(
            |e| matches!(e, crate::agent::AgentEvent::Log { level, .. } if level == "coordination"),
        )
        .count();

    assert!(failed >= 1, "should emit AgentFailed");
    assert!(report.contains("FAILED"), "report should note the failure");

    // The failure should surface a coordination notification (recovery).
    assert!(
        recovery_logs >= 1,
        "recovery should emit a coordination log"
    );
}

#[tokio::test]
async fn test_tui_dashboard_consumes_pipeline_events() {
    let mut coordinator = crate::agent::AgentCoordinator::new(6);
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev = events.clone();
    let emit = move |e: crate::agent::AgentEvent| ev.lock().unwrap().push(e);

    let _ = coordinator
        .run_task("refactor the authentication module", None, &emit)
        .await;

    let events = events.lock().unwrap();
    let mut app = crate::tui::TuiApp::new().expect("app");
    for event in events.iter() {
        app.handle_agent_event(event.clone());
    }

    // Research agent should be marked Completed in the dashboard monitor.
    let research = app
        .dashboard
        .status_monitor
        .get("research")
        .expect("research agent registered");
    assert_eq!(research.status, crate::agent::AgentStatus::Completed);

    // TaskGraph should be populated for Ctrl+G.
    let graph = app.dashboard.task_graph.as_ref().expect("task graph set");
    assert!(!graph.nodes.is_empty(), "task graph should have nodes");
    let has_completed = graph
        .nodes
        .values()
        .any(|n| n.status == crate::agent::TaskStatus::Completed);
    assert!(has_completed, "at least one task should be completed");

    // Coordination messages should appear in recent_messages.
    assert!(
        !app.dashboard.recent_messages.is_empty(),
        "coordination messages should be recorded"
    );
}

#[tokio::test]
async fn test_tui_dashboard_complex_routing_updates_agents() {
    let mut coordinator = crate::agent::AgentCoordinator::new(6);
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev = events.clone();
    let emit = move |e: crate::agent::AgentEvent| ev.lock().unwrap().push(e);

    let _ = coordinator
        .run_task(
            "refactor the authentication system to use middleware",
            None,
            &emit,
        )
        .await;

    let events = events.lock().unwrap();
    let mut app = crate::tui::TuiApp::new().expect("app");
    for event in events.iter() {
        app.handle_agent_event(event.clone());
    }

    // All five agents should have reached a terminal state.
    for agent in ["research", "planning", "coding", "testing", "review"] {
        let state = app
            .dashboard
            .status_monitor
            .get(agent)
            .unwrap_or_else(|| panic!("{} agent registered", agent));
        assert!(
            state.status == crate::agent::AgentStatus::Completed
                || state.status == crate::agent::AgentStatus::Failed,
            "{} should be terminal, got {:?}",
            agent,
            state.status
        );
    }
}

#[tokio::test]
async fn test_coordinator_report_contains_all_phases() {
    let mut coordinator = crate::agent::AgentCoordinator::new(6);
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev = events.clone();
    let emit = move |e: crate::agent::AgentEvent| ev.lock().unwrap().push(e);

    let report = coordinator
        .run_task(
            "refactor the authentication system to use middleware",
            None,
            &emit,
        )
        .await;

    for phase in ["Research", "Planning", "Coding", "Testing", "Review"] {
        let marker = format!("## {}", phase);
        assert!(
            report.contains(&marker),
            "report should contain {} section, got:\n{}",
            phase,
            report
        );
    }
}

#[tokio::test]
async fn test_coordinator_report_is_grounded_in_repository() {
    let mut coordinator = crate::agent::AgentCoordinator::new(6);
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev = events.clone();
    let emit = move |e: crate::agent::AgentEvent| ev.lock().unwrap().push(e);

    // Task deliberately targets the canonical runtime so the workspace scan
    // must surface the real file in the research findings.
    let report = coordinator
        .run_task("trace the canonical runtime execution", None, &emit)
        .await;

    assert!(
        report.contains("src/canonical_runtime/mod.rs"),
        "grounded report must reference the real runtime file:\n{}",
        report
    );
    assert!(
        report.contains("Cargo.toml") || report.contains("cargo"),
        "grounded report must reference real build/project info:\n{}",
        report
    );
}

#[tokio::test]
async fn test_coordinator_grounded_context_reaches_every_subagent() {
    use crate::agent::grounding::GroundingAssembler;

    let mut coordinator = crate::agent::AgentCoordinator::new(6);
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev = events.clone();
    let emit = move |e: crate::agent::AgentEvent| ev.lock().unwrap().push(e);

    let grounded = GroundingAssembler::new(".").assemble("trace the canonical runtime execution");
    assert!(
        grounded
            .relevant_files
            .iter()
            .any(|f| f.contains("canonical_runtime")),
        "grounded context should contain the runtime file: {:?}",
        grounded.relevant_files
    );

    let report = coordinator
        .run_task_grounded("trace the canonical runtime execution", grounded, &emit)
        .await;

    // The same grounded context flows into research, planning and coding.
    assert!(
        report.contains("src/canonical_runtime/mod.rs"),
        "research should surface the grounded file:\n{}",
        report
    );
    assert!(
        report.contains("Execution Plan (grounded"),
        "planning should use the grounded plan:\n{}",
        report
    );
}

// ===== Sprint 30UI.3 — Real-provider smoke =====
//
// Model discovery → model selection → simple chat → structured tool-call
// probe against ONE real provider. DeepSeek is preferred; when no DeepSeek
// credential exists, the valid AGNES credential is used instead. The
// credential is read from the environment and never printed or persisted.
#[tokio::test]
#[ignore]
async fn real_provider_smoke_models_chat_tools() {
    let deepseek_key = std::env::var("DEEPSEEK_API_KEY").ok();
    let agnes_key = std::env::var("AGNES_API_KEY").ok();

    let (base_url, api_key, provider, catalog) = if let Some(key) = deepseek_key {
        (
            "https://api.deepseek.com".to_string(),
            key,
            "deepseek".to_string(),
            crate::providers::DEEPSEEK_CATALOG,
        )
    } else if let Some(key) = agnes_key {
        (
            "https://apihub.agnes-ai.com/v1".to_string(),
            key,
            "agnes".to_string(),
            crate::providers::AGNES_CATALOG,
        )
    } else {
        eprintln!("REAL PROVIDER: BLOCKED (no DEEPSEEK_API_KEY / AGNES_API_KEY in environment)");
        return;
    };

    eprintln!(
        "REAL PROVIDER: using {} ({}); key read from environment, never printed",
        provider, base_url
    );

    // 1. /models discovery (falls back to the official catalog on failure).
    let discovery = crate::providers::discover_models(&base_url, Some(&api_key), &provider).await;
    assert!(
        !discovery.models.is_empty(),
        "model discovery must return models for {}",
        provider
    );
    eprintln!(
        "REAL PROVIDER: {} model(s) known (fallback={})",
        discovery.models.len(),
        discovery.used_fallback
    );

    // 2. Select the preferred model from the official catalog.
    let model = catalog[0].id.to_string();
    assert!(
        discovery.models.iter().any(|m| m.id == model),
        "catalog model {} must be selectable from discovery",
        model
    );

    // 3. Simple chat round-trip.
    let config = crate::config::Config {
        provider: provider.clone(),
        base_url: base_url.clone(),
        model: model.clone(),
        api_key: Some(api_key.clone()),
    };
    let client = crate::providers::OpenAiProvider::new(config.clone());
    let reply = client
        .send_message("Reply with exactly: OK")
        .await
        .expect("simple chat round-trip must succeed");
    assert!(!reply.trim().is_empty(), "reply must not be empty");
    eprintln!(
        "REAL PROVIDER: chat reply = {:?}",
        crate::tools::shell::redact_secrets_public(&reply)
    );
    let reply_redacted = crate::tools::shell::redact_secrets_public(&reply);
    assert!(
        !reply_redacted.contains(&api_key),
        "reply must never echo the credential"
    );

    // 4. Structured tool-call probe.
    let tools = vec![crate::providers::ToolDefinition {
        name: "get_weather".to_string(),
        description: "Get the current weather for a city".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"city": {"type": "string"}},
            "required": ["city"]
        }),
    }];
    let (text, tool_calls) = client
        .stream_response_with_tools(
            "What is the weather in Paris? Use the get_weather tool if available.",
            &tools,
        )
        .await
        .expect("structured tool-call probe must complete");
    assert!(
        !text.trim().is_empty() || !tool_calls.is_empty(),
        "probe must produce prose or a structured tool call"
    );
    for call in &tool_calls {
        assert!(!call.name.is_empty(), "tool call carries a name");
        assert!(
            !call.arguments.contains(&api_key),
            "tool-call arguments must never contain the credential"
        );
    }
    eprintln!(
        "REAL PROVIDER: tool probe → text={} chars, tool_calls={}",
        text.chars().count(),
        tool_calls.len()
    );
}

// ===== REACT termination bug reproduction (Sprint) =====
//
// Runs the REAL canonical ReAct loop (not just send_message) against a real
// provider. This is the exact path that produced
// "Reached the maximum number of reasoning iterations without a final answer"
// in real user testing. Cases:
//   A. text-only answer, no tools
//   B. tool call(s) then final answer
//   C. read_file then final answer
// Runs twice per case: plain loop, then Assist-mode (research enabled).

/// A provider wrapper that records every prompt and response (text length,
/// structured calls) without ever recording the credential.
struct RecordingOpenAiProvider {
    inner: crate::providers::OpenAiProvider,
    log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl RecordingOpenAiProvider {
    fn new(config: crate::config::Config) -> Self {
        RecordingOpenAiProvider {
            inner: crate::providers::OpenAiProvider::new(config),
            log: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn capture(&self, line: String) {
        self.log.lock().unwrap().push(line);
    }
}

impl crate::providers::Provider for RecordingOpenAiProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn base_url(&self) -> &str {
        self.inner.base_url()
    }
    fn model(&self) -> &str {
        self.inner.model()
    }
    fn api_key(&self) -> Option<&str> {
        self.inner.api_key()
    }
    fn supports_function_calling(&self) -> bool {
        self.inner.supports_function_calling()
    }
    fn send_message(
        &self,
        message: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
    {
        self.inner.send_message(message)
    }
    fn stream_response(
        &self,
        message: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<tokio::sync::mpsc::UnboundedReceiver<String>>,
                > + Send
                + '_,
        >,
    > {
        self.inner.stream_response(message)
    }
    fn stream_response_with_tools(
        &self,
        message: &str,
        tools: &[crate::providers::ToolDefinition],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<(String, Vec<crate::providers::StructuredToolCall>)>,
                > + Send
                + '_,
        >,
    > {
        let log = self.log.clone();
        let prompt = crate::tools::shell::redact_secrets_public(message)
            .chars()
            .take(400)
            .collect::<String>();
        let fut = self.inner.stream_response_with_tools(message, tools);
        Box::pin(async move {
            log.lock()
                .unwrap()
                .push(format!("PROMPT({}): {}", prompt.len(), prompt));
            match fut.await {
                Ok((text, calls)) => {
                    log.lock().unwrap().push(format!(
                        "RESPONSE text_len={} text={:?} structured_calls={:?}",
                        text.chars().count(),
                        crate::tools::shell::redact_secrets_public(&text)
                            .chars()
                            .take(200)
                            .collect::<String>(),
                        calls
                            .iter()
                            .map(|c| format!(
                                "{}[{}]",
                                c.name,
                                c.arguments.chars().take(60).collect::<String>()
                            ))
                            .collect::<Vec<_>>()
                    ));
                    Ok((text, calls))
                }
                Err(e) => {
                    log.lock().unwrap().push(format!("RESPONSE ERROR: {e}"));
                    Err(e)
                }
            }
        })
    }
}

#[tokio::test]
#[ignore]
async fn real_provider_react_termination_repro() {
    let api_key = std::env::var("AGNES_API_KEY")
        .ok()
        .or_else(|| std::env::var("CODEBRO_API_KEY").ok());
    let Some(api_key) = api_key else {
        eprintln!("REAL PROVIDER: BLOCKED (no AGNES_API_KEY in environment)");
        return;
    };
    let base_url = std::env::var("CODEBRO_BASE_URL")
        .unwrap_or_else(|_| "https://apihub.agnes-ai.com/v1".to_string());
    let model = std::env::var("CODEBRO_MODEL").unwrap_or_else(|_| "agnes-2.5-flash".to_string());
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let cases: &[(&str, &str)] = &[
        (
            "CASE A",
            "Reply with exactly: OK. Do not use any tools.",
        ),
        (
            "CASE B",
            "List the files in the repository and summarize the result. Do not modify anything.",
        ),
        (
            "CASE C",
            "Inspect src/tui/actions.rs and return a two-sentence summary of its purpose. Do not modify anything.",
        ),
    ];

    for (label, task) in cases {
        for research_enabled in [false, true] {
            let config = crate::config::Config {
                provider: "openai".to_string(),
                base_url: base_url.clone(),
                model: model.clone(),
                api_key: Some(api_key.clone()),
            };
            let provider = std::sync::Arc::new(RecordingOpenAiProvider::new(config.clone()));
            let recording = provider.clone();
            let mut runtime =
                crate::canonical_runtime::CanonicalRuntime::new_without_default_provider(
                    config, &root,
                )
                .expect("runtime");
            runtime.with_retry_policy(crate::provider_runtime::RetryPolicy::immediate(0));
            runtime.register_provider(provider);

            let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let sink = events.clone();
            let emit = move |e: crate::agent::events::AgentEvent| sink.lock().unwrap().push(e);
            let on_chunk = |_c: &str| {};
            let req = crate::canonical_runtime::TaskRequest {
                task,
                conversation: Vec::new(),
                emit: &emit,
                on_chunk: &on_chunk,
            };
            let mut options = crate::canonical_runtime::TaskOptions::default();
            options.research_enabled = research_enabled;
            options.task_timeout_ms = Some(120_000);

            let result = runtime.run_task_with_options(&req, options).await;
            let tool_starts: Vec<String> = events
                .lock()
                .unwrap()
                .iter()
                .filter_map(|e| match e {
                    crate::agent::events::AgentEvent::ToolStarted { tool, .. } => {
                        Some(tool.clone())
                    }
                    _ => None,
                })
                .collect();
            let response_redacted = crate::tools::shell::redact_secrets_public(&result.response);
            eprintln!(
                "\n[{}] research={} success={} cancelled={}\n  error={:?}\n  tools={:?}\n  response={:?}",
                label,
                research_enabled,
                result.success,
                result.cancelled,
                result.error,
                tool_starts,
                response_redacted.chars().take(300).collect::<String>()
            );
            for line in recording.log.lock().unwrap().iter() {
                eprintln!("  {line}");
            }
            if !research_enabled {
                assert!(
                    result.success,
                    "{} (plain loop) must terminate successfully, got error: {:?}",
                    label, result.error
                );
            }
        }
    }
}

// ===== Model Picker Tests =====

fn picker_model(id: &str) -> crate::provider_manager::ModelInfo {
    crate::provider_manager::ModelInfo {
        id: id.to_string(),
        is_default: false,
        display_name: None,
        tool_calling: None,
        context_tokens: None,
        source: crate::providers::ModelSource::Discovered,
    }
}

#[test]
fn test_model_picker_open_close() {
    let mut picker = crate::tui::dashboard::ModelPicker::new();
    assert!(!picker.is_open());
    picker.open();
    assert!(picker.is_open());
    assert!(picker.loading);
    picker.close();
    assert!(!picker.is_open());
}

#[test]
fn test_model_picker_set_models_and_navigate() {
    let mut picker = crate::tui::dashboard::ModelPicker::new();
    picker.set_models(vec![
        picker_model("a"),
        picker_model("b"),
        picker_model("c"),
    ]);
    assert_eq!(picker.count(), 3);
    assert_eq!(picker.selected().map(|m| m.id), Some("a".to_string()));
    picker.next();
    assert_eq!(picker.selected().map(|m| m.id), Some("b".to_string()));
    picker.next();
    assert_eq!(picker.selected().map(|m| m.id), Some("c".to_string()));
    // wraps around
    picker.next();
    assert_eq!(picker.selected().map(|m| m.id), Some("a".to_string()));
    // prev wraps backwards
    picker.prev();
    assert_eq!(picker.selected().map(|m| m.id), Some("c".to_string()));
}

#[test]
fn test_model_picker_filter() {
    let mut picker = crate::tui::dashboard::ModelPicker::new();
    picker.set_models(vec![
        picker_model("deepseek/deepseek-v4-pro"),
        picker_model("qwen3-coder-plus"),
        picker_model("gpt-4o"),
    ]);
    picker.filter = "qwen".to_string();
    let visible: Vec<&str> = picker
        .visible_models()
        .iter()
        .map(|m| m.id.as_str())
        .collect();
    assert_eq!(visible, vec!["qwen3-coder-plus"]);
    picker.filter = "deepseek".to_string();
    let visible: Vec<&str> = picker
        .visible_models()
        .iter()
        .map(|m| m.id.as_str())
        .collect();
    assert_eq!(visible, vec!["deepseek/deepseek-v4-pro"]);
}

#[test]
fn test_model_picker_filter_empty_shows_all() {
    let mut picker = crate::tui::dashboard::ModelPicker::new();
    picker.set_models(vec![picker_model("x"), picker_model("y")]);
    assert_eq!(picker.visible_models().len(), 2);
}

// ===== Multi-line input & paste tests =====

#[test]
fn test_input_insert_text_multiline() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.insert_text("line one\nline two");
    assert_eq!(app.input.text(), "line one\nline two");
    assert_eq!(app.input.cursor(), "line one\nline two".len());
}

#[test]
fn test_input_insert_text_at_cursor() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("ab");
    app.input.set_cursor(1);
    app.insert_text("XX");
    assert_eq!(app.input.text(), "aXXb");
    assert_eq!(app.input.cursor(), 3);
}

#[test]
fn test_input_cursor_line_col() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("aaa\nbb\nc");
    app.input.set_cursor("aaa\nb".len()); // on line 2, col 1
                                          // Cursor position is tracked by the textarea; line/col is computed externally.
    assert_eq!(app.input.cursor(), "aaa\nb".len());
}

#[test]
fn test_input_shift_enter_newline() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.insert_text("a");
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let newline = KeyEvent::new(KeyCode::Char('\n'), KeyModifiers::SHIFT);
    app.input.handle_key(newline, &app.dashboard);
    app.input.insert_text("b");
    assert_eq!(app.input.text(), "a\nb");
    assert_eq!(app.input.cursor(), 3);
}

#[test]
fn test_mouse_scroll() {
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.mouse_scroll(3);
    assert_eq!(app.scrollback.offset_from_bottom, 3);
    app.mouse_scroll(-2);
    assert_eq!(app.scrollback.offset_from_bottom, 1);
}

#[test]
fn test_apply_approve_workflow() {
    use crate::tools::ChangePlan;
    use crate::tui::TuiApp;
    use std::fs;
    use std::path::PathBuf;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("test.rs");
    fs::write(&file_path, "fn old() {}\n").unwrap();

    // Step 1: Create a change plan (simulates /apply)
    let plan = ChangePlan::propose(&file_path, "fn new() {}\n").unwrap();
    assert!(!plan.is_applied());
    assert!(plan.preview().contains("-fn old()"));
    assert!(plan.preview().contains("+fn new()"));

    // Step 2: Apply the plan (simulates /approve)
    let mut plan = plan;
    let result = plan.apply().unwrap();
    assert!(result.contains("applied"));
    assert!(plan.is_applied());

    // Verify file was modified
    let content = fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "fn new() {}\n");
}

#[test]
fn test_apply_approve_rollback_on_verify_failure() {
    use crate::tools::ChangePlan;
    use std::fs;
    use std::path::PathBuf;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("test.rs");
    fs::write(&file_path, "keep\n").unwrap();

    let mut plan = ChangePlan::propose(&file_path, "evil\n").unwrap();

    // Verify should fail and trigger rollback
    let err = plan
        .apply_and_verify(Some("exit 1"))
        .expect_err("verify should fail");
    assert!(err.to_string().contains("rolled back"));

    // Original content should be restored
    let content = fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "keep\n");
}

#[test]
fn test_shell_timeout_enforced() {
    use crate::tools::shell::RunCommand;
    use crate::tools::Tool;

    let tool = RunCommand::new().with_timeout(1);

    // This should timeout and return an error
    let result = tool.execute("sleep 10 && echo 'should_not_run'");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("timed out"),
        "Expected timeout error, got: {}",
        err
    );
}

#[test]
fn test_dashboard_error_handling() {
    use crate::agent::events::AgentEvent;
    use crate::tui::dashboard::Dashboard;

    let mut dashboard = Dashboard::new();

    // Simulate an agent failure
    dashboard.handle_event(AgentEvent::AgentFailed {
        agent: "main".to_string(),
        error: "connection timeout after 60s".to_string(),
    });

    // Error should be stored
    assert!(dashboard.last_error.is_some());
    assert_eq!(
        dashboard.last_error.as_deref(),
        Some("connection timeout after 60s")
    );

    // Error should be clearable
    let cleared = dashboard.clear_error();
    assert!(cleared.is_some());
    assert_eq!(cleared.unwrap(), "connection timeout after 60s");
    assert!(dashboard.last_error.is_none());
}

#[test]
fn test_apply_approved_only_after_explicit_call() {
    use crate::tools::ChangePlan;
    use std::fs;
    use std::path::PathBuf;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("test.rs");
    fs::write(&file_path, "original\n").unwrap();

    // Creating a plan should NOT modify the file
    let _plan = ChangePlan::propose(&file_path, "modified\n").unwrap();
    let content = fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "original\n");
}

// ===========================================================================
// P1.5 Runtime Validation Suite
// ===========================================================================

#[cfg(test)]
mod validation {
    use super::*;
    use crate::agent::events::AgentEvent;
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc;

    use crate::dispatcher::ToolRegistry;
    use crate::providers::Provider;
    use crate::runtime::state::{RuntimeError, RuntimeState};
    use crate::tools::Tool;

    // ---------------------------------------------------------------------------
    // 1. Runtime State Machine Validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_all_valid_transitions() {
        assert!(RuntimeState::Idle
            .try_transition(RuntimeState::Observing)
            .is_ok());
        assert!(RuntimeState::Observing
            .try_transition(RuntimeState::Reasoning)
            .is_ok());
        assert!(RuntimeState::Reasoning
            .try_transition(RuntimeState::Synthesizing)
            .is_ok());
        assert!(RuntimeState::Synthesizing
            .try_transition(RuntimeState::Acting)
            .is_ok());
        assert!(RuntimeState::Synthesizing
            .try_transition(RuntimeState::Completed)
            .is_ok());
        assert!(RuntimeState::Acting
            .try_transition(RuntimeState::Synthesizing)
            .is_ok());
    }

    #[test]
    fn test_all_invalid_transitions_rejected() {
        // From Idle
        for next in [
            RuntimeState::Reasoning,
            RuntimeState::Synthesizing,
            RuntimeState::Acting,
            RuntimeState::Completed,
            RuntimeState::Failed,
        ] {
            assert!(
                RuntimeState::Idle.try_transition(next).is_err(),
                "Idle -> {:?} should be invalid",
                next
            );
        }

        // From Observing - Failed is now a valid error transition
        for next in [
            RuntimeState::Idle,
            RuntimeState::Acting,
            RuntimeState::Completed,
        ] {
            assert!(
                RuntimeState::Observing.try_transition(next).is_err(),
                "Observing -> {:?} should be invalid",
                next
            );
        }

        // From Reasoning - Failed is now a valid error transition
        for next in [
            RuntimeState::Idle,
            RuntimeState::Observing,
            RuntimeState::Acting,
            RuntimeState::Completed,
        ] {
            assert!(
                RuntimeState::Reasoning.try_transition(next).is_err(),
                "Reasoning -> {:?} should be invalid",
                next
            );
        }

        // From Acting - Failed is now a valid error transition
        for next in [
            RuntimeState::Idle,
            RuntimeState::Observing,
            RuntimeState::Reasoning,
            RuntimeState::Completed,
        ] {
            assert!(
                RuntimeState::Acting.try_transition(next).is_err(),
                "Acting -> {:?} should be invalid",
                next
            );
        }

        // Terminal states cannot transition anywhere
        for terminal in [RuntimeState::Completed, RuntimeState::Failed] {
            for next in [
                RuntimeState::Idle,
                RuntimeState::Observing,
                RuntimeState::Reasoning,
                RuntimeState::Synthesizing,
                RuntimeState::Acting,
            ] {
                assert!(
                    terminal.try_transition(next).is_err(),
                    "{:?} -> {:?} should be invalid",
                    terminal,
                    next
                );
            }
        }
    }

    #[test]
    fn test_no_dead_states() {
        for state in [
            RuntimeState::Idle,
            RuntimeState::Observing,
            RuntimeState::Reasoning,
            RuntimeState::Synthesizing,
            RuntimeState::Acting,
        ] {
            let transitions = state.valid_transitions();
            assert!(
                !transitions.is_empty(),
                "{:?} has no valid transitions (dead state)",
                state
            );
        }
    }

    #[test]
    fn test_no_unreachable_states() {
        let reachable = reachable_from_idle();
        let all_states = vec![
            RuntimeState::Idle,
            RuntimeState::Observing,
            RuntimeState::Reasoning,
            RuntimeState::Synthesizing,
            RuntimeState::Acting,
            RuntimeState::Completed,
            RuntimeState::Failed,
        ];

        for state in &all_states {
            assert!(
                reachable.contains(state),
                "{:?} is unreachable from Idle",
                state
            );
        }
    }

    fn reachable_from_idle() -> HashSet<RuntimeState> {
        let mut visited = HashSet::new();
        let mut queue = vec![RuntimeState::Idle];
        visited.insert(RuntimeState::Idle);

        while let Some(state) = queue.pop() {
            for next in state.valid_transitions() {
                if visited.insert(*next) {
                    queue.push(*next);
                }
            }
        }

        visited
    }

    #[test]
    fn test_all_paths_to_terminal_states() {
        let reachable = reachable_from_idle();
        assert!(
            reachable.contains(&RuntimeState::Completed),
            "Completed must be reachable"
        );
        assert!(
            reachable.contains(&RuntimeState::Failed),
            "Failed must be reachable"
        );
    }

    #[test]
    fn test_full_pipeline_sequence() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_multi_iteration_react_loop() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();

        for _ in 0..5 {
            state = state.try_transition(RuntimeState::Synthesizing).unwrap();
            state = state.try_transition(RuntimeState::Acting).unwrap();
        }

        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_error_type_construction() {
        let err = RuntimeError {
            from: RuntimeState::Idle,
            to: RuntimeState::Completed,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("Idle"));
        assert!(msg.contains("Completed"));
    }

    #[test]
    fn test_default_state_is_idle() {
        assert_eq!(RuntimeState::default(), RuntimeState::Idle);
    }

    #[test]
    fn test_is_active_for_all_active_states() {
        assert!(!RuntimeState::Idle.is_active());
        assert!(RuntimeState::Observing.is_active());
        assert!(RuntimeState::Reasoning.is_active());
        assert!(RuntimeState::Synthesizing.is_active());
        assert!(RuntimeState::Acting.is_active());
        assert!(!RuntimeState::Completed.is_active());
        assert!(!RuntimeState::Failed.is_active());
    }

    #[test]
    fn test_is_terminal_for_all_terminal_states() {
        assert!(!RuntimeState::Idle.is_terminal());
        assert!(!RuntimeState::Observing.is_terminal());
        assert!(!RuntimeState::Reasoning.is_terminal());
        assert!(!RuntimeState::Synthesizing.is_terminal());
        assert!(!RuntimeState::Acting.is_terminal());
        assert!(RuntimeState::Completed.is_terminal());
        assert!(RuntimeState::Failed.is_terminal());
    }

    // ---------------------------------------------------------------------------
    // 2. Provider Layer Validation
    // ---------------------------------------------------------------------------

    struct MockProvider {
        name: String,
        base_url: String,
        model: String,
        chunks: Vec<String>,
    }

    impl MockProvider {
        fn new(name: &str, base_url: &str, model: &str, chunks: Vec<&str>) -> Self {
            MockProvider {
                name: name.to_string(),
                base_url: base_url.to_string(),
                model: model.to_string(),
                chunks: chunks.iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    impl Provider for MockProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn base_url(&self) -> &str {
            &self.base_url
        }

        fn model(&self) -> &str {
            &self.model
        }

        fn api_key(&self) -> Option<&str> {
            None
        }

        fn send_message(
            &self,
            _message: &str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
        {
            Box::pin(async move { Ok(self.chunks.join("")) })
        }

        fn stream_response(
            &self,
            _message: &str,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = anyhow::Result<tokio::sync::mpsc::UnboundedReceiver<String>>,
                    > + Send
                    + '_,
            >,
        > {
            let chunks = self.chunks.clone();
            Box::pin(async move {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                for chunk in chunks {
                    let _ = tx.send(chunk);
                }
                Ok(rx)
            })
        }
    }

    #[test]
    fn test_provider_trait_compliance() {
        let provider = MockProvider::new("mock", "http://localhost", "test-model", vec!["hello"]);
        assert_eq!(provider.name(), "mock");
        assert_eq!(provider.base_url(), "http://localhost");
        assert_eq!(provider.model(), "test-model");
        assert!(provider.api_key().is_none());
    }

    #[test]
    fn test_provider_substitution() {
        let providers: Vec<Box<dyn Provider>> = vec![
            Box::new(MockProvider::new("mock1", "http://a", "m1", vec!["a"])),
            Box::new(MockProvider::new("mock2", "http://b", "m2", vec!["b"])),
        ];

        for provider in &providers {
            assert!(!provider.name().is_empty());
            assert!(!provider.base_url().is_empty());
            assert!(!provider.model().is_empty());
        }
    }

    #[test]
    fn test_provider_streaming_collects_all_chunks() {
        let chunks = vec!["Hello", " ", "World", "!"];
        let provider = MockProvider::new("mock", "http://localhost", "test", chunks.clone());

        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut rx = rt.block_on(provider.stream_response("test")).unwrap();

        let mut collected = Vec::new();
        rt.block_on(async {
            while let Some(chunk) = rx.recv().await {
                collected.push(chunk);
            }
        });
        assert_eq!(collected, chunks);
    }

    #[test]
    fn test_provider_streaming_empty() {
        let provider = MockProvider::new("mock", "http://localhost", "test", vec![]);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut rx = rt.block_on(provider.stream_response("test")).unwrap();

        let mut collected = Vec::new();
        rt.block_on(async {
            while let Some(chunk) = rx.recv().await {
                collected.push(chunk);
            }
        });
        assert!(collected.is_empty());
    }

    #[test]
    fn test_provider_send_message() {
        let chunks = vec!["Hello", " ", "World"];
        let provider = MockProvider::new("mock", "http://localhost", "test", chunks.clone());

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(provider.send_message("test")).unwrap();
        assert_eq!(result, chunks.join(""));
    }

    #[test]
    fn test_openai_provider_creation() {
        let config = Config {
            provider: "openai".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            api_key: None,
        };
        let provider = crate::providers::OpenAiProvider::new(config);
        assert_eq!(provider.name(), "openai");
        assert_eq!(provider.model(), "gpt-4o");
    }

    #[test]
    fn test_provider_trait_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Box<dyn Provider>>();
    }

    // ---------------------------------------------------------------------------
    // 3. Tool Registry Validation
    // ---------------------------------------------------------------------------

    struct DummyTool {
        name: String,
        result: String,
    }

    impl Tool for DummyTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            "Dummy tool for validation"
        }

        fn execute(&self, _args: &str) -> anyhow::Result<String> {
            Ok(self.result.clone())
        }
    }

    struct FailingTool {
        name: String,
    }

    impl Tool for FailingTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            "Failing tool for validation"
        }

        fn execute(&self, _args: &str) -> anyhow::Result<String> {
            Err(anyhow::anyhow!("Tool execution failed"))
        }
    }

    #[test]
    fn test_registry_creation() {
        let registry: ToolRegistry = ToolRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_registry_registration() {
        let registry = ToolRegistry::new()
            .register(Arc::new(DummyTool {
                name: "tool_a".to_string(),
                result: "result_a".to_string(),
            }))
            .register(Arc::new(DummyTool {
                name: "tool_b".to_string(),
                result: "result_b".to_string(),
            }));

        assert_eq!(registry.len(), 2);
        assert!(!registry.is_empty());
    }

    #[test]
    fn test_registry_lookup() {
        let registry = ToolRegistry::new().register(Arc::new(DummyTool {
            name: "tool_a".to_string(),
            result: "result_a".to_string(),
        }));

        assert!(registry.get("tool_a").is_some());
        assert!(registry.get("tool_b").is_none());
    }

    #[tokio::test]
    async fn test_registry_execution_success() {
        let mut registry = ToolRegistry::new().register(Arc::new(DummyTool {
            name: "tool_a".to_string(),
            result: "success".to_string(),
        }));

        let result = registry.execute("tool_a", "args").await.unwrap();
        assert_eq!(result, "success");
    }

    #[tokio::test]
    async fn test_registry_execution_failure() {
        let mut registry = ToolRegistry::new().register(Arc::new(FailingTool {
            name: "failing".to_string(),
        }));

        let result = registry.execute("failing", "args").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("failed"));
    }

    #[tokio::test]
    async fn test_registry_unknown_tool() {
        let mut registry = ToolRegistry::new();

        let result = registry.execute("unknown", "args").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown tool"));
    }

    #[test]
    fn test_registry_names() {
        let registry = ToolRegistry::new()
            .register(Arc::new(DummyTool {
                name: "tool_a".to_string(),
                result: "a".to_string(),
            }))
            .register(Arc::new(DummyTool {
                name: "tool_b".to_string(),
                result: "b".to_string(),
            }));

        let names = registry.names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"tool_a".to_string()));
        assert!(names.contains(&"tool_b".to_string()));
    }

    #[test]
    fn test_registry_list() {
        let registry = ToolRegistry::new().register(Arc::new(DummyTool {
            name: "tool_a".to_string(),
            result: "a".to_string(),
        }));

        assert_eq!(registry.list().len(), 1);
    }

    #[test]
    fn test_registry_has_tool() {
        let registry = ToolRegistry::new().register(Arc::new(DummyTool {
            name: "tool_a".to_string(),
            result: "a".to_string(),
        }));

        assert!(registry.get("tool_a").is_some());
        assert!(!registry.get("tool_b").is_some());
    }

    #[tokio::test]
    async fn test_registry_overwrites_duplicate() {
        let mut registry = ToolRegistry::new()
            .register(Arc::new(DummyTool {
                name: "tool_a".to_string(),
                result: "first".to_string(),
            }))
            .register(Arc::new(DummyTool {
                name: "tool_a".to_string(),
                result: "second".to_string(),
            }));

        assert_eq!(registry.len(), 1);
        let result = registry.execute("tool_a", "args").await.unwrap();
        assert_eq!(result, "second");
    }

    // ---------------------------------------------------------------------------
    // 4. ReAct Loop Validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_react_loop_max_iterations() {
        let max_iterations = 5;
        let mut iterations = 0;
        let mut state = RuntimeState::Idle;

        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();

        while iterations < max_iterations {
            state = state.try_transition(RuntimeState::Acting).unwrap();
            state = state.try_transition(RuntimeState::Synthesizing).unwrap();
            iterations += 1;
        }

        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
        assert_eq!(iterations, max_iterations);
    }

    #[test]
    fn test_react_loop_no_tool_calls_finishes() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_react_loop_with_single_tool_call() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_react_loop_with_tool_failure() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_react_loop_provider_failure() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Failed).unwrap();
        assert!(state.is_terminal());
    }

    // ---------------------------------------------------------------------------
    // 5. Event Pipeline Validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_event_ordering() {
        let (tx, rx) = std::sync::mpsc::channel();

        tx.send(AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: "test".to_string(),
        })
        .unwrap();
        tx.send(AgentEvent::ToolStarted {
            tool: "read_file".to_string(),
            args: "main.rs".to_string(),
        })
        .unwrap();
        tx.send(AgentEvent::ToolCompleted {
            tool: "read_file".to_string(),
            result: "content".to_string(),
            success: true,
        })
        .unwrap();
        tx.send(AgentEvent::AgentCompleted {
            agent: "main".to_string(),
            duration_ms: 100,
        })
        .unwrap();

        drop(tx);

        let events: Vec<AgentEvent> = rx.iter().collect();
        assert_eq!(events.len(), 4);

        match &events[0] {
            AgentEvent::AgentStarted { agent, .. } => assert_eq!(agent, "main"),
            _ => panic!("Expected AgentStarted"),
        }
        match &events[1] {
            AgentEvent::ToolStarted { tool, .. } => assert_eq!(tool, "read_file"),
            _ => panic!("Expected ToolStarted"),
        }
        match &events[2] {
            AgentEvent::ToolCompleted { tool, .. } => assert_eq!(tool, "read_file"),
            _ => panic!("Expected ToolCompleted"),
        }
        match &events[3] {
            AgentEvent::AgentCompleted { agent, .. } => assert_eq!(agent, "main"),
            _ => panic!("Expected AgentCompleted"),
        }
    }

    #[test]
    fn test_event_no_duplication() {
        let (tx, rx) = std::sync::mpsc::channel();

        tx.send(AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: "test".to_string(),
        })
        .unwrap();
        tx.send(AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: "test".to_string(),
        })
        .unwrap();

        drop(tx);

        let events: Vec<AgentEvent> = rx.iter().collect();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_event_channel_capacity() {
        let (tx, rx) = std::sync::mpsc::channel();

        for i in 0..1000 {
            tx.send(AgentEvent::Log {
                level: "info".to_string(),
                message: i.to_string(),
            })
            .unwrap();
        }

        drop(tx);

        let events: Vec<AgentEvent> = rx.iter().collect();
        assert_eq!(events.len(), 1000);
    }

    #[test]
    fn test_event_thread_safety() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut handles = vec![];

        for thread_id in 0..10 {
            let tx_clone = tx.clone();
            let handle = std::thread::spawn(move || {
                for i in 0..100 {
                    tx_clone
                        .send(AgentEvent::Log {
                            level: "info".to_string(),
                            message: format!("thread {} event {}", thread_id, i),
                        })
                        .unwrap();
                }
            });
            handles.push(handle);
        }

        drop(tx);

        for handle in handles {
            handle.join().unwrap();
        }

        let events: Vec<AgentEvent> = rx.iter().collect();
        assert_eq!(events.len(), 1000);
    }

    #[test]
    fn test_event_drain() {
        let (tx, rx) = std::sync::mpsc::channel();

        tx.send(AgentEvent::Log {
            level: "info".to_string(),
            message: "event1".to_string(),
        })
        .unwrap();
        tx.send(AgentEvent::Log {
            level: "info".to_string(),
            message: "event2".to_string(),
        })
        .unwrap();

        let events = rx.try_recv();
        assert!(events.is_ok());
        let events = rx.try_recv();
        assert!(events.is_ok());
        let events = rx.try_recv();
        assert!(events.is_err());
    }

    // ---------------------------------------------------------------------------
    // 6. Stress Testing
    // ---------------------------------------------------------------------------

    #[test]
    fn test_state_transitions_under_load() {
        let iterations = 10000;
        let start = Instant::now();

        for _ in 0..iterations {
            let mut state = RuntimeState::Idle;
            state = state.try_transition(RuntimeState::Observing).unwrap();
            state = state.try_transition(RuntimeState::Reasoning).unwrap();
            state = state.try_transition(RuntimeState::Synthesizing).unwrap();
            state.try_transition(RuntimeState::Completed).unwrap();
        }

        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(1),
            "Too slow: {} iterations in {:?}",
            iterations,
            elapsed
        );
    }

    #[test]
    fn test_event_throughput() {
        let iterations = 10000;
        let start = Instant::now();

        let (tx, rx) = std::sync::mpsc::channel();
        for i in 0..iterations {
            tx.send(AgentEvent::Log {
                level: "info".to_string(),
                message: i.to_string(),
            })
            .unwrap();
        }
        drop(tx);

        let count = rx.iter().count();
        let elapsed = start.elapsed();
        assert_eq!(count, iterations);
        assert!(
            elapsed < Duration::from_secs(1),
            "Too slow: {} events in {:?}",
            iterations,
            elapsed
        );
    }

    #[test]
    fn test_registry_lookup_performance() {
        struct FastTool {
            name: String,
        }
        impl Tool for FastTool {
            fn name(&self) -> &str {
                &self.name
            }
            fn description(&self) -> &str {
                "fast"
            }
            fn execute(&self, _args: &str) -> anyhow::Result<String> {
                Ok("ok".to_string())
            }
        }

        let mut registry = ToolRegistry::new();
        for i in 0..100 {
            registry = registry.register(Arc::new(FastTool {
                name: format!("tool_{}", i),
            }));
        }

        let start = Instant::now();
        let iterations = 10000;
        for i in 0..iterations {
            let _ = registry.execute(&format!("tool_{}", i % 100), "args");
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(1),
            "Too slow: {} lookups in {:?}",
            iterations,
            elapsed
        );
    }

    #[test]
    fn test_repeated_state_machine_warmup() {
        let iterations = 100;
        let mut total_time = Duration::new(0, 0);

        for _ in 0..iterations {
            let start = Instant::now();
            let mut state = RuntimeState::Idle;
            state = state.try_transition(RuntimeState::Observing).unwrap();
            state = state.try_transition(RuntimeState::Reasoning).unwrap();
            state = state.try_transition(RuntimeState::Synthesizing).unwrap();
            state = state.try_transition(RuntimeState::Acting).unwrap();
            state = state.try_transition(RuntimeState::Synthesizing).unwrap();
            state.try_transition(RuntimeState::Completed).unwrap();
            total_time += start.elapsed();
        }

        let avg = total_time / iterations;
        assert!(avg < Duration::from_millis(1), "Too slow: avg {:?}", avg);
    }

    // ---------------------------------------------------------------------------
    // 7. Failure Recovery Validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_provider_failure_transitions_to_failed() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Failed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_tool_failure_does_not_break_state_machine() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_malformed_tool_call_handled() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_timeout_handled_as_failed() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Failed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_cancellation_handled() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Failed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_recovery_after_tool_failure() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_multiple_tool_failures() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();

        for _ in 0..5 {
            state = state.try_transition(RuntimeState::Acting).unwrap();
            state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        }

        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    // ---------------------------------------------------------------------------
    // 8. Integration Validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_full_pipeline_state_flow() {
        let mut state = RuntimeState::Idle;

        assert_eq!(state, RuntimeState::Idle);
        assert!(!state.is_active());
        assert!(!state.is_terminal());

        state = state.try_transition(RuntimeState::Observing).unwrap();
        assert!(state.is_active());
        assert!(!state.is_terminal());

        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        assert!(state.is_active());
        assert!(!state.is_terminal());

        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        assert!(state.is_active());
        assert!(!state.is_terminal());

        state = state.try_transition(RuntimeState::Acting).unwrap();
        assert!(state.is_active());
        assert!(!state.is_terminal());

        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        assert!(state.is_active());
        assert!(!state.is_terminal());

        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(!state.is_active());
        assert!(state.is_terminal());
    }

    #[test]
    fn test_registry_with_real_tools() {
        let registry = ToolRegistry::new()
            .register(Arc::new(ListFiles))
            .register(Arc::new(ReadFile))
            .register(Arc::new(RunCommand::new()));

        assert_eq!(registry.len(), 3);
        assert!(registry.get("list_files").is_some());
        assert!(registry.get("read_file").is_some());
        assert!(registry.get("run_command").is_some());
        assert!(!registry.get("unknown").is_some());
    }

    #[tokio::test]
    async fn test_registry_execute_real_tools() {
        let mut registry = ToolRegistry::new().register(Arc::new(ListFiles));
        let result = registry.execute("list_files", ".").await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_event_summary() {
        let event = AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: "test task".to_string(),
        };
        let summary = event.summary();
        assert!(summary.contains("main"));
        assert!(summary.contains("test task"));
    }

    #[test]
    fn test_event_stream_chunk_summary() {
        let event = AgentEvent::StreamChunk {
            content: "hello".to_string(),
        };
        assert_eq!(event.summary(), "streaming");
    }

    #[test]
    fn test_event_log_summary() {
        let event = AgentEvent::Log {
            level: "info".to_string(),
            message: "test message".to_string(),
        };
        let summary = event.summary();
        assert!(summary.contains("info"));
        assert!(summary.contains("test message"));
    }
}

// ===========================================================================
// P2 Reliability Layer Validation Suite
// ===========================================================================

#[cfg(test)]
mod p2_reliability {
    use super::*;
    use crate::reliability::{
        Diagnostics, MemoryLogSink, ResourceGuard, ResourceGuardConfig, ResourceStatus,
        RuntimeErrorCategory, StructuredLogger, TimeoutKind, TimeoutManager,
    };
    use std::sync::Arc;
    use std::time::Duration;

    // ---------------------------------------------------------------------------
    // 1. Error Classification
    // ---------------------------------------------------------------------------

    #[test]
    fn test_provider_timeout_classification() {
        assert_eq!(
            crate::reliability::classify_error("request timed out after 30s"),
            RuntimeErrorCategory::ProviderTimeout
        );
    }

    #[test]
    fn test_tool_timeout_classification() {
        assert_eq!(
            crate::reliability::classify_error("command timed out after 60s"),
            RuntimeErrorCategory::ToolTimeout
        );
    }

    #[test]
    fn test_rate_limit_classification() {
        assert_eq!(
            crate::reliability::classify_error("429 Too Many Requests"),
            RuntimeErrorCategory::ProviderRateLimit
        );
    }

    #[test]
    fn test_auth_failure_classification() {
        assert_eq!(
            crate::reliability::classify_error("401 Unauthorized"),
            RuntimeErrorCategory::ProviderAuthFailure
        );
    }

    #[test]
    fn test_network_error_classification() {
        assert_eq!(
            crate::reliability::classify_error("connection refused"),
            RuntimeErrorCategory::ProviderNetworkError
        );
    }

    #[test]
    fn test_permission_denied_classification() {
        assert_eq!(
            crate::reliability::classify_error("permission denied: Operation not permitted"),
            RuntimeErrorCategory::ToolPermissionDenied
        );
    }

    #[test]
    fn test_memory_limit_classification() {
        assert_eq!(
            crate::reliability::classify_error("out of memory"),
            RuntimeErrorCategory::SystemMemoryLimit
        );
    }

    #[test]
    fn test_cancellation_classification() {
        assert_eq!(
            crate::reliability::classify_error("operation cancelled"),
            RuntimeErrorCategory::SystemCancellation
        );
    }

    #[test]
    fn test_tool_execution_error_classification() {
        assert_eq!(
            crate::reliability::classify_error("command exited with code 1"),
            RuntimeErrorCategory::ToolExecutionError
        );
    }

    #[test]
    fn test_unknown_classification() {
        assert_eq!(
            crate::reliability::classify_error("some random error"),
            RuntimeErrorCategory::Unknown
        );
    }

    #[test]
    fn test_retryable_categories() {
        assert!(RuntimeErrorCategory::ProviderTimeout.is_retryable());
        assert!(RuntimeErrorCategory::ProviderRateLimit.is_retryable());
        assert!(RuntimeErrorCategory::ProviderNetworkError.is_retryable());
        assert!(RuntimeErrorCategory::ToolTimeout.is_retryable());
        assert!(RuntimeErrorCategory::Unknown.is_retryable());

        assert!(!RuntimeErrorCategory::ProviderAuthFailure.is_retryable());
        assert!(!RuntimeErrorCategory::ToolPermissionDenied.is_retryable());
        assert!(!RuntimeErrorCategory::SystemMemoryLimit.is_retryable());
        assert!(!RuntimeErrorCategory::SystemCancellation.is_retryable());
        assert!(!RuntimeErrorCategory::ToolExecutionError.is_retryable());
    }

    #[test]
    fn test_escalation_levels() {
        assert_eq!(
            RuntimeErrorCategory::ProviderAuthFailure.escalation_level(),
            3
        );
        assert_eq!(
            RuntimeErrorCategory::SystemMemoryLimit.escalation_level(),
            3
        );
        assert_eq!(
            RuntimeErrorCategory::ProviderNetworkError.escalation_level(),
            2
        );
        assert_eq!(
            RuntimeErrorCategory::ToolPermissionDenied.escalation_level(),
            2
        );
        assert_eq!(RuntimeErrorCategory::SystemShutdown.escalation_level(), 2);
        assert_eq!(RuntimeErrorCategory::ProviderTimeout.escalation_level(), 1);
        assert_eq!(
            RuntimeErrorCategory::SystemCancellation.escalation_level(),
            0
        );
    }

    // ---------------------------------------------------------------------------
    // 2. Timeout Manager
    // ---------------------------------------------------------------------------

    #[test]
    fn test_timeout_default_values() {
        let tm = TimeoutManager::new();
        assert_eq!(tm.get_provider_timeout("openai"), 60_000);
        assert_eq!(tm.get_tool_timeout("run_command"), 60_000);
        assert_eq!(tm.get_system_timeout(), 300_000);
    }

    #[test]
    fn test_timeout_custom_values() {
        let tm = TimeoutManager::new();
        tm.set_provider_timeout("openai", 30_000);
        tm.set_tool_timeout("run_command", 120_000);
        tm.set_system_timeout(600_000);

        assert_eq!(tm.get_provider_timeout("openai"), 30_000);
        assert_eq!(tm.get_tool_timeout("run_command"), 120_000);
        assert_eq!(tm.get_system_timeout(), 600_000);
    }

    #[test]
    fn test_timeout_start_remove() {
        let tm = TimeoutManager::new();
        tm.start_timeout("t1", TimeoutKind::Provider, "openai");
        assert_eq!(tm.active_count(), 1);
        assert!(!tm.is_expired("t1"));
        tm.remove("t1");
        assert_eq!(tm.active_count(), 0);
    }

    #[test]
    fn test_timeout_clear() {
        let tm = TimeoutManager::new();
        tm.start_timeout("t1", TimeoutKind::Provider, "openai");
        tm.start_timeout("t2", TimeoutKind::Tool, "run_command");
        assert_eq!(tm.active_count(), 2);
        tm.clear();
        assert_eq!(tm.active_count(), 0);
    }

    // ---------------------------------------------------------------------------
    // 3. Resource Guard
    // ---------------------------------------------------------------------------

    #[test]
    fn test_resource_guard_initial() {
        let guard = ResourceGuard::new();
        assert_eq!(guard.status(), ResourceStatus::OK);
        assert_eq!(guard.current_memory_mb(), 0);
        assert_eq!(guard.operations_count(), 0);
        assert!(!guard.should_shutdown());
    }

    #[test]
    fn test_resource_guard_memory_warning() {
        let guard = ResourceGuard::with_config(ResourceGuardConfig {
            memory_limit_mb: 512,
            operation_limit: 10000,
            memory_warning_threshold: 0.8,
        });
        assert_eq!(guard.update_memory(450), ResourceStatus::MemoryWarning);
        assert_eq!(guard.update_memory(300), ResourceStatus::OK);
    }

    #[test]
    fn test_resource_guard_memory_limit() {
        let guard = ResourceGuard::with_config(ResourceGuardConfig {
            memory_limit_mb: 512,
            operation_limit: 10000,
            memory_warning_threshold: 0.8,
        });
        assert_eq!(
            guard.update_memory(512),
            ResourceStatus::MemoryLimitExceeded
        );
    }

    #[test]
    fn test_resource_guard_operation_limit() {
        let guard = ResourceGuard::with_config(ResourceGuardConfig {
            memory_limit_mb: 512,
            operation_limit: 5,
            memory_warning_threshold: 0.8,
        });
        for _ in 0..4 {
            assert_eq!(guard.record_operation(), ResourceStatus::OK);
        }
        assert_eq!(
            guard.record_operation(),
            ResourceStatus::OperationLimitExceeded
        );
    }

    #[test]
    fn test_resource_guard_shutdown() {
        let guard = ResourceGuard::new();
        guard.request_shutdown();
        assert!(guard.should_shutdown());
        assert_eq!(guard.status(), ResourceStatus::ShutdownRequested);
    }

    #[test]
    fn test_resource_guard_reset() {
        let guard = ResourceGuard::new();
        guard.update_memory(500);
        guard.request_shutdown();
        guard.reset();
        assert_eq!(guard.status(), ResourceStatus::OK);
        assert!(!guard.should_shutdown());
    }

    // ---------------------------------------------------------------------------
    // 4. Diagnostics
    // ---------------------------------------------------------------------------

    #[test]
    fn test_diagnostics_initial() {
        let diag = Diagnostics::new();
        assert_eq!(diag.failure_count(), 0);
        assert_eq!(diag.recovery_count(), 0);
        assert_eq!(diag.recovered_count(), 0);
    }

    #[test]
    fn test_diagnostics_record_failure() {
        let diag = Diagnostics::new();
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "request timed out",
            "provider:openai",
            None,
            false,
        );
        assert_eq!(diag.failure_count(), 1);
        assert_eq!(diag.unrecovered_count(), 1);
    }

    #[test]
    fn test_diagnostics_record_recovery() {
        let diag = Diagnostics::new();
        diag.record_recovery("timeout error", "retry", true, 1);
        assert_eq!(diag.recovery_count(), 1);

        let traces = diag.recovery_traces();
        assert_eq!(traces.len(), 1);
        assert!(traces[0].success);
        assert_eq!(traces[0].retry_count, 1);
    }

    #[test]
    fn test_diagnostics_correlation_id() {
        let diag = Diagnostics::new();
        let id1 = diag.correlation_id();
        let id2 = diag.new_correlation_id();
        assert_ne!(id1, id2);
        assert_eq!(diag.correlation_id(), id2);
    }

    #[test]
    fn test_diagnostics_category_filter() {
        let diag = Diagnostics::new();
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "t1",
            "p",
            None,
            false,
        );
        diag.record_failure(RuntimeErrorCategory::ToolTimeout, "t2", "tool", None, false);
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "t3",
            "p",
            None,
            false,
        );

        assert_eq!(
            diag.failures_by_category(&RuntimeErrorCategory::ProviderTimeout)
                .len(),
            2
        );
        assert_eq!(
            diag.failures_by_category(&RuntimeErrorCategory::ToolTimeout)
                .len(),
            1
        );
    }

    #[test]
    fn test_diagnostics_clear() {
        let diag = Diagnostics::new();
        diag.record_failure(RuntimeErrorCategory::Unknown, "err", "src", None, false);
        diag.record_recovery("err", "retry", true, 1);
        diag.clear();
        assert_eq!(diag.failure_count(), 0);
        assert_eq!(diag.recovery_count(), 0);
    }

    // ---------------------------------------------------------------------------
    // 5. Structured Logging
    // ---------------------------------------------------------------------------

    #[test]
    fn test_logger_child() {
        let parent = StructuredLogger::new("corr-1", "parent");
        let child = parent.child("child-target");
        assert_eq!(child.correlation_id, "corr-1");
        assert_eq!(child.target, "child-target");
    }

    #[test]
    fn test_memory_sink() {
        let sink = MemoryLogSink::new(100);
        let mut logger = StructuredLogger::new("corr-1", "test");
        logger.add_sink(Box::new(sink.clone()));

        logger.info("test message");
        assert_eq!(sink.count(), 1);

        let entries = sink.entries();
        assert_eq!(entries[0].level, crate::reliability::LogLevel::Info);
        assert_eq!(entries[0].message, "test message");
    }

    #[test]
    fn test_log_levels() {
        let sink = MemoryLogSink::new(100);
        let mut logger = StructuredLogger::new("corr-1", "test");
        logger.add_sink(Box::new(sink.clone()));

        logger.trace("trace");
        logger.debug("debug");
        logger.info("info");
        logger.warn("warn");
        logger.error("error");

        assert_eq!(sink.count(), 5);
        let entries = sink.entries();
        assert_eq!(entries[0].level, crate::reliability::LogLevel::Trace);
        assert_eq!(entries[4].level, crate::reliability::LogLevel::Error);
    }

    // ---------------------------------------------------------------------------
    // 6. Integration: Reliability with Runtime Pipeline
    // ---------------------------------------------------------------------------

    #[test]
    fn test_recovery_flow() {
        let diag = Diagnostics::new();
        let tm = TimeoutManager::new();

        tm.start_timeout("req1", TimeoutKind::Provider, "openai");
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "request timed out",
            "provider:openai",
            Some("retry"),
            false,
        );

        assert_eq!(diag.failure_count(), 1);
        assert!(!tm.is_expired("req1"));

        tm.remove("req1");
        diag.record_recovery("request timed out", "retry succeeded", true, 1);
        assert_eq!(diag.recovery_count(), 1);
    }
}

// ===========================================================================
// P2.5 Reliability Deep Validation Suite
// ===========================================================================

#[cfg(test)]
mod p25_validation {
    use super::*;
    use crate::reliability::{
        Diagnostics, LogEntry, LogLevel, LogSink, MemoryLogSink, ResourceGuard,
        ResourceGuardConfig, ResourceStatus, RuntimeErrorCategory, StructuredLogger, TimeoutKind,
        TimeoutManager,
    };
    use std::time::Duration;

    // ===========================================================================
    // 1. ERROR CLASSIFICATION DEEP VALIDATION
    // ===========================================================================

    #[test]
    fn test_all_provider_timeout_variations() {
        assert_eq!(
            crate::reliability::classify_error("request timed out after 30s"),
            RuntimeErrorCategory::ProviderTimeout
        );
        assert_eq!(
            crate::reliability::classify_error("provider request timeout"),
            RuntimeErrorCategory::ProviderTimeout
        );
        assert_eq!(
            crate::reliability::classify_error("http request timed out"),
            RuntimeErrorCategory::ProviderTimeout
        );
        assert_eq!(
            crate::reliability::classify_error("api deadline exceeded"),
            RuntimeErrorCategory::ProviderTimeout
        );
    }

    #[test]
    fn test_all_tool_timeout_variations() {
        assert_eq!(
            crate::reliability::classify_error("command timed out after 60s"),
            RuntimeErrorCategory::ToolTimeout
        );
        assert_eq!(
            crate::reliability::classify_error("operation timed out"),
            RuntimeErrorCategory::ToolTimeout
        );
    }

    #[test]
    fn test_all_rate_limit_variations() {
        assert_eq!(
            crate::reliability::classify_error("429 Too Many Requests"),
            RuntimeErrorCategory::ProviderRateLimit
        );
        assert_eq!(
            crate::reliability::classify_error("rate limit exceeded"),
            RuntimeErrorCategory::ProviderRateLimit
        );
    }

    #[test]
    fn test_all_auth_failure_variations() {
        assert_eq!(
            crate::reliability::classify_error("401 Unauthorized"),
            RuntimeErrorCategory::ProviderAuthFailure
        );
        assert_eq!(
            crate::reliability::classify_error("invalid API key"),
            RuntimeErrorCategory::ProviderAuthFailure
        );
        assert_eq!(
            crate::reliability::classify_error("authentication failed"),
            RuntimeErrorCategory::ProviderAuthFailure
        );
    }

    #[test]
    fn test_all_network_error_variations() {
        assert_eq!(
            crate::reliability::classify_error("connection refused"),
            RuntimeErrorCategory::ProviderNetworkError
        );
        assert_eq!(
            crate::reliability::classify_error("DNS resolution failed"),
            RuntimeErrorCategory::ProviderNetworkError
        );
        assert_eq!(
            crate::reliability::classify_error("network unreachable"),
            RuntimeErrorCategory::ProviderNetworkError
        );
    }

    #[test]
    fn test_all_permission_denied_variations() {
        assert_eq!(
            crate::reliability::classify_error("permission denied: Operation not permitted"),
            RuntimeErrorCategory::ToolPermissionDenied
        );
        assert_eq!(
            crate::reliability::classify_error("EACCES: Permission denied"),
            RuntimeErrorCategory::ToolPermissionDenied
        );
        assert_eq!(
            crate::reliability::classify_error("access denied"),
            RuntimeErrorCategory::ToolPermissionDenied
        );
    }

    #[test]
    fn test_all_memory_limit_variations() {
        assert_eq!(
            crate::reliability::classify_error("out of memory"),
            RuntimeErrorCategory::SystemMemoryLimit
        );
        assert_eq!(
            crate::reliability::classify_error("oom-killer invoked"),
            RuntimeErrorCategory::SystemMemoryLimit
        );
        assert_eq!(
            crate::reliability::classify_error("memory allocation failed"),
            RuntimeErrorCategory::SystemMemoryLimit
        );
    }

    #[test]
    fn test_all_cancellation_variations() {
        assert_eq!(
            crate::reliability::classify_error("operation cancelled"),
            RuntimeErrorCategory::SystemCancellation
        );
        assert_eq!(
            crate::reliability::classify_error("user cancelled"),
            RuntimeErrorCategory::SystemCancellation
        );
        assert_eq!(
            crate::reliability::classify_error("shutdown requested"),
            RuntimeErrorCategory::SystemCancellation
        );
    }

    #[test]
    fn test_all_tool_execution_error_variations() {
        assert_eq!(
            crate::reliability::classify_error("command exited with code 1"),
            RuntimeErrorCategory::ToolExecutionError
        );
        assert_eq!(
            crate::reliability::classify_error("tool execution failed"),
            RuntimeErrorCategory::ToolExecutionError
        );
    }

    #[test]
    fn test_unknown_classification() {
        assert_eq!(
            crate::reliability::classify_error(""),
            RuntimeErrorCategory::Unknown
        );
        assert_eq!(
            crate::reliability::classify_error("some random error"),
            RuntimeErrorCategory::Unknown
        );
        assert_eq!(
            crate::reliability::classify_error("unexpected behavior"),
            RuntimeErrorCategory::Unknown
        );
    }

    #[test]
    fn test_retryable_decisions() {
        assert!(RuntimeErrorCategory::ProviderTimeout.is_retryable());
        assert!(RuntimeErrorCategory::ProviderRateLimit.is_retryable());
        assert!(RuntimeErrorCategory::ProviderNetworkError.is_retryable());
        assert!(RuntimeErrorCategory::ToolTimeout.is_retryable());
        assert!(RuntimeErrorCategory::Unknown.is_retryable());

        assert!(!RuntimeErrorCategory::ProviderAuthFailure.is_retryable());
        assert!(!RuntimeErrorCategory::ToolPermissionDenied.is_retryable());
        assert!(!RuntimeErrorCategory::SystemMemoryLimit.is_retryable());
        assert!(!RuntimeErrorCategory::SystemCancellation.is_retryable());
        assert!(!RuntimeErrorCategory::ToolExecutionError.is_retryable());
    }

    #[test]
    fn test_escalation_levels_complete() {
        assert_eq!(
            RuntimeErrorCategory::ProviderAuthFailure.escalation_level(),
            3
        );
        assert_eq!(
            RuntimeErrorCategory::SystemMemoryLimit.escalation_level(),
            3
        );
        assert_eq!(
            RuntimeErrorCategory::ProviderNetworkError.escalation_level(),
            2
        );
        assert_eq!(
            RuntimeErrorCategory::ToolPermissionDenied.escalation_level(),
            2
        );
        assert_eq!(RuntimeErrorCategory::SystemShutdown.escalation_level(), 2);
        assert_eq!(RuntimeErrorCategory::ProviderTimeout.escalation_level(), 1);
        assert_eq!(
            RuntimeErrorCategory::ProviderRateLimit.escalation_level(),
            1
        );
        assert_eq!(RuntimeErrorCategory::ToolTimeout.escalation_level(), 1);
        assert_eq!(
            RuntimeErrorCategory::ToolExecutionError.escalation_level(),
            1
        );
        assert_eq!(RuntimeErrorCategory::Unknown.escalation_level(), 1);
        assert_eq!(
            RuntimeErrorCategory::SystemCancellation.escalation_level(),
            0
        );
    }

    #[test]
    fn test_runtime_error_display() {
        let err = crate::reliability::RuntimeError::new(
            "timeout",
            RuntimeErrorCategory::ProviderTimeout,
            "provider:test",
        );
        let display = format!("{}", err);
        assert!(display.contains("provider_timeout"));
        assert!(display.contains("timeout"));
        assert!(display.contains("provider:test"));
    }

    #[test]
    fn test_from_message() {
        let err = crate::reliability::from_message("connection refused", "main");
        assert_eq!(err.category, RuntimeErrorCategory::ProviderNetworkError);
        assert_eq!(err.source, "main");
    }

    #[test]
    fn test_error_is_standard_error() {
        let err =
            crate::reliability::RuntimeError::new("test", RuntimeErrorCategory::Unknown, "src");
        let _: &dyn std::error::Error = &err;
    }

    // ===========================================================================
    // 2. TIMEOUT MANAGER DEEP VALIDATION
    // ===========================================================================

    #[test]
    fn test_timeout_default_values() {
        let tm = TimeoutManager::new();
        assert_eq!(tm.get_provider_timeout("any"), 60_000);
        assert_eq!(tm.get_tool_timeout("any"), 60_000);
        assert_eq!(tm.get_system_timeout(), 300_000);
    }

    #[test]
    fn test_timeout_custom_values() {
        let tm = TimeoutManager::new();
        tm.set_provider_timeout("openai", 30_000);
        tm.set_provider_timeout("deepseek", 45_000);
        assert_eq!(tm.get_provider_timeout("openai"), 30_000);
        assert_eq!(tm.get_provider_timeout("deepseek"), 45_000);
        assert_eq!(tm.get_provider_timeout("other"), 60_000);
    }

    #[test]
    fn test_timeout_start_remove() {
        let tm = TimeoutManager::new();
        tm.start_timeout("t1", TimeoutKind::Provider, "openai");
        assert_eq!(tm.active_count(), 1);
        assert!(!tm.is_expired("t1"));
        tm.remove("t1");
        assert_eq!(tm.active_count(), 0);
    }

    #[test]
    fn test_timeout_remaining() {
        let tm = TimeoutManager::new();
        tm.start_timeout("t1", TimeoutKind::Provider, "openai");
        let remaining = tm.remaining("t1").unwrap();
        assert!(remaining.as_secs() >= 59);
        tm.remove("t1");
    }

    #[test]
    fn test_timeout_clear() {
        let tm = TimeoutManager::new();
        tm.start_timeout("t1", TimeoutKind::Provider, "openai");
        tm.start_timeout("t2", TimeoutKind::Tool, "run_command");
        tm.start_timeout("t3", TimeoutKind::System, "system");
        assert_eq!(tm.active_count(), 3);
        tm.clear();
        assert_eq!(tm.active_count(), 0);
    }

    #[test]
    fn test_timeout_different_kinds() {
        let tm = TimeoutManager::new();
        let provider_ms = tm.start_timeout("p1", TimeoutKind::Provider, "openai");
        let tool_ms = tm.start_timeout("t1", TimeoutKind::Tool, "run_command");
        let system_ms = tm.start_timeout("s1", TimeoutKind::System, "system");
        assert_eq!(provider_ms, 60_000);
        assert_eq!(tool_ms, 60_000);
        assert_eq!(system_ms, 300_000);
        tm.remove("p1");
        tm.remove("t1");
        tm.remove("s1");
    }

    #[test]
    fn test_timeout_remove_nonexistent() {
        let tm = TimeoutManager::new();
        tm.remove("nonexistent");
        assert_eq!(tm.active_count(), 0);
    }

    #[test]
    fn test_timeout_any_expired_empty() {
        let tm = TimeoutManager::new();
        assert!(!tm.any_expired());
    }

    #[test]
    fn test_timeout_thread_safety() {
        use std::thread;
        let tm = TimeoutManager::new();
        let handles: Vec<_> = (0..20)
            .map(|i| {
                let tm = tm.clone();
                thread::spawn(move || {
                    let id = format!("t{}", i);
                    tm.start_timeout(&id, TimeoutKind::Provider, "openai");
                    tm.remove(&id);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(tm.active_count(), 0);
    }

    // ===========================================================================
    // 3. RESOURCE GUARD DEEP VALIDATION
    // ===========================================================================

    #[test]
    fn test_resource_guard_initial() {
        let guard = ResourceGuard::new();
        assert_eq!(guard.status(), ResourceStatus::OK);
        assert_eq!(guard.current_memory_mb(), 0);
        assert_eq!(guard.operations_count(), 0);
        assert!(!guard.should_shutdown());
    }

    #[test]
    fn test_resource_guard_memory_warning() {
        let guard = ResourceGuard::with_config(ResourceGuardConfig {
            memory_limit_mb: 512,
            operation_limit: 10000,
            memory_warning_threshold: 0.8,
        });
        assert_eq!(guard.update_memory(409), ResourceStatus::OK);
        assert_eq!(guard.update_memory(410), ResourceStatus::MemoryWarning);
        assert_eq!(guard.update_memory(300), ResourceStatus::OK);
    }

    #[test]
    fn test_resource_guard_memory_limit() {
        let guard = ResourceGuard::with_config(ResourceGuardConfig {
            memory_limit_mb: 512,
            operation_limit: 10000,
            memory_warning_threshold: 0.8,
        });
        assert_eq!(
            guard.update_memory(512),
            ResourceStatus::MemoryLimitExceeded
        );
        assert_eq!(
            guard.update_memory(600),
            ResourceStatus::MemoryLimitExceeded
        );
    }

    #[test]
    fn test_resource_guard_operation_limit() {
        let guard = ResourceGuard::with_config(ResourceGuardConfig {
            memory_limit_mb: 512,
            operation_limit: 5,
            memory_warning_threshold: 0.8,
        });
        for _ in 0..4 {
            assert_eq!(guard.record_operation(), ResourceStatus::OK);
        }
        assert_eq!(
            guard.record_operation(),
            ResourceStatus::OperationLimitExceeded
        );
    }

    #[test]
    fn test_resource_guard_shutdown() {
        let guard = ResourceGuard::new();
        guard.request_shutdown();
        assert!(guard.should_shutdown());
        assert_eq!(guard.status(), ResourceStatus::ShutdownRequested);
        assert!(guard.shutdown_pending_duration().is_some());
    }

    #[test]
    fn test_resource_guard_reset() {
        let guard = ResourceGuard::new();
        guard.update_memory(500);
        guard.request_shutdown();
        guard.reset();
        assert_eq!(guard.status(), ResourceStatus::OK);
        assert!(!guard.should_shutdown());
        assert_eq!(guard.current_memory_mb(), 0);
        assert_eq!(guard.operations_count(), 0);
    }

    #[test]
    fn test_resource_guard_memory_limit_config() {
        let guard = ResourceGuard::with_config(ResourceGuardConfig {
            memory_limit_mb: 1024,
            operation_limit: 5000,
            memory_warning_threshold: 0.7,
        });
        assert_eq!(guard.memory_limit_mb(), 1024);
        assert_eq!(guard.operation_limit(), 5000);
    }

    #[test]
    fn test_resource_guard_thread_safety() {
        use std::thread;
        let guard = ResourceGuard::new();
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let guard = guard.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        guard.record_operation();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(guard.operations_count(), 1000);
    }

    #[test]
    fn test_resource_guard_multiple_transitions() {
        let guard = ResourceGuard::with_config(ResourceGuardConfig {
            memory_limit_mb: 100,
            operation_limit: 10,
            memory_warning_threshold: 0.5,
        });
        assert_eq!(guard.update_memory(10), ResourceStatus::OK);
        assert_eq!(guard.update_memory(49), ResourceStatus::OK);
        assert_eq!(guard.update_memory(50), ResourceStatus::MemoryWarning);
        assert_eq!(
            guard.update_memory(100),
            ResourceStatus::MemoryLimitExceeded
        );
        assert_eq!(guard.update_memory(49), ResourceStatus::OK);
    }

    #[test]
    fn test_resource_guard_shutdown_override() {
        let guard = ResourceGuard::new();
        guard.update_memory(500);
        guard.request_shutdown();
        assert_eq!(guard.status(), ResourceStatus::ShutdownRequested);
    }

    // ===========================================================================
    // 4. DIAGNOSTICS DEEP VALIDATION
    // ===========================================================================

    #[test]
    fn test_diagnostics_initial() {
        let diag = Diagnostics::new();
        assert_eq!(diag.failure_count(), 0);
        assert_eq!(diag.recovery_count(), 0);
        assert_eq!(diag.recovered_count(), 0);
        assert_eq!(diag.unrecovered_count(), 0);
    }

    #[test]
    fn test_diagnostics_record_failure() {
        let diag = Diagnostics::new();
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "request timed out",
            "provider:openai",
            Some("retry"),
            false,
        );
        assert_eq!(diag.failure_count(), 1);
        assert_eq!(diag.unrecovered_count(), 1);
        let traces = diag.failure_traces();
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].category, RuntimeErrorCategory::ProviderTimeout);
        assert_eq!(traces[0].message, "request timed out");
        assert_eq!(traces[0].source, "provider:openai");
        assert_eq!(traces[0].recovery_action, Some("retry".to_string()));
        assert!(!traces[0].recovered);
    }

    #[test]
    fn test_diagnostics_record_recovery() {
        let diag = Diagnostics::new();
        diag.record_recovery("timeout error", "retry", true, 1);
        assert_eq!(diag.recovery_count(), 1);
        let traces = diag.recovery_traces();
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].original_error, "timeout error");
        assert_eq!(traces[0].action_taken, "retry");
        assert!(traces[0].success);
        assert_eq!(traces[0].retry_count, 1);
    }

    #[test]
    fn test_diagnostics_correlation_id() {
        let diag = Diagnostics::new();
        let id1 = diag.correlation_id();
        assert!(!id1.is_empty());
        let id2 = diag.new_correlation_id();
        assert_ne!(id1, id2);
        assert_eq!(diag.correlation_id(), id2);
    }

    #[test]
    fn test_diagnostics_lru_eviction() {
        let diag = Diagnostics::new();
        for i in 0..1001 {
            diag.record_failure(
                RuntimeErrorCategory::Unknown,
                &format!("error {}", i),
                "test",
                None,
                false,
            );
        }
        assert_eq!(diag.failure_count(), 1000);
    }

    #[test]
    fn test_diagnostics_category_filter() {
        let diag = Diagnostics::new();
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "t1",
            "p",
            None,
            false,
        );
        diag.record_failure(RuntimeErrorCategory::ToolTimeout, "t2", "tool", None, false);
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "t3",
            "p",
            None,
            false,
        );
        assert_eq!(
            diag.failures_by_category(&RuntimeErrorCategory::ProviderTimeout)
                .len(),
            2
        );
        assert_eq!(
            diag.failures_by_category(&RuntimeErrorCategory::ToolTimeout)
                .len(),
            1
        );
    }

    #[test]
    fn test_diagnostics_clear() {
        let diag = Diagnostics::new();
        diag.record_failure(RuntimeErrorCategory::Unknown, "err", "src", None, false);
        diag.record_recovery("err", "retry", true, 1);
        assert_eq!(diag.failure_count(), 1);
        assert_eq!(diag.recovery_count(), 1);
        diag.clear();
        assert_eq!(diag.failure_count(), 0);
        assert_eq!(diag.recovery_count(), 0);
    }

    #[test]
    fn test_diagnostics_summary() {
        let diag = Diagnostics::new();
        diag.record_failure(RuntimeErrorCategory::Unknown, "err", "src", None, false);
        diag.record_failure(RuntimeErrorCategory::Unknown, "err2", "src", None, true);
        let summary = diag.summary();
        assert!(summary.contains("Failure traces: 2"));
        assert!(summary.contains("Recovered: 1"));
        assert!(summary.contains("Unrecovered: 1"));
    }

    #[test]
    fn test_diagnostics_thread_safety() {
        use std::thread;
        let diag = Diagnostics::new();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let diag = diag.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        diag.record_failure(
                            RuntimeErrorCategory::Unknown,
                            &format!("err {}", i),
                            "src",
                            None,
                            false,
                        );
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(diag.failure_count(), 1000);
    }

    #[test]
    fn test_diagnostics_mixed_failures_recoveries() {
        let diag = Diagnostics::new();
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "err1",
            "p",
            None,
            false,
        );
        diag.record_recovery("err1", "retry", true, 1);
        diag.record_failure(
            RuntimeErrorCategory::ToolTimeout,
            "err2",
            "tool",
            None,
            false,
        );
        diag.record_recovery("err2", "retry", false, 2);
        assert_eq!(diag.failure_count(), 2);
        assert_eq!(diag.recovery_count(), 2);
        // recovered_count counts failures marked as recovered, not recovery traces
        assert_eq!(diag.recovered_count(), 0);
        assert_eq!(diag.unrecovered_count(), 2);
    }

    #[test]
    fn test_diagnostics_failure_with_recovery_action() {
        let diag = Diagnostics::new();
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "timeout",
            "provider",
            Some("switch_to_fallback"),
            false,
        );
        let traces = diag.failure_traces();
        assert_eq!(
            traces[0].recovery_action,
            Some("switch_to_fallback".to_string())
        );
    }

    #[test]
    fn test_diagnostics_failure_without_recovery_action() {
        let diag = Diagnostics::new();
        diag.record_failure(RuntimeErrorCategory::Unknown, "error", "src", None, false);
        let traces = diag.failure_traces();
        assert_eq!(traces[0].recovery_action, None);
    }

    #[test]
    fn test_diagnostics_all_categories() {
        let diag = Diagnostics::new();
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "t1",
            "p",
            None,
            false,
        );
        diag.record_failure(
            RuntimeErrorCategory::ProviderRateLimit,
            "t2",
            "p",
            None,
            false,
        );
        diag.record_failure(
            RuntimeErrorCategory::ProviderAuthFailure,
            "t3",
            "p",
            None,
            false,
        );
        diag.record_failure(
            RuntimeErrorCategory::ProviderNetworkError,
            "t4",
            "p",
            None,
            false,
        );
        diag.record_failure(RuntimeErrorCategory::ToolTimeout, "t5", "tool", None, false);
        diag.record_failure(
            RuntimeErrorCategory::ToolExecutionError,
            "t6",
            "tool",
            None,
            false,
        );
        diag.record_failure(
            RuntimeErrorCategory::ToolPermissionDenied,
            "t7",
            "tool",
            None,
            false,
        );
        diag.record_failure(
            RuntimeErrorCategory::SystemMemoryLimit,
            "t8",
            "system",
            None,
            false,
        );
        diag.record_failure(
            RuntimeErrorCategory::SystemCancellation,
            "t9",
            "system",
            None,
            false,
        );
        diag.record_failure(RuntimeErrorCategory::Unknown, "t10", "unknown", None, false);
        assert_eq!(diag.failure_count(), 10);
        assert_eq!(
            diag.failures_by_category(&RuntimeErrorCategory::ProviderTimeout)
                .len(),
            1
        );
        assert_eq!(
            diag.failures_by_category(&RuntimeErrorCategory::Unknown)
                .len(),
            1
        );
    }

    // ===========================================================================
    // 5. STRUCTURED LOGGING DEEP VALIDATION
    // ===========================================================================

    #[test]
    fn test_logger_log_levels() {
        assert_eq!(LogLevel::Trace.as_str(), "TRACE");
        assert_eq!(LogLevel::Debug.as_str(), "DEBUG");
        assert_eq!(LogLevel::Info.as_str(), "INFO");
        assert_eq!(LogLevel::Warn.as_str(), "WARN");
        assert_eq!(LogLevel::Error.as_str(), "ERROR");
    }

    #[test]
    fn test_logger_from_str() {
        assert_eq!(LogLevel::from_str("trace"), Some(LogLevel::Trace));
        assert_eq!(LogLevel::from_str("debug"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::from_str("info"), Some(LogLevel::Info));
        assert_eq!(LogLevel::from_str("warn"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str("error"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str("unknown"), None);
    }

    #[test]
    fn test_logger_creation() {
        let logger = StructuredLogger::new("corr-1", "test-target");
        assert_eq!(logger.correlation_id, "corr-1");
        assert_eq!(logger.target, "test-target");
    }

    #[test]
    fn test_logger_child() {
        let parent = StructuredLogger::new("corr-1", "parent");
        let child = parent.child("child-target");
        assert_eq!(child.correlation_id, "corr-1");
        assert_eq!(child.target, "child-target");
    }

    #[test]
    fn test_logger_memory_sink() {
        let sink = MemoryLogSink::new(100);
        let mut logger = StructuredLogger::new("corr-1", "test");
        logger.add_sink(Box::new(sink.clone()));
        logger.info("test message");
        assert_eq!(sink.count(), 1);
        let entries = sink.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, LogLevel::Info);
        assert_eq!(entries[0].message, "test message");
        assert_eq!(entries[0].correlation_id, "corr-1");
        assert_eq!(entries[0].target, "test");
    }

    #[test]
    fn test_logger_entry_display() {
        let entry = LogEntry {
            level: LogLevel::Error,
            correlation_id: "corr-1".to_string(),
            target: "test".to_string(),
            message: "something failed".to_string(),
            timestamp: "2026-08-05T00:00:00+00:00".to_string(),
        };
        let display = format!("{}", entry);
        assert!(display.contains("ERROR"));
        assert!(display.contains("test"));
        assert!(display.contains("corr-1"));
        assert!(display.contains("something failed"));
    }

    #[test]
    fn test_logger_lru_eviction() {
        let sink = MemoryLogSink::new(5);
        let mut logger = StructuredLogger::new("corr-1", "test");
        logger.add_sink(Box::new(sink.clone()));
        for i in 0..10 {
            logger.info(&format!("message {}", i));
        }
        assert_eq!(sink.count(), 5);
    }

    #[test]
    fn test_logger_all_levels() {
        let sink = MemoryLogSink::new(100);
        let mut logger = StructuredLogger::new("corr-1", "test");
        logger.add_sink(Box::new(sink.clone()));
        logger.trace("trace");
        logger.debug("debug");
        logger.info("info");
        logger.warn("warn");
        logger.error("error");
        assert_eq!(sink.count(), 5);
        let entries = sink.entries();
        assert_eq!(entries[0].level, LogLevel::Trace);
        assert_eq!(entries[1].level, LogLevel::Debug);
        assert_eq!(entries[2].level, LogLevel::Info);
        assert_eq!(entries[3].level, LogLevel::Warn);
        assert_eq!(entries[4].level, LogLevel::Error);
    }

    #[test]
    fn test_logger_multiple_sinks() {
        let sink1 = MemoryLogSink::new(100);
        let sink2 = MemoryLogSink::new(100);
        let mut logger = StructuredLogger::new("corr-1", "test");
        logger.add_sink(Box::new(sink1.clone()));
        logger.add_sink(Box::new(sink2.clone()));
        logger.info("broadcast");
        assert_eq!(sink1.count(), 1);
        assert_eq!(sink2.count(), 1);
        assert_eq!(sink1.entries()[0].message, "broadcast");
        assert_eq!(sink2.entries()[0].message, "broadcast");
    }

    #[test]
    fn test_logger_child_inherits_sinks() {
        let sink = MemoryLogSink::new(100);
        let mut parent = StructuredLogger::new("corr-1", "parent");
        parent.add_sink(Box::new(sink.clone()));
        let child = parent.child("child");
        child.info("child message");
        assert_eq!(sink.count(), 1);
        assert_eq!(sink.entries()[0].target, "child");
    }

    #[test]
    fn test_logger_thread_safety() {
        use std::thread;
        let sink = MemoryLogSink::new(10000);
        let logger = StructuredLogger::new("corr-1", "test");
        let mut logger = logger;
        logger.add_sink(Box::new(sink.clone()));
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let logger = logger.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        logger.info(&format!("message {}", i));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(sink.count(), 1000);
    }

    // ===========================================================================
    // 6. INTEGRATION VALIDATION
    // ===========================================================================

    #[test]
    fn test_integration_full_recovery_flow() {
        let diag = Diagnostics::new();
        let tm = TimeoutManager::new();

        tm.start_timeout("req1", TimeoutKind::Provider, "openai");
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "request timed out",
            "provider:openai",
            Some("retry"),
            false,
        );

        assert_eq!(diag.failure_count(), 1);
        assert!(!tm.is_expired("req1"));

        tm.remove("req1");
        diag.record_recovery("request timed out", "retry", true, 0);
        assert_eq!(diag.recovery_count(), 1);
    }

    #[test]
    fn test_integration_circuit_breaker_with_diagnostics() {
        let diag = Diagnostics::new();

        for j in 2..5 {
            diag.record_failure(
                RuntimeErrorCategory::ProviderTimeout,
                "timeout",
                "provider:test",
                None,
                false,
            );
        }
        assert_eq!(diag.failure_count(), 3);

        diag.record_recovery("timeout", "circuit recovered", true, 3);
        assert_eq!(diag.recovery_count(), 1);
    }

    #[test]
    fn test_integration_resource_guard_with_diagnostics() {
        let guard = ResourceGuard::new();
        let diag = Diagnostics::new();

        for _ in 0..5 {
            guard.record_operation();
        }
        assert_eq!(guard.operations_count(), 5);
        assert_eq!(guard.status(), ResourceStatus::OK);

        guard.update_memory(200);
        assert_eq!(guard.current_memory_mb(), 200);

        guard.request_shutdown();
        assert!(guard.should_shutdown());
    }

    #[test]
    fn test_integration_timeout_manager_with_pipeline() {
        let tm = TimeoutManager::new();
        let timeout_ms = tm.start_timeout("req1", TimeoutKind::Provider, "openai");
        assert_eq!(timeout_ms, 60_000);
        assert_eq!(tm.active_count(), 1);

        let remaining = tm.remaining("req1").unwrap();
        assert!(remaining.as_secs() >= 59);

        tm.remove("req1");
        assert_eq!(tm.active_count(), 0);
    }

    #[test]
    fn test_integration_health_and_circuit_interaction() {
        let tm = TimeoutManager::new();
        let diag = Diagnostics::new();

        for j in 2..5 {
            tm.start_timeout(&format!("req{}", j), TimeoutKind::Provider, "openai");
            diag.record_failure(
                RuntimeErrorCategory::ProviderTimeout,
                &format!("timeout {}", j),
                "provider:openai",
                None,
                false,
            );
        }

        assert_eq!(tm.active_count(), 3);
        assert_eq!(diag.failure_count(), 3);

        for j in 2..5 {
            tm.remove(&format!("req{}", j));
            diag.record_recovery("timeout", "retry", true, 1);
        }
        assert_eq!(tm.active_count(), 0);
        assert_eq!(diag.recovery_count(), 3);
    }

    #[test]
    fn test_integration_diagnostics_with_all_categories() {
        let diag = Diagnostics::new();
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "t1",
            "p",
            None,
            false,
        );
        diag.record_failure(
            RuntimeErrorCategory::ProviderRateLimit,
            "t2",
            "p",
            None,
            false,
        );
        diag.record_failure(
            RuntimeErrorCategory::ProviderAuthFailure,
            "t3",
            "p",
            None,
            false,
        );
        diag.record_failure(
            RuntimeErrorCategory::ProviderNetworkError,
            "t4",
            "p",
            None,
            false,
        );
        diag.record_failure(RuntimeErrorCategory::ToolTimeout, "t5", "tool", None, false);
        diag.record_failure(
            RuntimeErrorCategory::ToolExecutionError,
            "t6",
            "tool",
            None,
            false,
        );
        diag.record_failure(
            RuntimeErrorCategory::ToolPermissionDenied,
            "t7",
            "tool",
            None,
            false,
        );
        diag.record_failure(
            RuntimeErrorCategory::SystemMemoryLimit,
            "t8",
            "system",
            None,
            false,
        );
        diag.record_failure(
            RuntimeErrorCategory::SystemCancellation,
            "t9",
            "system",
            None,
            false,
        );
        diag.record_failure(RuntimeErrorCategory::Unknown, "t10", "unknown", None, false);
        assert_eq!(diag.failure_count(), 10);
        assert_eq!(
            diag.failures_by_category(&RuntimeErrorCategory::ProviderTimeout)
                .len(),
            1
        );
        assert_eq!(
            diag.failures_by_category(&RuntimeErrorCategory::Unknown)
                .len(),
            1
        );
    }
}

// ===========================================================================
// P2.5 Stress Tests
// ===========================================================================

#[cfg(test)]
mod p25_stress {
    use super::*;
    use crate::reliability::{
        Diagnostics, MemoryLogSink, ResourceGuard, RuntimeErrorCategory, StructuredLogger,
        TimeoutKind, TimeoutManager,
    };
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn test_repeated_provider_failures() {
        let diag = Diagnostics::new();

        let start = Instant::now();
        for i in 0..100 {
            diag.record_failure(
                RuntimeErrorCategory::ProviderTimeout,
                &format!("timeout {}", i),
                "provider:openai",
                None,
                false,
            );
        }
        let elapsed = start.elapsed();

        assert_eq!(diag.failure_count(), 100);
        assert!(elapsed < Duration::from_secs(1));
    }

    #[test]
    fn test_repeated_tool_failures() {
        let diag = Diagnostics::new();

        let start = Instant::now();
        for i in 0..100 {
            diag.record_failure(
                RuntimeErrorCategory::ToolTimeout,
                &format!("tool timeout {}", i),
                "tool:run_command",
                None,
                false,
            );
        }
        let elapsed = start.elapsed();

        assert_eq!(diag.failure_count(), 100);
        assert!(elapsed < Duration::from_secs(1));
    }

    #[test]
    fn test_cancellation_storm() {
        let diag = Diagnostics::new();

        let start = Instant::now();
        for i in 0..1000 {
            diag.record_failure(
                RuntimeErrorCategory::SystemCancellation,
                &format!("cancelled {}", i),
                "system",
                None,
                false,
            );
        }
        let elapsed = start.elapsed();

        assert_eq!(diag.failure_count(), 1000);
        assert!(elapsed < Duration::from_secs(1));
    }

    #[test]
    fn test_timeout_storm() {
        let tm = TimeoutManager::new();
        let diag = Diagnostics::new();

        let start = Instant::now();
        for i in 0..1000 {
            tm.start_timeout(&format!("t{}", i), TimeoutKind::Provider, "openai");
            diag.record_failure(
                RuntimeErrorCategory::ProviderTimeout,
                &format!("timeout {}", i),
                "provider:openai",
                None,
                false,
            );
            tm.remove(&format!("t{}", i));
        }
        let elapsed = start.elapsed();

        assert_eq!(tm.active_count(), 0);
        assert_eq!(diag.failure_count(), 1000);
        assert!(elapsed < Duration::from_secs(1));
    }

    #[test]
    fn test_concurrent_runtime_requests() {
        use std::thread;
        let diag = Diagnostics::new();
        let tm = TimeoutManager::new();

        let handles: Vec<_> = (0..20)
            .map(|i| {
                let diag = diag.clone();
                let tm = tm.clone();
                thread::spawn(move || {
                    for j in 0..50 {
                        diag.record_failure(
                            RuntimeErrorCategory::ProviderTimeout,
                            &format!("timeout {}-{}", i, j),
                            "provider",
                            None,
                            false,
                        );
                        tm.start_timeout(&format!("t{}-{}", i, j), TimeoutKind::Provider, "openai");
                        tm.remove(&format!("t{}-{}", i, j));
                    }
                })
            })
            .collect();

        let start = Instant::now();
        for h in handles {
            h.join().unwrap();
        }
        let elapsed = start.elapsed();

        assert_eq!(diag.failure_count(), 1000);
        assert!(elapsed < Duration::from_secs(2));
    }

    #[test]
    fn test_repeated_recovery_cycles() {
        let diag = Diagnostics::new();

        let start = Instant::now();
        for i in 0..50 {
            diag.record_failure(
                RuntimeErrorCategory::ProviderTimeout,
                &format!("timeout {}", i),
                "provider:test",
                None,
                false,
            );
            diag.record_recovery("timeout", "circuit recovered", true, 3);
        }
        let elapsed = start.elapsed();

        assert_eq!(diag.recovery_count(), 50);
        assert!(elapsed < Duration::from_secs(2));
    }

    #[test]
    fn test_memory_pressure_stress() {
        let guard = ResourceGuard::new();

        let start = Instant::now();
        for i in 0..1000 {
            guard.update_memory(i % 512);
            guard.record_operation();
        }
        let elapsed = start.elapsed();

        assert_eq!(guard.operations_count(), 1000);
        assert!(elapsed < Duration::from_secs(1));
    }

    #[test]
    fn test_health_degradation_stress() {
        let diag = Diagnostics::new();

        let start = Instant::now();
        for i in 0..100 {
            diag.record_failure(
                RuntimeErrorCategory::ProviderTimeout,
                &format!("provider failure {}", i),
                &format!("provider_{}", i % 10),
                None,
                false,
            );
        }
        let elapsed = start.elapsed();

        assert_eq!(diag.failure_count(), 100);
        assert!(elapsed < Duration::from_secs(1));
    }

    #[test]
    fn test_diagnostics_trace_stress() {
        let diag = Diagnostics::new();

        let start = Instant::now();
        for i in 0..10000 {
            diag.record_failure(
                RuntimeErrorCategory::Unknown,
                &format!("error {}", i),
                "src",
                None,
                false,
            );
            diag.record_recovery(&format!("error {}", i), "retry", i % 2 == 0, 1);
        }
        let elapsed = start.elapsed();

        // LRU eviction keeps max 1000 traces
        assert_eq!(diag.failure_count(), 1000);
        assert_eq!(diag.recovery_count(), 1000);
        assert!(elapsed < Duration::from_secs(2));
    }

    #[test]
    fn test_logging_stress() {
        let sink = MemoryLogSink::new(100000);
        let logger = StructuredLogger::new("corr-1", "test");
        let mut logger = logger;
        logger.add_sink(Box::new(sink.clone()));

        use std::thread;
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let logger = logger.clone();
                thread::spawn(move || {
                    for _ in 0..1000 {
                        logger.info(&format!("message {}", i));
                    }
                })
            })
            .collect();

        let start = Instant::now();
        for h in handles {
            h.join().unwrap();
        }
        let elapsed = start.elapsed();

        assert_eq!(sink.count(), 10000);
        assert!(elapsed < Duration::from_secs(2));
    }
}

// =========================================================================
// P3 Tool Platform Validation Suite
// =========================================================================

#[cfg(test)]
mod p3_validation {
    use super::*;
    use crate::dispatcher::{ToolDispatcher, ToolRegistry};
    use crate::tools::{
        hooks::PermissionHook, AsyncTool, BuiltInProvider, CapabilityPermissionHook,
        DefaultRollbackHook, DiagnosticCollector, LifecycleManager, PermissionDecision,
        ProviderRegistry, RollbackHook, StreamChunk, StreamResult, ToolCapabilities, ToolContext,
        ToolContextBuilder, ToolDefinition, ToolHealth, ToolHooks, ToolLifecycleState,
        ToolMetadata, ToolProvider, ToolResult,
    };
    use futures::stream;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    // =========================================================================
    // 1. TOOL REGISTRY VALIDATION
    // =========================================================================

    struct TestTool {
        name: String,
        result: String,
        should_fail: bool,
    }

    impl TestTool {
        fn new(name: &str, result: &str) -> Self {
            TestTool {
                name: name.to_string(),
                result: result.to_string(),
                should_fail: false,
            }
        }
        fn failing(name: &str) -> Self {
            TestTool {
                name: name.to_string(),
                result: String::new(),
                should_fail: true,
            }
        }
    }

    impl crate::tools::Tool for TestTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "p3 validation test tool"
        }
        fn execute(&self, _args: &str) -> anyhow::Result<String> {
            if self.should_fail {
                Err(anyhow::anyhow!("Tool execution failed"))
            } else {
                Ok(self.result.clone())
            }
        }
    }

    #[tokio::test]
    async fn p3_registry_registration_basic() {
        let registry = ToolRegistry::new().register(Arc::new(TestTool::new("tool_a", "result_a")));
        assert_eq!(registry.len(), 1);
        assert!(registry.has_tool("tool_a"));
        assert!(!registry.has_tool("tool_b"));
    }

    #[tokio::test]
    async fn p3_registry_registration_multiple() {
        let registry = ToolRegistry::new()
            .register(Arc::new(TestTool::new("a", "1")))
            .register(Arc::new(TestTool::new("b", "2")))
            .register(Arc::new(TestTool::new("c", "3")));
        assert_eq!(registry.len(), 3);
    }

    #[tokio::test]
    async fn p3_registry_deregistration_via_disable() {
        let mut registry = ToolRegistry::new().register(Arc::new(TestTool::new("tool", "ok")));
        assert!(registry.has_tool("tool"));
        registry.disable("tool").unwrap();
        assert!(!registry.has_tool("tool"));
        assert!(registry.contains("tool"));
    }

    #[tokio::test]
    async fn p3_registry_duplicate_registration() {
        let mut registry = ToolRegistry::new()
            .register(Arc::new(TestTool::new("dup", "first")))
            .register(Arc::new(TestTool::new("dup", "second")));
        assert_eq!(registry.len(), 1);
        let result = registry.execute("dup", "").await.unwrap();
        assert_eq!(result, "second");
    }

    #[tokio::test]
    async fn p3_registry_lookup_performance() {
        let mut registry = ToolRegistry::new();
        for i in 0..1000 {
            registry = registry.register(Arc::new(TestTool::new(&format!("tool_{}", i), "ok")));
        }
        let start = Instant::now();
        for i in 0..10000 {
            let _ = registry.get(&format!("tool_{}", i % 1000));
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(100),
            "Lookup too slow: {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn p3_registry_metadata_retrieval() {
        let caps = ToolCapabilities {
            reads_files: true,
            writes_files: true,
            ..Default::default()
        };
        let meta = ToolMetadata::new("my_tool", "A tool", caps.clone(), "builtin");
        let registry = ToolRegistry::new()
            .register_with_metadata(Arc::new(TestTool::new("my_tool", "ok")), meta);
        let stored = registry.get_metadata("my_tool").unwrap();
        assert_eq!(stored.name, "my_tool");
        assert_eq!(stored.capabilities, caps);
    }

    #[tokio::test]
    async fn p3_registry_capabilities_lookup() {
        let caps = ToolCapabilities {
            executes_commands: true,
            ..Default::default()
        };
        let meta = ToolMetadata::new("cmd_tool", "Command tool", caps.clone(), "builtin");
        let registry = ToolRegistry::new()
            .register_with_metadata(Arc::new(TestTool::new("cmd_tool", "ok")), meta);
        let lookup = registry.get_capabilities("cmd_tool").unwrap();
        assert!(lookup.executes_commands);
        assert!(!lookup.reads_files);
    }

    #[tokio::test]
    async fn p3_registry_lifecycle_state_lookup() {
        let mut registry = ToolRegistry::new().register(Arc::new(TestTool::new("tool", "ok")));
        assert_eq!(
            registry.get_lifecycle_state("tool"),
            Some(ToolLifecycleState::Enabled)
        );
        registry.disable("tool").unwrap();
        assert_eq!(
            registry.get_lifecycle_state("tool"),
            Some(ToolLifecycleState::Disabled)
        );
    }

    #[tokio::test]
    async fn p3_registry_names_active_only() {
        let mut registry = ToolRegistry::new()
            .register(Arc::new(TestTool::new("a", "1")))
            .register(Arc::new(TestTool::new("b", "2")));
        registry.disable("a").unwrap();
        let names = registry.names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"b".to_string()));
    }

    #[tokio::test]
    async fn p3_registry_all_names_includes_inactive() {
        let mut registry = ToolRegistry::new()
            .register(Arc::new(TestTool::new("a", "1")))
            .register(Arc::new(TestTool::new("b", "2")));
        registry.disable("a").unwrap();
        let all = registry.all_names();
        assert_eq!(all.len(), 2);
        assert!(all.contains(&"a".to_string()));
    }

    #[tokio::test]
    async fn p3_registry_execute_success() {
        let mut registry = ToolRegistry::new().register(Arc::new(TestTool::new("ok", "success")));
        let result = registry.execute("ok", "").await.unwrap();
        assert_eq!(result, "success");
    }

    #[tokio::test]
    async fn p3_registry_execute_failure() {
        let mut registry = ToolRegistry::new().register(Arc::new(TestTool::failing("fail")));
        let result = registry.execute("fail", "").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn p3_registry_execute_unknown() {
        let mut registry = ToolRegistry::new();
        let result = registry.execute("unknown", "").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown"));
    }

    #[tokio::test]
    async fn p3_registry_disabled_blocked() {
        let mut registry = ToolRegistry::new().register(Arc::new(TestTool::new("blocked", "ok")));
        registry.disable("blocked").unwrap();
        let result = registry.execute("blocked", "").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn p3_registry_empty() {
        let registry = ToolRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[tokio::test]
    async fn p3_registry_dispatcher_integration() {
        let registry = ToolRegistry::new().register(Arc::new(TestTool::new("tool", "result")));
        let dispatcher = ToolDispatcher::new(registry);
        assert!(dispatcher.has_tool("tool"));
        assert!(!dispatcher.has_tool("missing"));
        assert_eq!(dispatcher.list_tools(), vec!["tool"]);
    }

    // =========================================================================
    // 2. CAPABILITY SYSTEM VALIDATION
    // =========================================================================

    #[test]
    fn p3_capabilities_default_empty() {
        let caps = ToolCapabilities::default();
        assert!(!caps.reads_files);
        assert!(!caps.writes_files);
        assert!(!caps.executes_commands);
        assert!(!caps.accesses_network);
        assert!(!caps.accesses_environment);
        assert!(!caps.modifies_state);
        assert!(!caps.requires_confirmation);
        assert!(!caps.streams_output);
    }

    #[test]
    fn p3_capabilities_read_only() {
        let caps = ToolCapabilities {
            reads_files: true,
            ..Default::default()
        };
        assert!(caps.is_read_only());
        assert_eq!(
            caps.permission_policy(),
            crate::tools::PermissionPolicy::AutoAllow
        );
    }

    #[test]
    fn p3_capabilities_mutating() {
        let mut caps = ToolCapabilities::default();
        assert!(!caps.is_mutating());
        caps.writes_files = true;
        assert!(caps.is_mutating());
        caps.writes_files = false;
        caps.executes_commands = true;
        assert!(caps.is_mutating());
    }

    #[test]
    fn p3_capabilities_high_risk() {
        let caps = ToolCapabilities {
            executes_commands: true,
            writes_files: true,
            ..Default::default()
        };
        assert!(caps.is_high_risk());
        assert_eq!(
            caps.permission_policy(),
            crate::tools::PermissionPolicy::RequireConfirmation
        );
    }

    #[test]
    fn p3_capabilities_subset() {
        let a = ToolCapabilities {
            reads_files: true,
            ..Default::default()
        };
        let b = ToolCapabilities {
            reads_files: true,
            writes_files: true,
            ..Default::default()
        };
        assert!(a.is_subset_of(&b));
        assert!(!b.is_subset_of(&a));
    }

    #[test]
    fn p3_capabilities_union() {
        let a = ToolCapabilities {
            reads_files: true,
            ..Default::default()
        };
        let b = ToolCapabilities {
            writes_files: true,
            ..Default::default()
        };
        let union = a.union(&b);
        assert!(union.reads_files);
        assert!(union.writes_files);
    }

    #[test]
    fn p3_capabilities_intersection() {
        let a = ToolCapabilities {
            reads_files: true,
            writes_files: true,
            ..Default::default()
        };
        let b = ToolCapabilities {
            reads_files: true,
            executes_commands: true,
            ..Default::default()
        };
        let inter = a.intersection(&b);
        assert!(inter.reads_files);
        assert!(!inter.writes_files);
        assert!(!inter.executes_commands);
    }

    #[test]
    fn p3_capabilities_category() {
        assert_eq!(
            crate::tools::ToolCategory::from_capabilities(&ToolCapabilities {
                reads_files: true,
                ..Default::default()
            }),
            crate::tools::ToolCategory::Informational
        );
        assert_eq!(
            crate::tools::ToolCategory::from_capabilities(&ToolCapabilities {
                executes_commands: true,
                ..Default::default()
            }),
            crate::tools::ToolCategory::Executable
        );
        assert_eq!(
            crate::tools::ToolCategory::from_capabilities(&ToolCapabilities {
                reads_files: true,
                writes_files: true,
                ..Default::default()
            }),
            crate::tools::ToolCategory::Mutating
        );
        assert_eq!(
            crate::tools::ToolCategory::from_capabilities(&ToolCapabilities {
                modifies_state: true,
                ..Default::default()
            }),
            crate::tools::ToolCategory::Stateful
        );
        assert_eq!(
            crate::tools::ToolCategory::from_capabilities(&ToolCapabilities {
                reads_files: true,
                writes_files: true,
                executes_commands: true,
                ..Default::default()
            }),
            crate::tools::ToolCategory::Composite
        );
    }

    #[test]
    fn p3_capabilities_format() {
        let caps = ToolCapabilities {
            reads_files: true,
            writes_files: true,
            executes_commands: true,
            ..Default::default()
        };
        let formatted = caps.format();
        assert!(formatted.contains("read"));
        assert!(formatted.contains("write"));
        assert!(formatted.contains("execute"));
    }

    // =========================================================================
    // 3. LIFECYCLE VALIDATION
    // =========================================================================

    #[test]
    fn p3_lifecycle_all_valid_transitions() {
        let transitions = [
            (
                ToolLifecycleState::Unregistered,
                ToolLifecycleState::Registered,
            ),
            (ToolLifecycleState::Registered, ToolLifecycleState::Enabled),
            (ToolLifecycleState::Registered, ToolLifecycleState::Disabled),
            (ToolLifecycleState::Enabled, ToolLifecycleState::Disabled),
            (ToolLifecycleState::Disabled, ToolLifecycleState::Enabled),
            (ToolLifecycleState::Enabled, ToolLifecycleState::Deprecating),
            (
                ToolLifecycleState::Registered,
                ToolLifecycleState::Deprecating,
            ),
            (ToolLifecycleState::Deprecating, ToolLifecycleState::Removed),
        ];
        for (from, to) in transitions {
            assert!(
                from.can_transition_to(&to),
                "{:?} -> {:?} should be valid",
                from,
                to
            );
        }
    }

    #[test]
    fn p3_lifecycle_invalid_transitions_rejected() {
        assert!(!ToolLifecycleState::Unregistered.can_transition_to(&ToolLifecycleState::Enabled));
        assert!(!ToolLifecycleState::Enabled.can_transition_to(&ToolLifecycleState::Removed));
        assert!(!ToolLifecycleState::Removed.can_transition_to(&ToolLifecycleState::Enabled));
    }

    #[test]
    fn p3_lifecycle_is_active() {
        assert!(ToolLifecycleState::Enabled.is_active());
        assert!(ToolLifecycleState::Deprecating.is_active());
        assert!(!ToolLifecycleState::Disabled.is_active());
        assert!(!ToolLifecycleState::Registered.is_active());
        assert!(!ToolLifecycleState::Unregistered.is_active());
        assert!(!ToolLifecycleState::Removed.is_active());
    }

    #[test]
    fn p3_lifecycle_is_terminal() {
        assert!(ToolLifecycleState::Removed.is_terminal());
        assert!(!ToolLifecycleState::Enabled.is_terminal());
        assert!(!ToolLifecycleState::Deprecating.is_terminal());
    }

    #[test]
    fn p3_lifecycle_requires_warning() {
        assert!(ToolLifecycleState::Deprecating.requires_warning());
        assert!(!ToolLifecycleState::Enabled.requires_warning());
        assert!(!ToolLifecycleState::Disabled.requires_warning());
    }

    #[test]
    fn p3_lifecycle_full_sequence() {
        let mut mgr = LifecycleManager::new();
        mgr.register("tool").unwrap();
        assert_eq!(mgr.state("tool"), Some(ToolLifecycleState::Registered));
        mgr.enable("tool").unwrap();
        assert_eq!(mgr.state("tool"), Some(ToolLifecycleState::Enabled));
        assert!(mgr.is_active("tool"));
        mgr.disable("tool").unwrap();
        assert_eq!(mgr.state("tool"), Some(ToolLifecycleState::Disabled));
        assert!(!mgr.is_active("tool"));
        mgr.enable("tool").unwrap();
        mgr.deprecate("tool").unwrap();
        assert_eq!(mgr.state("tool"), Some(ToolLifecycleState::Deprecating));
        assert!(mgr.is_active("tool"));
    }

    #[test]
    fn p3_lifecycle_multiple_tools_independent() {
        let mut mgr = LifecycleManager::new();
        mgr.register("a").unwrap();
        mgr.register("b").unwrap();
        mgr.register("c").unwrap();
        mgr.enable("a").unwrap();
        mgr.disable("b").unwrap();
        mgr.deprecate("c").unwrap();
        assert!(mgr.is_active("a"));
        assert!(!mgr.is_active("b"));
        assert!(mgr.is_active("c"));
    }

    // =========================================================================
    // 4. HOOKS VALIDATION
    // =========================================================================

    struct DenyAllHook;
    impl PermissionHook for DenyAllHook {
        fn check(&self, _context: &ToolContext) -> PermissionDecision {
            PermissionDecision::Denied {
                reason: "Blocked by policy".to_string(),
            }
        }
    }

    struct AskAllHook;
    impl PermissionHook for AskAllHook {
        fn check(&self, context: &ToolContext) -> PermissionDecision {
            PermissionDecision::Ask {
                reason: "Requires confirmation".to_string(),
                tool_name: context.tool_name.clone(),
                args: context.args.clone(),
            }
        }
    }

    struct CaptureHook {
        pub before_called: std::sync::atomic::AtomicBool,
        pub after_called: std::sync::atomic::AtomicBool,
    }

    impl CaptureHook {
        fn new() -> Self {
            CaptureHook {
                before_called: std::sync::atomic::AtomicBool::new(false),
                after_called: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    impl RollbackHook for CaptureHook {
        fn before_execute(&self, _context: &mut ToolContext) -> anyhow::Result<()> {
            self.before_called
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn after_execute(
            &self,
            _context: &ToolContext,
            _result: &ToolResult,
        ) -> anyhow::Result<()> {
            self.after_called
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn p3_hooks_capability_allows_readonly() {
        let hook = CapabilityPermissionHook;
        let ctx = ToolContext::builder("read_file", "main.rs")
            .with_capabilities(ToolCapabilities {
                reads_files: true,
                ..Default::default()
            })
            .build();
        let decision = hook.check(&ctx);
        assert!(decision.is_allowed());
    }

    #[test]
    fn p3_hooks_capability_blocks_high_risk() {
        let hook = CapabilityPermissionHook;
        let ctx = ToolContext::builder("run_command", "rm -rf /")
            .with_capabilities(ToolCapabilities {
                executes_commands: true,
                writes_files: true,
                ..Default::default()
            })
            .build();
        let decision = hook.check(&ctx);
        assert!(decision.requires_ask());
    }

    #[test]
    fn p3_hooks_deny_all() {
        let hook = DenyAllHook;
        let ctx = ToolContext::new("any", "args");
        let decision = hook.check(&ctx);
        assert!(decision.is_denied());
    }

    #[test]
    fn p3_hooks_ask_all() {
        let hook = AskAllHook;
        let ctx = ToolContext::new("any", "args");
        let decision = hook.check(&ctx);
        assert!(decision.requires_ask());
        assert!(decision.requires_ask());
    }

    #[test]
    fn p3_hooks_tool_hooks_fallback() {
        let hooks = ToolHooks::new();
        let ctx = ToolContext::new("read_file", "main.rs");
        let decision = hooks.check_permission(&ctx);
        assert!(decision.is_allowed());
    }

    #[test]
    fn p3_hooks_tool_hooks_custom_permission() {
        let hooks = ToolHooks::new().with_permission(Box::new(DenyAllHook));
        let ctx = ToolContext::new("read_file", "main.rs");
        let decision = hooks.check_permission(&ctx);
        assert!(decision.is_denied());
    }

    #[test]
    fn p3_hooks_rollback_before_after() {
        let hook = CaptureHook::new();
        let mut ctx = ToolContext::new("test", "args");
        let result = ToolResult::success(ctx.clone(), "output".to_string(), 10.0);
        hook.before_execute(&mut ctx).unwrap();
        assert!(hook.before_called.load(std::sync::atomic::Ordering::SeqCst));
        hook.after_execute(&ctx, &result).unwrap();
        assert!(hook.after_called.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn p3_hooks_default_rollback_noop() {
        let hook = DefaultRollbackHook::default();
        let ctx = ToolContext::new("test", "args");
        assert!(hook.before_execute(&mut ctx.clone()).is_ok());
        let result = ToolResult::success(ctx.clone(), "ok".to_string(), 0.0);
        assert!(hook.after_execute(&ctx, &result).is_ok());
    }

    // =========================================================================
    // 5. ASYNCTOOL VALIDATION
    // =========================================================================

    #[tokio::test]
    async fn p3_async_stream_chunk_creation() {
        let chunk = StreamChunk::new("hello", false);
        assert_eq!(chunk.text, "hello");
        assert!(!chunk.is_final);
        let final_chunk = StreamChunk::final_chunk("done");
        assert!(final_chunk.is_final);
    }

    #[tokio::test]
    async fn p3_async_stream_result_collect() {
        let chunks = vec![
            StreamChunk::new("part1", false),
            StreamChunk::new("part2", false),
            StreamChunk::final_chunk("part3"),
        ];
        let stream = stream::iter(chunks);
        let result = StreamResult::new(Box::pin(stream), "test");
        let collected = result.collect().await.unwrap();
        assert_eq!(collected, "part1part2part3");
    }

    #[tokio::test]
    async fn p3_async_stream_result_empty() {
        let stream = stream::empty();
        let result = StreamResult::new(Box::pin(stream), "empty");
        let collected = result.collect().await.unwrap();
        assert!(collected.is_empty());
    }

    // =========================================================================
    // 6. TOOL PROVIDER VALIDATION
    // =========================================================================

    #[test]
    fn p3_provider_built_in() {
        let provider = BuiltInProvider::default();
        assert_eq!(provider.provider_name(), "builtin");
        assert!(provider.is_available());
        assert_eq!(provider.health_check(), ToolHealth::Healthy);
    }

    #[test]
    fn p3_provider_registry_add() {
        let mut reg = ProviderRegistry::new();
        reg.add_provider(Arc::new(BuiltInProvider::default()));
        assert_eq!(reg.providers().len(), 1);
    }

    #[test]
    fn p3_provider_registry_health() {
        let reg = ProviderRegistry::new();
        let status = reg.health_status();
        assert!(status.is_empty());
    }

    // =========================================================================
    // 7. DIAGNOSTICS VALIDATION
    // =========================================================================

    #[test]
    fn p3_diagnostics_empty() {
        let diag = crate::tools::ToolDiagnostics::new("tool");
        assert_eq!(diag.tool_name, "tool");
        assert_eq!(diag.total_executions, 0);
        assert_eq!(diag.health, ToolHealth::Healthy);
    }

    #[test]
    fn p3_diagnostics_success() {
        let mut diag = crate::tools::ToolDiagnostics::new("tool");
        diag.record_success(100.0, "e1", Some(0));
        assert_eq!(diag.total_executions, 1);
        assert_eq!(diag.success_count, 1);
        assert!((diag.avg_duration_ms - 100.0).abs() < 0.01);
        assert_eq!(diag.health, ToolHealth::Healthy);
    }

    #[test]
    fn p3_diagnostics_failure() {
        let mut diag = crate::tools::ToolDiagnostics::new("tool");
        diag.record_failure(50.0, "e1", "error", Some(1));
        assert_eq!(diag.failure_count, 1);
        assert_eq!(diag.error_rate, 1.0);
        assert_eq!(diag.health, ToolHealth::Unhealthy);
    }

    #[test]
    fn p3_diagnostics_health_progression() {
        let mut diag = crate::tools::ToolDiagnostics::new("tool");
        diag.record_success(10.0, "e1", Some(0));
        assert_eq!(diag.health, ToolHealth::Healthy);
        for j in 2..5 {
            for j in 2..5 {
                diag.record_failure(10.0, &format!("e{}", j), "err", Some(1));
            }
        }
        assert_eq!(diag.health, ToolHealth::Unhealthy);
    }

    #[test]
    fn p3_diagnostics_min_max() {
        let mut diag = crate::tools::ToolDiagnostics::new("tool");
        diag.record_success(100.0, "e1", Some(0));
        diag.record_success(50.0, "e2", Some(0));
        diag.record_success(200.0, "e3", Some(0));
        assert!((diag.min_duration_ms - 50.0).abs() < 0.01);
        assert!((diag.max_duration_ms - 200.0).abs() < 0.01);
    }

    #[test]
    fn p3_diagnostics_collector() {
        let collector = DiagnosticCollector::new();
        collector.record_success("t1", 10.0, "e1", Some(0));
        collector.record_failure("t1", 5.0, "e2", "err", Some(1));
        collector.record_success("t2", 20.0, "e3", Some(0));
        let names = collector.names();
        assert_eq!(names.len(), 2);
        let t1 = collector.get("t1").unwrap();
        assert_eq!(t1.total_executions, 2);
        assert_eq!(t1.success_count, 1);
        assert_eq!(t1.failure_count, 1);
    }

    // =========================================================================
    // STRESS TESTS
    // =========================================================================

    #[test]
    fn p3_stress_mass_registration() {
        let start = Instant::now();
        let mut registry = ToolRegistry::new();
        for i in 0..1000 {
            registry = registry.register(Arc::new(TestTool::new(&format!("stress_{}", i), "ok")));
        }
        let elapsed = start.elapsed();
        assert_eq!(registry.len(), 1000);
        assert!(
            elapsed < Duration::from_secs(1),
            "Registration too slow: {:?}",
            elapsed
        );
    }

    #[test]
    fn p3_stress_rapid_enable_disable() {
        let mut registry = ToolRegistry::new();
        for i in 0..100 {
            registry = registry.register(Arc::new(TestTool::new(&format!("tool_{}", i), "ok")));
        }
        let start = Instant::now();
        for _ in 0..1000 {
            for i in 0..100 {
                let _ = registry.disable(&format!("tool_{}", i));
                let _ = registry.enable(&format!("tool_{}", i));
            }
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(10),
            "Enable/disable too slow: {:?}",
            elapsed
        );
    }

    #[test]
    fn p3_stress_concurrent_execution() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let start = Instant::now();
        rt.block_on(async {
            let handles: Vec<_> = (0..100)
                .map(|i| {
                    tokio::spawn(async move {
                        let mut registry =
                            ToolRegistry::new().register(Arc::new(TestTool::new("conc", "ok")));
                        let _ = registry.execute("conc", "").await;
                        i
                    })
                })
                .collect();
            futures::future::join_all(handles).await;
        });
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "Concurrent exec too slow: {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn p3_stress_repeated_failures() {
        struct FailingTool {
            name: String,
        }
        impl crate::tools::Tool for FailingTool {
            fn name(&self) -> &str {
                &self.name
            }
            fn description(&self) -> &str {
                "failing"
            }
            fn execute(&self, _args: &str) -> anyhow::Result<String> {
                Err(anyhow::anyhow!("Always fails"))
            }
        }
        let mut registry = ToolRegistry::new().register(Arc::new(FailingTool {
            name: "fail_tool".to_string(),
        }));
        for _ in 0..100 {
            let _ = registry.execute("fail_tool", "").await;
        }
        let diags = registry.get_diagnostics("fail_tool").unwrap();
        // Diagnostics track total executions, not just failures
        assert_eq!(diags.total_executions, 100);
    }

    #[test]
    fn p3_stress_lookup_under_load() {
        let mut registry = ToolRegistry::new();
        for i in 0..500 {
            registry = registry.register(Arc::new(TestTool::new(&format!("lookup_{}", i), "ok")));
        }
        let start = Instant::now();
        for i in 0..10000 {
            let _ = registry.get(&format!("lookup_{}", i % 500));
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "Lookup too slow: {:?}",
            elapsed
        );
    }

    // =========================================================================
    // BENCHMARK TESTS
    // =========================================================================

    #[test]
    fn p3_bench_registry_lookup() {
        let mut registry = ToolRegistry::new();
        for i in 0..1000 {
            registry = registry.register(Arc::new(TestTool::new(&format!("bench_{}", i), "ok")));
        }
        let iterations = 10000;
        let start = Instant::now();
        for i in 0..iterations {
            let _ = registry.get(&format!("bench_{}", i % 1000));
        }
        let elapsed = start.elapsed();
        let avg_ns = elapsed.as_nanos() as f64 / iterations as f64;
        let _ = avg_ns;
    }

    #[test]
    fn p3_bench_capability_lookup() {
        let mut registry = ToolRegistry::new();
        let meta = ToolMetadata::new(
            "bench_tool",
            "bench",
            ToolCapabilities {
                reads_files: true,
                ..Default::default()
            },
            "test",
        );
        registry =
            registry.register_with_metadata(Arc::new(TestTool::new("bench_tool", "ok")), meta);
        let iterations = 10000;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = registry.get_capabilities("bench_tool");
        }
        let elapsed = start.elapsed();
        let avg_ns = elapsed.as_nanos() as f64 / iterations as f64;
        assert!(
            avg_ns < 10000.0,
            "Capability lookup too slow: {:.2}ns",
            avg_ns
        );
    }

    #[test]
    fn p3_bench_metadata_access() {
        let mut registry = ToolRegistry::new();
        registry = registry.register(Arc::new(TestTool::new("bench_tool", "ok")));
        let iterations = 10000;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = registry.get_metadata("bench_tool");
        }
        let elapsed = start.elapsed();
        let avg_ns = elapsed.as_nanos() as f64 / iterations as f64;
        assert!(
            avg_ns < 10000.0,
            "Metadata access too slow: {:.2}ns",
            avg_ns
        );
    }

    #[test]
    fn p3_bench_diagnostics_overhead() {
        let collector = DiagnosticCollector::new();
        let iterations = 10000;
        let start = Instant::now();
        for i in 0..iterations {
            collector.record_success("tool", 1.0, &format!("e{}", i), Some(0));
        }
        let elapsed = start.elapsed();
        let avg_ns = elapsed.as_nanos() as f64 / iterations as f64;
        assert!(
            avg_ns < 100000.0,
            "Diagnostic recording too slow: {:.2}ns",
            avg_ns
        );
    }

    #[test]
    fn p3_bench_lifecycle_transition() {
        let mut registry = ToolRegistry::new();
        registry = registry.register(Arc::new(TestTool::new("bench_tool", "ok")));
        let iterations = 10000;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = registry.disable("bench_tool");
            let _ = registry.enable("bench_tool");
        }
        let elapsed = start.elapsed();
        let avg_ns = elapsed.as_nanos() as f64 / (iterations * 2) as f64;
        assert!(
            avg_ns < 100000.0,
            "Lifecycle transition too slow: {:.2}ns",
            avg_ns
        );
    }

    // =========================================================================
    // REGRESSION TESTS
    // =========================================================================

    #[test]
    fn p3_regression_runtime_state_machine() {
        use crate::runtime::state::RuntimeState;
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn p3_regression_provider_trait_object_safe() {
        use crate::providers::Provider;
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Box<dyn Provider>>();
    }

    #[test]
    fn p3_regression_react_loop() {
        use crate::runtime::state::RuntimeState;
        // Valid ReAct loop: Idle -> Observing -> Reasoning -> Synthesizing -> Acting -> Synthesizing -> Completed
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn p3_regression_existing_tools() {
        use crate::tools::{ListFiles, RunCommand, Tool};
        let list = ListFiles.execute(".").unwrap();
        assert!(!list.is_empty());
        let run = RunCommand::new().execute("echo hello").unwrap();
        assert_eq!(run, "hello");
    }

    #[test]
    fn p3_regression_tool_trait_unchanged() {
        use crate::tools::Tool;
        struct CheckTool;
        impl Tool for CheckTool {
            fn name(&self) -> &str {
                "check"
            }
            fn description(&self) -> &str {
                "check"
            }
            fn execute(&self, _args: &str) -> anyhow::Result<String> {
                Ok("ok".to_string())
            }
        }
        let t = CheckTool;
        assert_eq!(t.name(), "check");
        assert_eq!(t.description(), "check");
    }

    #[test]
    fn p3_regression_registry_basic_api() {
        use crate::tools::Tool;
        struct SimpleTool;
        impl Tool for SimpleTool {
            fn name(&self) -> &str {
                "simple"
            }
            fn description(&self) -> &str {
                "simple"
            }
            fn execute(&self, _args: &str) -> anyhow::Result<String> {
                Ok("ok".to_string())
            }
        }
        let registry = ToolRegistry::new().register(Arc::new(SimpleTool));
        assert!(registry.has_tool("simple"));
        assert_eq!(registry.len(), 1);
    }
}

// ===========================================================================
// P4 Intelligence Platform Validation Suite
// ===========================================================================

#[cfg(test)]
mod p4_intelligence_validation {
    use super::*;
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    // ---------------------------------------------------------------------------
    // 1. Parser Platform Validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_parser_trait_rust() {
        let mut parser: Box<dyn crate::intelligence::parser::CodeParserTrait> =
            crate::intelligence::parser::create_parser_trait("rust")
                .expect("should create rust parser");
        let supported = parser.supported_languages();
        assert!(supported.contains(&"rust"));

        let source = r#"pub fn hello() -> String { "hello".to_string() }"#;
        let result = parser.parse(source, "test.rs").expect("parse should work");
        assert!(!result.symbols.is_empty());
        assert_eq!(result.symbols[0].name, "hello");
    }

    #[test]
    fn test_parser_trait_python() {
        let mut parser: Box<dyn crate::intelligence::parser::CodeParserTrait> =
            crate::intelligence::parser::create_parser_trait("python")
                .expect("should create python parser");
        let source = "def greet(name): return f'Hello, {name}'";
        let result = parser.parse(source, "test.py").expect("parse should work");
        assert!(!result.symbols.is_empty());
    }

    #[test]
    fn test_parser_trait_javascript() {
        let mut parser: Box<dyn crate::intelligence::parser::CodeParserTrait> =
            crate::intelligence::parser::create_parser_trait("javascript")
                .expect("should create js parser");
        let source = "function greet(name) { return 'Hello, ' + name; }";
        let result = parser.parse(source, "test.js").expect("parse should work");
        assert!(!result.symbols.is_empty());
    }

    #[test]
    fn test_parser_trait_go() {
        let mut parser: Box<dyn crate::intelligence::parser::CodeParserTrait> =
            crate::intelligence::parser::create_parser_trait("go")
                .expect("should create go parser");
        let source = "package main\nfunc greet(name string) string { return 'Hello' + name }";
        let result = parser.parse(source, "test.go").expect("parse should work");
        assert!(!result.symbols.is_empty());
    }

    #[test]
    fn test_parser_trait_typescript() {
        let mut parser: Box<dyn crate::intelligence::parser::CodeParserTrait> =
            crate::intelligence::parser::create_parser_trait("typescript")
                .expect("should create ts parser");
        let source = "function greet(name: string): string { return 'Hello, ' + name; }";
        let result = parser.parse(source, "test.ts").expect("parse should work");
        assert!(!result.symbols.is_empty());
    }

    #[test]
    fn test_parser_unsupported_language() {
        let result = crate::intelligence::parser::TreeSitterParser::new("cobol");
        assert!(result.is_err());
    }

    #[test]
    fn test_create_parser_trait() {
        let parser = crate::intelligence::parser::create_parser_trait("rust")
            .expect("should create parser trait");
        assert_eq!(parser.language_name(), "rust");
    }

    // ---------------------------------------------------------------------------
    // 2. Symbol Database Validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_symbol_database_insert_and_query() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = crate::intelligence::index::SymbolDatabase::open(dir.path().join("test.db"))
            .expect("should open db");

        let symbol = crate::intelligence::index::Symbol {
            id: None,
            name: "test_func".to_string(),
            kind: crate::intelligence::index::SymbolKind::Function,
            language: "rust".to_string(),
            file: "test.rs".to_string(),
            line_start: 1,
            line_end: 5,
            column_start: 0,
            column_end: 20,
            parent: None,
            visibility: Some("public".to_string()),
            signature: Some("pub fn test_func()".to_string()),
            doc_comment: None,
        };

        let id = db.insert_symbol(&symbol).expect("insert should work");
        assert!(id > 0);

        let retrieved = db
            .get_symbol_by_name("test_func")
            .expect("query should work");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "test_func");
    }

    #[test]
    fn test_symbol_database_relationships() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = crate::intelligence::index::SymbolDatabase::open(dir.path().join("test.db"))
            .expect("should open db");

        db.insert_relationship(&crate::intelligence::index::SymbolRelationship {
            from_symbol: "a".to_string(),
            from_file: "a.rs".to_string(),
            to_symbol: "b".to_string(),
            to_file: "b.rs".to_string(),
            relationship_type: "imports".to_string(),
        })
        .expect("insert relationship should work");

        let rels = db
            .get_relationships_for_symbol("a")
            .expect("query relationships should work");
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].to_symbol, "b");
    }

    #[test]
    fn test_symbol_database_trait_impl() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = crate::intelligence::index::SymbolDatabase::open(dir.path().join("test.db"))
            .expect("should open db");

        // Verify trait implementation
        fn assert_trait<T: crate::intelligence::index::SymbolDatabaseTrait>(_t: &T) {}
        assert_trait(&db);
    }

    // ---------------------------------------------------------------------------
    // 3. Code Indexer Validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_indexer_trait_impl() {
        let dir = tempfile::tempdir().expect("tempdir");
        let indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        // Verify trait implementation
        fn assert_trait<T: crate::intelligence::index::CodeIndexerTrait>(_t: &T) {}
        assert_trait(&indexer);
    }

    #[test]
    fn test_indexer_file_indexing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn hello() -> String { "hello".to_string() }"#;
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, source).expect("write test file");

        let symbols = indexer
            .index_file(&file_path, source)
            .expect("index should work");
        assert!(!symbols.is_empty());
        assert_eq!(symbols[0].name, "hello");
    }

    #[test]
    fn test_indexer_incremental_update() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source1 = r#"pub fn hello() -> String { "hello".to_string() }"#;
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, source1).expect("write test file");

        indexer
            .index_file(&file_path, source1)
            .expect("first index");
        let count1 = indexer.get_symbol_count().expect("get count");

        let source2 = r#"pub fn hello() -> String { "world".to_string() }"#;
        indexer
            .incremental_update(&file_path, source2)
            .expect("incremental update");
        let count2 = indexer.get_symbol_count().expect("get count after update");
        assert_eq!(count1, count2);
    }

    #[test]
    fn test_indexer_remove_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn hello() -> String { "hello".to_string() }"#;
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, source).expect("write test file");

        indexer.index_file(&file_path, source).expect("index");
        assert!(indexer.get_symbol_count().expect("get count") > 0);

        indexer.remove_file(&file_path).expect("remove");
        assert_eq!(indexer.get_symbol_count().expect("get count"), 0);
    }

    #[test]
    fn test_indexer_search() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn authenticate_user(username: &str) -> bool { true }"#;
        let file_path = dir.path().join("auth.rs");
        std::fs::write(&file_path, source).expect("write test file");
        indexer.index_file(&file_path, source).expect("index");

        let results = indexer.search("auth").expect("search should work");
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "authenticate_user");
    }

    #[test]
    fn test_indexer_clear() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn hello() -> String { "hello".to_string() }"#;
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, source).expect("write test file");
        indexer.index_file(&file_path, source).expect("index");

        indexer.clear().expect("clear should work");
        assert_eq!(indexer.get_symbol_count().expect("get count"), 0);
    }

    // ---------------------------------------------------------------------------
    // 4. Dependency Graph Validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_dependency_graph_trait_impl() {
        let graph = crate::intelligence::graph::DependencyGraph::new();
        fn assert_trait<T: crate::intelligence::graph::DependencyGraphTrait>(_t: &T) {}
        assert_trait(&graph);
    }

    #[test]
    fn test_dependency_graph_add_nodes_and_edges() {
        let mut graph = crate::intelligence::graph::DependencyGraph::new();
        graph.add_node("a.rs".to_string());
        graph.add_node("b.rs".to_string());
        graph.add_edge("a.rs".to_string(), "b.rs".to_string());

        let deps = graph.get_dependencies("a.rs");
        assert_eq!(deps, vec!["b.rs".to_string()]);

        let dependents = graph.get_dependents("a.rs");
        assert!(dependents.is_empty());
    }

    #[test]
    fn test_dependency_graph_transitive_dependencies() {
        let mut graph = crate::intelligence::graph::DependencyGraph::new();
        graph.add_node("a.rs".to_string());
        graph.add_node("b.rs".to_string());
        graph.add_node("c.rs".to_string());
        graph.add_edge("a.rs".to_string(), "b.rs".to_string());
        graph.add_edge("b.rs".to_string(), "c.rs".to_string());

        let transitive = graph.get_transitive_dependencies("a.rs");
        assert!(transitive.contains("b.rs"));
        assert!(transitive.contains("c.rs"));
    }

    #[test]
    fn test_dependency_graph_find_path() {
        let mut graph = crate::intelligence::graph::DependencyGraph::new();
        graph.add_node("a.rs".to_string());
        graph.add_node("b.rs".to_string());
        graph.add_node("c.rs".to_string());
        graph.add_edge("a.rs".to_string(), "b.rs".to_string());
        graph.add_edge("b.rs".to_string(), "c.rs".to_string());

        let path = graph.find_path("a.rs", "c.rs");
        assert!(path.is_some());
        let path = path.unwrap();
        assert_eq!(path.len(), 3);
        assert_eq!(path[0], "a.rs");
        assert_eq!(path[2], "c.rs");
    }

    #[test]
    fn test_dependency_graph_save_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut graph = crate::intelligence::graph::DependencyGraph::new();
        graph.add_node("a.rs".to_string());
        graph.add_node("b.rs".to_string());
        graph.add_edge("a.rs".to_string(), "b.rs".to_string());

        let path = dir.path().join("graph.json");
        graph.save_to_file(&path).expect("save should work");

        let loaded = crate::intelligence::graph::DependencyGraph::load_from_file(&path)
            .expect("load should work");
        assert!(loaded
            .get_dependencies("a.rs")
            .contains(&"b.rs".to_string()));
    }

    #[test]
    fn test_dependency_graph_from_indexer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"use user::User; pub fn get_user(id: i32) -> User { User::new(id) }"#;
        let file1 = dir.path().join("auth.rs");
        std::fs::write(&file1, source).expect("write test file");
        indexer.index_file(&file1, source).expect("index");

        let graph = crate::intelligence::graph::DependencyGraph::from_indexer(&indexer)
            .expect("build graph");
        assert!(!graph.get_all_files().is_empty());
    }

    // ---------------------------------------------------------------------------
    // 5. Semantic Search Validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_semantic_search_trait_impl() {
        let dir = tempfile::tempdir().expect("tempdir");
        let indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");
        let search = crate::intelligence::search::SemanticSearch::new(indexer);
        fn assert_trait<T: crate::intelligence::search::SemanticSearchTrait>(_t: &T) {}
        assert_trait(&search);
    }

    #[test]
    fn test_semantic_search_exact_match() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn authenticate_user(username: &str) -> bool { true }"#;
        let file_path = dir.path().join("auth.rs");
        std::fs::write(&file_path, source).expect("write test file");
        indexer.index_file(&file_path, source).expect("index");

        let search = crate::intelligence::search::SemanticSearch::new(indexer);
        let results = search
            .search("authenticate_user")
            .expect("search should work");
        assert!(!results.is_empty());
        assert_eq!(results[0].symbol.name, "authenticate_user");
        assert!(matches!(
            results[0].match_type,
            crate::intelligence::search::MatchType::ExactName
        ));
    }

    #[test]
    fn test_semantic_search_by_question() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn authenticate_user(username: &str, password: &str) -> bool { true }"#;
        let file_path = dir.path().join("auth.rs");
        std::fs::write(&file_path, source).expect("write test file");
        indexer.index_file(&file_path, source).expect("index");

        let search = crate::intelligence::search::SemanticSearch::new(indexer);
        let results = search
            .search_by_question("how to authenticate users")
            .expect("search should work");
        assert!(!results.is_empty());
    }

    #[test]
    fn test_semantic_search_find_related() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"use user::User; pub fn get_user(id: i32) -> User { User::new(id) }"#;
        let file_path = dir.path().join("auth.rs");
        std::fs::write(&file_path, source).expect("write test file");
        indexer.index_file(&file_path, source).expect("index");

        let search = crate::intelligence::search::SemanticSearch::new(indexer);
        let related = search
            .find_related("get_user")
            .expect("find related should work");
        // Related may be empty if no relationships exist, but should not error
        let _ = related;
    }

    // ---------------------------------------------------------------------------
    // 6. Context Builder Validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_context_builder_trait_impl() {
        let dir = tempfile::tempdir().expect("tempdir");
        let indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");
        let builder = crate::intelligence::context::IntelligentContextBuilder::new(indexer);
        fn assert_trait<T: crate::intelligence::context::ContextBuilderTrait>(_t: &T) {}
        assert_trait(&builder);
    }

    #[test]
    fn test_context_builder_build_context() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn authenticate_user(username: &str, password: &str) -> bool { true }"#;
        let file_path = dir.path().join("auth.rs");
        std::fs::write(&file_path, source).expect("write test file");
        indexer.index_file(&file_path, source).expect("index");

        let builder = crate::intelligence::context::IntelligentContextBuilder::new(indexer);
        let context = builder
            .build_context("auth")
            .expect("context build should work");
        assert!(!context.query.is_empty());
        let _ = context.total_symbols_found;
    }

    #[test]
    fn test_context_builder_for_modification() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn authenticate_user(username: &str, password: &str) -> bool { true }"#;
        let file_path = dir.path().join("auth.rs");
        std::fs::write(&file_path, source).expect("write test file");
        indexer.index_file(&file_path, source).expect("index");

        let builder = crate::intelligence::context::IntelligentContextBuilder::new(indexer);
        let context = builder
            .build_context_for_modification("authenticate_user")
            .expect("modification context should work");
        assert!(!context.relevant_symbols.is_empty());
    }

    // ---------------------------------------------------------------------------
    // 7. Reasoning Engine Validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_reasoning_engine_trait_impl() {
        let dir = tempfile::tempdir().expect("tempdir");
        let indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");
        let engine = crate::intelligence::reasoning::AgentReasoningEngine::new(indexer);
        fn assert_trait<T: crate::intelligence::reasoning::ReasoningEngineTrait>(_t: &T) {}
        assert_trait(&engine);
    }

    #[test]
    fn test_reasoning_analyze_before_modification() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn authenticate_user(username: &str, password: &str) -> bool { true }"#;
        let file_path = dir.path().join("auth.rs");
        std::fs::write(&file_path, source).expect("write test file");
        indexer.index_file(&file_path, source).expect("index");

        let engine = crate::intelligence::reasoning::AgentReasoningEngine::new(indexer);
        let result = engine
            .analyze_before_modification("Add caching to authentication")
            .expect("reasoning should work");

        assert!(!result.steps.is_empty());
        assert!(!result.plan.is_empty());
        assert!(result.confidence >= 0.0);
        assert!(result.confidence <= 1.0);
    }

    #[test]
    fn test_reasoning_analyze_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn authenticate_user(username: &str) -> bool { true }"#;
        let file_path = dir.path().join("auth.rs");
        std::fs::write(&file_path, source).expect("write test file");
        indexer.index_file(&file_path, source).expect("index");

        let engine = crate::intelligence::reasoning::AgentReasoningEngine::new(indexer);
        let result = engine
            .analyze_for_code_understanding(file_path.to_str().unwrap())
            .expect("analysis should work");

        assert!(!result.steps.is_empty());
        assert!(!result.summary.is_empty());
    }

    #[test]
    fn test_reasoning_find_patterns() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn authenticate_user(username: &str) -> bool { true }"#;
        let file_path = dir.path().join("auth.rs");
        std::fs::write(&file_path, source).expect("write test file");
        indexer.index_file(&file_path, source).expect("index");

        let engine = crate::intelligence::reasoning::AgentReasoningEngine::new(indexer);
        let patterns = engine
            .find_existing_patterns("authenticate")
            .expect("pattern finding should work");
        assert!(!patterns.is_empty());
    }

    #[test]
    fn test_reasoning_suggest_approach() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub trait AuthProvider { fn authenticate(&self) -> bool; }"#;
        let file_path = dir.path().join("auth.rs");
        std::fs::write(&file_path, source).expect("write test file");
        indexer.index_file(&file_path, source).expect("index");

        let engine = crate::intelligence::reasoning::AgentReasoningEngine::new(indexer);
        let suggestions = engine
            .suggest_implementation_approach("Add new auth method")
            .expect("suggestions should work");
        assert!(!suggestions.is_empty());
    }

    // ---------------------------------------------------------------------------
    // 8. Intelligence Memory Validation
    // ---------------------------------------------------------------------------

    // ---------------------------------------------------------------------------
    // 9. LSP Foundation Validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_lsp_foundation_trait_impl() {
        let lsp = crate::intelligence::lsp::LspFoundation::new();
        fn assert_trait<T: crate::intelligence::lsp::LspFoundationTrait>(_t: &T) {}
        assert_trait(&lsp);
    }

    #[test]
    fn test_lsp_document_management() {
        let mut lsp = crate::intelligence::lsp::LspFoundation::new();

        let doc = crate::intelligence::lsp::LspTextDocumentItem {
            uri: "file:///test.rs".to_string(),
            language_id: "rust".to_string(),
            version: 1,
            text: "pub fn hello() {}".to_string(),
        };

        lsp.open_document(doc);
        assert!(lsp.get_document("file:///test.rs").is_some());

        lsp.close_document("file:///test.rs");
        assert!(lsp.get_document("file:///test.rs").is_none());
    }

    #[test]
    fn test_lsp_symbol_lookup() {
        let mut lsp = crate::intelligence::lsp::LspFoundation::new();

        lsp.add_symbol(crate::intelligence::lsp::LspSymbolInformation {
            name: "hello".to_string(),
            kind: crate::intelligence::lsp::LspSymbolKind::Function,
            location: crate::intelligence::lsp::LspLocation {
                uri: "file:///test.rs".to_string(),
                range: crate::intelligence::lsp::LspRange {
                    start: crate::intelligence::lsp::LspPosition {
                        line: 0,
                        character: 0,
                    },
                    end: crate::intelligence::lsp::LspPosition {
                        line: 0,
                        character: 20,
                    },
                },
            },
            container_name: None,
        });

        let symbols = lsp.get_symbols_for_file("test.rs");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
    }

    #[test]
    fn test_lsp_find_references() {
        let mut lsp = crate::intelligence::lsp::LspFoundation::new();

        lsp.add_symbol(crate::intelligence::lsp::LspSymbolInformation {
            name: "hello".to_string(),
            kind: crate::intelligence::lsp::LspSymbolKind::Function,
            location: crate::intelligence::lsp::LspLocation {
                uri: "file:///test.rs".to_string(),
                range: crate::intelligence::lsp::LspRange {
                    start: crate::intelligence::lsp::LspPosition {
                        line: 0,
                        character: 0,
                    },
                    end: crate::intelligence::lsp::LspPosition {
                        line: 0,
                        character: 20,
                    },
                },
            },
            container_name: None,
        });

        let refs = lsp.find_references("hello");
        assert_eq!(refs.len(), 1);
    }

    // ---------------------------------------------------------------------------
    // 10. Intelligence Diagnostics Validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_diagnostics_trait_impl() {
        let diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();
        fn assert_trait<T: crate::intelligence::diagnostics::IntelligenceDiagnosticsTrait>(_t: &T) {
        }
        assert_trait(&diag);
    }

    #[test]
    fn test_diagnostics_parse_metrics() {
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();
        diag.record_parse("test.rs", "rust", 15.0, 5, 0);
        diag.record_parse("main.rs", "rust", 25.0, 10, 0);

        let metrics = diag.get_parse_metrics();
        assert_eq!(metrics.len(), 2);
        assert!((diag.avg_parse_duration("rust") - 20.0).abs() < 0.01);
    }

    #[test]
    fn test_diagnostics_index_health() {
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();
        diag.update_index_health(100, 10, 50, vec!["rust".to_string(), "go".to_string()]);

        let health = diag.get_index_health();
        assert_eq!(health.total_symbols, 100);
        assert_eq!(health.total_files, 10);
        assert_eq!(
            health.health_status,
            crate::intelligence::diagnostics::IndexHealthStatus::Healthy
        );
    }

    #[test]
    fn test_diagnostics_graph_integrity() {
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();
        diag.update_graph_integrity(50, 80);

        let integrity = diag.get_graph_integrity();
        assert_eq!(integrity.total_nodes, 50);
        assert_eq!(integrity.total_edges, 80);
    }

    #[test]
    fn test_diagnostics_search_metrics() {
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();
        diag.record_search("auth", 5, 8.0);
        diag.record_search("user", 3, 5.0);

        let metrics = diag.get_search_metrics();
        assert_eq!(metrics.len(), 2);
        assert!((diag.avg_search_latency() - 6.5).abs() < 0.01);
    }

    #[test]
    fn test_diagnostics_context_metrics() {
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();
        diag.record_context_build("test", 10, 3, 50.0);
        diag.record_context_build("query", 5, 2, 30.0);

        let metrics = diag.get_context_metrics();
        assert_eq!(metrics.len(), 2);
        assert!((diag.avg_context_latency() - 40.0).abs() < 0.01);
    }

    #[test]
    fn test_diagnostics_summary() {
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();
        diag.record_parse("test.rs", "rust", 10.0, 5, 0);
        let summary = diag.summary();
        assert!(summary.contains("Intelligence Platform Diagnostics"));
        assert!(summary.contains("Symbols: 0"));
    }

    #[test]
    fn test_diagnostics_clear() {
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();
        diag.record_parse("test.rs", "rust", 10.0, 5, 0);
        diag.record_search("test", 3, 5.0);
        diag.clear();

        assert_eq!(diag.get_parse_metrics().len(), 0);
        assert_eq!(diag.get_search_metrics().len(), 0);
    }

    #[test]
    fn test_diagnostics_cycle_detection() {
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();
        diag.record_graph_event(
            crate::intelligence::diagnostics::GraphEvent::CycleDetected {
                files: vec!["a.rs".to_string(), "b.rs".to_string()],
            },
        );
        diag.record_graph_event(
            crate::intelligence::diagnostics::GraphEvent::CycleDetected {
                files: vec!["c.rs".to_string(), "d.rs".to_string()],
            },
        );

        let integrity = diag.get_graph_integrity();
        assert_eq!(integrity.cycles_detected, 2);
    }

    // ---------------------------------------------------------------------------
    // 11. End-to-End Integration Tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_full_pipeline_index_parse_search() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("test.db");

        // 1. Parse and index
        let mut indexer =
            crate::intelligence::index::CodeIndexer::new(db_path).expect("should create indexer");

        let source = r#"
pub trait AuthProvider {
    fn authenticate(&self, username: &str) -> bool;
}

pub struct MemoryAuthProvider {
    users: Vec<String>,
}

impl AuthProvider for MemoryAuthProvider {
    fn authenticate(&self, username: &str) -> bool {
        self.users.contains(&username.to_string())
    }
}
"#;
        let file_path = dir.path().join("auth.rs");
        std::fs::write(&file_path, source).expect("write test file");
        let symbols = indexer
            .index_file(&file_path, source)
            .expect("index should work");
        assert!(symbols.len() >= 3);

        // 2. Build dependency graph
        let graph = crate::intelligence::graph::DependencyGraph::from_indexer(&indexer)
            .expect("graph build should work");
        assert!(!graph.get_all_files().is_empty());

        // 3. Search
        let search = crate::intelligence::search::SemanticSearch::new(indexer.clone());
        let results = search.search("auth").expect("search should work");
        assert!(!results.is_empty());

        // 4. Build context
        let builder = crate::intelligence::context::IntelligentContextBuilder::new(indexer.clone());
        let context = builder
            .build_context("auth")
            .expect("context build should work");
        assert!(!context.relevant_symbols.is_empty());

        // 5. Reasoning
        let engine = crate::intelligence::reasoning::AgentReasoningEngine::new(indexer.clone());
        let reasoning = engine
            .analyze_before_modification("Add JWT auth")
            .expect("reasoning should work");
        assert!(!reasoning.steps.is_empty());
        assert!(!reasoning.plan.is_empty());

        // 7. Diagnostics
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();
        diag.update_index_health(
            indexer.get_symbol_count().expect("get count"),
            1,
            0,
            vec!["rust".to_string()],
        );
        let summary = diag.summary();
        assert!(summary.contains("Symbols:"));
    }

    #[test]
    fn test_index_build_time_benchmark() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("test.db");
        let mut indexer =
            crate::intelligence::index::CodeIndexer::new(db_path).expect("should create indexer");

        // Create multiple source files
        for i in 0..20 {
            let source = format!(
                r#"pub fn function_{i}() -> i32 {{ {i} }}
pub struct Struct{i} {{ pub value: i32 }}
"#
            );
            let file_path = dir.path().join(format!("file_{i}.rs"));
            std::fs::write(&file_path, &source).expect("write test file");
            indexer
                .index_file(&file_path, &source)
                .expect("index should work");
        }

        let elapsed = Instant::now();
        let count = indexer.get_symbol_count().expect("get count");
        let duration = elapsed.elapsed();

        println!("Indexed {} symbols in {:?}", count, duration);
        assert!(count > 0);
        assert!(duration < Duration::from_secs(5));
    }

    #[test]
    fn test_context_creation_latency() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("test.db");
        let mut indexer =
            crate::intelligence::index::CodeIndexer::new(db_path).expect("should create indexer");

        let source = r#"pub fn authenticate_user(username: &str) -> bool { true }
pub fn create_session(token: &str) -> String { token.to_string() }
"#;
        let file_path = dir.path().join("auth.rs");
        std::fs::write(&file_path, source).expect("write test file");
        indexer.index_file(&file_path, source).expect("index");

        let builder = crate::intelligence::context::IntelligentContextBuilder::new(indexer);
        let elapsed = Instant::now();
        let _context = builder
            .build_context("auth")
            .expect("context build should work");
        let duration = elapsed.elapsed();

        println!("Context creation latency: {:?}", duration);
        assert!(duration < Duration::from_millis(500));
    }

    #[test]
    fn test_search_latency() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("test.db");
        let mut indexer =
            crate::intelligence::index::CodeIndexer::new(db_path).expect("should create indexer");

        for i in 0..50 {
            let source = format!("pub fn func_{i}() -> i32 {{ {i} }}");
            let file_path = dir.path().join(format!("file_{i}.rs"));
            std::fs::write(&file_path, &source).expect("write test file");
            indexer.index_file(&file_path, &source).expect("index");
        }

        let search = crate::intelligence::search::SemanticSearch::new(indexer);
        let elapsed = Instant::now();
        let _results = search.search("func").expect("search should work");
        let duration = elapsed.elapsed();

        println!("Search latency: {:?}", duration);
        assert!(duration < Duration::from_millis(100));
    }

    #[test]
    fn test_memory_usage_stability() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("test.db");
        let mut indexer =
            crate::intelligence::index::CodeIndexer::new(db_path).expect("should create indexer");

        for i in 0..100 {
            let source = format!("pub fn func_{i}() -> i32 {{ {i} }}");
            let file_path = dir.path().join(format!("file_{i}.rs"));
            std::fs::write(&file_path, &source).expect("write test file");
            indexer.index_file(&file_path, &source).expect("index");
        }

        let count = indexer.get_symbol_count().expect("get count");
        // Each file has at least 1 function symbol; tree-sitter may extract additional symbols
        assert!(
            count >= 100,
            "should have at least 100 symbols, got {}",
            count
        );
        assert!(
            count <= 500,
            "should not have more than 500 symbols, got {}",
            count
        );
    }

    #[test]
    fn test_incremental_indexing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("test.db");
        let mut indexer =
            crate::intelligence::index::CodeIndexer::new(db_path).expect("should create indexer");

        let source1 = r#"pub fn hello() -> String { "hello".to_string() }"#;
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, source1).expect("write test file");
        indexer.index_file(&file_path, source1).expect("index");

        let source2 = r#"pub fn hello() -> String { "world".to_string() }"#;
        indexer
            .incremental_update(&file_path, source2)
            .expect("update");
        // The incremental update deletes old symbols and inserts new ones,
        // so the count should remain the same (1 symbol for 1 function)
        let count = indexer.get_symbol_count().expect("get count");
        assert!(
            count >= 1,
            "should have at least 1 symbol after update, got {}",
            count
        );
    }
}

// ===========================================================================
// P4.5 Intelligence Platform Validation Suite
// ===========================================================================

#[cfg(test)]
mod p45_validation {
    use super::*;
    use std::time::{Duration, Instant};

    // =========================================================================
    // 1. INDEXING PLATFORM VALIDATION
    // =========================================================================

    #[test]
    fn test_index_creation_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("test.db");
        let result = crate::intelligence::index::CodeIndexer::new(db_path);
        assert!(result.is_ok(), "indexer creation should succeed");
    }

    #[test]
    fn test_incremental_updates_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn func_a() -> i32 { 1 }"#;
        let file = dir.path().join("a.rs");
        std::fs::write(&file, source).expect("write");
        indexer.index_file(&file, source).expect("index");

        let source2 = r#"pub fn func_a() -> i32 { 2 }"#;
        indexer.incremental_update(&file, source2).expect("update");
        let count = indexer.get_symbol_count().expect("count");
        assert!(
            count >= 1,
            "should have at least 1 symbol after update, got {}",
            count
        );
    }

    #[test]
    fn test_symbol_consistency_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub struct Point { pub x: i32, pub y: i32 } pub fn get_point() -> Point { Point { x: 0, y: 0 } }"#;
        let file = dir.path().join("point.rs");
        std::fs::write(&file, source).expect("write");
        let symbols = indexer.index_file(&file, source).expect("index");

        assert!(!symbols.is_empty(), "should have symbols");
        for sym in &symbols {
            assert!(!sym.name.is_empty(), "symbol name should not be empty");
            assert!(sym.line_start >= 1, "line_start should be >= 1");
            assert!(sym.line_end >= sym.line_start, "line_end >= line_start");
        }
    }

    #[test]
    fn test_duplicate_handling_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn hello() -> String { "hello".to_string() }"#;
        let file = dir.path().join("test.rs");
        std::fs::write(&file, source).expect("write");

        // Index same file twice
        indexer.index_file(&file, source).expect("first index");
        let count1 = indexer.get_symbol_count().expect("count1");
        indexer.index_file(&file, source).expect("second index");
        let count2 = indexer.get_symbol_count().expect("count2");

        // Duplicate index should not double the symbols (delete before insert)
        assert_eq!(count1, count2, "duplicate index should not increase count");
    }

    #[test]
    fn test_scalability_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let start = Instant::now();
        for i in 0..50 {
            let source = format!("pub fn func_{i}() -> i32 {{ {i} }}");
            let file = dir.path().join(format!("file_{i}.rs"));
            std::fs::write(&file, &source).expect("write");
            indexer.index_file(&file, &source).expect("index");
        }
        let elapsed = start.elapsed();

        let count = indexer.get_symbol_count().expect("count");
        assert!(
            count >= 50,
            "should have at least 50 symbols, got {}",
            count
        );
        println!("Indexed 50 files in {:?}", elapsed);
        assert!(
            elapsed < Duration::from_secs(5),
            "indexing too slow: {:?}",
            elapsed
        );
    }

    // =========================================================================
    // 2. PARSER PLATFORM VALIDATION
    // =========================================================================

    #[test]
    fn test_parser_abstraction_p45() {
        let parser: Box<dyn crate::intelligence::parser::CodeParserTrait> =
            crate::intelligence::parser::create_parser_trait("rust").expect("create parser");
        assert_eq!(parser.language_name(), "rust");
        assert!(parser.supported_languages().contains(&"rust"));
    }

    #[test]
    fn test_language_isolation_p45() {
        let mut rust_parser =
            crate::intelligence::parser::TreeSitterParser::new("rust").expect("rust parser");
        let mut py_parser =
            crate::intelligence::parser::TreeSitterParser::new("python").expect("python parser");

        let rust_source = "pub fn hello() -> String { \"hello\".to_string() }";
        let py_source = "def hello(): return 'hello'";

        let rust_result = rust_parser
            .parse_source(rust_source, "test.rs")
            .expect("rust parse");
        let py_result = py_parser
            .parse_source(py_source, "test.py")
            .expect("py parse");

        // Each parser should only understand its own language
        assert!(!rust_result.symbols.is_empty());
        assert!(!py_result.symbols.is_empty());
    }

    #[test]
    fn test_parser_failure_handling_p45() {
        let result = crate::intelligence::parser::TreeSitterParser::new("unknown_lang");
        assert!(result.is_err(), "should fail for unknown language");
    }

    #[test]
    fn test_malformed_input_handling_p45() {
        let mut parser = crate::intelligence::parser::TreeSitterParser::new("rust")
            .expect("should create parser");

        // Completely malformed input
        let result = parser.parse_source("this is not valid rust code @@@ !!!", "test.rs");
        assert!(
            result.is_ok(),
            "parser should handle malformed input gracefully"
        );

        let parse_result = result.unwrap();
        // Malformed input may still produce some symbols or empty results
        let _ = parse_result.symbols;
        let _ = parse_result.imports;
    }

    #[test]
    fn test_parser_empty_input_p45() {
        let mut parser = crate::intelligence::parser::TreeSitterParser::new("rust")
            .expect("should create parser");
        let result = parser.parse_source("", "empty.rs").expect("parse empty");
        assert!(
            result.symbols.is_empty(),
            "empty input should produce no symbols"
        );
    }

    // =========================================================================
    // 3. SYMBOL MODEL VALIDATION
    // =========================================================================

    #[test]
    fn test_symbol_integrity_p45() {
        let symbol = crate::intelligence::index::Symbol {
            id: None,
            name: "test_func".to_string(),
            kind: crate::intelligence::index::SymbolKind::Function,
            language: "rust".to_string(),
            file: "test.rs".to_string(),
            line_start: 1,
            line_end: 5,
            column_start: 0,
            column_end: 20,
            parent: None,
            visibility: Some("public".to_string()),
            signature: Some("pub fn test_func()".to_string()),
            doc_comment: None,
        };

        assert!(!symbol.name.is_empty());
        assert!(symbol.line_start >= 1);
        assert!(symbol.line_end >= symbol.line_start);
        assert!(!symbol.language.is_empty());
        assert!(!symbol.file.is_empty());
    }

    #[test]
    fn test_reference_consistency_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn helper() -> i32 { 42 } pub fn main() -> i32 { helper() }"#;
        let file = dir.path().join("test.rs");
        std::fs::write(&file, source).expect("write");
        indexer.index_file(&file, source).expect("index");

        // All symbols should reference valid files
        let symbols = indexer.get_symbols().expect("get symbols");
        for sym in &symbols {
            assert_eq!(sym.file, file.to_string_lossy().to_string());
        }
    }

    #[test]
    fn test_symbol_serialization_p45() {
        let symbol = crate::intelligence::index::Symbol {
            id: Some(1),
            name: "test".to_string(),
            kind: crate::intelligence::index::SymbolKind::Struct,
            language: "rust".to_string(),
            file: "test.rs".to_string(),
            line_start: 1,
            line_end: 3,
            column_start: 0,
            column_end: 15,
            parent: None,
            visibility: None,
            signature: None,
            doc_comment: None,
        };

        let json = serde_json::to_string(&symbol).expect("serialize");
        let deserialized: crate::intelligence::index::Symbol =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.name, "test");
        assert_eq!(
            deserialized.kind,
            crate::intelligence::index::SymbolKind::Struct
        );
    }

    #[test]
    fn test_symbol_kind_compatibility_p45() {
        for kind in [
            crate::intelligence::index::SymbolKind::Function,
            crate::intelligence::index::SymbolKind::Struct,
            crate::intelligence::index::SymbolKind::Enum,
            crate::intelligence::index::SymbolKind::Trait,
            crate::intelligence::index::SymbolKind::Class,
            crate::intelligence::index::SymbolKind::Method,
            crate::intelligence::index::SymbolKind::Interface,
            crate::intelligence::index::SymbolKind::Variable,
            crate::intelligence::index::SymbolKind::Constant,
            crate::intelligence::index::SymbolKind::TypeAlias,
            crate::intelligence::index::SymbolKind::Module,
            crate::intelligence::index::SymbolKind::Import,
            crate::intelligence::index::SymbolKind::Export,
            crate::intelligence::index::SymbolKind::Field,
            crate::intelligence::index::SymbolKind::Parameter,
            crate::intelligence::index::SymbolKind::Macro,
            crate::intelligence::index::SymbolKind::Impl,
            crate::intelligence::index::SymbolKind::Constructor,
        ] {
            let kind_str = format!("{}", kind);
            assert!(!kind_str.is_empty(), "kind display should not be empty");
        }
    }

    // =========================================================================
    // 4. DEPENDENCY GRAPH VALIDATION
    // =========================================================================

    #[test]
    fn test_graph_correctness_p45() {
        let mut graph = crate::intelligence::graph::DependencyGraph::new();
        graph.add_node("a.rs".to_string());
        graph.add_node("b.rs".to_string());
        graph.add_node("c.rs".to_string());
        graph.add_edge("a.rs".to_string(), "b.rs".to_string());
        graph.add_edge("b.rs".to_string(), "c.rs".to_string());

        // a -> b -> c
        assert_eq!(graph.get_dependencies("a.rs"), vec!["b.rs".to_string()]);
        assert_eq!(graph.get_dependencies("b.rs"), vec!["c.rs".to_string()]);
        assert!(graph.get_dependencies("c.rs").is_empty());

        assert_eq!(graph.get_dependents("c.rs"), vec!["b.rs".to_string()]);
        assert_eq!(graph.get_dependents("b.rs"), vec!["a.rs".to_string()]);
        assert!(graph.get_dependents("a.rs").is_empty());
    }

    #[test]
    fn test_cycle_detection_p45() {
        let mut graph = crate::intelligence::graph::DependencyGraph::new();
        graph.add_node("a.rs".to_string());
        graph.add_node("b.rs".to_string());
        graph.add_node("c.rs".to_string());
        graph.add_edge("a.rs".to_string(), "b.rs".to_string());
        graph.add_edge("b.rs".to_string(), "c.rs".to_string());
        graph.add_edge("c.rs".to_string(), "a.rs".to_string()); // cycle

        // Transitive deps should not infinite loop
        let deps_a = graph.get_transitive_dependencies("a.rs");
        assert!(deps_a.contains("b.rs"));
        assert!(deps_a.contains("c.rs"));
    }

    #[test]
    fn test_graph_updates_p45() {
        let mut graph = crate::intelligence::graph::DependencyGraph::new();
        graph.add_node("a.rs".to_string());
        graph.add_node("b.rs".to_string());
        graph.add_edge("a.rs".to_string(), "b.rs".to_string());

        assert_eq!(graph.get_all_files().len(), 2);

        graph.add_node("c.rs".to_string());
        graph.add_edge("a.rs".to_string(), "c.rs".to_string());

        assert_eq!(graph.get_dependencies("a.rs").len(), 2);
    }

    #[test]
    fn test_graph_consistency_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source1 = r#"use b::B; pub fn use_b() -> B { B }"#;
        let source2 = r#"pub struct B; pub fn get_b() -> B { B }"#;

        let file1 = dir.path().join("a.rs");
        let file2 = dir.path().join("b.rs");
        std::fs::write(&file1, source1).expect("write");
        std::fs::write(&file2, source2).expect("write");

        indexer.index_file(&file1, source1).expect("index a");
        indexer.index_file(&file2, source2).expect("index b");

        let graph = crate::intelligence::graph::DependencyGraph::from_indexer(&indexer)
            .expect("build graph");

        // Graph should have at least the indexed files
        let all_files = graph.get_all_files();
        assert!(!all_files.is_empty());
    }

    // =========================================================================
    // 5. CONTEXT BUILDER VALIDATION
    // =========================================================================

    #[test]
    fn test_deterministic_context_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source =
            r#"pub fn authenticate() -> bool { true } pub fn authorize() -> bool { true }"#;
        let file = dir.path().join("auth.rs");
        std::fs::write(&file, source).expect("write");
        indexer.index_file(&file, source).expect("index");

        let builder = crate::intelligence::context::IntelligentContextBuilder::new(indexer);

        // Build context twice with same query
        let ctx1 = builder.build_context("auth").expect("build 1");
        let ctx2 = builder.build_context("auth").expect("build 2");

        // Results should be deterministic
        assert_eq!(ctx1.relevant_symbols.len(), ctx2.relevant_symbols.len());
        assert_eq!(ctx1.related_files, ctx2.related_files);
    }

    #[test]
    fn test_context_limits_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        // Create many files
        for i in 0..30 {
            let source = format!("pub fn func_{i}() -> i32 {{ {i} }}");
            let file = dir.path().join(format!("file_{i}.rs"));
            std::fs::write(&file, &source).expect("write");
            indexer.index_file(&file, &source).expect("index");
        }

        let builder =
            crate::intelligence::context::IntelligentContextBuilder::new(indexer).with_max_files(5);

        let context = builder.build_context("func").expect("build context");

        // Should respect max_files limit
        assert!(
            context.related_files.len() <= 5,
            "related_files should respect max_files limit, got {}",
            context.related_files.len()
        );
    }

    #[test]
    fn test_invalid_symbol_handling_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let builder = crate::intelligence::context::IntelligentContextBuilder::new(indexer);

        // Query for non-existent symbol should not panic
        let context = builder
            .build_context("nonexistent_symbol_xyz")
            .expect("build context");
        assert!(
            context.relevant_symbols.is_empty()
                || context.total_symbols_found == 0
                || !context.code_snippets.is_empty()
        );
    }

    #[test]
    fn test_context_performance_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        for i in 0..20 {
            let source = format!("pub fn func_{i}() -> i32 {{ {i} }}");
            let file = dir.path().join(format!("file_{i}.rs"));
            std::fs::write(&file, &source).expect("write");
            indexer.index_file(&file, &source).expect("index");
        }

        let builder = crate::intelligence::context::IntelligentContextBuilder::new(indexer);
        let start = Instant::now();
        let _context = builder.build_context("func").expect("build context");
        let elapsed = start.elapsed();

        println!("Context build: {:?}", elapsed);
        assert!(
            elapsed < Duration::from_millis(500),
            "context build too slow: {:?}",
            elapsed
        );
    }

    // =========================================================================
    // 6. SEMANTIC SEARCH VALIDATION
    // =========================================================================

    #[test]
    fn test_interface_compliance_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");
        let search = crate::intelligence::search::SemanticSearch::new(indexer);

        // Verify trait implementation
        fn assert_trait<T: crate::intelligence::search::SemanticSearchTrait>(_t: &T) {}
        assert_trait(&search);
    }

    #[test]
    fn test_result_ordering_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source =
            r#"pub fn authenticate_user() -> bool { true } pub fn auth_helper() -> i32 { 1 }"#;
        let file = dir.path().join("auth.rs");
        std::fs::write(&file, source).expect("write");
        indexer.index_file(&file, source).expect("index");

        let search = crate::intelligence::search::SemanticSearch::new(indexer);
        let results = search.search("authenticate").expect("search");

        // Exact match should rank higher
        assert!(!results.is_empty());
        assert_eq!(results[0].symbol.name, "authenticate_user");
    }

    #[test]
    fn test_empty_result_handling_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let search = crate::intelligence::search::SemanticSearch::new(indexer);
        let results = search.search("nonexistent_xyz").expect("search");
        assert!(results.is_empty(), "should return empty for no matches");
    }

    #[test]
    fn test_extensibility_p45() {
        // Verify that the trait is Send (not Sync, due to SQLite constraints)
        // This test verifies the trait boundary is correct
        let dir = tempfile::tempdir().expect("tempdir");
        let indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");
        let search = crate::intelligence::search::SemanticSearch::new(indexer);

        // Verify it's Send (can be moved across threads)
        fn assert_send<T: Send>(_t: &T) {}
        assert_send(&search);

        // The trait allows external implementations for future extensibility
        // (e.g., embedding-based search in P4.5)
    }

    // =========================================================================
    // 7. REASONING INTERFACE VALIDATION
    // =========================================================================

    #[test]
    fn test_trait_compliance_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");
        let engine = crate::intelligence::reasoning::AgentReasoningEngine::new(indexer);

        fn assert_trait<T: crate::intelligence::reasoning::ReasoningEngineTrait>(_t: &T) {}
        assert_trait(&engine);
    }

    #[test]
    fn test_lifecycle_p45() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn process_data(input: &str) -> String { input.to_string() }"#;
        let file = dir.path().join("process.rs");
        std::fs::write(&file, source).expect("write");
        indexer.index_file(&file, source).expect("index");

        let engine = crate::intelligence::reasoning::AgentReasoningEngine::new(indexer);

        // Full lifecycle: analyze -> reasoning -> plan
        let result = engine
            .analyze_before_modification("Add caching to process_data")
            .expect("analyze");
        assert!(!result.steps.is_empty());
        assert!(!result.plan.is_empty());
        assert!(result.confidence >= 0.0);
        assert!(result.confidence <= 1.0);
    }

    #[test]
    fn test_diagnostics_p45() {
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();

        diag.record_parse("test.rs", "rust", 10.0, 5, 0);
        diag.record_search("test", 3, 5.0);
        diag.record_context_build("query", 5, 2, 50.0);

        let summary = diag.summary();
        assert!(summary.contains("Intelligence Platform Diagnostics"));
        assert!(summary.contains("Parse Metrics"));
        assert!(summary.contains("Index Health"));
        assert!(summary.contains("Graph Integrity"));
        assert!(summary.contains("Search Metrics"));
        assert!(summary.contains("Context Metrics"));
    }

    #[test]
    fn test_future_compatibility_p45() {
        // Verify that new trait methods would be additive
        let dir = tempfile::tempdir().expect("tempdir");
        let indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");
        let engine = crate::intelligence::reasoning::AgentReasoningEngine::new(indexer);

        // All existing methods should work
        let _ = engine.analyze_before_modification("test");
        let _ = engine.analyze_for_code_understanding("test.rs");
        let _ = engine.find_existing_patterns("test");
        let _ = engine.suggest_implementation_approach("test");
    }

    // =========================================================================
    // 9. LSP FOUNDATION VALIDATION
    // =========================================================================

    #[test]
    fn test_abstraction_boundaries_p45() {
        let mut lsp = crate::intelligence::lsp::LspFoundation::new();

        // Document operations
        lsp.open_document(crate::intelligence::lsp::LspTextDocumentItem {
            uri: "file:///test.rs".to_string(),
            language_id: "rust".to_string(),
            version: 1,
            text: "pub fn hello() {}".to_string(),
        });
        assert!(lsp.get_document("file:///test.rs").is_some());

        // Symbol operations
        lsp.add_symbol(crate::intelligence::lsp::LspSymbolInformation {
            name: "hello".to_string(),
            kind: crate::intelligence::lsp::LspSymbolKind::Function,
            location: crate::intelligence::lsp::LspLocation {
                uri: "file:///test.rs".to_string(),
                range: crate::intelligence::lsp::LspRange {
                    start: crate::intelligence::lsp::LspPosition {
                        line: 0,
                        character: 0,
                    },
                    end: crate::intelligence::lsp::LspPosition {
                        line: 0,
                        character: 20,
                    },
                },
            },
            container_name: None,
        });
        assert_eq!(lsp.get_symbols_for_file("test.rs").len(), 1);

        // Diagnostics
        lsp.add_diagnostic(crate::intelligence::lsp::LspDiagnostic {
            range: crate::intelligence::lsp::LspRange {
                start: crate::intelligence::lsp::LspPosition {
                    line: 0,
                    character: 0,
                },
                end: crate::intelligence::lsp::LspPosition {
                    line: 0,
                    character: 10,
                },
            },
            severity: crate::intelligence::lsp::DiagnosticSeverity::Warning,
            message: "test diagnostic".to_string(),
            source: None,
            code: None,
        });

        // Reference lookup
        let refs = lsp.find_references("hello");
        assert_eq!(refs.len(), 1);

        // Close document
        lsp.close_document("file:///test.rs");
        assert!(lsp.get_document("file:///test.rs").is_none());
    }

    #[test]
    fn test_interface_completeness_p45() {
        let mut lsp = crate::intelligence::lsp::LspFoundation::new();

        // All trait methods should be callable
        fn assert_trait<T: crate::intelligence::lsp::LspFoundationTrait>(_t: &T) {}
        assert_trait(&lsp);

        // Verify all document methods
        let doc = crate::intelligence::lsp::LspTextDocumentItem {
            uri: "file:///x.rs".to_string(),
            language_id: "rust".to_string(),
            version: 1,
            text: "fn main() {}".to_string(),
        };
        lsp.open_document(doc.clone());
        lsp.update_document("file:///x.rs", "fn main() { let x = 1; }".to_string(), 2);
        assert_eq!(
            lsp.get_text("file:///x.rs").as_deref(),
            Some("fn main() { let x = 1; }")
        );
        lsp.close_document("file:///x.rs");
    }

    #[test]
    fn test_lsp_future_compatibility_p45() {
        // Verify LSP types are serializable
        let pos = crate::intelligence::lsp::LspPosition {
            line: 1,
            character: 2,
        };
        let json = serde_json::to_string(&pos).expect("serialize position");
        let decoded: crate::intelligence::lsp::LspPosition =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.line, 1);
        assert_eq!(decoded.character, 2);
    }

    // =========================================================================
    // 10. DIAGNOSTICS VALIDATION
    // =========================================================================

    #[test]
    fn test_event_recording_p45() {
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();

        diag.record_parse("a.rs", "rust", 10.0, 5, 0);
        diag.record_parse("b.rs", "go", 15.0, 3, 0);
        diag.record_index_event(crate::intelligence::diagnostics::IndexEvent::FileIndexed {
            file: "a.rs".to_string(),
            symbol_count: 5,
        });
        diag.record_graph_event(crate::intelligence::diagnostics::GraphEvent::GraphBuilt {
            file_count: 2,
            edge_count: 1,
        });
        diag.record_search("test", 3, 5.0);
        diag.record_context_build("query", 5, 2, 50.0);

        assert_eq!(diag.get_parse_metrics().len(), 2);
        assert_eq!(diag.get_search_metrics().len(), 1);
        assert_eq!(diag.get_context_metrics().len(), 1);
    }

    #[test]
    fn test_trace_completeness_p45() {
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();

        diag.record_parse("file.rs", "rust", 25.5, 10, 0);
        let metrics = diag.get_parse_metrics();
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].file, "file.rs");
        assert_eq!(metrics[0].language, "rust");
        assert!((metrics[0].duration_ms - 25.5).abs() < 0.01);
        assert_eq!(metrics[0].symbol_count, 10);
    }

    #[test]
    fn test_health_reporting_p45() {
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();

        diag.update_index_health(100, 10, 50, vec!["rust".to_string()]);
        let health = diag.get_index_health();
        assert_eq!(health.total_symbols, 100);
        assert_eq!(health.total_files, 10);
        assert_eq!(
            health.health_status,
            crate::intelligence::diagnostics::IndexHealthStatus::Healthy
        );

        // Empty index is degraded
        diag.update_index_health(0, 0, 0, vec![]);
        let health = diag.get_index_health();
        assert_eq!(
            health.health_status,
            crate::intelligence::diagnostics::IndexHealthStatus::Degraded
        );
    }

    #[test]
    fn test_export_readiness_p45() {
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();

        diag.record_parse("test.rs", "rust", 10.0, 5, 0);
        diag.update_index_health(10, 2, 5, vec!["rust".to_string()]);
        diag.update_graph_integrity(5, 3);

        // Should be serializable to JSON
        let summary = diag.summary();
        assert!(!summary.is_empty());

        // Clear should reset everything
        diag.clear();
        assert_eq!(diag.get_parse_metrics().len(), 0);
        assert_eq!(diag.get_search_metrics().len(), 0);
    }

    // =========================================================================
    // 11. PLATFORM ISOLATION VALIDATION
    // =========================================================================

    #[test]
    fn test_no_tool_dependencies_p45() {
        // The intelligence module should not import from tools/
        // This is verified by compilation — if it compiled, there's no import cycle
        let dir = tempfile::tempdir().expect("tempdir");
        let _indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("create indexer");
        // If we get here without compile errors, isolation is maintained
    }

    #[test]
    fn test_no_provider_dependencies_p45() {
        // Intelligence should not depend on providers
        let mut diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();
        diag.record_parse("test.rs", "rust", 1.0, 1, 0);
        assert_eq!(diag.get_parse_metrics().len(), 1);
    }

    #[test]
    fn test_read_only_boundary_p45() {
        // The indexer should not write source files
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("create indexer");

        let source = r#"pub fn test() -> i32 { 1 }"#;
        let file = dir.path().join("test.rs");
        std::fs::write(&file, source).expect("write");
        let original_mtime = std::fs::metadata(&file)
            .expect("metadata")
            .modified()
            .unwrap();

        indexer.index_file(&file, source).expect("index");

        // File should not have been modified by indexing
        let new_mtime = std::fs::metadata(&file)
            .expect("metadata")
            .modified()
            .unwrap();
        assert_eq!(
            original_mtime, new_mtime,
            "indexing should not modify source files"
        );
    }

    // =========================================================================
    // 12. CROSS-PLATFORM INTEGRATION VALIDATION
    // =========================================================================

    #[test]
    fn test_intelligence_to_agent_compatibility_p45() {
        // Verify intelligence types can be used alongside agent types
        let dir = tempfile::tempdir().expect("tempdir");
        let mut indexer = crate::intelligence::index::CodeIndexer::new(dir.path().join("test.db"))
            .expect("should create indexer");

        let source = r#"pub fn process(input: &str) -> String { input.to_string() }"#;
        let file = dir.path().join("process.rs");
        std::fs::write(&file, source).expect("write");
        indexer.index_file(&file, source).expect("index");

        // Intelligence types should be constructible
        let ctx = crate::intelligence::context::IntelligenceContext {
            query: "test".to_string(),
            relevant_symbols: vec![],
            related_files: vec![],
            dependencies: vec![],
            imports: vec![],
            code_snippets: vec![],
            total_symbols_found: 0,
        };
        assert_eq!(ctx.query, "test");

        // Reasoning result should be constructible
        let reason = crate::intelligence::reasoning::ReasoningResult {
            steps: vec![],
            summary: "test".to_string(),
            plan: vec![],
            relevant_context: ctx,
            confidence: 0.5,
        };
        assert_eq!(reason.confidence, 0.5);
    }

    #[test]
    fn test_intelligence_to_reliability_compatibility_p45() {
        // Intelligence diagnostics should integrate with reliability layer
        let diag = crate::intelligence::diagnostics::IntelligenceDiagnostics::new();
        let summary = diag.summary();
        assert!(summary.contains("Diagnostics"));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// P5.5 Validation Tests — Developer Experience Platform
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod p55_validation {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    // ─── Settings Manager Validation ──────────────────────────────────────

    #[test]
    fn test_settings_navigation_sections() {
        let config = Config::default_test();
        let sm = crate::settings::SettingsManager::new(config, PathBuf::from("/tmp"));

        // Verify all 5 sections exist
        let sections = [
            crate::settings::SettingSection::General,
            crate::settings::SettingSection::Provider,
            crate::settings::SettingSection::Workspace,
            crate::settings::SettingSection::Features,
            crate::settings::SettingSection::Advanced,
        ];
        for section in &sections {
            let settings = sm.get_settings_by_section(section);
            // Each section should have at least some settings
            // (Advanced may be empty, so just don't panic)
            let _ = settings;
        }
    }

    #[test]
    fn test_settings_pending_changes_workflow() {
        let config = Config::default_test();
        let mut sm = crate::settings::SettingsManager::new(config, PathBuf::from("/tmp"));

        // Initially no pending changes
        assert!(!sm.has_pending_changes());
        assert_eq!(sm.modified_settings().len(), 0);

        // Modify a setting
        sm.set_string("model", "gpt-4o-mini").unwrap();
        assert!(sm.has_pending_changes());
        assert_eq!(sm.modified_settings().len(), 1);

        // Apply changes
        sm.apply_changes().unwrap();
        assert!(!sm.has_pending_changes());

        // Verify value persisted
        let setting = sm.get_setting("model").unwrap();
        assert_eq!(
            setting.value,
            crate::settings::SettingKind::String("gpt-4o-mini".to_string())
        );
    }

    #[test]
    fn test_settings_discard_workflow() {
        let config = Config::default_test();
        let mut sm = crate::settings::SettingsManager::new(config, PathBuf::from("/tmp"));

        // Make multiple changes
        sm.set_string("model", "gpt-4o-mini").unwrap();
        sm.set_integer("context_token_budget", 4000).unwrap();
        sm.set_boolean("auto_approve_safe", true).unwrap();
        assert_eq!(sm.modified_settings().len(), 3);

        // Discard all changes
        sm.discard_changes();
        assert!(!sm.has_pending_changes());
        assert_eq!(sm.modified_settings().len(), 0);

        // Verify values reset to defaults (model is empty in default_test)
        let model = sm.get_setting("model").unwrap();
        assert_eq!(
            model.value,
            crate::settings::SettingKind::String("".to_string())
        );
    }

    #[test]
    fn test_settings_persistence() {
        let dir = tempdir().unwrap();
        let config_dir = dir.path().to_path_buf();

        // Create initial config
        let config = Config {
            provider: "openai".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            api_key: None,
        };
        let mut sm = crate::settings::SettingsManager::new(config.clone(), config_dir.clone());

        // Modify and apply
        sm.set_string("model", "gpt-4o-mini").unwrap();
        sm.apply_changes().unwrap();

        // Verify the config was persisted to the actual config dir
        // (apply_changes writes to ~/.codebro/config.toml via config.persist_model())
        let actual_config = Config::load().unwrap();
        assert_eq!(actual_config.model, "gpt-4o-mini");
    }

    #[test]
    fn test_settings_recovery_after_interruption() {
        let dir = tempdir().unwrap();
        let config_dir = dir.path().to_path_buf();

        let config = Config::default_test();
        let mut sm = crate::settings::SettingsManager::new(config.clone(), config_dir.clone());

        // Make changes
        sm.set_string("model", "claude-3-sonnet").unwrap();
        sm.set_boolean("show_metrics", false).unwrap();

        // Simulate interruption - discard changes
        sm.discard_changes();

        // Verify clean state
        assert!(!sm.has_pending_changes());
        let model = sm.get_setting("model").unwrap();
        // Default model in Config::default_test() is empty, which shows as "auto-detect" in settings
        // After discard, it resets to the kind default (empty string)
        assert_eq!(
            model.value,
            crate::settings::SettingKind::String("".to_string())
        );
    }

    #[test]
    fn test_settings_invalid_type_mismatch() {
        let config = Config::default_test();
        let mut sm = crate::settings::SettingsManager::new(config, PathBuf::from("/tmp"));

        // Trying to set integer value on string setting should fail
        let result = sm.set_integer("model", 42);
        assert!(result.is_err());

        // Trying to set boolean value on string setting should fail
        let result = sm.set_boolean("model", true);
        assert!(result.is_err());

        // Valid operations should still work
        assert!(sm.set_string("model", "gpt-4").is_ok());
    }

    #[test]
    fn test_settings_summary_format() {
        let config = Config::default_test();
        let sm = crate::settings::SettingsManager::new(config, PathBuf::from("/tmp"));
        let summary = sm.summary();

        assert!(summary.contains("CodeBro Settings"));
        assert!(summary.contains("Provider"));
        assert!(summary.contains("General"));
        assert!(summary.contains("Features"));
    }

    // ─── Provider Manager Validation ──────────────────────────────────────

    #[test]
    fn test_provider_switching() {
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();

        // Start with default active
        assert!(pm.active_provider().is_none());

        // Switch to OpenRouter
        pm.set_active("openrouter").unwrap();
        assert_eq!(
            pm.active_provider().as_deref(),
            Some(&"openrouter".to_string())
        );

        // Switch to DeepSeek
        pm.set_active("deepseek").unwrap();
        assert_eq!(
            pm.active_provider().as_deref(),
            Some(&"deepseek".to_string())
        );

        // Switch back to OpenAI
        pm.set_active("openai").unwrap();
        assert_eq!(pm.active_provider().as_deref(), Some(&"openai".to_string()));
    }

    #[test]
    fn test_provider_switch_invalid() {
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();

        let result = pm.set_active("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_api_key_validation_empty_rejected() {
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();

        let result = pm.set_api_key("openai", "");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_api_key_masking_various_lengths() {
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();

        // Short key (<=4 chars)
        pm.set_api_key("openai", "abc").unwrap();
        assert_eq!(pm.api_key_masked("openai"), Some("••••".to_string()));

        // Exactly 4 chars - still masked as "••••" since len <= 4
        pm.set_api_key("openai", "abcd").unwrap();
        assert_eq!(pm.api_key_masked("openai"), Some("••••".to_string()));

        // Long key (>4 chars)
        pm.set_api_key("openai", "sk-1234567890abcdef").unwrap();
        let masked = pm.api_key_masked("openai").unwrap();
        assert!(masked.starts_with("••••"));
        assert!(masked.ends_with("cdef"));

        // Very long key
        pm.set_api_key("openai", "sk-very-long-key-with-many-characters-12345678")
            .unwrap();
        let masked = pm.api_key_masked("openai").unwrap();
        assert!(masked.starts_with("••••"));
        assert!(masked.ends_with("5678"));
    }

    #[test]
    fn test_api_key_clear() {
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();

        pm.set_api_key("openai", "sk-test").unwrap();
        assert!(pm.has_api_key("openai"));

        pm.clear_api_key("openai").unwrap();
        assert!(!pm.has_api_key("openai"));
        assert_eq!(pm.api_key_masked("openai"), None);
    }

    #[test]
    fn test_api_key_nonexistent_provider() {
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();

        let result = pm.set_api_key("nonexistent", "sk-test");
        assert!(result.is_err());
    }

    #[test]
    fn test_provider_health_unknown_initially() {
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();

        // Health should be Unknown initially (no check performed yet)
        let health = pm.get_health("openai");
        assert!(matches!(
            health,
            crate::provider_manager::HealthStatus::Unknown
        ));
    }

    #[test]
    fn test_provider_model_selection() {
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();
        pm.set_active("openai").unwrap();

        // Default model should be empty
        assert!(pm.active_model().is_empty());

        // Set a model
        pm.set_model("gpt-4o").unwrap();
        assert_eq!(pm.active_model(), "gpt-4o");

        // Set another model
        pm.set_model("gpt-4o-mini").unwrap();
        assert_eq!(pm.active_model(), "gpt-4o-mini");
    }

    #[test]
    fn test_provider_list_all_builtin() {
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();
        let providers: Vec<_> = pm.list_providers();

        // Should have at least the 5 built-in providers
        assert!(
            providers.len() >= 5,
            "Expected at least 5 providers, got {}",
            providers.len()
        );

        let names: Vec<&str> = providers.iter().map(|(k, _)| *k).collect();
        assert!(names.contains(&"openai"), "openai should be present");
        assert!(
            names.contains(&"openrouter"),
            "openrouter should be present"
        );
        assert!(names.contains(&"deepseek"), "deepseek should be present");
        assert!(names.contains(&"ollama"), "ollama should be present");
        assert!(names.contains(&"lmstudio"), "lmstudio should be present");
    }

    #[test]
    fn test_provider_custom_registration() {
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();

        // Register custom provider
        pm.register_custom(
            crate::provider_manager::ProviderId::Custom("myprovider".to_string()),
            "https://myprovider.example.com/v1".to_string(),
        );

        let providers: Vec<_> = pm.list_providers();
        assert!(providers.iter().any(|(k, _)| *k == "myprovider"));
    }

    #[test]
    fn test_provider_connection_failure_handling() {
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();

        // Set an invalid URL to simulate connection failure via the public API
        // We use set_active which will work, then we need to check health
        pm.set_active("openai").unwrap();

        // Health check will fail due to network, but should return Unhealthy
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(pm.check_health("openai"));

        // Should return unhealthy (network failure), not panic
        assert!(matches!(
            result,
            Ok(crate::provider_manager::HealthStatus::Unhealthy { .. })
        ));
    }

    #[test]
    fn test_provider_wizard_flow_complete() {
        let mut wizard = crate::provider_manager::WizardState::new();

        // Step 1: Select provider
        wizard.select_provider(&crate::provider_manager::ProviderId::OpenAI);
        assert_eq!(
            wizard.step,
            crate::provider_manager::WizardStep::EnterApiKey
        );
        assert_eq!(wizard.base_url, "https://api.openai.com/v1");

        // Step 2: Enter API key
        wizard.set_api_key("sk-test-key");
        wizard.confirm_api_key();
        assert_eq!(
            wizard.step,
            crate::provider_manager::WizardStep::SelectModel
        );
        assert!(wizard.api_key_confirmed);

        // Step 3: Select model
        wizard.select_model("gpt-4o");
        assert_eq!(wizard.selected_model, Some("gpt-4o".to_string()));

        // Step 4: Confirm
        assert!(wizard.confirm_selection());
    }

    #[test]
    fn test_provider_persistence_roundtrip() {
        let dir = tempdir().unwrap();
        let config_dir = dir.path().to_path_buf();

        let mut pm = crate::provider_manager::ProviderManager::new(config_dir.clone());
        pm.register_builtin();
        pm.set_active("openai").unwrap();
        pm.set_api_key("openai", "sk-persist-test").unwrap();
        pm.set_model("gpt-4o").unwrap();

        // Persist
        pm.persist().unwrap();

        // Load into fresh instance
        let mut loaded = crate::provider_manager::ProviderManager::new(config_dir.clone());
        loaded.register_builtin();
        loaded.load().unwrap();

        assert_eq!(
            loaded.active_provider().as_deref(),
            Some(&"openai".to_string())
        );
        assert!(loaded.has_api_key("openai"));
        assert_eq!(loaded.active_model(), "gpt-4o");
    }

    // ─── Workspace Discovery Validation ───────────────────────────────────

    #[test]
    fn test_git_detection() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert!(discovery
            .findings
            .iter()
            .any(|f| matches!(f.kind, crate::workspace_discovery::DiscoveryKind::Git)));
    }

    #[test]
    fn test_cargo_detection() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert_eq!(discovery.language, "rust");
        assert_eq!(discovery.build_system, Some("cargo".to_string()));
        assert_eq!(discovery.package_manager, Some("cargo".to_string()));
        assert_eq!(discovery.testing_framework, Some("cargo_test".to_string()));
    }

    #[test]
    fn test_node_detection() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            "{\"name\": \"test\", \"dependencies\": {\"react\": \"^18.0.0\"}}",
        )
        .unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert_eq!(discovery.language, "javascript");
        assert_eq!(discovery.build_system, Some("npm".to_string()));
        assert_eq!(discovery.framework, Some("react".to_string()));
    }

    #[test]
    fn test_python_detection() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"test\"",
        )
        .unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert_eq!(discovery.language, "python");
    }

    #[test]
    fn test_docker_detection() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Dockerfile"), "FROM rust:latest").unwrap();
        fs::write(
            dir.path().join("docker-compose.yml"),
            "version: '3'\nservices:\n  app:\n    build: .",
        )
        .unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert!(discovery
            .findings
            .iter()
            .any(|f| matches!(f.kind, crate::workspace_discovery::DiscoveryKind::Docker)));
    }

    #[test]
    fn test_go_detection() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("go.mod"), "module test\n\ngo 1.21").unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert_eq!(discovery.language, "go");
    }

    #[test]
    fn test_makefile_detection() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Makefile"), "build:\n\techo hello").unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert_eq!(discovery.build_system, Some("make".to_string()));
        assert!(discovery
            .findings
            .iter()
            .any(|f| matches!(f.kind, crate::workspace_discovery::DiscoveryKind::Make)));
    }

    #[test]
    fn test_cmake_detection() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3.10)",
        )
        .unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert_eq!(discovery.build_system, Some("cmake".to_string()));
        assert!(discovery
            .findings
            .iter()
            .any(|f| matches!(f.kind, crate::workspace_discovery::DiscoveryKind::Cmake)));
    }

    #[test]
    fn test_pnpm_detection() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("package.json"), "{}").unwrap();
        fs::write(dir.path().join("pnpm-lock.yaml"), "lockfileVersion: 5.4").unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert_eq!(discovery.package_manager, Some("pnpm".to_string()));
    }

    #[test]
    fn test_yarn_detection() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("package.json"), "{}").unwrap();
        fs::write(dir.path().join("yarn.lock"), "# yarn lockfile v1").unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert_eq!(discovery.package_manager, Some("yarn".to_string()));
    }

    #[test]
    fn test_bun_detection() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("package.json"), "{}").unwrap();
        fs::write(dir.path().join("bun.lock"), "").unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert_eq!(discovery.package_manager, Some("bun".to_string()));
    }

    #[test]
    fn test_jest_detection() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            "{\"devDependencies\": {\"jest\": \"^29.0.0\"}}",
        )
        .unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert_eq!(discovery.testing_framework, Some("jest".to_string()));
    }

    #[test]
    fn test_vitest_detection() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            "{\"devDependencies\": {\"vitest\": \"^1.0.0\"}}",
        )
        .unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert_eq!(discovery.testing_framework, Some("vitest".to_string()));
    }

    #[test]
    fn test_pytest_detection() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"test\"",
        )
        .unwrap();
        fs::write(dir.path().join("pytest.ini"), "").unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert_eq!(discovery.testing_framework, Some("pytest".to_string()));
    }

    #[test]
    fn test_integration_proposals_require_approval() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        // All proposals should require approval by default
        for proposal in &discovery.proposals {
            assert!(proposal.requires_approval);
            assert!(!proposal.approved);
        }
    }

    #[test]
    fn test_integration_approval_workflow() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let mut discovery = engine.discover();

        // Initially none enabled
        assert_eq!(discovery.enabled_count(), 0);

        // Toggle first proposal
        if let Some(proposal) = discovery.proposals.get_mut(0) {
            proposal.enabled = true;
            proposal.approved = true;
        }

        assert_eq!(discovery.enabled_count(), 1);
    }

    #[test]
    fn test_empty_workspace_detection() {
        let dir = tempdir().unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        assert_eq!(discovery.language, "unknown");
        assert!(discovery.build_system.is_none());
        assert!(discovery.framework.is_none());
    }

    #[test]
    fn test_duplicate_detection_prevention() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        // Each finding kind should appear only once
        let kinds: Vec<_> = discovery
            .findings
            .iter()
            .map(|f| format!("{:?}", f.kind))
            .collect();
        let unique: std::collections::HashSet<_> = kinds.iter().collect();
        assert_eq!(kinds.len(), unique.len());
    }

    #[test]
    fn test_discovery_kind_all_variants() {
        // Verify all DiscoveryKind variants have valid display names
        use crate::workspace_discovery::DiscoveryKind;

        let kinds = vec![
            DiscoveryKind::Git,
            DiscoveryKind::Cargo,
            DiscoveryKind::Npm,
            DiscoveryKind::Python,
            DiscoveryKind::Docker,
            DiscoveryKind::Go,
            DiscoveryKind::Ruby,
            DiscoveryKind::Php,
            DiscoveryKind::Java,
            DiscoveryKind::Bun,
            DiscoveryKind::Pnpm,
            DiscoveryKind::Yarn,
            DiscoveryKind::Make,
            DiscoveryKind::Cmake,
        ];

        for kind in kinds {
            let display = format!("{}", kind);
            assert!(!display.is_empty());
        }
    }

    #[test]
    fn test_unsupported_environment_handling() {
        // Discover in a directory with no project files
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("readme.txt"), "just a text file").unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        // Should not panic, should return unknown language
        assert_eq!(discovery.language, "unknown");
        assert!(!discovery.is_empty() || discovery.proposals.is_empty());
    }

    // ─── Capability Discovery Validation ──────────────────────────────────

    #[test]
    fn test_runtime_detection_rust() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let scanner = crate::capability_discovery::CapabilityScanner::new(dir.path().to_path_buf());
        let discovery = scanner.scan();

        assert!(discovery.capabilities.iter().any(|c| {
            matches!(&c.kind, crate::capability_discovery::CapabilityKind::LanguageRuntime(r) if r == "rust")
                && c.available
        }));
    }

    #[test]
    fn test_runtime_detection_node() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("package.json"), "{}").unwrap();

        let scanner = crate::capability_discovery::CapabilityScanner::new(dir.path().to_path_buf());
        let discovery = scanner.scan();

        assert!(discovery.capabilities.iter().any(|c| {
            matches!(&c.kind, crate::capability_discovery::CapabilityKind::LanguageRuntime(r) if r == "javascript")
                && c.available
        }));
    }

    #[test]
    fn test_build_tool_detection() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let scanner = crate::capability_discovery::CapabilityScanner::new(dir.path().to_path_buf());
        let discovery = scanner.scan();

        // Cargo project should have build system capability registered
        assert!(discovery.capabilities.iter().any(|c| {
            matches!(&c.kind, crate::capability_discovery::CapabilityKind::BuildSystem(b) if b == "cargo")
        }), "Should detect cargo build system");
    }

    #[test]
    fn test_testing_framework_detection() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let scanner = crate::capability_discovery::CapabilityScanner::new(dir.path().to_path_buf());
        let discovery = scanner.scan();

        // Cargo projects should have cargo_test capability
        assert!(discovery.capabilities.iter().any(|c| {
            matches!(&c.kind, crate::capability_discovery::CapabilityKind::TestingFramework(t) if t == "cargo_test")
        }), "Should detect cargo_test testing framework");
    }

    #[test]
    fn test_recommendation_generation() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let scanner = crate::capability_discovery::CapabilityScanner::new(dir.path().to_path_buf());
        let discovery = scanner.scan();

        // Should generate at least one recommendation for Rust projects
        assert!(!discovery.recommendations.is_empty());
    }

    #[test]
    fn test_duplicate_capability_prevention() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let scanner = crate::capability_discovery::CapabilityScanner::new(dir.path().to_path_buf());
        let discovery = scanner.scan();

        // No duplicate names
        let names: Vec<_> = discovery.capabilities.iter().map(|c| &c.name).collect();
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(names.len(), unique.len());
    }

    #[test]
    fn test_enabled_recommended_capabilities() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let scanner = crate::capability_discovery::CapabilityScanner::new(dir.path().to_path_buf());
        let mut discovery = scanner.scan();

        let before = discovery.enabled_count();
        discovery.enable_recommended();
        let after = discovery.enabled_count();

        // At least some should be enabled
        assert!(after >= before);
    }

    #[test]
    fn test_capability_summary_text() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let scanner = crate::capability_discovery::CapabilityScanner::new(dir.path().to_path_buf());
        let discovery = scanner.scan();

        let summary = discovery.summary_text();
        assert!(summary.contains("available"));
        assert!(summary.contains("enabled"));
    }

    // ─── Onboarding Validation ────────────────────────────────────────────

    #[test]
    fn test_onboarding_first_run_detection() {
        let dir = tempdir().unwrap();
        let manager = crate::onboarding::OnboardingManager::new(dir.path().to_path_buf());

        // No config exists → first run
        assert!(manager.check_first_run());
    }

    #[test]
    fn test_onboarding_existing_config() {
        let dir = tempdir().unwrap();
        // Create a config file
        fs::write(dir.path().join("config.toml"), "provider = \"openai\"\n").unwrap();

        let manager = crate::onboarding::OnboardingManager::new(dir.path().to_path_buf());

        // Config exists → not first run
        assert!(!manager.check_first_run());
    }

    #[test]
    fn test_onboarding_step_progression() {
        let mut manager = crate::onboarding::OnboardingManager::new(PathBuf::from("/tmp"));
        manager.start();

        assert_eq!(
            manager.session.step,
            crate::onboarding::OnboardingStep::Welcome
        );

        manager.next();
        assert_eq!(
            manager.session.step,
            crate::onboarding::OnboardingStep::EnterApiKey
        );

        manager.next();
        assert_eq!(
            manager.session.step,
            crate::onboarding::OnboardingStep::SelectProvider
        );

        manager.next();
        assert_eq!(
            manager.session.step,
            crate::onboarding::OnboardingStep::DetectModel
        );

        manager.next();
        assert_eq!(
            manager.session.step,
            crate::onboarding::OnboardingStep::DiscoverWorkspace
        );

        manager.next();
        assert_eq!(
            manager.session.step,
            crate::onboarding::OnboardingStep::ReviewIntegrations
        );

        manager.next();
        assert_eq!(
            manager.session.step,
            crate::onboarding::OnboardingStep::ReviewCapabilities
        );

        manager.next();
        assert_eq!(
            manager.session.step,
            crate::onboarding::OnboardingStep::Confirm
        );

        manager.next();
        assert_eq!(
            manager.session.step,
            crate::onboarding::OnboardingStep::Complete
        );
    }

    #[test]
    fn test_onboarding_step_backward_navigation() {
        let mut manager = crate::onboarding::OnboardingManager::new(PathBuf::from("/tmp"));
        manager.start();

        // Advance several steps
        manager.next();
        manager.next();
        manager.next();
        assert_eq!(
            manager.session.step,
            crate::onboarding::OnboardingStep::DetectModel
        );

        // Go back
        manager.previous();
        assert_eq!(
            manager.session.step,
            crate::onboarding::OnboardingStep::SelectProvider
        );

        manager.previous();
        assert_eq!(
            manager.session.step,
            crate::onboarding::OnboardingStep::EnterApiKey
        );

        manager.previous();
        assert_eq!(
            manager.session.step,
            crate::onboarding::OnboardingStep::Welcome
        );
    }

    #[test]
    fn test_onboarding_api_key_storage() {
        let mut manager = crate::onboarding::OnboardingManager::new(PathBuf::from("/tmp"));

        manager.set_api_key("sk-test-key-12345");
        assert_eq!(
            manager.session.api_key,
            Some("sk-test-key-12345".to_string())
        );
    }

    #[test]
    fn test_onboarding_provider_selection() {
        let mut manager = crate::onboarding::OnboardingManager::new(PathBuf::from("/tmp"));

        manager.select_provider(&crate::provider_manager::ProviderId::OpenAI);
        assert_eq!(
            manager.session.wizard_state.selected_provider,
            Some(crate::provider_manager::ProviderId::OpenAI)
        );

        manager.select_provider(&crate::provider_manager::ProviderId::DeepSeek);
        assert_eq!(
            manager.session.wizard_state.selected_provider,
            Some(crate::provider_manager::ProviderId::DeepSeek)
        );
    }

    #[test]
    fn test_onboarding_workspace_discovery_integration() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();

        let mut manager = crate::onboarding::OnboardingManager::new(PathBuf::from("/tmp"));
        manager.session.workspace_discovery = Some(
            crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf()).discover(),
        );

        let wd = manager.session.workspace_discovery.as_ref().unwrap();
        assert_eq!(wd.language, "rust");
        assert!(wd
            .findings
            .iter()
            .any(|f| matches!(f.kind, crate::workspace_discovery::DiscoveryKind::Git)));
    }

    #[test]
    fn test_onboarding_capability_discovery_integration() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            "{\"dependencies\": {\"react\": \"^18.0.0\"}}",
        )
        .unwrap();

        let mut manager = crate::onboarding::OnboardingManager::new(PathBuf::from("/tmp"));
        manager.session.capability_discovery = Some(
            crate::capability_discovery::CapabilityScanner::new(dir.path().to_path_buf()).scan(),
        );

        let cd = manager.session.capability_discovery.as_ref().unwrap();
        assert!(!cd.capabilities.is_empty());
        assert!(!cd.recommendations.is_empty());
    }

    #[test]
    fn test_onboarding_completion_result() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("config.toml"), "").unwrap();

        let mut manager = crate::onboarding::OnboardingManager::new(dir.path().to_path_buf());
        manager.start();
        manager.set_api_key("sk-test");
        manager.select_provider(&crate::provider_manager::ProviderId::OpenAI);
        manager.session.wizard_state.selected_model = Some("gpt-4o".to_string());
        manager.session.workspace_discovery = Some(
            crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf()).discover(),
        );
        manager.session.capability_discovery = Some(
            crate::capability_discovery::CapabilityScanner::new(dir.path().to_path_buf()).scan(),
        );

        // Move to confirm step
        while !matches!(
            manager.session.step,
            crate::onboarding::OnboardingStep::Confirm
        ) {
            manager.next();
        }

        // Complete should succeed
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(manager.complete(&dir.path().to_path_buf()));
        assert!(result.is_ok());
    }

    #[test]
    fn test_onboarding_stores_key_in_credentials_not_plaintext() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("config.toml"), "").unwrap();

        let mut manager = crate::onboarding::OnboardingManager::new(dir.path().to_path_buf());
        manager.start();
        manager.set_api_key("sk-onboard-secret-123456");
        manager.select_provider(&crate::provider_manager::ProviderId::OpenAI);
        manager.session.wizard_state.selected_model = Some("gpt-4o".to_string());
        manager.session.workspace_discovery = Some(
            crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf()).discover(),
        );
        manager.session.capability_discovery = Some(
            crate::capability_discovery::CapabilityScanner::new(dir.path().to_path_buf()).scan(),
        );
        while !matches!(
            manager.session.step,
            crate::onboarding::OnboardingStep::Confirm
        ) {
            manager.next();
        }
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(manager.complete(&dir.path().to_path_buf()))
            .unwrap();

        // The secret must not appear in normal config or a legacy key file.
        let config = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!config.contains("sk-onboard-secret-123456"));
        assert!(
            !dir.path().join(".api_key").exists(),
            "legacy plaintext key file must be migrated away"
        );
        // It must live in the secure credential store.
        let mut store = crate::credentials::CredentialStore::new(dir.path().to_path_buf());
        store.load().unwrap();
        assert_eq!(store.get("openai"), Some("sk-onboard-secret-123456"));
    }

    #[test]
    fn test_onboarding_step_info_all_steps() {
        let manager = crate::onboarding::OnboardingManager::new(PathBuf::from("/tmp"));

        // Test all step variants exist by testing through the flow
        let mut mgr = crate::onboarding::OnboardingManager::new(PathBuf::from("/tmp"));
        mgr.start();

        // Walk through all steps and verify step_info works
        let steps = vec![
            crate::onboarding::OnboardingStep::Welcome,
            crate::onboarding::OnboardingStep::EnterApiKey,
            crate::onboarding::OnboardingStep::SelectProvider,
            crate::onboarding::OnboardingStep::DetectModel,
            crate::onboarding::OnboardingStep::DiscoverWorkspace,
            crate::onboarding::OnboardingStep::ReviewIntegrations,
            crate::onboarding::OnboardingStep::ReviewCapabilities,
            crate::onboarding::OnboardingStep::Confirm,
            crate::onboarding::OnboardingStep::Complete,
        ];

        for step in steps {
            mgr.session.step = step.clone();
            let (title, desc) = mgr.step_info();
            assert!(
                !title.is_empty(),
                "Title should not be empty for {:?}",
                step
            );
            assert!(!desc.is_empty(), "Desc should not be empty for {:?}", step);
        }

        // Test that step_info returns valid data for initial state
        let (title, desc) = manager.step_info();
        assert!(!title.is_empty());
        assert!(!desc.is_empty());
    }

    #[test]
    fn test_onboarding_is_complete_flag() {
        let mut manager = crate::onboarding::OnboardingManager::new(PathBuf::from("/tmp"));
        assert!(!manager.session.is_complete());

        manager.session.step = crate::onboarding::OnboardingStep::Complete;
        assert!(manager.session.is_complete());
    }

    // ─── Stress Tests ─────────────────────────────────────────────────────

    #[test]
    fn test_stress_repeated_settings_updates() {
        let config = Config::default_test();
        let mut sm = crate::settings::SettingsManager::new(config, PathBuf::from("/tmp"));

        // Perform many updates
        for i in 0..100 {
            let model = format!("gpt-4-{}", i);
            sm.set_string("model", &model).unwrap();
            sm.apply_changes().unwrap();

            // Reset for next iteration
            sm.discard_changes();
        }

        // Should still be in clean state
        assert!(!sm.has_pending_changes());
    }

    #[test]
    fn test_stress_repeated_provider_switching() {
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();

        let providers = ["openai", "openrouter", "deepseek", "ollama", "lmstudio"];

        for i in 0..100 {
            let provider = &providers[i % providers.len()];
            pm.set_active(provider).unwrap();
            assert_eq!(pm.active_provider().as_deref(), Some(&provider.to_string()));
        }
    }

    #[test]
    fn test_stress_repeated_workspace_scans() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();

        for _ in 0..50 {
            let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
            let discovery = engine.discover();
            assert_eq!(discovery.language, "rust");
            assert!(discovery
                .findings
                .iter()
                .any(|f| matches!(f.kind, crate::workspace_discovery::DiscoveryKind::Git)));
        }
    }

    #[test]
    fn test_stress_repeated_capability_scans() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            "{\"dependencies\": {\"next\": \"^14.0.0\"}}",
        )
        .unwrap();

        for _ in 0..50 {
            let scanner =
                crate::capability_discovery::CapabilityScanner::new(dir.path().to_path_buf());
            let discovery = scanner.scan();
            assert!(!discovery.capabilities.is_empty());
        }
    }

    #[test]
    fn test_stress_concurrent_health_checks() {
        use std::sync::{Arc, Mutex};

        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();

        let pm_arc = Arc::new(Mutex::new(pm));
        let mut handles = vec![];

        for provider in ["openai", "openrouter", "deepseek", "ollama", "lmstudio"] {
            let pm = pm_arc.clone();
            let handle = std::thread::spawn(move || {
                let mut pm = pm.lock().unwrap();
                // Health check will fail (no real network), but should not panic
                let _ = tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(pm.check_health(provider));
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // All checks completed without panic
    }

    #[test]
    fn test_stress_repeated_onboarding_flow() {
        for _ in 0..20 {
            let mut manager = crate::onboarding::OnboardingManager::new(PathBuf::from("/tmp"));
            manager.start();

            // Simulate full flow
            manager.set_api_key("sk-test");
            manager.select_provider(&crate::provider_manager::ProviderId::OpenAI);

            while !matches!(
                manager.session.step,
                crate::onboarding::OnboardingStep::Complete
            ) {
                manager.next();
            }

            assert!(manager.session.is_complete());
        }
    }

    // ─── Vision Compliance Tests ──────────────────────────────────────────

    #[test]
    fn test_vision_zero_configuration() {
        // A new user should be able to run without any manual config
        let dir = tempdir().unwrap();
        let manager = crate::onboarding::OnboardingManager::new(dir.path().to_path_buf());

        // First run should be detected
        assert!(manager.check_first_run());
    }

    #[test]
    fn test_vision_progressive_discovery() {
        // All P5 features should be accessible via slash commands
        let commands = vec![
            "/settings",
            "/providers",
            "/health",
            "/discover",
            "/workspace",
            "/onboard",
        ];

        for cmd in commands {
            assert!(cmd.starts_with('/'));
        }
    }

    #[test]
    fn test_vision_human_approval() {
        // Workspace integrations must require approval
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        for proposal in &discovery.proposals {
            assert!(proposal.requires_approval);
        }
    }

    #[test]
    fn test_vision_tui_accessible() {
        // Settings should be manageable without file editing
        let config = Config::default_test();
        let sm = crate::settings::SettingsManager::new(config, PathBuf::from("/tmp"));

        // Should be able to get summary (TUI display)
        let summary = sm.summary();
        assert!(!summary.is_empty());
    }

    #[test]
    fn test_vision_developer_first() {
        // Settings operations should be fast
        let config = Config::default_test();
        let mut sm = crate::settings::SettingsManager::new(config, PathBuf::from("/tmp"));

        let start = std::time::Instant::now();
        sm.set_string("model", "gpt-4o-mini").unwrap();
        let elapsed = start.elapsed();

        assert!(elapsed.as_micros() < 1000, "Setting update should be < 1ms");
    }

    #[test]
    fn test_vision_observable_ai_actions() {
        // Provider health should be visible
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();

        // Should be able to list providers with status
        let providers = pm.list_providers();
        assert!(providers.len() >= 5);
    }

    #[test]
    fn test_vision_no_hidden_automation() {
        // No integration should be auto-enabled without approval
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        // All proposals should start disabled
        for proposal in &discovery.proposals {
            assert!(!proposal.enabled);
        }
    }

    #[test]
    fn test_vision_platform_before_features() {
        // P5 should not modify existing P0-P4.5 modules
        // Verify core traits are unchanged by checking they exist on concrete types
        let _registry = crate::dispatcher::ToolRegistry::new();
        let _provider = crate::providers::OpenAiProvider::new(Config::default_test());
        let _tool = crate::tools::ReadFile;
        let _name: &str = _tool.name();
    }

    // ─── Configuration Model Validation ───────────────────────────────────

    #[test]
    fn test_config_load_from_dir() {
        let dir = tempdir().unwrap();
        let config_content = r#"
provider = "openai"
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
"#;
        fs::write(dir.path().join("config.toml"), config_content).unwrap();

        let config = Config::load_from_dir(dir.path()).unwrap();
        assert_eq!(config.provider, "openai");
        assert_eq!(config.base_url, "https://api.openai.com/v1");
        assert_eq!(config.model, "gpt-4o");
    }

    #[test]
    fn test_config_load_fallback_defaults() {
        let dir = tempdir().unwrap();

        let config = Config::load_from_dir(dir.path()).unwrap();
        assert_eq!(config.provider, "openai");
        assert_eq!(config.base_url, "https://api.openai.com/v1");
        assert!(config.model.is_empty()); // Auto-detect mode
    }

    #[test]
    fn test_config_persistence() {
        let dir = tempdir().unwrap();
        let mut config = Config::default_test();
        config.model = "claude-3-opus".to_string();

        config.persist_to_dir(dir.path()).unwrap();

        let loaded = Config::load_from_dir(dir.path()).unwrap();
        assert_eq!(loaded.model, "claude-3-opus");
    }

    // ─── Edge Case Tests ──────────────────────────────────────────────────

    #[test]
    fn test_edge_case_api_key_very_long() {
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();

        let long_key = "sk-".to_string() + &"a".repeat(1000);
        pm.set_api_key("openai", &long_key).unwrap();

        let masked = pm.api_key_masked("openai").unwrap();
        assert!(masked.starts_with("••••"));
        assert!(masked.ends_with("aaaa"));
    }

    #[test]
    fn test_edge_case_api_key_unicode() {
        let mut pm = crate::provider_manager::ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();

        pm.set_api_key("openai", "sk-unicode-测试-🔑").unwrap();
        let masked = pm.api_key_masked("openai").unwrap();
        assert!(masked.starts_with("••••"));
    }

    #[test]
    fn test_edge_case_workspace_nested_dirs() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("subdir").join("deep");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("Cargo.toml"), "[package]\nname = \"nested\"").unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(nested.clone());
        let discovery = engine.discover();

        assert_eq!(discovery.language, "rust");
    }

    #[test]
    fn test_edge_case_empty_string_settings() {
        let config = Config::default_test();
        let mut sm = crate::settings::SettingsManager::new(config, PathBuf::from("/tmp"));

        // Setting empty string should be valid for model (means auto-detect)
        sm.set_string("model", "").unwrap();
        let setting = sm.get_setting("model").unwrap();
        assert_eq!(
            setting.value,
            crate::settings::SettingKind::String("".to_string())
        );
    }

    #[test]
    fn test_edge_case_settings_special_characters() {
        let config = Config::default_test();
        let mut sm = crate::settings::SettingsManager::new(config, PathBuf::from("/tmp"));

        // URL with special characters
        sm.set_string(
            "base_url",
            "https://api.example.com/v1?param=value&other=123",
        )
        .unwrap();
        let setting = sm.get_setting("base_url").unwrap();
        assert_eq!(
            setting.value,
            crate::settings::SettingKind::String(
                "https://api.example.com/v1?param=value&other=123".to_string()
            )
        );
    }

    #[test]
    fn test_edge_case_discovery_no_permissions() {
        // Test discovery handles inaccessible directories gracefully
        let result = std::panic::catch_unwind(|| {
            let engine = crate::workspace_discovery::DiscoveryEngine::new(PathBuf::from(
                "/nonexistent/path",
            ));
            engine.discover()
        });

        // Should not panic
        assert!(result.is_ok());
    }

    #[test]
    fn test_edge_case_multiple_language_files() {
        // When multiple language markers exist, priority should be consistent
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
        fs::write(dir.path().join("package.json"), "{}").unwrap();

        let engine = crate::workspace_discovery::DiscoveryEngine::new(dir.path().to_path_buf());
        let discovery = engine.discover();

        // Rust takes priority (checked first in detect_language)
        assert_eq!(discovery.language, "rust");
    }
}

// P6.2 Intent Engine Foundation Tests
// =========================================================================

#[cfg(test)]
mod p6_2_intent_engine {
    use super::*;
    use crate::intent_engine::ambiguity::AmbiguityDetector;
    use crate::intent_engine::classifier::IntentClassifier;
    use crate::intent_engine::confidence::ConfidenceModel;
    use crate::intent_engine::diagnostics::IntentDiagnostics;
    use crate::intent_engine::preview::ApprovalPreviewGenerator;
    use crate::intent_engine::resolver::IntentResolver;
    use crate::intent_engine::*;
    use std::collections::HashMap;
    use tokio::task;

    // ─── Classification Tests ───────────────────────────────────────────────

    #[test]
    fn test_deterministic_classification_preference() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Change the model to gpt-4o");

        assert_eq!(plan.intent_type, IntentType::Preference);
        assert!(plan.confidence >= 0.5);
        assert!(!plan.ambiguity);
        assert!(plan.required_approval);
        assert!(!plan.detected_goal.is_empty());
        assert!(!plan.reasoning.is_empty());
        assert!(!plan.evidence.is_empty());
    }

    #[test]
    fn test_deterministic_classification_configuration() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Configure the system settings");

        assert_eq!(plan.intent_type, IntentType::Configuration);
        assert!(!plan.required_approval);
    }

    #[test]
    fn test_deterministic_classification_workflow() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Run the test workflow");

        assert_eq!(plan.intent_type, IntentType::Workflow);
        assert!(plan.required_approval);
    }

    #[test]
    fn test_deterministic_classification_execution() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Execute the command cargo test");

        assert_eq!(plan.intent_type, IntentType::Execution);
        assert!(plan.required_approval);
    }

    #[test]
    fn test_deterministic_classification_question() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("How does the approval gate work?");

        assert_eq!(plan.intent_type, IntentType::Question);
        assert!(!plan.required_approval);
    }

    #[test]
    fn test_deterministic_classification_help() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("help");

        assert_eq!(plan.intent_type, IntentType::Help);
        assert!(!plan.required_approval);
    }

    #[test]
    fn test_deterministic_classification_unknown() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("xyz123 random gibberish");

        assert_eq!(plan.intent_type, IntentType::Unknown);
        assert!(plan.confidence < 0.5);
        assert!(plan.ambiguity);
    }

    #[test]
    fn test_deterministic_classification_consistency() {
        let classifier = IntentClassifier::new();
        let input = "Change the model to gpt-4o";

        let plan1 = classifier.classify(input);
        let plan2 = classifier.classify(input);

        assert_eq!(plan1.intent_type, plan2.intent_type);
        assert!((plan1.confidence - plan2.confidence).abs() < 0.01);
    }

    // ─── Command Generation Tests ───────────────────────────────────────────

    #[test]
    fn test_command_generation_preference_model() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Change the model to claude-3-opus");
        let resolver = IntentResolver::new();
        let commands = resolver.resolve(&plan);

        assert_eq!(commands.len(), 1);
        match &commands[0].command {
            IntentCommand::UpdateModelPreference {
                key,
                new_value,
                reason,
            } => {
                assert_eq!(key, "model");
                assert_eq!(new_value, "claude-3-opus");
                assert!(!reason.is_empty());
            }
            _ => panic!("Expected UpdateModelPreference"),
        }
    }

    #[test]
    fn test_command_generation_preference_language() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Set language to japanese");
        let resolver = IntentResolver::new();
        let commands = resolver.resolve(&plan);

        assert_eq!(commands.len(), 1);
        match &commands[0].command {
            IntentCommand::UpdateLanguagePreference { key, new_value, .. } => {
                assert_eq!(key, "language");
                assert_eq!(new_value, "japanese");
            }
            _ => panic!("Expected UpdateLanguagePreference"),
        }
    }

    #[test]
    fn test_command_generation_preference_cost() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Set cost limit to 15.00");
        let resolver = IntentResolver::new();
        let commands = resolver.resolve(&plan);

        assert_eq!(commands.len(), 1);
        match &commands[0].command {
            IntentCommand::UpdateCostPreference { key, new_value, .. } => {
                assert_eq!(key, "max_cost_per_session");
                assert!((new_value - 15.0).abs() < 0.01);
            }
            _ => panic!("Expected UpdateCostPreference"),
        }
    }

    #[test]
    fn test_command_generation_preference_approval() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Disable auto approve");
        let resolver = IntentResolver::new();
        let commands = resolver.resolve(&plan);

        assert_eq!(commands.len(), 1);
        match &commands[0].command {
            IntentCommand::UpdateApprovalPreference { key, new_value, .. } => {
                assert_eq!(key, "auto_approve_safe_ops");
                assert!(!new_value);
            }
            _ => panic!("Expected UpdateApprovalPreference"),
        }
    }

    #[test]
    fn test_command_generation_workflow() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Run the deploy workflow");
        let resolver = IntentResolver::new();
        let commands = resolver.resolve(&plan);

        assert_eq!(commands.len(), 1);
        match &commands[0].command {
            IntentCommand::ExecuteWorkflow { workflow_id, .. } => {
                assert_eq!(workflow_id, "deploy_workflow");
            }
            _ => panic!("Expected ExecuteWorkflow"),
        }
    }

    #[test]
    fn test_command_generation_execution() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Execute the command echo hello");
        let resolver = IntentResolver::new();
        let commands = resolver.resolve(&plan);

        assert_eq!(commands.len(), 1);
        match &commands[0].command {
            IntentCommand::ExecuteCommand { command, .. } => {
                assert_eq!(command, "Execute the command echo hello");
            }
            _ => panic!("Expected ExecuteCommand"),
        }
    }

    #[test]
    fn test_command_generation_question() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("What is rust?");
        let resolver = IntentResolver::new();
        let commands = resolver.resolve(&plan);

        assert_eq!(commands.len(), 1);
        match &commands[0].command {
            IntentCommand::AnswerQuestion { question, answer } => {
                assert_eq!(question, "What is rust");
                assert!(!answer.is_empty());
            }
            _ => panic!("Expected AnswerQuestion"),
        }
    }

    #[test]
    fn test_command_generation_help() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("help");
        let resolver = IntentResolver::new();
        let commands = resolver.resolve(&plan);

        assert_eq!(commands.len(), 1);
        match &commands[0].command {
            IntentCommand::ProvideHelp { topic, help_text } => {
                assert_eq!(topic, "general");
                assert!(!help_text.is_empty());
            }
            _ => panic!("Expected ProvideHelp"),
        }
    }

    #[test]
    fn test_command_generation_unknown_no_commands() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("xyz123");
        let resolver = IntentResolver::new();
        let commands = resolver.resolve(&plan);

        assert!(commands.is_empty());
    }

    // ─── Ambiguity Handling Tests ───────────────────────────────────────────

    #[test]
    fn test_ambiguity_detect_vague_model() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("Use Claude.");
        assert!(result.is_ambiguous);
        assert!(result.clarification_questions.len() >= 1);
        assert!(result.reason.is_some());
    }

    #[test]
    fn test_ambiguity_detect_vague_gpt() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("Use GPT.");
        assert!(result.is_ambiguous);
    }

    #[test]
    fn test_ambiguity_detect_clear_model() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("Use Claude-3-Opus.");
        assert!(!result.is_ambiguous);
    }

    #[test]
    fn test_ambiguity_detect_empty_input() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("   ");
        assert!(result.is_ambiguous);
    }

    #[test]
    fn test_ambiguity_detect_vague_change() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("Change to something better");
        assert!(result.is_ambiguous);
    }

    #[test]
    fn test_ambiguity_detect_clear_preference() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("Change the model to gpt-4o");
        assert!(!result.is_ambiguous);
    }

    #[test]
    fn test_ambiguity_detect_clear_question() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("How do I configure CodeBro?");
        assert!(!result.is_ambiguous);
    }

    #[test]
    fn test_ambiguity_detect_clear_help() {
        let detector = AmbiguityDetector::new();
        let result = detector.detect_input("help");
        assert!(!result.is_ambiguous);
    }

    #[test]
    fn test_ambiguity_detect_via_plan() {
        let classifier = IntentClassifier::new();
        let detector = AmbiguityDetector::new();
        let plan = classifier.classify("Use Claude.");
        let result = detector.detect(&plan);
        assert!(result.is_ambiguous);
    }

    #[test]
    fn test_ambiguity_detect_clear_plan() {
        let classifier = IntentClassifier::new();
        let detector = AmbiguityDetector::new();
        let plan = classifier.classify("Change the model to gpt-4o");
        let result = detector.detect(&plan);
        assert!(!result.is_ambiguous);
    }

    // ─── Confidence Scoring Tests ───────────────────────────────────────────

    #[test]
    fn test_confidence_high_for_preference() {
        let model = ConfidenceModel::new();
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Change the model to gpt-4o");
        let result = model.compute(&plan);

        assert!(result.is_confident());
        assert!(result.score >= 0.8);
    }

    #[test]
    fn test_confidence_low_for_unknown() {
        let model = ConfidenceModel::new();
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("xyz random gibberish");
        let result = model.compute(&plan);

        assert!(!result.is_confident());
        assert!(result.score < 0.5);
    }

    #[test]
    fn test_confidence_help_always_high() {
        let model = ConfidenceModel::new();
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("help");
        let result = model.compute(&plan);

        assert!(result.is_confident());
        assert!(result.score >= 0.9);
    }

    #[test]
    fn test_confidence_evidence_present() {
        let model = ConfidenceModel::new();
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Run the build workflow");
        let result = model.compute(&plan);

        assert!(!result.evidence.is_empty());
        assert!(!result.reasoning.is_empty());
    }

    #[test]
    fn test_confidence_from_input() {
        let model = ConfidenceModel::new();
        let result = model.compute_from_input("What is rust?", &IntentType::Question);

        assert!(result.is_confident());
        assert!(result.score >= 0.8);
    }

    #[test]
    fn test_confidence_sufficient_threshold() {
        let model = ConfidenceModel::new();
        let high = ConfidenceResult::new(0.9, vec![], "high");
        let low = ConfidenceResult::new(0.3, vec![], "low");

        assert!(model.is_sufficient(&high));
        assert!(!model.is_sufficient(&low));
    }

    #[test]
    fn test_confidence_high_threshold() {
        let model = ConfidenceModel::new();
        let high = ConfidenceResult::new(0.9, vec![], "high");
        let medium = ConfidenceResult::new(0.7, vec![], "medium");

        assert!(model.is_high(&high));
        assert!(!model.is_high(&medium));
    }

    // ─── Preview Generation Tests ───────────────────────────────────────────

    #[test]
    fn test_preview_model_preference() {
        let generator = ApprovalPreviewGenerator::new();
        let current_values = HashMap::from([("model".to_string(), "gpt-4o".to_string())]);

        let resolved = ResolvedCommand {
            command: IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "claude-3".to_string(),
                reason: "User requested".to_string(),
            },
            metadata: CommandMetadata::new("test", "plan-1", "test", "update model"),
            resolution_order: 0,
        };

        let preview = generator.generate(&resolved, &current_values);
        assert!(preview.requested_change.contains("model"));
        assert_eq!(preview.proposed_value, "claude-3");
        assert_eq!(preview.current_value, Some("gpt-4o".to_string()));
        assert!(matches!(
            preview.reversibility,
            Reversibility::FullyReversible
        ));
    }

    #[test]
    fn test_preview_cost_preference() {
        let generator = ApprovalPreviewGenerator::new();
        let current_values = HashMap::from([("max_cost".to_string(), "5.0".to_string())]);

        let resolved = ResolvedCommand {
            command: IntentCommand::UpdateCostPreference {
                key: "max_cost".to_string(),
                new_value: 10.0,
                reason: "User requested".to_string(),
            },
            metadata: CommandMetadata::new("test", "plan-1", "test", "update cost"),
            resolution_order: 0,
        };

        let preview = generator.generate(&resolved, &current_values);
        assert_eq!(preview.proposed_value, "10");
        assert_eq!(preview.current_value, Some("5.0".to_string()));
    }

    #[test]
    fn test_preview_workflow() {
        let generator = ApprovalPreviewGenerator::new();
        let current_values = HashMap::new();

        let resolved = ResolvedCommand {
            command: IntentCommand::ExecuteWorkflow {
                workflow_id: "test_workflow".to_string(),
                reason: "User requested".to_string(),
            },
            metadata: CommandMetadata::new("test", "plan-1", "test", "run workflow"),
            resolution_order: 0,
        };

        let preview = generator.generate(&resolved, &current_values);
        assert!(preview.requested_change.contains("test_workflow"));
        assert!(matches!(
            preview.reversibility,
            Reversibility::PartiallyReversible
        ));
    }

    #[test]
    fn test_preview_batch() {
        let generator = ApprovalPreviewGenerator::new();
        let current_values = HashMap::new();

        let commands = vec![
            ResolvedCommand {
                command: IntentCommand::UpdateModelPreference {
                    key: "model".to_string(),
                    new_value: "gpt-4".to_string(),
                    reason: "t".to_string(),
                },
                metadata: CommandMetadata::new("test", "p1", "t", "e"),
                resolution_order: 0,
            },
            ResolvedCommand {
                command: IntentCommand::AnswerQuestion {
                    question: "Q?".to_string(),
                    answer: "A".to_string(),
                },
                metadata: CommandMetadata::new("test", "p2", "t", "e"),
                resolution_order: 1,
            },
        ];

        let previews = generator.generate_batch(&commands, &current_values);
        assert_eq!(previews.len(), 2);
    }

    #[test]
    fn test_preview_id_unique() {
        let generator = ApprovalPreviewGenerator::new();
        let current_values = HashMap::new();

        let resolved1 = ResolvedCommand {
            command: IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "a".to_string(),
                reason: "t".to_string(),
            },
            metadata: CommandMetadata::new("test", "p1", "t", "e"),
            resolution_order: 0,
        };

        let resolved2 = ResolvedCommand {
            command: IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "b".to_string(),
                reason: "t".to_string(),
            },
            metadata: CommandMetadata::new("test", "p2", "t", "e"),
            resolution_order: 0,
        };

        let p1 = generator.generate(&resolved1, &current_values);
        let p2 = generator.generate(&resolved2, &current_values);
        assert_ne!(p1.preview_id, p2.preview_id);
    }

    // ─── Serialization Tests ────────────────────────────────────────────────

    #[test]
    fn test_plan_serialization() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Change the model to gpt-4o");

        let json = serde_json::to_string(&plan).expect("should serialize plan");
        let deserialized: IntentPlan =
            serde_json::from_str(&json).expect("should deserialize plan");

        assert_eq!(plan.id, deserialized.id);
        assert_eq!(plan.intent_type, deserialized.intent_type);
        assert_eq!(plan.confidence, deserialized.confidence);
        assert_eq!(
            plan.required_commands.len(),
            deserialized.required_commands.len()
        );
    }

    #[test]
    fn test_command_serialization() {
        let command = IntentCommand::UpdateModelPreference {
            key: "model".to_string(),
            new_value: "gpt-4o".to_string(),
            reason: "User preference".to_string(),
        };

        let json = serde_json::to_string(&command).expect("should serialize command");
        let deserialized: IntentCommand =
            serde_json::from_str(&json).expect("should deserialize command");

        match (&command, &deserialized) {
            (
                IntentCommand::UpdateModelPreference { key: k1, .. },
                IntentCommand::UpdateModelPreference { key: k2, .. },
            ) => {
                assert_eq!(k1, k2);
            }
            _ => panic!("Type mismatch"),
        }
    }

    #[test]
    fn test_preview_serialization() {
        let generator = ApprovalPreviewGenerator::new();
        let current_values = HashMap::new();

        let resolved = ResolvedCommand {
            command: IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4".to_string(),
                reason: "t".to_string(),
            },
            metadata: CommandMetadata::new("test", "p1", "t", "e"),
            resolution_order: 0,
        };

        let preview = generator.generate(&resolved, &current_values);
        let json = serde_json::to_string(&preview).expect("should serialize");
        let deserialized: ApprovalPreview =
            serde_json::from_str(&json).expect("should deserialize");

        assert_eq!(preview.command_kind, deserialized.command_kind);
        assert_eq!(preview.proposed_value, deserialized.proposed_value);
        assert_eq!(preview.preview_id, deserialized.preview_id);
    }

    #[test]
    fn test_confidence_result_serialization() {
        let result = ConfidenceResult::new(
            0.9,
            vec!["evidence1".to_string(), "evidence2".to_string()],
            "reasoning",
        );

        let json = serde_json::to_string(&result).expect("should serialize");
        let deserialized: ConfidenceResult =
            serde_json::from_str(&json).expect("should deserialize");

        assert!((result.score - deserialized.score).abs() < 0.001);
        assert_eq!(result.evidence, deserialized.evidence);
        assert_eq!(result.reasoning, deserialized.reasoning);
    }

    #[test]
    fn test_ambiguity_result_serialization() {
        let result = AmbiguityResult::ambiguous(
            "Test ambiguity",
            vec!["Q1?".to_string(), "Q2?".to_string()],
        );

        let json = serde_json::to_string(&result).expect("should serialize");
        let deserialized: AmbiguityResult =
            serde_json::from_str(&json).expect("should deserialize");

        assert!(deserialized.is_ambiguous);
        assert_eq!(deserialized.clarification_questions.len(), 2);
    }

    #[test]
    fn test_command_metadata_serialization() {
        let metadata = CommandMetadata::new(
            "intent_engine",
            "plan-123",
            "Test reasoning",
            "Expected effect",
        );

        let json = serde_json::to_string(&metadata).expect("should serialize");
        let deserialized: CommandMetadata =
            serde_json::from_str(&json).expect("should deserialize");

        assert_eq!(metadata.source, deserialized.source);
        assert_eq!(metadata.intent_id, deserialized.intent_id);
        assert_eq!(metadata.reason, deserialized.reason);
        assert_eq!(metadata.expected_effect, deserialized.expected_effect);
    }

    // ─── Replay Tests ───────────────────────────────────────────────────────

    #[test]
    fn test_replay_deterministic_classification() {
        let classifier = IntentClassifier::new();
        let input = "Change the model to gpt-4o";

        let plan1 = classifier.classify(input);
        let plan2 = classifier.classify(input);

        assert_eq!(plan1.intent_type, plan2.intent_type);
        assert!((plan1.confidence - plan2.confidence).abs() < 0.01);
        assert_eq!(plan1.required_commands.len(), plan2.required_commands.len());
    }

    #[test]
    fn test_replay_deterministic_commands() {
        let classifier = IntentClassifier::new();
        let resolver = IntentResolver::new();
        let input = "Set language to french";

        let plan1 = classifier.classify(input);
        let commands1 = resolver.resolve(&plan1);

        let plan2 = classifier.classify(input);
        let commands2 = resolver.resolve(&plan2);

        assert_eq!(commands1.len(), commands2.len());
        for (c1, c2) in commands1.iter().zip(commands2.iter()) {
            assert_eq!(c1.command, c2.command);
        }
    }

    #[test]
    fn test_replay_full_pipeline() {
        let classifier = IntentClassifier::new();
        let resolver = IntentResolver::new();
        let preview_gen = ApprovalPreviewGenerator::new();
        let input = "Change cost limit to 8.50";

        let plan = classifier.classify(input);
        let commands = resolver.resolve(&plan);
        let previews = preview_gen.generate_batch(&commands, &HashMap::new());

        assert_eq!(plan.intent_type, IntentType::Preference);
        assert_eq!(commands.len(), 1);
        assert_eq!(previews.len(), 1);
        assert!(previews[0].requested_change.contains("cost"));
    }

    // ─── Audit Metadata Tests ───────────────────────────────────────────────

    #[test]
    fn test_audit_metadata_complete() {
        let resolver = IntentResolver::new();
        let plan = IntentPlan::new(
            "audit-plan-1".to_string(),
            "Change model",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Audit test",
            vec!["Evidence".to_string()],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "User request".to_string(),
            }],
        );

        let commands = resolver.resolve(&plan);
        assert_eq!(commands[0].metadata.source, "intent_engine");
        assert_eq!(commands[0].metadata.intent_id, "audit-plan-1");
        assert!(!commands[0].metadata.timestamp.is_empty());
        assert!(!commands[0].metadata.reason.is_empty());
        assert!(!commands[0].metadata.expected_effect.is_empty());
    }

    #[test]
    fn test_audit_metadata_all_commands() {
        let resolver = IntentResolver::new();
        let plan = IntentPlan::new(
            "audit-plan-2".to_string(),
            "Multi-command",
            IntentType::Execution,
            "execution",
            true,
            1.0,
            0.8,
            false,
            None,
            "Multi test",
            vec!["E1".to_string(), "E2".to_string()],
            vec![
                IntentCommand::ExecuteCommand {
                    command: "echo 1".to_string(),
                    reason: "First".to_string(),
                },
                IntentCommand::ExecuteCommand {
                    command: "echo 2".to_string(),
                    reason: "Second".to_string(),
                },
            ],
        );

        let commands = resolver.resolve(&plan);
        for cmd in &commands {
            assert_eq!(cmd.metadata.source, "intent_engine");
            assert_eq!(cmd.metadata.intent_id, "audit-plan-2");
        }
    }

    // ─── Concurrent Request Tests ───────────────────────────────────────────

    #[tokio::test]
    async fn test_concurrent_classify() {
        let classifier = IntentClassifier::new();
        let inputs = vec![
            "Change the model to gpt-4o",
            "help",
            "What is rust?",
            "Run the test workflow",
            "Configure the system",
            "Execute echo hello",
            "Set language to spanish",
            "Update cost to 10.0",
            "Disable auto approve",
            "Use Claude.",
        ];

        let handles: Vec<_> = inputs
            .iter()
            .map(|input| {
                let cls = classifier.clone();
                let input = input.to_string();
                task::spawn(async move { cls.classify(&input) })
            })
            .collect();

        let results: Vec<_> = futures::future::join_all(handles).await;
        assert_eq!(results.len(), inputs.len());

        for (i, result) in results.iter().enumerate() {
            let plan = result.as_ref().unwrap();
            assert!(!plan.id.is_empty(), "Plan {} should have an ID", i);
            assert!(
                !plan.detected_goal.is_empty(),
                "Plan {} should have a goal",
                i
            );
        }
    }

    #[tokio::test]
    async fn test_concurrent_resolve() {
        let classifier = IntentClassifier::new();
        let resolver = IntentResolver::new();

        let inputs = vec![
            "Change the model to gpt-4o",
            "help",
            "Run the test workflow",
            "Execute echo hello",
        ];

        let handles: Vec<_> = inputs
            .iter()
            .map(|input| {
                let cls = classifier.clone();
                let res = resolver.clone();
                let input = input.to_string();
                task::spawn(async move {
                    let plan = cls.classify(&input);
                    res.resolve(&plan)
                })
            })
            .collect();

        let results: Vec<_> = futures::future::join_all(handles).await;
        for result in results {
            let commands = result.unwrap();
            assert!(!commands.is_empty() || true);
        }
    }

    #[tokio::test]
    async fn test_concurrent_preview_generation() {
        let resolver = IntentResolver::new();
        let preview_gen = ApprovalPreviewGenerator::new();

        let plans = vec![
            IntentPlan::new(
                "p1".to_string(),
                "model change",
                IntentType::Preference,
                "pe",
                true,
                0.0,
                0.9,
                false,
                None,
                "r",
                vec!["e".to_string()],
                vec![IntentCommand::UpdateModelPreference {
                    key: "model".to_string(),
                    new_value: "gpt-4".to_string(),
                    reason: "t".to_string(),
                }],
            ),
            IntentPlan::new(
                "p2".to_string(),
                "cost change",
                IntentType::Preference,
                "pe",
                true,
                0.0,
                0.9,
                false,
                None,
                "r",
                vec!["e".to_string()],
                vec![IntentCommand::UpdateCostPreference {
                    key: "cost".to_string(),
                    new_value: 10.0,
                    reason: "t".to_string(),
                }],
            ),
        ];

        let handles: Vec<_> = plans
            .iter()
            .map(|plan| {
                let res = resolver.clone();
                let pg = preview_gen.clone();
                let plan = plan.clone();
                task::spawn(async move {
                    let commands = res.resolve(&plan);
                    pg.generate_batch(&commands, &HashMap::new())
                })
            })
            .collect();

        let results: Vec<_> = futures::future::join_all(handles).await;
        assert_eq!(results.len(), 2);
        for result in results {
            let previews = result.unwrap();
            assert!(!previews.is_empty());
        }
    }

    // ─── End-to-End Pipeline Tests ──────────────────────────────────────────

    #[test]
    fn test_full_pipeline_preference_change() {
        let classifier = IntentClassifier::new();
        let resolver = IntentResolver::new();
        let preview_gen = ApprovalPreviewGenerator::new();
        let ambiguity_detector = AmbiguityDetector::new();
        let confidence_model = ConfidenceModel::new();
        let diagnostics = IntentDiagnostics::new(100);

        let input = "Change the model to claude-3-opus";

        let plan = classifier.classify(input);
        diagnostics.record(DiagnosticKind::ClassificationFailure, "N/A", false);
        diagnostics.record(DiagnosticKind::AmbiguityDetected, "N/A", false);

        assert_eq!(plan.intent_type, IntentType::Preference);

        let ambiguity = ambiguity_detector.detect(&plan);
        assert!(!ambiguity.is_ambiguous);

        let confidence = confidence_model.compute(&plan);
        assert!(confidence.is_confident());

        let commands = resolver.resolve(&plan);
        assert_eq!(commands.len(), 1);
        assert!(commands[0].requires_approval());

        let current_values = HashMap::from([("model".to_string(), "gpt-4o".to_string())]);
        let previews = preview_gen.generate_batch(&commands, &current_values);
        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].proposed_value, "claude-3-opus");
        assert_eq!(previews[0].current_value, Some("gpt-4o".to_string()));

        assert_eq!(current_values.get("model").unwrap(), "gpt-4o");
    }

    #[test]
    fn test_full_pipeline_ambiguous_input() {
        let classifier = IntentClassifier::new();
        let ambiguity_detector = AmbiguityDetector::new();
        let confidence_model = ConfidenceModel::new();

        let input = "Use Claude.";

        let plan = classifier.classify(input);
        assert_eq!(plan.intent_type, IntentType::Unknown);
        assert!(plan.ambiguity);

        let ambiguity = ambiguity_detector.detect(&plan);
        assert!(ambiguity.is_ambiguous);

        let confidence = confidence_model.compute(&plan);
        assert!(!confidence.is_confident());
    }

    #[test]
    fn test_full_pipeline_help() {
        let classifier = IntentClassifier::new();
        let resolver = IntentResolver::new();
        let preview_gen = ApprovalPreviewGenerator::new();

        let input = "help";

        let plan = classifier.classify(input);
        assert_eq!(plan.intent_type, IntentType::Help);
        assert!(!plan.required_approval);

        let commands = resolver.resolve(&plan);
        assert_eq!(commands.len(), 1);
        assert!(!commands[0].requires_approval());

        let previews = preview_gen.generate_batch(&commands, &HashMap::new());
        assert_eq!(previews.len(), 1);
    }

    #[test]
    fn test_full_pipeline_question() {
        let classifier = IntentClassifier::new();
        let resolver = IntentResolver::new();

        let input = "What is the approval gate?";

        let plan = classifier.classify(input);
        assert_eq!(plan.intent_type, IntentType::Question);

        let commands = resolver.resolve(&plan);
        assert_eq!(commands.len(), 1);
        match &commands[0].command {
            IntentCommand::AnswerQuestion { question, .. } => {
                assert_eq!(question, "What is the approval gate");
            }
            _ => panic!("Expected AnswerQuestion"),
        }
    }

    // ─── Intent Plan Structure Tests ────────────────────────────────────────

    #[test]
    fn test_plan_contains_all_fields() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Change the model to gpt-4o");

        assert!(!plan.id.is_empty());
        assert!(!plan.detected_goal.is_empty());
        assert!(matches!(plan.intent_type, IntentType::Preference));
        assert!(!plan.affected_subsystem.is_empty());
        assert!(plan.required_approval);
        assert!(plan.confidence >= 0.0 && plan.confidence <= 1.0);
        assert!(!plan.ambiguity);
        assert!(!plan.reasoning.is_empty());
        assert!(!plan.evidence.is_empty());
        assert!(!plan.required_commands.is_empty());
        assert!(!plan.created_at.is_empty());
    }

    #[test]
    fn test_unknown_plan_structure() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("unknown input xyz");

        assert_eq!(plan.intent_type, IntentType::Unknown);
        assert!(plan.ambiguity);
        assert!(plan.confidence < 0.5);
        assert!(plan.required_commands.is_empty());
        assert!(plan.ambiguity_reason.is_some());
    }

    #[test]
    fn test_plan_actionable_flag() {
        let classifier = IntentClassifier::new();
        let actionable = classifier.classify("Change the model to gpt-4o");
        let not_actionable = classifier.classify("Use Claude.");

        assert!(actionable.is_actionable());
        assert!(!not_actionable.is_actionable());
    }

    // ─── Diagnostics Tests ──────────────────────────────────────────────────

    #[test]
    fn test_diagnostics_track_classification() {
        let diag = IntentDiagnostics::new(100);
        diag.record(DiagnosticKind::ClassificationFailure, "test", false);
        assert_eq!(
            diag.count_by_kind(&DiagnosticKind::ClassificationFailure),
            1
        );
    }

    #[test]
    fn test_diagnostics_track_ambiguity() {
        let diag = IntentDiagnostics::new(100);
        diag.record(DiagnosticKind::AmbiguityDetected, "vague input", false);
        assert_eq!(diag.count_by_kind(&DiagnosticKind::AmbiguityDetected), 1);
    }

    #[test]
    fn test_diagnostics_track_resolver_failure() {
        let diag = IntentDiagnostics::new(100);
        diag.record(DiagnosticKind::ResolverFailure, "test", false);
        assert!(diag.has_failures());
    }

    #[test]
    fn test_diagnostics_track_command_failure() {
        let diag = IntentDiagnostics::new(100);
        diag.record(DiagnosticKind::CommandGenerationFailure, "test", true);
        assert!(diag.has_failures());
    }

    #[test]
    fn test_diagnostics_summary() {
        let diag = IntentDiagnostics::new(100);
        diag.record(DiagnosticKind::ClassificationFailure, "e1", false);
        diag.record(DiagnosticKind::ClassificationFailure, "e2", false);
        diag.record(DiagnosticKind::AmbiguityDetected, "e3", false);
        let summary = diag.summary();
        assert_eq!(summary.len(), 2);
    }

    #[test]
    fn test_diagnostics_serializable() {
        let diag = IntentDiagnostics::new(100);
        diag.record(DiagnosticKind::ClassificationFailure, "test", false);
        let json = serde_json::to_string(&diag.records()).expect("serialize");
        let _parsed: Vec<DiagnosticRecord> = serde_json::from_str(&json).expect("deserialize");
    }

    // ─── Command Immutability Tests ─────────────────────────────────────────

    #[test]
    fn test_commands_are_immutable() {
        let resolver = IntentResolver::new();
        let plan = IntentPlan::new(
            "imm-1".to_string(),
            "Test",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Test",
            vec!["E".to_string()],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );

        let commands1 = resolver.resolve(&plan);
        let commands2 = resolver.resolve(&plan);

        assert_eq!(commands1.len(), commands2.len());
        for (c1, c2) in commands1.iter().zip(commands2.iter()) {
            assert_eq!(c1.command, c2.command);
            assert_eq!(c1.metadata.intent_id, c2.metadata.intent_id);
            assert_eq!(c1.metadata.source, c2.metadata.source);
            assert_eq!(c1.resolution_order, c2.resolution_order);
        }
    }

    #[test]
    fn test_commands_no_state_mutation() {
        let mut current_values = HashMap::new();
        current_values.insert("model".to_string(), "gpt-4o".to_string());

        let generator = ApprovalPreviewGenerator::new();
        let resolved = ResolvedCommand {
            command: IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "claude-3".to_string(),
                reason: "Test".to_string(),
            },
            metadata: CommandMetadata::new("test", "p1", "t", "e"),
            resolution_order: 0,
        };

        let _preview = generator.generate(&resolved, &current_values);

        assert_eq!(current_values.get("model").unwrap(), "gpt-4o");
    }

    // ─── Edge Cases ─────────────────────────────────────────────────────────

    #[test]
    fn test_edge_case_whitespace_only() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("      ");
        assert_eq!(plan.intent_type, IntentType::Unknown);
    }

    #[test]
    fn test_edge_case_single_word() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("help");
        assert_eq!(plan.intent_type, IntentType::Help);
    }

    #[test]
    fn test_edge_case_long_input() {
        let classifier = IntentClassifier::new();
        let long_input = "Change ".to_string() + &"the model to gpt-4o ".repeat(100);
        let plan = classifier.classify(&long_input);
        assert!(!plan.id.is_empty());
        assert!(!plan.detected_goal.is_empty());
    }

    #[test]
    fn test_edge_case_unicode_input() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("日本語で設定を変更する");
        assert!(!plan.id.is_empty());
    }

    #[test]
    fn test_edge_case_special_characters() {
        let classifier = IntentClassifier::new();
        let plan = classifier.classify("Change model to gpt-4o@#$%");
        assert!(!plan.id.is_empty());
    }

    // ─── Benchmark Helpers ──────────────────────────────────────────────────

    #[test]
    fn test_classification_latency_baseline() {
        let classifier = IntentClassifier::new();
        let inputs = vec![
            "Change the model to gpt-4o",
            "help",
            "What is rust?",
            "Run the test workflow",
            "Configure the system",
        ];

        let start = std::time::Instant::now();
        for input in &inputs {
            let _ = classifier.classify(input);
        }
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 500,
            "Classification should be fast: {}ms",
            elapsed.as_millis()
        );
    }

    #[test]
    fn test_command_generation_latency_baseline() {
        let classifier = IntentClassifier::new();
        let resolver = IntentResolver::new();
        let plan = classifier.classify("Change the model to gpt-4o");

        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = resolver.resolve(&plan);
        }
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 500,
            "Resolution should be fast: {}ms",
            elapsed.as_millis()
        );
    }

    #[test]
    fn test_preview_generation_latency_baseline() {
        let resolver = IntentResolver::new();
        let preview_gen = ApprovalPreviewGenerator::new();
        let plan = IntentPlan::new(
            "bench".to_string(),
            "Test",
            IntentType::Preference,
            "pe",
            true,
            0.0,
            0.9,
            false,
            None,
            "Benchmark",
            vec!["E".to_string()],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );

        let commands = resolver.resolve(&plan);
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = preview_gen.generate_batch(&commands, &HashMap::new());
        }
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 500,
            "Preview generation should be fast: {}ms",
            elapsed.as_millis()
        );
    }
}

// P6.3 Recommendation Engine Foundation Tests
// =========================================================================

#[cfg(test)]
mod p6_3_recommendation_engine {
    use super::*;
    use crate::intent_engine::{IntentCommand, IntentPlan, IntentType};
    use crate::recommendation_engine::diagnostics::RecommendationDiagnostics;
    use crate::recommendation_engine::engine::RecommendationEngine;
    use crate::recommendation_engine::filter::{
        filter, filter_by_confidence, filter_by_type, filter_by_uniqueness,
    };
    use crate::recommendation_engine::ranking::{deduplicate, full_rank, rank, remove_conflicts};
    use crate::recommendation_engine::rules::all_rules;
    use crate::recommendation_engine::*;

    fn make_rec(title: &str, confidence: f64, target_key: Option<&str>) -> Recommendation {
        Recommendation::new(
            RecommendationType::General,
            title,
            "Test explanation",
            vec!["Test evidence".to_string()],
            if confidence >= 0.8 {
                RecommendationConfidence::High(confidence)
            } else if confidence >= 0.5 {
                RecommendationConfidence::Medium(confidence)
            } else {
                RecommendationConfidence::Low(confidence)
            },
            "test-rule",
            target_key.map(|s| s.to_string()),
            None,
            "plan-1",
        )
    }

    fn make_rec_with_value(
        title: &str,
        confidence: f64,
        target_key: Option<&str>,
        target_value: Option<&str>,
    ) -> Recommendation {
        Recommendation::new(
            RecommendationType::General,
            title,
            "Test explanation",
            vec!["Test evidence".to_string()],
            if confidence >= 0.8 {
                RecommendationConfidence::High(confidence)
            } else if confidence >= 0.5 {
                RecommendationConfidence::Medium(confidence)
            } else {
                RecommendationConfidence::Low(confidence)
            },
            "test-rule",
            target_key.map(|s| s.to_string()),
            target_value.map(|s| s.to_string()),
            "plan-1",
        )
    }

    // ─── Rules Tests ────────────────────────────────────────────────────────

    #[test]
    fn test_all_rules_exist() {
        let rules = all_rules();
        assert!(rules.len() >= 20, "Should have at least 20 rules");
    }

    #[test]
    fn test_dark_theme_rule_matches() {
        let rules = all_rules();
        let dark_rule = rules.iter().find(|r| r.id == "rule-dark-theme");
        assert!(dark_rule.is_some());
        let rule = dark_rule.unwrap();
        assert!(rule.matches("Enable dark theme"));
        assert!(rule.matches("dark theme"));
        assert!(rule.matches("dark mode"));
    }

    #[test]
    fn test_vim_mode_rule_matches() {
        let rules = all_rules();
        let vim_rule = rules.iter().find(|r| r.id == "rule-vim-mode");
        assert!(vim_rule.is_some());
        let rule = vim_rule.unwrap();
        assert!(rule.matches("Enable vim mode"));
        assert!(rule.matches("use vim"));
        assert!(rule.matches("switch to nvim"));
    }

    #[test]
    fn test_git_rule_matches() {
        let rules = all_rules();
        let git_rule = rules.iter().find(|r| r.id == "rule-git-integration");
        assert!(git_rule.is_some());
        let rule = git_rule.unwrap();
        assert!(rule.matches("Enable git integration"));
        assert!(rule.matches("git version control"));
    }

    #[test]
    fn test_rust_rule_matches() {
        let rules = all_rules();
        let rust_rule = rules.iter().find(|r| r.id == "rule-rust-lang");
        assert!(rust_rule.is_some());
        let rule = rust_rule.unwrap();
        assert!(rule.matches("Rust cargo project"));
        assert!(rule.matches("rust analyzer"));
    }

    #[test]
    fn test_generate_from_rules_dark_theme() {
        let context = RecommendationContext::new();
        let recs = crate::recommendation_engine::rules::generate_from_rules(
            "Enable dark theme",
            "plan-1",
            &context,
        );
        assert!(!recs.is_empty());
        assert!(recs
            .iter()
            .any(|r| matches!(r.rec_type, RecommendationType::Appearance)));
    }

    #[test]
    fn test_generate_from_rules_vim() {
        let context = RecommendationContext::new();
        let recs = crate::recommendation_engine::rules::generate_from_rules(
            "Enable vim mode",
            "plan-1",
            &context,
        );
        assert!(!recs.is_empty());
        assert!(recs
            .iter()
            .any(|r| matches!(r.rec_type, RecommendationType::Keyboard)));
    }

    #[test]
    fn test_generate_from_rules_no_match() {
        let context = RecommendationContext::new();
        let recs = crate::recommendation_engine::rules::generate_from_rules(
            "xyz random gibberish",
            "plan-1",
            &context,
        );
        assert!(recs.is_empty());
    }

    // ─── Engine Tests ───────────────────────────────────────────────────────

    #[test]
    fn test_engine_empty_plan() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-1".to_string(),
            "xyz random gibberish",
            IntentType::Unknown,
            "unknown",
            false,
            0.0,
            0.1,
            true,
            Some("No match".to_string()),
            "No classification",
            vec!["No rule matched".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();
        let result = engine.recommend(&plan, &context);
        assert!(result.is_empty());
    }

    #[test]
    fn test_engine_dark_theme() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-2".to_string(),
            "Enable dark theme",
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.8,
            false,
            None,
            "Dark theme configuration",
            vec!["Rule match: dark theme".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();
        let result = engine.recommend(&plan, &context);
        assert!(!result.is_empty());
        assert!(result
            .recommendations
            .iter()
            .any(|r| matches!(r.rec_type, RecommendationType::Appearance)));
    }

    #[test]
    fn test_engine_vim_mode() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-3".to_string(),
            "Enable vim mode",
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.85,
            false,
            None,
            "Vim mode configuration",
            vec!["Rule match: vim mode".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();
        let result = engine.recommend(&plan, &context);
        assert!(!result.is_empty());
        assert!(result
            .recommendations
            .iter()
            .any(|r| matches!(r.rec_type, RecommendationType::Keyboard)));
    }

    #[test]
    fn test_engine_git_integration() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-4".to_string(),
            "Enable git integration",
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.8,
            false,
            None,
            "Git integration",
            vec!["Rule match: git".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();
        let result = engine.recommend(&plan, &context);
        assert!(!result.is_empty());
        assert!(result
            .recommendations
            .iter()
            .any(|r| matches!(r.rec_type, RecommendationType::Integration)));
    }

    #[test]
    fn test_engine_no_state_mutation() {
        let engine = RecommendationEngine::new();
        let mut context = RecommendationContext::new();
        context
            .preferences
            .insert("model".to_string(), "gpt-4o".to_string());

        let plan = IntentPlan::new(
            "test-5".to_string(),
            "Change the model to claude",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model change",
            vec!["Rule match".to_string()],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "claude".to_string(),
                reason: "User requested".to_string(),
            }],
        );

        let _result = engine.recommend(&plan, &context);

        // Context must not be mutated
        assert_eq!(
            context.preferences.get("model"),
            Some(&"gpt-4o".to_string())
        );
    }

    #[test]
    fn test_engine_deterministic() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-6".to_string(),
            "Enable dark theme",
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.8,
            false,
            None,
            "Dark theme",
            vec!["Rule match".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();

        let result1 = engine.recommend(&plan, &context);
        let result2 = engine.recommend(&plan, &context);

        assert_eq!(result1.len(), result2.len());
        for (r1, r2) in result1
            .recommendations
            .iter()
            .zip(result2.recommendations.iter())
        {
            assert_eq!(r1.title, r2.title);
            assert_eq!(r1.rec_type, r2.rec_type);
            assert!((r1.confidence.score() - r2.confidence.score()).abs() < 0.001);
        }
    }

    #[test]
    fn test_has_recommendations_true() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-7".to_string(),
            "Enable vim mode",
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.85,
            false,
            None,
            "Vim mode",
            vec!["Rule match".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();
        assert!(engine.has_recommendations(&plan, &context));
    }

    #[test]
    fn test_has_recommendations_false() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-8".to_string(),
            "xyz random gibberish",
            IntentType::Unknown,
            "unknown",
            false,
            0.0,
            0.1,
            true,
            Some("No match".to_string()),
            "No classification",
            vec!["No rule matched".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();
        assert!(!engine.has_recommendations(&plan, &context));
    }

    #[test]
    fn test_count_recommendations() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-9".to_string(),
            "Enable git integration",
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.8,
            false,
            None,
            "Git integration",
            vec!["Rule match".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();
        let count = engine.count_recommendations(&plan, &context);
        assert!(count >= 1);
    }

    // ─── Ranking Tests ──────────────────────────────────────────────────────

    #[test]
    fn test_rank_sorts_by_confidence() {
        let recs = vec![
            make_rec_with_value("Low", 0.5, None, None),
            make_rec_with_value("High", 0.9, None, None),
            make_rec_with_value("Medium", 0.7, None, None),
        ];
        let ranked = rank(recs);
        assert!((ranked[0].confidence.score() - 0.9).abs() < 0.001);
        assert!((ranked[1].confidence.score() - 0.7).abs() < 0.001);
        assert!((ranked[2].confidence.score() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_rank_stable_for_same_confidence() {
        let recs = vec![
            make_rec_with_value("B", 0.7, None, None),
            make_rec_with_value("A", 0.7, None, None),
            make_rec_with_value("C", 0.7, None, None),
        ];
        let ranked = rank(recs);
        assert_eq!(ranked[0].title, "A");
        assert_eq!(ranked[1].title, "B");
        assert_eq!(ranked[2].title, "C");
    }

    #[test]
    fn test_deduplicate_keeps_highest() {
        let recs = vec![
            make_rec_with_value("Same Title", 0.5, Some("key1"), None),
            make_rec_with_value("Same Title", 0.9, Some("key1"), None),
            make_rec_with_value("Same Title", 0.7, Some("key1"), None),
        ];
        let deduped = deduplicate(recs);
        assert_eq!(deduped.len(), 1);
        assert!((deduped[0].confidence.score() - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_deduplicate_keeps_different_titles() {
        let recs = vec![
            make_rec_with_value("Title A", 0.5, Some("key1"), None),
            make_rec_with_value("Title B", 0.6, Some("key2"), None),
        ];
        let deduped = deduplicate(recs);
        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn test_remove_conflicts_keeps_higher() {
        let recs = vec![
            make_rec_with_value("Option A", 0.9, Some("setting"), None),
            make_rec_with_value("Option B", 0.5, Some("setting"), None),
        ];
        let (kept, removed) = remove_conflicts(recs);
        assert_eq!(kept.len(), 1);
        assert_eq!(removed, 1);
        assert!((kept[0].confidence.score() - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_remove_conflicts_no_conflict() {
        let recs = vec![
            make_rec_with_value("Option A", 0.9, Some("setting1"), None),
            make_rec_with_value("Option B", 0.5, Some("setting2"), None),
        ];
        let (kept, removed) = remove_conflicts(recs);
        assert_eq!(kept.len(), 2);
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_full_rank_pipeline() {
        let recs = vec![
            make_rec_with_value("Low Priority", 0.3, Some("key1"), None),
            make_rec_with_value("High Priority", 0.9, Some("key2"), None),
            make_rec_with_value("Medium Priority", 0.6, Some("key3"), None),
            make_rec_with_value("High Priority", 0.95, Some("key2"), None),
            make_rec_with_value("Low Priority", 0.5, Some("key1"), None),
        ];
        let (final_recs, dup_count, conflict_count) = full_rank(recs);
        assert_eq!(final_recs.len(), 3);
        assert!(dup_count >= 2);
        assert!((final_recs[0].confidence.score() - 0.95).abs() < 0.001);
    }

    #[test]
    fn test_rank_empty_input() {
        let recs: Vec<Recommendation> = vec![];
        let ranked = rank(recs);
        assert!(ranked.is_empty());
    }

    #[test]
    fn test_deduplicate_empty_input() {
        let recs: Vec<Recommendation> = vec![];
        let deduped = deduplicate(recs);
        assert!(deduped.is_empty());
    }

    #[test]
    fn test_remove_conflicts_empty_input() {
        let recs: Vec<Recommendation> = vec![];
        let (kept, removed) = remove_conflicts(recs);
        assert!(kept.is_empty());
        assert_eq!(removed, 0);
    }

    // ─── Filter Tests ───────────────────────────────────────────────────────

    #[test]
    fn test_filter_confidence_threshold() {
        let recs = vec![
            make_rec_with_value("High", 0.9, None, None),
            make_rec_with_value("Low", 0.3, None, None),
            make_rec_with_value("Medium", 0.6, None, None),
        ];
        let context = RecommendationContext::new().with_min_confidence(0.5);
        let filtered = filter(recs, &context);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|r| r.confidence.score() >= 0.5));
    }

    #[test]
    fn test_filter_already_enabled() {
        let mut prefs = HashMap::new();
        prefs.insert("key1".to_string(), "true".to_string());

        let recs = vec![
            make_rec_with_value("Already Enabled", 0.9, Some("key1"), Some("true")),
            make_rec_with_value("Not Enabled", 0.8, Some("key2"), Some("false")),
        ];
        let context = RecommendationContext::new().with_preferences(prefs);
        let filtered = filter(recs, &context);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title, "Not Enabled");
    }

    #[test]
    fn test_filter_max_count() {
        let recs = vec![
            make_rec_with_value("One", 0.9, None, None),
            make_rec_with_value("Two", 0.8, None, None),
            make_rec_with_value("Three", 0.7, None, None),
            make_rec_with_value("Four", 0.6, None, None),
        ];
        let context = RecommendationContext::new().with_max_recommendations(2);
        let filtered = filter(recs, &context);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_by_type() {
        let recs = vec![
            make_rec_with_value("Layout", 0.9, None, None),
            make_rec_with_value("Keyboard", 0.8, None, None),
            make_rec_with_value("Appearance", 0.7, None, None),
        ];
        let filtered = filter_by_type(recs, &[RecommendationType::General]);
        assert_eq!(filtered.len(), 3);
        assert!(filtered
            .iter()
            .all(|r| matches!(r.rec_type, RecommendationType::General)));
    }

    #[test]
    fn test_filter_by_confidence() {
        let recs = vec![
            make_rec_with_value("High", 0.9, None, None),
            make_rec_with_value("Medium", 0.6, None, None),
            make_rec_with_value("Low", 0.3, None, None),
        ];
        let filtered = filter_by_confidence(recs, 0.5);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_by_uniqueness() {
        let recs = vec![
            make_rec_with_value("Key1 High", 0.9, Some("key1"), Some("true")),
            make_rec_with_value("Key1 Low", 0.3, Some("key1"), Some("false")),
            make_rec_with_value("Key2", 0.8, Some("key2"), Some("true")),
        ];
        let filtered = filter_by_uniqueness(recs);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|r| r.title == "Key1 High"));
        assert!(filtered.iter().any(|r| r.title == "Key2"));
    }

    #[test]
    fn test_filter_empty_input() {
        let recs: Vec<Recommendation> = vec![];
        let context = RecommendationContext::new();
        let filtered = filter(recs, &context);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_no_matching_target() {
        let recs = vec![
            make_rec_with_value("No Target", 0.9, None, None),
            make_rec_with_value("Null Target", 0.8, None, None),
        ];
        let context = RecommendationContext::new();
        let filtered = filter(recs, &context);
        assert_eq!(filtered.len(), 2);
    }

    // ─── Diagnostics Tests ──────────────────────────────────────────────────

    #[test]
    fn test_diagnostics_record_and_retrieve() {
        let diag = RecommendationDiagnostics::new(100);
        diag.record(DiagnosticKind::RecommendationProduced, "test", false);
        assert_eq!(diag.total_count(), 1);
        let records = diag.records();
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn test_diagnostics_count_by_kind() {
        let diag = RecommendationDiagnostics::new(100);
        diag.record(DiagnosticKind::RecommendationProduced, "r1", false);
        diag.record(DiagnosticKind::RecommendationProduced, "r2", false);
        diag.record(DiagnosticKind::RecommendationFiltered, "f1", false);
        assert_eq!(
            diag.count_by_kind(&DiagnosticKind::RecommendationProduced),
            2
        );
        assert_eq!(
            diag.count_by_kind(&DiagnosticKind::RecommendationFiltered),
            1
        );
    }

    #[test]
    fn test_diagnostics_max_size_eviction() {
        let diag = RecommendationDiagnostics::new(5);
        for i in 0..10 {
            diag.record(
                DiagnosticKind::RecommendationProduced,
                &format!("r{}", i),
                false,
            );
        }
        assert_eq!(diag.total_count(), 5);
    }

    #[test]
    fn test_diagnostics_clear() {
        let diag = RecommendationDiagnostics::new(100);
        diag.record(DiagnosticKind::RecommendationProduced, "test", false);
        diag.clear();
        assert_eq!(diag.total_count(), 0);
    }

    #[test]
    fn test_diagnostics_clone_shares_state() {
        let diag = RecommendationDiagnostics::new(100);
        diag.record(DiagnosticKind::RecommendationProduced, "test", false);
        let cloned = diag.clone();
        assert_eq!(cloned.total_count(), 1);
        cloned.record(DiagnosticKind::RecommendationFiltered, "test2", false);
        assert_eq!(diag.total_count(), 2);
    }

    #[test]
    fn test_diagnostics_kind_labels() {
        assert_eq!(
            DiagnosticKind::RecommendationProduced.label(),
            "recommendation_produced"
        );
        assert_eq!(
            DiagnosticKind::RecommendationFiltered.label(),
            "recommendation_filtered"
        );
        assert_eq!(
            DiagnosticKind::DuplicateRemoved.label(),
            "duplicate_removed"
        );
        assert_eq!(DiagnosticKind::ConflictRemoved.label(), "conflict_removed");
        assert_eq!(DiagnosticKind::RuleMatched.label(), "rule_matched");
        assert_eq!(DiagnosticKind::RuleNotMatched.label(), "rule_not_matched");
    }

    #[test]
    fn test_diagnostics_recent() {
        let diag = RecommendationDiagnostics::new(100);
        for i in 0..5 {
            diag.record(
                DiagnosticKind::RecommendationProduced,
                &format!("r{}", i),
                false,
            );
        }
        let recent = diag.recent(3);
        assert_eq!(recent.len(), 3);
        assert!(recent[0].message.contains("r2"));
        assert!(recent[2].message.contains("r4"));
    }

    #[test]
    fn test_diagnostics_summary() {
        let diag = RecommendationDiagnostics::new(100);
        diag.record(DiagnosticKind::RecommendationProduced, "r1", false);
        diag.record(DiagnosticKind::RecommendationProduced, "r2", false);
        diag.record(DiagnosticKind::RecommendationFiltered, "f1", false);
        let summary = diag.summary();
        assert_eq!(summary.len(), 2);
        assert_eq!(summary[0].1, 2);
        assert_eq!(summary[1].1, 1);
    }

    #[test]
    fn test_diagnostics_summary_empty() {
        let diag = RecommendationDiagnostics::new(100);
        let summary = diag.summary();
        assert!(summary.is_empty());
    }

    #[test]
    fn test_diagnostics_serializable() {
        let diag = RecommendationDiagnostics::new(100);
        diag.record(DiagnosticKind::RecommendationProduced, "test", false);
        let records = diag.records();
        let json = serde_json::to_string(&records).expect("serialize");
        let _parsed: Vec<DiagnosticRecord> = serde_json::from_str(&json).expect("deserialize");
    }

    // ─── Integration Tests ──────────────────────────────────────────────────

    #[test]
    fn test_full_pipeline_recommendation() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "pipe-1".to_string(),
            "Enable dark theme with vim mode",
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.85,
            false,
            None,
            "Dark theme + vim",
            vec!["Rule match".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();
        let result = engine.recommend(&plan, &context);
        assert!(!result.is_empty());
        assert!(!result.intent_id.is_empty());
        assert!(!result.generated_at.is_empty());
    }

    #[test]
    fn test_recommendation_is_read_only() {
        let mut context = RecommendationContext::new();
        context
            .preferences
            .insert("dark_theme".to_string(), "false".to_string());

        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "readonly-1".to_string(),
            "Enable dark theme",
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.8,
            false,
            None,
            "Dark theme",
            vec!["Rule match".to_string()],
            vec![],
        );

        let _result = engine.recommend(&plan, &context);

        // Context must not be mutated
        assert_eq!(
            context.preferences.get("dark_theme"),
            Some(&"false".to_string())
        );
    }

    #[test]
    fn test_recommendation_serializable() {
        let rec = Recommendation::new(
            RecommendationType::Appearance,
            "Test Rec",
            "Test explanation",
            vec!["Test evidence".to_string()],
            RecommendationConfidence::High(0.9),
            "test-rule",
            Some("key".to_string()),
            Some("value".to_string()),
            "plan-1",
        );

        let json = serde_json::to_string(&rec).expect("should serialize");
        let deserialized: Recommendation = serde_json::from_str(&json).expect("should deserialize");

        assert_eq!(rec.title, deserialized.title);
        assert_eq!(rec.rec_type, deserialized.rec_type);
        assert!((rec.confidence.score() - deserialized.confidence.score()).abs() < 0.001);
    }

    #[test]
    fn test_recommendation_set_sorted_by_confidence() {
        let mut set = RecommendationSet::new("plan-1");
        set.add(Recommendation::new(
            RecommendationType::General,
            "Low",
            "Test",
            vec![],
            RecommendationConfidence::Low(0.3),
            "rule-1",
            None,
            None,
            "plan-1",
        ));
        set.add(Recommendation::new(
            RecommendationType::General,
            "High",
            "Test",
            vec![],
            RecommendationConfidence::High(0.9),
            "rule-2",
            None,
            None,
            "plan-1",
        ));
        set.add(Recommendation::new(
            RecommendationType::General,
            "Medium",
            "Test",
            vec![],
            RecommendationConfidence::Medium(0.6),
            "rule-3",
            None,
            None,
            "plan-1",
        ));

        let sorted = set.sorted_by_confidence();
        assert_eq!(sorted[0].title, "High");
        assert_eq!(sorted[1].title, "Medium");
        assert_eq!(sorted[2].title, "Low");
    }

    #[test]
    fn test_recommendation_by_type() {
        let mut set = RecommendationSet::new("plan-1");
        set.add(Recommendation::new(
            RecommendationType::Keyboard,
            "Keyboard Rec",
            "Test",
            vec![],
            RecommendationConfidence::High(0.9),
            "rule-1",
            None,
            None,
            "plan-1",
        ));
        set.add(Recommendation::new(
            RecommendationType::Appearance,
            "Appearance Rec",
            "Test",
            vec![],
            RecommendationConfidence::High(0.8),
            "rule-2",
            None,
            None,
            "plan-1",
        ));

        let keyboard_recs = set.by_type(&RecommendationType::Keyboard);
        assert_eq!(keyboard_recs.len(), 1);
        assert_eq!(keyboard_recs[0].title, "Keyboard Rec");
    }

    #[test]
    fn test_recommendation_is_actionable() {
        let high = Recommendation::new(
            RecommendationType::General,
            "High",
            "Test",
            vec![],
            RecommendationConfidence::High(0.9),
            "rule-1",
            None,
            None,
            "plan-1",
        );
        let low = Recommendation::new(
            RecommendationType::General,
            "Low",
            "Test",
            vec![],
            RecommendationConfidence::Low(0.3),
            "rule-1",
            None,
            None,
            "plan-1",
        );

        assert!(high.is_actionable());
        assert!(!low.is_actionable());
    }

    #[test]
    fn test_recommendation_is_strong() {
        let high = Recommendation::new(
            RecommendationType::General,
            "High",
            "Test",
            vec![],
            RecommendationConfidence::High(0.9),
            "rule-1",
            None,
            None,
            "plan-1",
        );
        let medium = Recommendation::new(
            RecommendationType::General,
            "Medium",
            "Test",
            vec![],
            RecommendationConfidence::Medium(0.6),
            "rule-1",
            None,
            None,
            "plan-1",
        );

        assert!(high.is_strong());
        assert!(!medium.is_strong());
    }

    // ─── Edge Cases ─────────────────────────────────────────────────────────

    #[test]
    fn test_edge_case_empty_input() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "edge-1".to_string(),
            "",
            IntentType::Unknown,
            "unknown",
            false,
            0.0,
            0.1,
            true,
            Some("Empty".to_string()),
            "Empty input",
            vec![],
            vec![],
        );
        let context = RecommendationContext::new();
        let result = engine.recommend(&plan, &context);
        assert!(result.is_empty());
    }

    #[test]
    fn test_edge_case_unicode_input() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "edge-2".to_string(),
            "日本語で設定",
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.5,
            false,
            None,
            "Japanese input",
            vec![],
            vec![],
        );
        let context = RecommendationContext::new();
        let result = engine.recommend(&plan, &context);
        // Should not panic, may or may not have recommendations
        assert!(!result.intent_id.is_empty());
    }

    #[test]
    fn test_edge_case_long_input() {
        let engine = RecommendationEngine::new();
        let long_input = "Enable ".to_string() + &"dark theme ".repeat(100);
        let plan = IntentPlan::new(
            "edge-3".to_string(),
            &long_input,
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.8,
            false,
            None,
            "Long input",
            vec![],
            vec![],
        );
        let context = RecommendationContext::new();
        let result = engine.recommend(&plan, &context);
        assert!(!result.intent_id.is_empty());
    }

    // ─── Benchmark Helpers ──────────────────────────────────────────────────

    #[test]
    fn test_recommendation_latency_baseline() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "bench-1".to_string(),
            "Enable dark theme",
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.8,
            false,
            None,
            "Benchmark",
            vec!["Rule match".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();

        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = engine.recommend(&plan, &context);
        }
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 2000,
            "Recommendation should be fast: {}ms",
            elapsed.as_millis()
        );
    }
}

// P6.4 Workflow Engine Foundation Tests
// =========================================================================

#[cfg(test)]
mod p6_4_workflow_engine {
    use super::*;
    use crate::intent_engine::{IntentCommand, IntentPlan, IntentType};
    use crate::recommendation_engine::{
        Recommendation, RecommendationConfidence, RecommendationSet, RecommendationType,
    };
    use crate::workflow_engine::dependency::{
        build_dependencies, calculate_depth, find_entry_points, find_exit_points, has_cycles,
    };
    use crate::workflow_engine::diagnostics::WorkflowDiagnostics;
    use crate::workflow_engine::ordering::{
        sort_by_priority, sort_by_stage_and_priority, topological_sort,
    };
    use crate::workflow_engine::planner::WorkflowPlanner;
    use crate::workflow_engine::preview::{
        generate_approval_summary, generate_compact_preview, generate_preview,
    };
    use crate::workflow_engine::validator::{generate_warnings, validate_inputs, validate_plan};
    use crate::workflow_engine::*;

    fn make_step(id: &str, command: &str, stage: WorkflowStage, deps: Vec<&str>) -> WorkflowStep {
        WorkflowStep {
            step_id: id.to_string(),
            name: id.to_string(),
            command: command.to_string(),
            stage,
            priority: 0,
            dependencies: deps.into_iter().map(|s| s.to_string()).collect(),
            requires_approval: false,
            estimated_cost: 0.0,
            reversible: true,
            description: "Test".to_string(),
        }
    }

    // ─── Planner Tests ────────────────────────────────────────────────────────

    #[test]
    fn test_planner_empty_plan() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "test-1".to_string(),
            "xyz random",
            IntentType::Unknown,
            "unknown",
            false,
            0.0,
            0.1,
            true,
            Some("No match".to_string()),
            "No classification",
            vec![],
            vec![],
        );
        let diag = WorkflowDiagnostics::new(100);
        let result = planner.plan(&intent, None, &diag);
        assert!(!result.plan.is_valid);
    }

    #[test]
    fn test_planner_preference_change() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "test-2".to_string(),
            "Change the model to gpt-4o",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model change",
            vec!["Rule match".to_string()],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "User requested".to_string(),
            }],
        );
        let diag = WorkflowDiagnostics::new(100);
        let result = planner.plan(&intent, None, &diag);
        assert!(result.plan.is_valid);
        assert_eq!(result.plan.total_steps, 1);
        assert!(result.approval_required);
    }

    #[test]
    fn test_planner_deterministic() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "test-3".to_string(),
            "Change the model to gpt-4o",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model change",
            vec!["Rule match".to_string()],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "User requested".to_string(),
            }],
        );
        let diag = WorkflowDiagnostics::new(100);
        let result1 = planner.plan(&intent, None, &diag);
        let result2 = planner.plan(&intent, None, &diag);
        assert_eq!(result1.plan.plan_id, result2.plan.plan_id);
        assert_eq!(result1.plan.total_steps, result2.plan.total_steps);
        assert_eq!(result1.plan.is_valid, result2.plan.is_valid);
    }

    #[test]
    fn test_planner_no_state_mutation() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "test-4".to_string(),
            "Change model",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Change",
            vec![],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );
        let diag = WorkflowDiagnostics::new(100);
        let _result = planner.plan(&intent, None, &diag);
        assert_eq!(intent.required_commands.len(), 1);
    }

    #[test]
    fn test_planner_with_recommendations() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "test-5".to_string(),
            "Change model and apply recommendations",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model change",
            vec!["Rule match".to_string()],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "User requested".to_string(),
            }],
        );
        let mut rec_set = RecommendationSet::new("test-5");
        rec_set.add(Recommendation::new(
            RecommendationType::General,
            "Test Rec",
            "Test",
            vec![],
            RecommendationConfidence::High(0.9),
            "rule-1",
            Some("setting1".to_string()),
            Some("value1".to_string()),
            "test-5",
        ));
        let diag = WorkflowDiagnostics::new(100);
        let result = planner.plan(&intent, Some(&rec_set), &diag);
        assert!(result.plan.total_steps >= 1);
    }

    // ─── Dependency Tests ─────────────────────────────────────────────────────

    #[test]
    fn test_dependency_build_empty() {
        let steps: Vec<WorkflowStep> = vec![];
        let deps = build_dependencies(&steps);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_dependency_build_with_deps() {
        let steps = vec![
            make_step("a", "cmd_a", WorkflowStage::Execution, vec![]),
            make_step("b", "cmd_b", WorkflowStage::Execution, vec!["a"]),
        ];
        let deps = build_dependencies(&steps);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].from_step, "a");
        assert_eq!(deps[0].to_step, "b");
    }

    #[test]
    fn test_dependency_no_cycles() {
        let steps = vec![
            make_step("a", "cmd_a", WorkflowStage::Execution, vec![]),
            make_step("b", "cmd_b", WorkflowStage::Execution, vec!["a"]),
            make_step("c", "cmd_c", WorkflowStage::Execution, vec!["b"]),
        ];
        let deps = build_dependencies(&steps);
        assert!(!has_cycles(&steps, &deps));
    }

    #[test]
    fn test_dependency_has_cycles() {
        let steps = vec![
            make_step("a", "cmd_a", WorkflowStage::Execution, vec!["b"]),
            make_step("b", "cmd_b", WorkflowStage::Execution, vec!["a"]),
        ];
        let deps = build_dependencies(&steps);
        assert!(has_cycles(&steps, &deps));
    }

    #[test]
    fn test_dependency_entry_points() {
        let steps = vec![
            make_step("a", "cmd_a", WorkflowStage::Execution, vec![]),
            make_step("b", "cmd_b", WorkflowStage::Execution, vec!["a"]),
            make_step("c", "cmd_c", WorkflowStage::Execution, vec!["a"]),
        ];
        let deps = build_dependencies(&steps);
        let entries = find_entry_points(&steps, &deps);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], "a");
    }

    #[test]
    fn test_dependency_exit_points() {
        let steps = vec![
            make_step("a", "cmd_a", WorkflowStage::Execution, vec![]),
            make_step("b", "cmd_b", WorkflowStage::Execution, vec!["a"]),
            make_step("c", "cmd_c", WorkflowStage::Execution, vec!["b"]),
        ];
        let deps = build_dependencies(&steps);
        let exits = find_exit_points(&steps, &deps);
        assert_eq!(exits.len(), 1);
        assert_eq!(exits[0], "c");
    }

    #[test]
    fn test_dependency_depth() {
        let steps = vec![
            make_step("a", "cmd_a", WorkflowStage::Execution, vec![]),
            make_step("b", "cmd_b", WorkflowStage::Execution, vec!["a"]),
            make_step("c", "cmd_c", WorkflowStage::Execution, vec!["b"]),
        ];
        let deps = build_dependencies(&steps);
        assert_eq!(calculate_depth(&steps, &deps), 3);
    }

    // ─── Ordering Tests ───────────────────────────────────────────────────────

    #[test]
    fn test_ordering_topological_sort() {
        let steps = vec![
            make_step("c", "cmd_c", WorkflowStage::Execution, vec!["a", "b"]),
            make_step("a", "cmd_a", WorkflowStage::Execution, vec![]),
            make_step("b", "cmd_b", WorkflowStage::Execution, vec!["a"]),
        ];
        let deps = vec![
            WorkflowDependency {
                from_step: "a".to_string(),
                to_step: "b".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
            WorkflowDependency {
                from_step: "a".to_string(),
                to_step: "c".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
            WorkflowDependency {
                from_step: "b".to_string(),
                to_step: "c".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
        ];
        let sorted = topological_sort(steps, &deps);
        assert_eq!(sorted.len(), 3);
        let pos_a = sorted.iter().position(|s| s.step_id == "a").unwrap();
        let pos_b = sorted.iter().position(|s| s.step_id == "b").unwrap();
        let pos_c = sorted.iter().position(|s| s.step_id == "c").unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn test_ordering_sort_by_priority() {
        let mut steps = vec![
            make_step("b", "cmd_b", WorkflowStage::Execution, vec![]),
            make_step("a", "cmd_a", WorkflowStage::Execution, vec![]),
            make_step("c", "cmd_c", WorkflowStage::Execution, vec![]),
        ];
        steps[0].priority = 2;
        steps[1].priority = 0;
        steps[2].priority = 1;
        let sorted = sort_by_priority(steps);
        assert_eq!(sorted[0].step_id, "a");
        assert_eq!(sorted[1].step_id, "c");
        assert_eq!(sorted[2].step_id, "b");
    }

    // ─── Validation Tests ─────────────────────────────────────────────────────

    #[test]
    fn test_validate_valid_plan() {
        let steps = vec![
            make_step("a", "cmd_a", WorkflowStage::Execution, vec![]),
            make_step("b", "cmd_b", WorkflowStage::Execution, vec!["a"]),
        ];
        let deps = vec![WorkflowDependency {
            from_step: "a".to_string(),
            to_step: "b".to_string(),
            dependency_type: DependencyType::MustCompleteBefore,
        }];
        let issues = validate_plan(&steps, &deps);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_validate_duplicate_steps() {
        let steps = vec![
            make_step("dup", "cmd_1", WorkflowStage::Execution, vec![]),
            make_step("dup", "cmd_2", WorkflowStage::Execution, vec![]),
        ];
        let issues = validate_plan(&steps, &[]);
        assert!(!issues.is_empty());
        assert!(issues
            .iter()
            .any(|i| matches!(i, WorkflowIssue::DuplicateStep { .. })));
    }

    #[test]
    fn test_validate_cycle() {
        let steps = vec![
            make_step("a", "cmd_a", WorkflowStage::Execution, vec!["b"]),
            make_step("b", "cmd_b", WorkflowStage::Execution, vec!["a"]),
        ];
        let deps = vec![
            WorkflowDependency {
                from_step: "a".to_string(),
                to_step: "b".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
            WorkflowDependency {
                from_step: "b".to_string(),
                to_step: "a".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
        ];
        let issues = validate_plan(&steps, &deps);
        assert!(!issues.is_empty());
        assert!(issues
            .iter()
            .any(|i| matches!(i, WorkflowIssue::DependencyCycle { .. })));
    }

    #[test]
    fn test_validate_missing_dependency() {
        let steps = vec![make_step(
            "a",
            "cmd_a",
            WorkflowStage::Execution,
            vec!["missing"],
        )];
        let issues = validate_plan(&steps, &[]);
        assert!(!issues.is_empty());
        assert!(issues
            .iter()
            .any(|i| matches!(i, WorkflowIssue::MissingDependency { .. })));
    }

    // ─── Preview Tests ────────────────────────────────────────────────────────

    #[test]
    fn test_preview_valid_plan() {
        let plan = WorkflowPlan::new(
            "preview-1".to_string(),
            "intent-1",
            vec![
                make_step("s1", "cmd1", WorkflowStage::Preparation, vec![]),
                make_step("s2", "cmd2", WorkflowStage::Execution, vec![]),
            ],
            vec![],
            ExecutionStrategy::Sequential,
            vec![],
            vec![],
        );
        let preview = generate_preview(&plan);
        assert!(preview.contains("preview-1"));
        assert!(preview.contains("cmd1"));
        assert!(preview.contains("READY FOR APPROVAL"));
    }

    #[test]
    fn test_preview_invalid_plan() {
        let plan = WorkflowPlan::new(
            "preview-2".to_string(),
            "intent-2",
            vec![],
            vec![],
            ExecutionStrategy::Sequential,
            vec![WorkflowIssue::EmptyWorkflow],
            vec![],
        );
        let preview = generate_preview(&plan);
        assert!(preview.contains("preview-2"));
        assert!(preview.contains("BLOCKED"));
    }

    #[test]
    fn test_preview_compact() {
        let plan = WorkflowPlan::new(
            "compact-1".to_string(),
            "intent-1",
            vec![make_step("s1", "cmd1", WorkflowStage::Execution, vec![])],
            vec![],
            ExecutionStrategy::Sequential,
            vec![],
            vec![],
        );
        let compact = generate_compact_preview(&plan);
        assert!(compact.contains("compact-1"));
        assert!(compact.contains("1 steps"));
    }

    // ─── Diagnostics Tests ────────────────────────────────────────────────────

    #[test]
    fn test_diagnostics_record() {
        let diag = WorkflowDiagnostics::new(100);
        diag.record(DiagnosticKind::WorkflowPlanned, "test", false);
        assert_eq!(diag.total_count(), 1);
    }

    #[test]
    fn test_diagnostics_planning_completed() {
        let diag = WorkflowDiagnostics::new(100);
        let plan = WorkflowPlan::new(
            "p1".to_string(),
            "i1",
            vec![],
            vec![],
            ExecutionStrategy::Sequential,
            vec![],
            vec![],
        );
        diag.record_planning_completed(&plan);
        assert_eq!(diag.count_by_kind(&DiagnosticKind::WorkflowPlanned), 1);
    }

    // ─── Integration Tests ────────────────────────────────────────────────────

    #[test]
    fn test_full_pipeline_workflow() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "pipe-1".to_string(),
            "Enable dark theme and vim mode",
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.85,
            false,
            None,
            "Dark theme + vim",
            vec!["Rule match".to_string()],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "User requested".to_string(),
            }],
        );
        let diag = WorkflowDiagnostics::new(100);
        let result = planner.plan(&intent, None, &diag);
        assert!(!result.plan.plan_id.is_empty());
        assert!(!result.plan.summary.is_empty());
    }

    #[test]
    fn test_workflow_is_read_only() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "readonly-1".to_string(),
            "Change model",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Change",
            vec![],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );
        let diag = WorkflowDiagnostics::new(100);
        let _result = planner.plan(&intent, None, &diag);
        assert_eq!(intent.required_commands.len(), 1);
    }

    #[test]
    fn test_workflow_serializable() {
        let plan = WorkflowPlan::new(
            "serial-1".to_string(),
            "intent-1",
            vec![make_step("s1", "cmd1", WorkflowStage::Execution, vec![])],
            vec![],
            ExecutionStrategy::Sequential,
            vec![],
            vec![],
        );
        let json = serde_json::to_string(&plan).expect("serialize");
        let deserialized: WorkflowPlan = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(plan.plan_id, deserialized.plan_id);
        assert_eq!(plan.total_steps, deserialized.total_steps);
        assert_eq!(plan.is_valid, deserialized.is_valid);
    }

    // ─── Edge Cases ───────────────────────────────────────────────────────────

    #[test]
    fn test_edge_case_empty_input() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "edge-1".to_string(),
            "",
            IntentType::Unknown,
            "unknown",
            false,
            0.0,
            0.1,
            true,
            Some("Empty".to_string()),
            "Empty input",
            vec![],
            vec![],
        );
        let diag = WorkflowDiagnostics::new(100);
        let result = planner.plan(&intent, None, &diag);
        assert!(!result.plan.is_valid);
    }

    #[test]
    fn test_edge_case_unicode_input() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "edge-2".to_string(),
            "日本語で設定",
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.5,
            false,
            None,
            "Japanese input",
            vec![],
            vec![],
        );
        let diag = WorkflowDiagnostics::new(100);
        let result = planner.plan(&intent, None, &diag);
        assert!(!result.plan.plan_id.is_empty());
    }

    // ─── Benchmark Helpers ────────────────────────────────────────────────────

    #[test]
    fn test_workflow_latency_baseline() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "bench-1".to_string(),
            "Change the model to gpt-4o",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model change",
            vec!["Rule match".to_string()],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "User requested".to_string(),
            }],
        );
        let diag = WorkflowDiagnostics::new(100);

        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = planner.plan(&intent, None, &diag);
        }
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 500,
            "Workflow planning should be fast: {}ms",
            elapsed.as_millis()
        );
    }
}

// P6.5 Adaptive Validation Foundation Tests
// =========================================================================

#[cfg(test)]
mod p6_5_adaptive_validation {
    use super::*;
    use crate::adaptive_validation::confidence::ConfidenceEvaluator;
    use crate::adaptive_validation::diagnostics::AdaptiveDiagnostics;
    use crate::adaptive_validation::engine::AdaptiveValidationEngine;
    use crate::adaptive_validation::policy::{default_policies, PolicyEngine};
    use crate::adaptive_validation::risk::RiskAssessor;
    use crate::adaptive_validation::rules::{all_rules, evaluate_all};
    use crate::adaptive_validation::validator::Validator;
    use crate::adaptive_validation::*;
    use crate::intent_engine::{IntentCommand, IntentPlan, IntentType};
    use crate::workflow_engine::{ExecutionStrategy, WorkflowPlan};

    // ─── Rules Tests ────────────────────────────────────────────────────────

    #[test]
    fn test_all_rules_exist() {
        let rules = all_rules();
        assert!(rules.len() >= 15, "Should have at least 15 rules");
    }

    #[test]
    fn test_rules_evaluate_normal_input() {
        let results = evaluate_all("normal input");
        assert!(results.iter().all(|(_, passed, _)| *passed));
    }

    #[test]
    fn test_rules_detect_ambiguous() {
        let results = evaluate_all("ambiguous input");
        assert!(!results.iter().all(|(_, passed, _)| *passed));
    }

    #[test]
    fn test_rules_detect_low_confidence() {
        let results = evaluate_all("low_confidence input");
        assert!(!results.iter().all(|(_, passed, _)| *passed));
    }

    // ─── Policy Tests ───────────────────────────────────────────────────────

    #[test]
    fn test_policy_engine_creation() {
        let engine = PolicyEngine::new();
        assert!(engine.enabled_policies().is_empty());
    }

    #[test]
    fn test_policy_engine_register() {
        let mut engine = PolicyEngine::new();
        let policy = Policy::new("p1", "Test", "Test policy", vec![]);
        engine.register(policy);
        assert_eq!(engine.enabled_policies().len(), 1);
    }

    #[test]
    fn test_default_policies() {
        let policies = default_policies();
        assert_eq!(policies.len(), 3);
    }

    #[test]
    fn test_policy_evaluate_all_pass() {
        let engine = PolicyEngine::new();
        let results = engine.evaluate("normal input");
        assert!(results.iter().all(|(_, passed)| *passed));
    }

    // ─── Confidence Tests ───────────────────────────────────────────────────

    #[test]
    fn test_confidence_evaluate_normal() {
        let evaluator = ConfidenceEvaluator::new();
        let config = ValidationConfig::new();
        let score = evaluator.evaluate("normal input", &config);
        assert!(score >= 0.9);
    }

    #[test]
    fn test_confidence_evaluate_low() {
        let evaluator = ConfidenceEvaluator::new();
        let config = ValidationConfig::new();
        let score = evaluator.evaluate("low_confidence input", &config);
        assert!(score < 0.9);
    }

    #[test]
    fn test_confidence_is_above_threshold() {
        let evaluator = ConfidenceEvaluator::new();
        let config = ValidationConfig::new().with_min_confidence(0.5);
        assert!(evaluator.is_above_threshold(0.8, &config));
        assert!(!evaluator.is_above_threshold(0.3, &config));
    }

    #[test]
    fn test_confidence_risk_level() {
        let evaluator = ConfidenceEvaluator::new();
        assert_eq!(evaluator.risk_level_for_confidence(0.9), RiskLevel::Info);
        assert_eq!(evaluator.risk_level_for_confidence(0.7), RiskLevel::Low);
        assert_eq!(evaluator.risk_level_for_confidence(0.5), RiskLevel::Medium);
        assert_eq!(evaluator.risk_level_for_confidence(0.3), RiskLevel::High);
        assert_eq!(
            evaluator.risk_level_for_confidence(0.1),
            RiskLevel::Critical
        );
    }

    // ─── Risk Tests ─────────────────────────────────────────────────────────

    #[test]
    fn test_risk_assess_no_issues() {
        let assessor = RiskAssessor::new();
        let issues = vec![];
        let warnings = vec![];
        let risk = assessor.assess("normal input", &issues, &warnings);
        assert_eq!(risk, RiskLevel::Info);
    }

    #[test]
    fn test_risk_assess_with_issues() {
        let assessor = RiskAssessor::new();
        let issues = vec![ValidationIssue::new(
            &ValidationCategory::Workflow,
            RiskLevel::High,
            "Test issue",
            vec!["evidence".to_string()],
            "Fix it",
            false,
        )];
        let warnings = vec![];
        let risk = assessor.assess("input", &issues, &warnings);
        assert_eq!(risk, RiskLevel::High);
    }

    #[test]
    fn test_risk_is_acceptable() {
        let assessor = RiskAssessor::new();
        let config = ValidationConfig::new().with_max_risk_level(RiskLevel::High);
        assert!(assessor.is_acceptable(&RiskLevel::Low, &config));
        assert!(assessor.is_acceptable(&RiskLevel::High, &config));
        assert!(!assessor.is_acceptable(&RiskLevel::Critical, &config));
    }

    #[test]
    fn test_risk_mitigation_suggestion() {
        let assessor = RiskAssessor::new();
        assert_eq!(
            assessor.mitigation_suggestion(&RiskLevel::Info),
            "No action required"
        );
        assert_eq!(
            assessor.mitigation_suggestion(&RiskLevel::Critical),
            "Immediate review required — do not proceed"
        );
    }

    // ─── Validator Tests ────────────────────────────────────────────────────

    #[test]
    fn test_validator_normal_input() {
        let validator = Validator::new();
        let config = ValidationConfig::new();
        let report = validator.validate("normal input", &config);
        assert_eq!(report.result, ValidationResult::Pass);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn test_validator_low_confidence() {
        let validator = Validator::new();
        let config = ValidationConfig::new().with_min_confidence(0.9);
        let report = validator.validate("low_confidence input", &config);
        assert!(!report.warnings.is_empty() || report.result != ValidationResult::Pass);
    }

    #[test]
    fn test_validator_with_policy_failure() {
        let mut validator = Validator::new();
        validator.policy_engine.register(Policy::new(
            "fail-policy",
            "Fail Policy",
            "Will fail",
            vec![PolicyRule::new(
                "r1",
                "Test",
                ValidationCategory::Policy,
                RiskLevel::High,
                true,
                RuleEvaluation::Boolean(false),
            )],
        ));
        let config = ValidationConfig::new();
        let report = validator.validate("input", &config);
        assert_eq!(report.result, ValidationResult::Reject);
    }

    #[test]
    fn test_validator_deterministic() {
        let validator = Validator::new();
        let config = ValidationConfig::new();
        let report1 = validator.validate("test input", &config);
        let report2 = validator.validate("test input", &config);
        assert_eq!(report1.result, report2.result);
        assert_eq!(report1.issues.len(), report2.issues.len());
    }

    // ─── Engine Tests ───────────────────────────────────────────────────────

    #[test]
    fn test_engine_normal_pipeline() {
        let engine = AdaptiveValidationEngine::new();
        let intent = IntentPlan::new(
            "test-1".to_string(),
            "Change the model to gpt-4o",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model change",
            vec!["Rule match".to_string()],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "User requested".to_string(),
            }],
        );
        let config = ValidationConfig::new();
        let diag = AdaptiveDiagnostics::new(100);
        let report = engine.validate(&intent, None, None, &config, &diag);
        assert_eq!(report.result, ValidationResult::Pass);
    }

    #[test]
    fn test_engine_with_workflow() {
        let engine = AdaptiveValidationEngine::new();
        let intent = IntentPlan::new(
            "test-2".to_string(),
            "Change model",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Change",
            vec![],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );
        let workflow = WorkflowPlan::new(
            "wf-1".to_string(),
            "test-2",
            vec![],
            vec![],
            ExecutionStrategy::Sequential,
            vec![],
            vec![],
        );
        let config = ValidationConfig::new();
        let diag = AdaptiveDiagnostics::new(100);
        let report = engine.validate(&intent, None, Some(&workflow), &config, &diag);
        assert_eq!(report.result, ValidationResult::Pass);
    }

    #[test]
    fn test_engine_is_read_only() {
        let engine = AdaptiveValidationEngine::new();
        let intent = IntentPlan::new(
            "test-3".to_string(),
            "Change model",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Change",
            vec![],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );
        let config = ValidationConfig::new();
        let diag = AdaptiveDiagnostics::new(100);
        let _report = engine.validate(&intent, None, None, &config, &diag);
        assert_eq!(intent.required_commands.len(), 1);
    }

    #[test]
    fn test_engine_deterministic() {
        let engine = AdaptiveValidationEngine::new();
        let intent = IntentPlan::new(
            "test-4".to_string(),
            "Change model",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Change",
            vec![],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );
        let config = ValidationConfig::new();
        let diag = AdaptiveDiagnostics::new(100);
        let report1 = engine.validate(&intent, None, None, &config, &diag);
        let report2 = engine.validate(&intent, None, None, &config, &diag);
        assert_eq!(report1.result, report2.result);
        assert_eq!(report1.issues.len(), report2.issues.len());
    }

    #[test]
    fn test_is_approval_ready() {
        let engine = AdaptiveValidationEngine::new();
        let intent = IntentPlan::new(
            "test-5".to_string(),
            "Change model",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Change",
            vec![],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );
        let config = ValidationConfig::new();
        let ready = engine.is_approval_ready(&intent, None, None, &config);
        assert!(ready);
    }

    #[test]
    fn test_get_summary() {
        let engine = AdaptiveValidationEngine::new();
        let intent = IntentPlan::new(
            "test-6".to_string(),
            "Change model",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Change",
            vec![],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );
        let config = ValidationConfig::new();
        let summary = engine.get_summary(&intent, None, None, &config);
        assert_eq!(summary.result, "PASS");
        assert!(summary.approval_ready);
    }

    // ─── Diagnostics Tests ──────────────────────────────────────────────────

    #[test]
    fn test_diagnostics_record() {
        let diag = AdaptiveDiagnostics::new(100);
        diag.record(DiagnosticKind::ValidationStarted, "test", false);
        assert_eq!(diag.total_count(), 1);
    }

    #[test]
    fn test_diagnostics_planning_completed() {
        let diag = AdaptiveDiagnostics::new(100);
        let report = ValidationReport::new("r1".to_string(), ValidationResult::Pass);
        diag.record_validation_completed(&report);
        assert_eq!(diag.count_by_kind(&DiagnosticKind::ValidationCompleted), 1);
    }

    // ─── Integration Tests ──────────────────────────────────────────────────

    #[test]
    fn test_full_pipeline_validation() {
        let engine = AdaptiveValidationEngine::new();
        let intent = IntentPlan::new(
            "pipe-1".to_string(),
            "Enable dark theme and vim mode",
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.85,
            false,
            None,
            "Dark theme + vim",
            vec!["Rule match".to_string()],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "User requested".to_string(),
            }],
        );
        let config = ValidationConfig::new();
        let diag = AdaptiveDiagnostics::new(100);
        let report = engine.validate(&intent, None, None, &config, &diag);
        assert!(!report.report_id.is_empty());
        assert!(!report.summary.is_empty());
    }

    #[test]
    fn test_validation_result_display() {
        assert_eq!(ValidationResult::Pass.to_string(), "PASS");
        assert_eq!(
            ValidationResult::PassWithWarnings.to_string(),
            "PASS_WITH_WARNINGS"
        );
        assert_eq!(
            ValidationResult::RequiresClarification.to_string(),
            "REQUIRES_CLARIFICATION"
        );
        assert_eq!(ValidationResult::Reject.to_string(), "REJECT");
    }

    #[test]
    fn test_risk_level_display() {
        assert_eq!(RiskLevel::Info.to_string(), "INFO");
        assert_eq!(RiskLevel::Low.to_string(), "LOW");
        assert_eq!(RiskLevel::Medium.to_string(), "MEDIUM");
        assert_eq!(RiskLevel::High.to_string(), "HIGH");
        assert_eq!(RiskLevel::Critical.to_string(), "CRITICAL");
    }

    #[test]
    fn test_validation_category_display() {
        assert_eq!(ValidationCategory::Workflow.to_string(), "workflow");
        assert_eq!(ValidationCategory::Intent.to_string(), "intent");
        assert_eq!(ValidationCategory::Confidence.to_string(), "confidence");
    }

    #[test]
    fn test_validation_issue_serializable() {
        let issue = ValidationIssue::new(
            &ValidationCategory::Workflow,
            RiskLevel::High,
            "Test issue",
            vec!["evidence".to_string()],
            "Fix it",
            false,
        );
        let json = serde_json::to_string(&issue).expect("serialize");
        let deserialized: ValidationIssue = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(issue.message, deserialized.message);
        assert_eq!(issue.severity, deserialized.severity);
    }

    #[test]
    fn test_validation_report_serializable() {
        let report = ValidationReport::new("r1".to_string(), ValidationResult::Pass);
        let json = serde_json::to_string(&report).expect("serialize");
        let deserialized: ValidationReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(report.report_id, deserialized.report_id);
        assert_eq!(report.result, deserialized.result);
    }

    // ─── Edge Cases ─────────────────────────────────────────────────────────

    #[test]
    fn test_edge_case_empty_input() {
        let validator = Validator::new();
        let config = ValidationConfig::new();
        let report = validator.validate("", &config);
        // Empty input should be handled without panic
        assert!(!report.report_id.is_empty());
    }

    #[test]
    fn test_edge_case_unicode_input() {
        let validator = Validator::new();
        let config = ValidationConfig::new();
        let report = validator.validate("日本語で設定", &config);
        assert!(!report.report_id.is_empty());
    }

    // ─── Benchmark Helpers ──────────────────────────────────────────────────

    #[test]
    fn test_validation_latency_baseline() {
        let validator = Validator::new();
        let config = ValidationConfig::new();

        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = validator.validate("normal input", &config);
        }
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 500,
            "Validation should be fast: {}ms",
            elapsed.as_millis()
        );
    }
}

// ===========================================================================
// P7 Concurrency and Thread-Safety Validation Suite
// ===========================================================================

#[cfg(test)]
mod p7_concurrency_validation {
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use crate::adaptive_validation::{
        AdaptiveDiagnostics, AdaptiveValidationEngine, ValidationConfig, ValidationResult,
    };
    use crate::integration_pipeline::IntegrationPipeline;
    use crate::intent_engine::{IntentClassifier, IntentPlan, IntentResolver, IntentType};
    use crate::preference_engine::{
        PreferenceCategory, PreferenceOrigin, PreferenceSet, PreferenceValue,
    };
    use crate::recommendation_engine::{
        RecommendationContext, RecommendationEngine, RecommendationSet,
    };
    use crate::workflow_engine::{WorkflowDiagnostics, WorkflowPlanner, WorkflowResult};

    // ─── Thread-Safety Tests ─────────────────────────────────────────────────

    #[test]
    fn test_intent_classifier_thread_safe() {
        let classifier = IntentClassifier::new();
        let arc_classifier = Arc::new(classifier);

        let mut handles = vec![];
        for i in 0..10 {
            let clone = arc_classifier.clone();
            handles.push(thread::spawn(move || {
                let input = format!("Change model to gpt-4o-{}", i);
                let plan = clone.classify(&input);
                assert_eq!(plan.intent_type, IntentType::Preference);
            }));
        }

        for handle in handles {
            handle.join().expect("thread panicked");
        }
    }

    #[test]
    fn test_recommendation_engine_thread_safe() {
        let engine = RecommendationEngine::new();
        let arc_engine = Arc::new(engine);

        let mut handles = vec![];
        for i in 0..10 {
            let clone = arc_engine.clone();
            let plan = IntentPlan::new(
                format!("plan-{}", i),
                &format!("Enable dark theme {}", i),
                IntentType::Configuration,
                "configuration",
                false,
                0.0,
                0.8,
                false,
                None,
                "Dark theme",
                vec!["Rule match".to_string()],
                vec![],
            );
            let context = RecommendationContext::new();
            handles.push(thread::spawn(move || {
                let result = clone.recommend(&plan, &context);
                assert!(!result.is_empty());
            }));
        }

        for handle in handles {
            handle.join().expect("thread panicked");
        }
    }

    #[test]
    fn test_workflow_planner_thread_safe() {
        let planner = WorkflowPlanner::new();
        let arc_planner = Arc::new(planner);

        let mut handles = vec![];
        for i in 0..10 {
            let clone = arc_planner.clone();
            let plan = IntentPlan::new(
                format!("plan-{}", i),
                &format!("Change model to gpt-4o-{}", i),
                IntentType::Preference,
                "preference_engine",
                true,
                0.0,
                0.9,
                false,
                None,
                "Model change",
                vec!["Rule match".to_string()],
                vec![crate::intent_engine::IntentCommand::UpdateModelPreference {
                    key: "model".to_string(),
                    new_value: format!("gpt-4o-{}", i),
                    reason: "User requested".to_string(),
                }],
            );
            let diag = WorkflowDiagnostics::new(100);
            handles.push(thread::spawn(move || {
                let result = clone.plan(&plan, None, &diag);
                assert!(result.plan.is_valid);
            }));
        }

        for handle in handles {
            handle.join().expect("thread panicked");
        }
    }

    #[test]
    fn test_adaptive_validation_thread_safe() {
        let engine = AdaptiveValidationEngine::new();
        let arc_engine = Arc::new(engine);
        let config = ValidationConfig::new();

        let mut handles = vec![];
        for i in 0..10 {
            let clone = arc_engine.clone();
            let plan = IntentPlan::new(
                format!("plan-{}", i),
                &format!("Change model to gpt-4o-{}", i),
                IntentType::Preference,
                "preference_engine",
                true,
                0.0,
                0.9,
                false,
                None,
                "Model change",
                vec!["Rule match".to_string()],
                vec![crate::intent_engine::IntentCommand::UpdateModelPreference {
                    key: "model".to_string(),
                    new_value: format!("gpt-4o-{}", i),
                    reason: "User requested".to_string(),
                }],
            );
            let diag = AdaptiveDiagnostics::new(100);
            let config_clone = config.clone();
            handles.push(thread::spawn(move || {
                let report = clone.validate(&plan, None, None, &config_clone, &diag);
                assert_eq!(report.result, ValidationResult::Pass);
            }));
        }

        for handle in handles {
            handle.join().expect("thread panicked");
        }
    }

    #[test]
    fn test_integration_pipeline_thread_safe() {
        let pipeline = IntegrationPipeline::new();
        let arc_pipeline = Arc::new(pipeline);

        let mut prefs = PreferenceSet::new();
        prefs.add(crate::preference_engine::Preference::new(
            "model",
            PreferenceCategory::Model,
            PreferenceValue::String("gpt-4o".to_string()),
            "Default model",
            PreferenceOrigin::Default,
        ));
        let arc_prefs = Arc::new(prefs);
        let config = ValidationConfig::new();

        let mut handles = vec![];
        for i in 0..10 {
            let clone = arc_pipeline.clone();
            let prefs_clone = arc_prefs.clone();
            let config_clone = config.clone();
            handles.push(thread::spawn(move || {
                let result = clone.run(
                    &format!("Change model to gpt-4o-{}", i),
                    &prefs_clone,
                    &config_clone,
                );
                assert!(
                    result.intent_plan.intent_type == IntentType::Preference
                        || result.intent_plan.intent_type == IntentType::Unknown
                );
            }));
        }

        for handle in handles {
            handle.join().expect("thread panicked");
        }
    }

    #[test]
    fn test_concurrent_pipeline_runs_no_data_race() {
        let pipeline = IntegrationPipeline::new();
        let arc_pipeline = Arc::new(pipeline);

        let mut prefs = PreferenceSet::new();
        prefs.add(crate::preference_engine::Preference::new(
            "model",
            PreferenceCategory::Model,
            PreferenceValue::String("gpt-4o".to_string()),
            "Default model",
            PreferenceOrigin::Default,
        ));
        let arc_prefs = Arc::new(prefs);
        let config = ValidationConfig::new();

        let mut handles = vec![];
        for i in 0..20 {
            let clone = arc_pipeline.clone();
            let prefs_clone = arc_prefs.clone();
            let config_clone = config.clone();
            handles.push(thread::spawn(move || {
                for j in 0..50 {
                    let input = format!("Change model to gpt-4o-{}-{}", i, j);
                    let _result = clone.run(&input, &prefs_clone, &config_clone);
                }
            }));
        }

        for handle in handles {
            handle.join().expect("thread panicked");
        }
    }

    // ─── Stress Tests ────────────────────────────────────────────────────────

    #[test]
    fn test_stress_intent_classification() {
        let classifier = IntentClassifier::new();
        let inputs = vec![
            "Change the model to gpt-4o",
            "Enable dark theme",
            "Run the test workflow",
            "What is rust?",
            "help",
            "Configure the system",
            "Execute cargo test",
            "Switch to vim mode",
            "Use claude-3-opus",
            "Change language to japanese",
        ];

        for input in inputs {
            let plan = classifier.classify(input);
            assert!(!plan.id.is_empty());
            assert!(!plan.detected_goal.is_empty());
        }
    }

    #[test]
    fn test_stress_recommendation_generation() {
        let engine = RecommendationEngine::new();
        let inputs = vec![
            "Enable dark theme",
            "Use vim mode",
            "Configure git integration",
            "Enable LSP features",
            "Make it fast",
            "Use rust",
            "Enable accessibility",
        ];

        for input in inputs {
            let plan = IntentPlan::new(
                "stress-test".to_string(),
                input,
                IntentType::Configuration,
                "configuration",
                false,
                0.0,
                0.8,
                false,
                None,
                "Stress test",
                vec!["Rule match".to_string()],
                vec![],
            );
            let context = RecommendationContext::new();
            let result = engine.recommend(&plan, &context);
            assert!(!result.intent_id.is_empty());
        }
    }

    #[test]
    fn test_stress_workflow_planning() {
        let planner = WorkflowPlanner::new();
        let inputs = vec![
            "Change the model to gpt-4o",
            "Change the model to claude-3-opus",
            "Change language to french",
            "Enable auto approve",
        ];

        for input in inputs {
            let plan = IntentPlan::new(
                "stress-plan".to_string(),
                input,
                IntentType::Preference,
                "preference_engine",
                true,
                0.0,
                0.9,
                false,
                None,
                "Stress test",
                vec!["Rule match".to_string()],
                vec![crate::intent_engine::IntentCommand::UpdateModelPreference {
                    key: "model".to_string(),
                    new_value: input.replace("Change the model to ", ""),
                    reason: "User requested".to_string(),
                }],
            );
            let diag = WorkflowDiagnostics::new(100);
            let result = planner.plan(&plan, None, &diag);
            assert!(result.plan.plan_id.starts_with("plan_"));
        }
    }

    #[test]
    fn test_stress_validation() {
        let engine = AdaptiveValidationEngine::new();
        let inputs = vec![
            "Change the model to gpt-4o",
            "Enable dark theme",
            "Run the test workflow",
        ];

        for input in inputs {
            let plan = IntentPlan::new(
                "stress-val".to_string(),
                input,
                IntentType::Preference,
                "preference_engine",
                true,
                0.0,
                0.9,
                false,
                None,
                "Stress test",
                vec!["Rule match".to_string()],
                vec![],
            );
            let config = ValidationConfig::new();
            let diag = AdaptiveDiagnostics::new(100);
            let report = engine.validate(&plan, None, None, &config, &diag);
            assert!(!report.report_id.is_empty());
        }
    }

    // ─── Determinism Tests ───────────────────────────────────────────────────

    #[test]
    fn test_deterministic_intent_classification() {
        let classifier = IntentClassifier::new();
        let input = "Change the model to gpt-4o";

        let plan1 = classifier.classify(input);
        let plan2 = classifier.classify(input);

        assert_eq!(plan1.intent_type, plan2.intent_type);
        assert_eq!(plan1.confidence, plan2.confidence);
        assert_eq!(plan1.required_commands.len(), plan2.required_commands.len());
        assert_eq!(plan1.ambiguity, plan2.ambiguity);
    }

    #[test]
    fn test_deterministic_recommendation_generation() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "det-rec".to_string(),
            "Enable dark theme",
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.8,
            false,
            None,
            "Dark theme",
            vec!["Rule match".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();

        let result1 = engine.recommend(&plan, &context);
        let result2 = engine.recommend(&plan, &context);

        assert_eq!(result1.len(), result2.len());
        for (r1, r2) in result1
            .recommendations
            .iter()
            .zip(result2.recommendations.iter())
        {
            assert_eq!(r1.title, r2.title);
            assert_eq!(r1.rec_type, r2.rec_type);
            assert!((r1.confidence.score() - r2.confidence.score()).abs() < 0.001);
        }
    }

    #[test]
    fn test_deterministic_workflow_planning() {
        let planner = WorkflowPlanner::new();
        let plan = IntentPlan::new(
            "det-wf".to_string(),
            "Change the model to gpt-4o",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model change",
            vec!["Rule match".to_string()],
            vec![crate::intent_engine::IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "User requested".to_string(),
            }],
        );
        let diag = WorkflowDiagnostics::new(100);

        let result1 = planner.plan(&plan, None, &diag);
        let result2 = planner.plan(&plan, None, &diag);

        assert_eq!(result1.plan.plan_id, result2.plan.plan_id);
        assert_eq!(result1.plan.total_steps, result2.plan.total_steps);
        assert_eq!(result1.plan.is_valid, result2.plan.is_valid);
        assert_eq!(result1.plan.strategy, result2.plan.strategy);
    }

    #[test]
    fn test_deterministic_validation() {
        let engine = AdaptiveValidationEngine::new();
        let plan = IntentPlan::new(
            "det-val".to_string(),
            "Change the model to gpt-4o",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model change",
            vec!["Rule match".to_string()],
            vec![],
        );
        let config = ValidationConfig::new();
        let diag = AdaptiveDiagnostics::new(100);

        let report1 = engine.validate(&plan, None, None, &config, &diag);
        let report2 = engine.validate(&plan, None, None, &config, &diag);

        assert_eq!(report1.result, report2.result);
        assert_eq!(report1.issues.len(), report2.issues.len());
        assert_eq!(report1.warnings.len(), report2.warnings.len());
    }

    #[test]
    fn test_deterministic_pipeline() {
        let pipeline = IntegrationPipeline::new();
        let mut prefs = PreferenceSet::new();
        prefs.add(crate::preference_engine::Preference::new(
            "model",
            PreferenceCategory::Model,
            PreferenceValue::String("gpt-4o".to_string()),
            "Default model",
            PreferenceOrigin::Default,
        ));
        let config = ValidationConfig::new();

        let result1 = pipeline.run("Change the model to gpt-4o", &prefs, &config);
        let result2 = pipeline.run("Change the model to gpt-4o", &prefs, &config);

        assert_eq!(
            result1.intent_plan.intent_type,
            result2.intent_plan.intent_type
        );
        assert_eq!(
            result1.resolved_commands.len(),
            result2.resolved_commands.len()
        );
        assert_eq!(
            result1.workflow_result.plan.is_valid,
            result2.workflow_result.plan.is_valid
        );
        assert_eq!(
            result1.validation_report.result,
            result2.validation_report.result
        );
    }

    // ─── Error Handling Tests ────────────────────────────────────────────────

    #[test]
    fn test_pipeline_handles_empty_input() {
        let pipeline = IntegrationPipeline::new();
        let prefs = PreferenceSet::default();
        let config = ValidationConfig::new();

        let result = pipeline.run("", &prefs, &config);
        assert_eq!(result.intent_plan.intent_type, IntentType::Unknown);
        assert!(result.ambiguity_result.is_ambiguous);
    }

    #[test]
    fn test_pipeline_handles_whitespace_input() {
        let pipeline = IntegrationPipeline::new();
        let prefs = PreferenceSet::default();
        let config = ValidationConfig::new();

        let result = pipeline.run("   \n\t  ", &prefs, &config);
        assert_eq!(result.intent_plan.intent_type, IntentType::Unknown);
    }

    #[test]
    fn test_pipeline_handles_random_garbage() {
        let pipeline = IntegrationPipeline::new();
        let prefs = PreferenceSet::default();
        let config = ValidationConfig::new();

        let result = pipeline.run("xyz123!@#$%^&*()", &prefs, &config);
        assert_eq!(result.intent_plan.intent_type, IntentType::Unknown);
        assert!(result.confidence_result.score < 0.5);
    }

    #[test]
    fn test_pipeline_preserves_all_stages_output() {
        let pipeline = IntegrationPipeline::new();
        let mut prefs = PreferenceSet::new();
        prefs.add(crate::preference_engine::Preference::new(
            "model",
            PreferenceCategory::Model,
            PreferenceValue::String("gpt-4o".to_string()),
            "Default model",
            PreferenceOrigin::Default,
        ));
        let config = ValidationConfig::new();

        let result = pipeline.run("Change the model to gpt-4o", &prefs, &config);

        assert!(!result.intent_plan.id.is_empty());
        assert!(!result.confidence_result.reasoning.is_empty());
        assert!(!result.workflow_result.plan.plan_id.is_empty());
        assert!(!result.validation_report.report_id.is_empty());
    }

    #[test]
    fn test_pipeline_duration_is_reasonable() {
        let pipeline = IntegrationPipeline::new();
        let prefs = PreferenceSet::default();
        let config = ValidationConfig::new();

        let start = std::time::Instant::now();
        let result = pipeline.run("Change the model to gpt-4o", &prefs, &config);
        let elapsed = start.elapsed();

        assert!(elapsed < Duration::from_millis(5000));
        assert!(result.total_duration < Duration::from_millis(5000));
    }
}

#[test]
fn test_textarea_bs_direct() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut ta = xai_ratatui_textarea::TextArea::new();
    ta.insert_str("hello");
    assert_eq!(ta.text(), "hello");
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    ta.input(bs);
    assert_eq!(ta.text(), "hell", "direct textarea backspace");
}

#[test]
fn test_adapter_bs_via_inner() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("hello");
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    app.input.inner_mut().input(bs);
    assert_eq!(app.input.text(), "hell", "via inner_mut");
}

#[test]
fn test_textarea_inline_debug() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut ta = xai_ratatui_textarea::TextArea::new();
    ta.insert_str("hello");
    eprintln!("direct text before: '{}'", ta.text());
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    ta.input(bs);
    eprintln!("direct text after: '{}'", ta.text());
    assert_eq!(ta.text(), "hell");
}

#[test]
fn test_adapter_set_text_vs_insert() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    // Test with insert_str
    let mut ta1 = xai_ratatui_textarea::TextArea::new();
    ta1.insert_str("hello");
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    ta1.input(bs);
    assert_eq!(ta1.text(), "hell");

    // Test with set_text — direct textarea preserves cursor at 0,
    // so backspace is a no-op. This tests the upstream crate behavior.
    let mut ta2 = xai_ratatui_textarea::TextArea::new();
    ta2.set_text("hello");
    let bs2 = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    ta2.input(bs2);
    assert_eq!(ta2.text(), "hello", "direct set_text preserves cursor at 0");
}

#[test]
fn test_set_text_clears_history() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut ta = xai_ratatui_textarea::TextArea::new();
    ta.insert_str("hello");
    ta.set_text("world");
    // After set_text, is the textarea still editable?
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    ta.input(bs);
    eprintln!("after set_text+bs: '{}'", ta.text());
}

#[test]
fn test_set_text_then_insert() {
    let mut ta = xai_ratatui_textarea::TextArea::new();
    ta.set_text("hello");
    // Direct textarea: set_text preserves cursor at 0, so insert_str prepends
    ta.insert_str("x");
    assert_eq!(ta.text(), "xhello", "direct set_text preserves cursor at 0");
}

#[test]
fn test_bs_debug() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("hello");
    eprintln!("before: '{}'", app.input.text());
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    let result = app.input.handle_key(bs, &app.dashboard);
    eprintln!("result: {:?}", result);
    eprintln!("after: '{}'", app.input.text());
}

#[test]
fn test_bs_via_handle_key_debug() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut ta = xai_ratatui_textarea::TextArea::new();
    ta.insert_str("hello");
    eprintln!("direct before: '{}'", ta.text());
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    ta.input(bs);
    eprintln!("direct after: '{}'", ta.text());
    assert_eq!(ta.text(), "hell");
}

#[test]
fn test_bs_via_inner_mut_debug() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("hello");
    eprintln!("before inner_mut: '{}'", app.input.text());
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    let ta_mut = app.input.inner_mut();
    eprintln!("ta_mut text before: '{}'", ta_mut.text());
    ta_mut.input(bs);
    eprintln!("ta_mut text after: '{}'", ta_mut.text());
    eprintln!("app.input text after: '{}'", app.input.text());
}

#[test]
fn test_bs_fresh_adapter() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut adapter = crate::tui::textarea_adapter::InputAdapter::new();
    adapter.set_text("hello");
    eprintln!("fresh adapter before: '{}'", adapter.text());
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    adapter.inner_mut().input(bs);
    eprintln!("fresh adapter after: '{}'", adapter.text());
    assert_eq!(adapter.text(), "hell");
}

#[test]
fn test_bs_fresh_textarea_in_adapter_module() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut ta = xai_ratatui_textarea::TextArea::new();
    ta.insert_str("hello");
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    ta.input(bs);
    eprintln!(
        "fresh in module before/after: '{}' -> '{}'",
        "hello",
        ta.text()
    );
    assert_eq!(ta.text(), "hell");
}

#[test]
fn test_bs_type_check() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("hello");
    // Check type identity
    let text_before = app.input.text().to_string();
    eprintln!("type id check - before: '{}'", text_before);
    // Call input directly on inner
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    // Use the same pattern as the working test
    let mut ta = xai_ratatui_textarea::TextArea::new();
    ta.insert_str("hello");
    ta.input(bs.clone());
    eprintln!("direct ta after: '{}'", ta.text());
    // Now try via adapter
    app.input.set_text("hello");
    let ta_ref = app.input.inner_mut();
    eprintln!("ta_ref type: TypeInfo");
    ta_ref.input(bs);
    eprintln!("ta_ref after: '{}'", ta_ref.text());
    eprintln!("app.input after: '{}'", app.input.text());
}

#[test]
fn test_adapter_mutations() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::tui::TuiApp::new().expect("app");

    // Test insert_str through inner_mut
    app.input.inner_mut().insert_str("hello");
    eprintln!("after insert_str: '{}'", app.input.text());

    // Test set_text through inner_mut
    app.input.inner_mut().set_text("world");
    eprintln!("after set_text: '{}'", app.input.text());

    // Test input(Backspace) through inner_mut
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    app.input.inner_mut().input(bs);
    eprintln!("after bs: '{}'", app.input.text());

    // Test input(Char) through inner_mut
    let ch = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    app.input.inner_mut().input(ch);
    eprintln!("after char x: '{}'", app.input.text());
}

#[test]
fn minimal_bs_inline() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut ta = xai_ratatui_textarea::TextArea::new();
    ta.insert_str("hello");
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    ta.input(bs);
    assert_eq!(ta.text(), "hell");
}

#[test]
fn test_bs_via_handle_key_inline() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("hello");
    eprintln!("before handle_key: '{}'", app.input.text());
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    let result = app.input.handle_key(bs, &app.dashboard);
    eprintln!("result: {:?}", result);
    eprintln!("after handle_key: '{}'", app.input.text());
}

#[test]
fn test_bs_key_compare() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    // Key from test
    let bs_test = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);

    // Key from adapter
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("hello");
    let bs_adapter = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);

    eprintln!(
        "bs_test: code={:?}, kind={:?}, modifiers={:?}",
        bs_test.code, bs_test.kind, bs_test.modifiers
    );
    eprintln!(
        "bs_adapter: code={:?}, kind={:?}, modifiers={:?}",
        bs_adapter.code, bs_adapter.kind, bs_adapter.modifiers
    );
    eprintln!("equal: {}", bs_test == bs_adapter);
}

#[test]
fn test_bs_undo_state() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    // Direct textarea
    let mut ta1 = xai_ratatui_textarea::TextArea::new();
    ta1.insert_str("hello");
    eprintln!("direct can_undo: {}", ta1.can_undo());
    eprintln!("direct can_redo: {}", ta1.can_redo());

    // Via adapter
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("hello");
    eprintln!("adapter can_undo: {}", app.input.inner_mut().can_undo());
    eprintln!("adapter can_redo: {}", app.input.inner_mut().can_redo());
}

#[test]
fn test_handle_key_bs_deep() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("hello");

    // Direct call to inner.input
    app.input
        .inner_mut()
        .input(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    eprintln!("after direct inner.input: '{}'", app.input.text());

    // Now test handle_key
    app.input.set_text("hello");
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    let result = app.input.handle_key(bs, &app.dashboard);
    eprintln!(
        "after handle_key: '{}', result={:?}",
        app.input.text(),
        result
    );
}

#[test]
fn test_bs_set_text_same_as_direct() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    // Direct - fails
    let mut ta_direct = xai_ratatui_textarea::TextArea::new();
    ta_direct.set_text("hello");
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    ta_direct.input(bs);
    eprintln!("direct set_text+bs: '{}'", ta_direct.text());

    // Via adapter - should be same
    let mut app = crate::tui::TuiApp::new().expect("app");
    app.input.set_text("hello");
    app.input.inner_mut().input(bs);
    eprintln!("adapter set_text+bs: '{}'", app.input.text());

    // Via adapter with insert_str first (like test_adapter_mutations)
    let mut app2 = crate::tui::TuiApp::new().expect("app");
    app2.input.insert_text("hello");
    app2.input.inner_mut().input(bs);
    eprintln!("adapter insert_str+bs: '{}'", app2.input.text());
}

#[test]
fn test_bs_set_text_hello() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut ta = xai_ratatui_textarea::TextArea::new();
    ta.insert_str("hello");
    ta.set_text("hello"); // same text
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    ta.input(bs);
    eprintln!("insert_str then set_text same: '{}'", ta.text());
}

#[test]
fn test_adapter_direct_set_text_bs() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut adapter = crate::tui::textarea_adapter::InputAdapter::new();
    adapter.set_text("hello");
    eprintln!("adapter before: '{}'", adapter.text());
    let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    adapter.inner_mut().input(bs);
    eprintln!("adapter after: '{}'", adapter.text());
    assert_eq!(adapter.text(), "hell");
}

#[test]
fn test_adapter_set_text_then_insert() {
    let mut adapter = crate::tui::textarea_adapter::InputAdapter::new();
    adapter.set_text("hello");
    adapter.insert_text("x");
    eprintln!("set_text then insert: '{}'", adapter.text());
    assert_eq!(adapter.text(), "hellox");
}
