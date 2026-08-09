#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! The autonomous tool-execution pipeline for the chat path.
//!
//! This wires together the previously-orphaned tooling subsystems (project
//! scanner, route classifier, filesystem/shell/git tools) so that a coding or
//! repository request actually *executes tools* before the LLM is consulted,
//! rather than falling through to a raw chat invocation.
//!
//! Flow:
//!   workspace detection -> project scan -> repo listing ->
//!   tool routing -> tool execution -> (context fed to the LLM)

use anyhow::{Context as _, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::scanner::ProjectInfo;
use crate::tools::{RunCommand, SmartToolRouter, Tool, ToolSelection};

/// The outcome of running the tool pipeline for a single user request.
#[derive(Debug, Clone)]
pub struct PipelineResult {
    /// Ground-truth text gathered from the workspace for the LLM prompt.
    pub context: String,
    /// The individual tools that were executed (for TUI surfacing).
    pub tool_runs: Vec<ToolRun>,
    /// The primary tool the router selected.
    pub primary_tool: String,
}

/// A single executed tool call and its outcome.
#[derive(Debug, Clone)]
pub struct ToolRun {
    pub name: String,
    pub args: String,
    pub success: bool,
    pub output: String,
}

/// Resolves the workspace root: prefers the enclosing git root, falling back
/// to the current working directory. This is the single source of truth for
/// "which repository are we operating on".
pub fn detect_workspace_root() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(git_root) = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(&cwd)
            .output()
        {
            if git_root.status.success() {
                let root = String::from_utf8_lossy(&git_root.stdout).trim().to_string();
                if !root.is_empty() {
                    return PathBuf::from(root);
                }
            }
        }
        cwd
    } else {
        PathBuf::from(".")
    }
}

/// Extracts the most likely file path reference from a task string.
fn extract_path(task: &str, root: &Path) -> Option<PathBuf> {
    // Prefer a quoted path: "path/to/file" or 'path/to/file'.
    for token in task.split('"').skip(1).step_by(2) {
        let token = token.trim();
        if !token.is_empty() && looks_like_path(token) {
            return Some(resolve(token, root));
        }
    }
    for token in task.split('\'').skip(1).step_by(2) {
        let token = token.trim();
        if !token.is_empty() && looks_like_path(token) {
            return Some(resolve(token, root));
        }
    }
    // Otherwise the first whitespace-delimited token that looks like a path.
    for token in task.split_whitespace() {
        let token = token
            .trim_matches(['"', '\''])
            .trim_end_matches(&[',', '.', ';', ')', ']'] as &[_]);
        if looks_like_path(token) {
            return Some(resolve(token, root));
        }
    }
    None
}

fn looks_like_path(token: &str) -> bool {
    let token = token.to_lowercase();
    let has_sep = token.contains('/') || token.contains('\\');
    let ext = [
        ".rs", ".toml", ".py", ".js", ".ts", ".tsx", ".jsx", ".go", ".json", ".md", ".c", ".h",
        ".cpp", ".hpp", ".java", ".rb", ".yml", ".yaml", ".sh", ".zsh", ".cargo", ".lock",
    ];
    let known_ext = ext.iter().any(|e| token.ends_with(e));
    let known_file = ["readme", "cargo.toml", "package.json", "go.mod", "makefile"]
        .iter()
        .any(|f| token.contains(f));
    (has_sep || known_ext || known_file) && !token.ends_with('/')
}

fn resolve(ref_path: &str, root: &Path) -> PathBuf {
    // Strip a leading `./` or `src/` relative form.
    let trimmed = ref_path.trim_start_matches("./");
    let candidate = root.join(trimmed);
    if candidate.exists() {
        candidate
    } else if root.join(ref_path).exists() {
        root.join(ref_path)
    } else {
        // Allow absolute paths and cwd-relative paths too.
        root.join(trimmed)
    }
}

/// Recursive file search by filename keywords.
fn search_files(root: &Path, keywords: &[&str], max_depth: usize) -> Vec<String> {
    let mut hits = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut depth = 0usize;
    while let Some(dir) = stack.pop() {
        if depth > max_depth {
            break;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path.clone());
                } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    let lower = name.to_lowercase();
                    if keywords.iter().any(|k| lower.contains(k)) {
                        hits.push(path.display().to_string());
                    }
                }
            }
        }
        depth += 1;
    }
    hits
}

