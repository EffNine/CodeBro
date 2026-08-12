//! Command registry for the three input namespaces.
//!
//! ```text
//!   /   engineering commands   — operate on the project (code, tasks, workflows)
//!   //  runtime commands       — operate on CodeBro itself (config, display, session)
//!   !   shell commands         — execute directly in the user's shell
//! ```
//!
//! The registry is the single source of truth for autocomplete, `Tab`
//! completion, the command palette, and `/help`. Completion is context-aware:
//! commands that are irrelevant to the current runtime state are hidden (for
//! example `//approve` only appears while a file change is pending).

use crate::tui::app::TuiApp;

/// Which namespace an input line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandNamespace {
    /// `/` — operates on the project.
    Engineering,
    /// `//` — operates on CodeBro.
    Runtime,
    /// `!` — direct shell execution.
    Shell,
}

/// A registered command.
#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    /// The namespace the command belongs to.
    pub namespace: CommandNamespace,
    /// The command token, including its prefix, without arguments
    /// (e.g. `/build`, `//verbose`).
    pub command: &'static str,
    /// Usage with placeholders (e.g. `/build`).
    pub usage: &'static str,
    /// One-line description.
    pub description: &'static str,
}

/// Engineering commands (`/`).
pub const ENGINEERING_COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        namespace: CommandNamespace::Engineering,
        command: "/help",
        usage: "/help",
        description: "Show available commands and shortcuts",
    },
    CommandSpec {
        namespace: CommandNamespace::Engineering,
        command: "/review",
        usage: "/review",
        description: "Review pending or recent code changes",
    },
    CommandSpec {
        namespace: CommandNamespace::Engineering,
        command: "/build",
        usage: "/build",
        description: "Build the project",
    },
    CommandSpec {
        namespace: CommandNamespace::Engineering,
        command: "/test",
        usage: "/test",
        description: "Run the project's test suite",
    },
    CommandSpec {
        namespace: CommandNamespace::Engineering,
        command: "/fix",
        usage: "/fix",
        description: "Fix the last error or failing test",
    },
    CommandSpec {
        namespace: CommandNamespace::Engineering,
        command: "/search",
        usage: "/search <pattern>",
        description: "Search the codebase (symbols, text, patterns)",
    },
    CommandSpec {
        namespace: CommandNamespace::Engineering,
        command: "/refactor",
        usage: "/refactor <target>",
        description: "Refactor a selected piece of code",
    },
    CommandSpec {
        namespace: CommandNamespace::Engineering,
        command: "/explain",
        usage: "/explain <target>",
        description: "Explain a selected piece of code or error",
    },
    CommandSpec {
        namespace: CommandNamespace::Engineering,
        command: "/benchmark",
        usage: "/benchmark",
        description: "Run benchmarks and compare results",
    },
    CommandSpec {
        namespace: CommandNamespace::Engineering,
        command: "/doctor",
        usage: "/doctor",
        description: "Diagnose project health (deps, lints, tests)",
    },
    CommandSpec {
        namespace: CommandNamespace::Engineering,
        command: "/playwright",
        usage: "/playwright [args]",
        description: "Run Playwright browser tests",
    },
    CommandSpec {
        namespace: CommandNamespace::Engineering,
        command: "/apply",
        usage: "/apply <file> <new content>",
        description: "Stage a reviewed file change (no writes)",
    },
    CommandSpec {
        namespace: CommandNamespace::Engineering,
        command: "/copy",
        usage: "/copy",
        description: "Copy the conversation to the clipboard",
    },
    CommandSpec {
        namespace: CommandNamespace::Engineering,
        command: "/status",
        usage: "/status",
        description: "Show current task state, model, and provider",
    },
];

/// Runtime commands (`//`).
pub const RUNTIME_COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//model",
        usage: "//model [name]",
        description: "Show or change the current model",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//provider",
        usage: "//provider [id]",
        description: "Show or change the current provider",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//apikey",
        usage: "//apikey [provider]",
        description: "Set a provider API key via a masked prompt (stored securely)",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//settings",
        usage: "//settings",
        description: "View and edit all settings",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//preferences",
        usage: "//preferences",
        description: "View and edit preferences",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//profile",
        usage: "//profile [name]",
        description: "Switch CodeBro profiles",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//session",
        usage: "//session [id]",
        description: "Manage sessions (list, resume, delete)",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//resume",
        usage: "//resume",
        description: "Resume the last session",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//theme",
        usage: "//theme [name]",
        description: "Change the color theme",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//verbose",
        usage: "//verbose",
        description: "Toggle detailed output mode",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//compact",
        usage: "//compact",
        description: "Toggle compact display mode",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//rail",
        usage: "//rail",
        description: "Toggle the right intelligence rail",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//console",
        usage: "//console",
        description: "Open the live PTY console overlay",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//memory",
        usage: "//memory",
        description: "View or clear project memory",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//mcp",
        usage: "//mcp",
        description: "Manage MCP servers",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//skills",
        usage: "//skills",
        description: "Manage installed skills",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//plugins",
        usage: "//plugins",
        description: "Manage plugins",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//update",
        usage: "//update",
        description: "Check for and apply updates",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//version",
        usage: "//version",
        description: "Show CodeBro version and build info",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//export",
        usage: "//export [path]",
        description: "Export current configuration or session",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//import",
        usage: "//import <file>",
        description: "Import configuration or session from file",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//clear",
        usage: "//clear",
        description: "Clear the terminal screen",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//tasks",
        usage: "//tasks",
        description: "Show the current task graph",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//agents",
        usage: "//agents",
        description: "Show subagent status",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//metrics",
        usage: "//metrics",
        description: "Show task metrics (tokens, cost, duration)",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//approve",
        usage: "//approve [verify-cmd]",
        description: "Approve pending file changes",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//reject",
        usage: "//reject",
        description: "Reject pending file changes",
    },
    CommandSpec {
        namespace: CommandNamespace::Runtime,
        command: "//save",
        usage: "//save",
        description: "Save the current session",
    },
];

/// Common shell commands offered as `!` completion suggestions. Shell commands
/// are not a closed registry — anything after `!` is executed directly.
pub const SHELL_SUGGESTIONS: &[&str] = &[
    "!ls",
    "!pwd",
    "!git status",
    "!git diff",
    "!cargo build",
    "!cargo test",
    "!cargo check",
    "!cargo clippy",
    "!npm test",
    "!docker ps",
    "!grep -r ",
];

/// All registered commands.
pub fn all_commands() -> impl Iterator<Item = &'static CommandSpec> {
    ENGINEERING_COMMANDS.iter().chain(RUNTIME_COMMANDS.iter())
}

/// Detect the namespace of an input line.
pub fn namespace_of(input: &str) -> Option<CommandNamespace> {
    match input.chars().next()? {
        '/' => {
            if input.chars().nth(1) == Some('/') {
                Some(CommandNamespace::Runtime)
            } else {
                Some(CommandNamespace::Engineering)
            }
        }
        '!' => Some(CommandNamespace::Shell),
        _ => None,
    }
}

/// Whether a command is applicable in the current runtime state. Context-aware
/// completion hides commands that are irrelevant right now.
pub fn is_applicable(spec: &CommandSpec, app: &TuiApp) -> bool {
    match spec.command {
        "//approve" | "//reject" => app.pending_change.is_some(),
        "//tasks" => app.dashboard.task_graph.is_some(),
        "//agents" => {
            app.dashboard.status_monitor.active_count() > 0
                || !app.dashboard.activity_log.is_empty()
        }
        "//resume" | "//session" => app.session_tracker.as_ref().is_some(),
        _ => true,
    }
}

/// Live-filtered, context-aware completion candidates for an input line.
pub fn completion_candidates(input: &str, app: &TuiApp) -> Vec<CommandSpec> {
    let ns = match namespace_of(input) {
        Some(ns) => ns,
        None => return Vec::new(),
    };

    match ns {
        CommandNamespace::Shell => {
            let token = input.trim_start();
            if token.len() <= 1 {
                return SHELL_SUGGESTIONS
                    .iter()
                    .map(|s| CommandSpec {
                        namespace: CommandNamespace::Shell,
                        command: s,
                        usage: s,
                        description: "Execute directly in the shell",
                    })
                    .collect();
            }
            SHELL_SUGGESTIONS
                .iter()
                .filter(|s| s.starts_with(&token) && s.len() > token.len())
                .map(|s| CommandSpec {
                    namespace: CommandNamespace::Shell,
                    command: s,
                    usage: s,
                    description: "Execute directly in the shell",
                })
                .collect()
        }
        CommandNamespace::Engineering => {
            let token = input.trim();
            ENGINEERING_COMMANDS
                .iter()
                .copied()
                .filter(|spec| spec.command.starts_with(token) && spec.command != token)
                .filter(|spec| is_applicable(spec, app))
                .collect()
        }
        CommandNamespace::Runtime => {
            let token = input.trim();
            RUNTIME_COMMANDS
                .iter()
                .copied()
                .filter(|spec| spec.command.starts_with(token) && spec.command != token)
                .filter(|spec| is_applicable(spec, app))
                .collect()
        }
    }
}