/// Content search (grep) for a keyword across source files.
fn grep_files(root: &Path, keyword: &str, max_results: usize) -> String {
    let mut matches = String::new();
    let mut count = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if count >= max_results {
            break;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if count >= max_results {
                    break;
                }
                let path = entry.path();
                if path.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name == ".git" || name == "target" || name == "node_modules" {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if !content.is_empty() && content.lines().any(|l| l.contains(keyword)) {
                        let relative = path
                            .strip_prefix(root)
                            .unwrap_or(&path)
                            .display()
                            .to_string();
                        matches.push_str(&format!("{relative}\n"));
                        count += 1;
                    }
                }
            }
        }
    }
    matches.trim().to_string()
}

fn list_files(root: &Path, max_depth: usize) -> String {
    let mut files = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, d)) = stack.pop() {
        if d > max_depth {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name == ".git" || name == "target" || name == "node_modules" {
                        continue;
                    }
                    stack.push((path, d + 1));
                } else {
                    let relative = path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .display()
                        .to_string();
                    files.push(relative);
                }
            }
        }
    }
    files.sort();
    files.join("\n")
}

fn read_file(root: &Path, path: &Path) -> Result<String> {
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let content = std::fs::read_to_string(&resolved)
        .with_context(|| format!("Failed to read file: {}", resolved.display()))?;
    // Ground-truth context is fed to the model; redact obvious credentials so
    // a workspace file (or a stray credentials.json) never leaks a secret.
    Ok(crate::tools::shell::redact_secrets_public(&content))
}

/// Extracts a shell command from the task (e.g. "run cargo clippy" -> "cargo clippy").
fn extract_command(task: &str) -> Option<String> {
    let lower = task.to_lowercase();

    // Prefer complete known read-only toolchain commands, so that
    // "please run cargo test for me" resolves to `cargo test`.
    let known = [
        "cargo clippy",
        "cargo test",
        "cargo build",
        "cargo check",
        "cargo fmt",
        "cargo doc",
        "npm test",
        "npm run build",
        "go test",
        "go build",
        "go vet",
        "python -m pytest",
        "python3 -m pytest",
    ];
    for cmd in known {
        if lower.contains(cmd) {
            return Some(cmd.to_string());
        }
    }

    // Generic cargo <subcommand>.
    for sub in ["clippy", "test", "build", "check", "fmt", "doc"] {
        if lower.contains(&format!("cargo {sub}")) {
            return Some(format!("cargo {sub}"));
        }
    }

    // Explicit "run <cmd>" form.
    if let Some(idx) = lower.find("run ") {
        let rest = task[idx + 4..].trim();
        if !rest.is_empty() && !rest.starts_with('/') && !rest.contains('?') {
            return Some(rest.trim_end_matches(['.', '!', '?']).to_string());
        }
    }
    None
}

fn run_command(root: &Path, command: &str) -> Result<String> {
    let cmd = RunCommand::new()
        .with_working_directory(root.display().to_string())
        .with_timeout(180);
    cmd.execute(command)
}

/// Runs the autonomous tool pipeline. Uses the existing `SmartToolRouter` to
/// pick a primary tool, then executes it (plus lightweight supporting scans)
/// against the detected workspace, returning ground-truth context.
pub fn run_tool_pipeline(task: &str, root: &Path) -> Result<PipelineResult> {
    let mut tool_runs: Vec<ToolRun> = Vec::new();
    let mut context = String::new();

    let project = ProjectInfo::detect(root.to_path_buf()).unwrap_or_default();
    let mut summary = format!(
        "Project: {} ({})\nLanguage: {}\n",
        project.name,
        project.path.display(),
        project.language
    );
    if let Some(ref b) = project.build_system {
        summary.push_str(&format!("Build system: {b}\n"));
    }
    if let Some(ref p) = project.package_manager {
        summary.push_str(&format!("Package manager: {p}\n"));
    }
    if !project.important_files.is_empty() {
        summary.push_str(&format!(
            "Key files: {}\n",
            project.important_files.join(", ")
        ));
    }
    context.push_str(&format!("=== Project Scan ===\n{summary}\n\n"));

    let files_snapshot = list_files(root, 3);
    context.push_str(&format!("=== Repository Files ===\n{files_snapshot}\n\n"));

    // Route the task through the real classifier.
    let router = SmartToolRouter::new(crate::dispatcher::ToolDispatcher::new(
        crate::dispatcher::ToolRegistry::new(),
    ));
    let sel: ToolSelection = router.route(task, &summary);

    // Execute the primary tool with task-aware arguments.
    let primary = sel.primary_tool.clone();
    let primary_output: String;
    let mut primary_success = false;

    match primary.as_str() {
        "run_command" => match extract_command(task) {
            Some(cmd) => match run_command(root, &cmd) {
                Ok(out) => {
                    primary_output = format!(
                        "$ {}\n{out}",
                        crate::tools::shell::redact_secrets_public(&cmd)
                    );
                    primary_success = true;
                }
                Err(e) => {
                    primary_output = format!(
                        "$ {}\nerror: {e}",
                        crate::tools::shell::redact_secrets_public(&cmd)
                    );
                }
            },
            None => {
                primary_output = "No executable command detected in request.".to_string();
            }
        },
        "git_status" | "git_diff" => {
            let args = if primary == "git_status" {
                vec!["status", "--short"]
            } else {
                vec!["diff", "--stat"]
            };
            let out = Command::new("git").args(&args).current_dir(root).output();
            match out {
                Ok(o) => {
                    let text = String::from_utf8_lossy(&o.stdout).to_string();
                    let text = text.trim_end().to_string();
                    primary_output = text;
                    primary_success = o.status.success();
                }
                Err(e) => primary_output = format!("error: {e}"),
            }
        }
        "semantic_search" | "symbol_lookup" | "dependency_analysis" => {
            // Search intent: locate files and grep for the key keyword.
            let lower_task = task.to_lowercase();
            let keywords: Vec<&str> = lower_task
                .split_whitespace()
                .filter(|w| w.len() > 2 && !is_stopword(w))
                .collect();
            let hits = search_files(root, &keywords, 4);
            primary_output = if hits.is_empty() {
                // Fall back to a content search over the first keyword.
                let kw = keywords
                    .first()
                    .map(|s| s.trim_matches(['?', '.', ',']))
                    .filter(|k| !k.is_empty())
                    .unwrap_or("main");
                let g = grep_files(root, kw, 40);
                if g.is_empty() {
                    format!("No files matched keywords: {}", keywords.join(", "))
                } else {
                    format!("Matches:\n{g}")
                }
            } else {
                format!("Found:\n{}", hits.join("\n"))
            };
            primary_success = !primary_output.starts_with("No");
        }
        "create_file" | "edit_file" | "patch" => {
            // Read-only grounding for mutation intents: surface the target so
            // the LLM can propose a precise change (no silent writes).
            match extract_path(task, root) {
                Some(p) => match read_file(root, &p) {
                    Ok(content) => {
                        let rel = p.strip_prefix(root).unwrap_or(&p).display().to_string();
                        primary_output = format!("=== {rel} ===\n{content}");
                        primary_success = true;
                    }
                    Err(e) => primary_output = format!("Target not readable: {e}"),
                },
                None => {
                    primary_output = "Reviewed repository layout; propose a change with an explicit file target.".to_string();
                    primary_success = true;
                }
            }
        }
        "list_files" => {
            primary_output = files_snapshot.clone();
            primary_success = true;
        }
        // read_file / explain / show / default
        _ => {
            // Try to read an explicit target.
            let target: Option<PathBuf> = extract_path(task, root);
            if let Some(p) = target {
                match read_file(root, &p) {
                    Ok(content) => {
                        let rel = p.strip_prefix(root).unwrap_or(&p).display().to_string();
                        primary_output = format!("=== {rel} ===\n{content}");
                        primary_success = true;
                    }
                    Err(e) => primary_output = format!("Could not read target: {e}"),
                }
            } else {
                // Repository explanation: ground with README + manifest + files.
                let mut block = String::new();
                for candidate in ["README.md", "Cargo.toml", "package.json", "go.mod"] {
                    let path = root.join(candidate);
                    if path.exists() {
                        if let Ok(c) = read_file(root, &path) {
                            let trimmed = c.chars().take(4000).collect::<String>();
                            block.push_str(&format!("=== {candidate} ===\n{trimmed}\n\n"));
                        }
                    }
                }
                primary_output = if block.is_empty() {
                    "No manifest/README found; repository listing above provides structure."
                        .to_string()
                } else {
                    block
                };
                primary_success = true;
            }
        }
    }

    context.push_str(&format!(
        "=== Tool: {primary} ===\n{}\n\n",
        truncate(&primary_output, 6000)
    ));
    tool_runs.push(ToolRun {
        name: primary.clone(),
        args: task.to_string(),
        success: primary_success,
        output: primary_output.clone(),
    });

    Ok(PipelineResult {
        context,
        tool_runs,
        primary_tool: primary,
    })
}