/// Whether the input is a fully-typed command (no completion offered).
pub fn is_complete_command(input: &str) -> bool {
    match namespace_of(input) {
        Some(CommandNamespace::Engineering) => ENGINEERING_COMMANDS
            .iter()
            .any(|spec| spec.command == input.trim()),
        Some(CommandNamespace::Runtime) => RUNTIME_COMMANDS
            .iter()
            .any(|spec| spec.command == input.trim()),
        _ => false,
    }
}

/// Resolve a fully-typed command spec (for dispatch), ignoring arguments.
pub fn resolve(input: &str) -> Option<&'static CommandSpec> {
    let token = input.split_whitespace().next().unwrap_or("");
    all_commands().find(|spec| spec.command == token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::TuiApp;

    fn app() -> TuiApp {
        TuiApp::new().expect("app creation")
    }

    #[test]
    fn test_namespace_detection() {
        assert_eq!(namespace_of("/build"), Some(CommandNamespace::Engineering));
        assert_eq!(namespace_of("//verbose"), Some(CommandNamespace::Runtime));
        assert_eq!(namespace_of("!git status"), Some(CommandNamespace::Shell));
        assert_eq!(namespace_of("a normal task"), None);
        assert_eq!(namespace_of(""), None);
    }

    #[test]
    fn test_engineering_completion_prefix() {
        let app = app();
        let candidates = completion_candidates("/te", &app);
        assert!(candidates.iter().any(|c| c.command == "/test"));
        assert!(candidates.iter().any(|c| c.command == "/test"));
    }

    #[test]
    fn test_runtime_completion_prefix() {
        let app = app();
        let candidates = completion_candidates("//verb", &app);
        assert!(candidates.iter().any(|c| c.command == "//verbose"));
    }

    #[test]
    fn test_shell_completion() {
        let app = app();
        let candidates = completion_candidates("!git s", &app);
        assert!(candidates.iter().any(|c| c.command == "!git status"));
    }

    #[test]
    fn test_context_aware_approve_hidden_without_pending() {
        let app = app();
        let candidates = completion_candidates("//app", &app);
        assert!(
            candidates.iter().all(|c| c.command != "//approve"),
            "//approve must be hidden when no change is pending"
        );
    }

    #[test]
    fn test_context_aware_approve_visible_with_pending() {
        let mut app = app();
        // A minimal pending change: stage a real change to a temp file.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.txt");
        std::fs::write(&path, "old").unwrap();
        let plan = crate::tools::ChangePlan::propose(&path, "new").unwrap();
        app.pending_change = Some(plan);
        let candidates = completion_candidates("//app", &app);
        assert!(
            candidates.iter().any(|c| c.command == "//approve"),
            "//approve must appear when a change is pending"
        );
    }

    #[test]
    fn test_all_required_commands_present() {
        let required = [
            "/help",
            "/review",
            "/build",
            "/test",
            "/fix",
            "/search",
            "/refactor",
            "/explain",
            "/benchmark",
            "/doctor",
            "/playwright",
            "/apply",
            "/copy",
            "/status",
            "//model",
            "//provider",
            "//apikey",
            "//settings",
            "//preferences",
            "//profile",
            "//session",
            "//resume",
            "//theme",
            "//verbose",
            "//compact",
            "//memory",
            "//mcp",
            "//skills",
            "//plugins",
            "//update",
            "//version",
            "//export",
            "//import",
            "//clear",
            "//tasks",
            "//agents",
            "//metrics",
            "//approve",
            "//reject",
            "//save",
        ];
        for cmd in required {
            assert!(
                all_commands().any(|spec| spec.command == cmd),
                "missing command: {}",
                cmd
            );
        }
    }

    #[test]
    fn test_resolve_ignores_args() {
        assert_eq!(
            resolve("/playwright tests/e2e").map(|s| s.command),
            Some("/playwright")
        );
        assert_eq!(
            resolve("//apikey openai sk-x").map(|s| s.command),
            Some("//apikey")
        );
        assert!(resolve("/bogus").is_none());
    }
}