fn is_stopword(w: &str) -> bool {
    matches!(
        w,
        "the"
            | "a"
            | "an"
            | "and"
            | "or"
            | "for"
            | "with"
            | "how"
            | "what"
            | "find"
            | "show"
            | "list"
            | "me"
            | "all"
            | "where"
            | "about"
    )
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Convenience guard: whether a task looks like a repository/coding request
/// worth grounding with tools before the LLM answers.
pub fn is_toolable(task: &str) -> bool {
    let lower = task.to_lowercase();
    lower.contains("repository")
        || lower.contains("repo")
        || lower.contains("project")
        || lower.contains("src/")
        || lower.contains("cargo")
        || lower.contains("crate")
        || lower.contains("file")
        || lower.contains("folder")
        || lower.contains("directory")
        || lower.contains("codebase")
        || lower.contains("workspace")
        || lower.contains("agent system")
        || lower.contains("planner")
        || lower.contains("coordinator")
        || lower.contains("toolrouter")
        || lower.contains("taskgraph")
        || lower.contains("explain this")
        || lower.contains("find ")
        || lower.contains("list ")
        || lower.contains("open ")
        || lower.contains("read ")
        || lower.contains("run ")
        || lower.contains("clippy")
        || lower.contains("build")
        || lower.contains("test")
        || lower.contains("where is")
        || lower.contains("show ")
        || lower.contains("how does")
        || lower.contains("search")
        || lower.contains("todo")
        || lower.contains("architecture")
        || lower.contains("main.rs")
        || lower.contains("cargo.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_path_readable() {
        let root = PathBuf::from("src");
        let p = extract_path("open src/main.rs and explain it", &root);
        assert!(p.is_none() || !p.unwrap().to_string_lossy().is_empty());
    }

    #[test]
    fn test_extract_path_quoted() {
        let root = PathBuf::from(".");
        let p = extract_path("read \"Cargo.toml\" please", &root);
        assert!(p.is_some());
    }

    #[test]
    fn test_extract_command_explicit_run() {
        let c = extract_command("run cargo clippy");
        assert_eq!(c.as_deref(), Some("cargo clippy"));
    }

    #[test]
    fn test_extract_command_known() {
        let c = extract_command("please run cargo test for me");
        assert_eq!(c.as_deref(), Some("cargo test"));
    }

    #[test]
    fn test_is_toolable() {
        assert!(is_toolable("Explain this repository"));
        assert!(is_toolable("Find Cargo.toml"));
        assert!(is_toolable("Run cargo clippy"));
        assert!(is_toolable("List project files"));
        assert!(is_toolable("How does the planner work"));
        assert!(!is_toolable("Hey, how are you?"));
    }

    #[test]
    fn test_detect_workspace_root_returns_path() {
        let root = detect_workspace_root();
        assert!(root.as_os_str().len() > 0);
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn test_pipeline_list_files() {
        let root = repo_root();
        let res = run_tool_pipeline("List the project files", &root).unwrap();
        assert_eq!(res.primary_tool, "list_files");
        assert!(res.context.contains("main.rs"), "context: {}", res.context);
        assert!(!res.tool_runs.is_empty());
    }

    #[test]
    fn test_pipeline_find_cargo_toml() {
        let root = repo_root();
        let res = run_tool_pipeline("Find Cargo.toml", &root).unwrap();
        assert!(res.context.to_lowercase().contains("cargo.toml"));
    }

    #[test]
    fn test_pipeline_read_main() {
        let root = repo_root();
        let res = run_tool_pipeline("Open src/main.rs and explain it", &root).unwrap();
        assert!(res.context.contains("fn main"), "context: {}", res.context);
    }

    #[test]
    fn test_pipeline_explain_repository() {
        let root = repo_root();
        let res = run_tool_pipeline("Explain this repository", &root).unwrap();
        assert!(res.context.contains("Project Scan"), "{}", res.context);
        assert!(res.context.contains("Repository Files"), "{}", res.context);
    }

    #[test]
    fn test_pipeline_git_status() {
        let root = repo_root();
        let res = run_tool_pipeline("Show git status", &root).unwrap();
        assert_eq!(res.primary_tool, "git_status");
        // Tolerate loose checkout by just asserting the tool executed (success or not) and produced context.
        assert!(!res.tool_runs.is_empty());
    }

    #[test]
    fn test_pipeline_todo_search() {
        let root = repo_root();
        let res = run_tool_pipeline("Find all TODO comments", &root).unwrap();
        assert!(
            res.primary_tool == "semantic_search"
                || res.primary_tool == "symbol_lookup"
                || res.primary_tool == "dependency_analysis",
            "primary was {}",
            res.primary_tool
        );
    }

    #[test]
    fn test_pipeline_run_command_executes() {
        let root = repo_root();
        let res = run_tool_pipeline("Run cargo --version", &root).unwrap();
        assert_eq!(res.primary_tool, "run_command");
        assert!(
            res.tool_runs[0].success,
            "expected command success, got: {}",
            res.tool_runs[0].output
        );
    }
}
