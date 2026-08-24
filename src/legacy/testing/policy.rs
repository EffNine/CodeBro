//! Testing command policy (Sprint 30D).
//!
//! The Testing subagent is allowed to execute *bounded validation commands*,
//! never arbitrary shell. `run_command` is the single execution surface, and
//! every command string is checked here before it can reach a shell.
//!
//! The policy is project-aware: the permitted validation surface is derived
//! from the detected project metadata (Cargo.toml, package.json, go.mod,
//! Makefile) plus a small set of harmless diagnostic primitives used by tests.
//! It is NOT a giant generic allowlist.
//!
//! Enforcement is defense-in-depth:
//! 1. The permission hook (`TestingPermissionHook`) denies any `run_command`
//!    whose args fail this policy, before the registry can execute it.
//! 2. The restricted registry's own execution path re-checks the same policy
//!    before spawning anything.
//!
//! Two structural guarantees make this a real boundary rather than a keyword
//! filter:
//! - **No shell metacharacters.** The command is executed via `sh -c`, so any
//!   `;`, `&&`, `|`, redirection, command substitution or globbing would turn
//!   it into an unrestricted shell escape. Commands containing any of these are
//!   rejected outright, before argument parsing.
//! - **Program + subcommand allowlist.** The first token must be an allowed
//!   program (`cargo`, `git`, `npm`, ...), and its subcommand/args must be on
//!   that program's read-only allowlist. Anything else — including every
//!   common mutation (`rm`, `mv`, `git commit`, `cargo clean`, `sed -i`,
//!   redirection) — is denied with an explicit reason.

use std::path::Path;

/// The outcome of checking one command against the Testing policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandDecision {
    /// The command is permitted and may be executed.
    Allowed,
    /// The command is denied. The reason is surfaced to the model as an
    /// authoritative observation (the command never executes).
    Denied { reason: String },
}

impl CommandDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, CommandDecision::Allowed)
    }
    pub fn is_denied(&self) -> bool {
        matches!(self, CommandDecision::Denied { .. })
    }
    pub fn reason(&self) -> Option<&str> {
        match self {
            CommandDecision::Allowed => None,
            CommandDecision::Denied { reason } => Some(reason),
        }
    }
}

/// Shell metacharacters that would escape the intended single command.
///
/// The command is executed through `sh -c`, so these enable chaining,
/// pipelines, redirection (file writes), command substitution, globbing and
/// history expansion. Their presence means the request is not a single
/// validation command.
///
/// `\` is intentionally NOT in this set: `$`, `;`, `|`, `` ` ``, `&`, `>`,
/// `<`, `(`, `)` and newline are all denied individually, so a backslash
/// cannot smuggle in any of them (it can only escape a space, which is
/// harmless, or quote a format string such as `printf '%s\n'`).
const SHELL_METACHARACTERS: &[char] = &[
    ';', '&', '|', '>', '<', '$', '`', '\n', '\r', '{', '}', '*', '!', '(', ')',
];

/// Commands that modify source or repository state are denied regardless of
/// program. This is checked as an explicit subcommand/arg deny-list on top of
/// the per-program allowlists, so a single miss in the allowlist can never
/// widen the surface into a mutation.
const MUTATING_ARG_TOKENS: &[&str] = &[
    "commit",
    "add",
    "rm",
    "mv",
    "checkout",
    "reset",
    "clean",
    "apply",
    "restore",
    "rebase",
    "merge",
    "push",
    "pull",
    "fetch",
    "tag",
    "stash",
    "cherry-pick",
    "revert",
    "switch",
    "config",
    "--fix",
    "--write",
    "-w",
    "--in-place",
    "--apply",
    "--amend",
    "--force",
    "-f",
    "--delete",
    "--remove",
    "--purge",
    "--install",
    "--push",
    "--save",
    "--overwrite",
    "--no-verify",
    "-i",
];

/// Programs the Testing subagent may invoke. Every other binary (including
/// `sh`, `bash`, `python`, `cat`, `ls`, `grep`, `sed`, `awk`, `rm`, ...) is
/// denied: inspection goes through the dedicated read-only tools, and mutation
/// is never permitted.
const ALLOWED_PROGRAMS: &[&str] = &["cargo", "git", "npm", "npx", "pnpm", "yarn", "go", "make"];

/// Harmless diagnostic primitives permitted so tests can deterministically
/// exercise exit codes and timeouts. None of them touch repository state;
/// `sleep` is bounded by the per-command PTY timeout and the session deadline.
const ALLOWED_DIAGNOSTIC_PROGRAMS: &[&str] = &["true", "false", "echo", "printf", "sleep"];

/// Cargo subcommands allowed. `fmt` is additionally required to carry a
/// `--check` flag (bare `cargo fmt` rewrites files); `clippy` must not carry
/// `--fix` (checked again by the mutating-arg list).
const CARGO_SUBCOMMANDS: &[&str] = &[
    "check", "test", "build", "clippy", "fmt", "doc", "metadata", "tree",
];

/// Safe cargo flags. Any unknown `-…` token is denied.
const CARGO_ALLOWED_FLAGS: &[&str] = &[
    "--all-targets",
    "--all-features",
    "--lib",
    "--bins",
    "--examples",
    "--benches",
    "--tests",
    "--workspace",
    "--no-run",
    "--release",
    "--offline",
    "--locked",
    "--no-deps",
    "--quiet",
    "-q",
    "-p",
    "--package",
    "--test",
    "--doc",
    "--manifest-path",
    "--message-format",
    "--check",
    "--",
    "--nocapture",
    "--ignored",
    "--exact",
    "--skip",
    "--list",
    "--test-threads",
    "--include-ignored",
    "--show-output",
    "--color",
    "--format",
];

/// Git subcommands that are purely read-only.
const GIT_READ_ONLY_SUBCOMMANDS: &[&str] =
    &["status", "diff", "log", "show", "rev-parse", "ls-files"];

/// Npm subcommands allowed. `run` is restricted to a small set of safe script
/// names that do not mutate source.
const NPM_ALLOWED_SUBCOMMANDS: &[&str] = &["test", "run"];
const NPM_ALLOWED_RUN_SCRIPTS: &[&str] = &["build", "test", "lint", "check", "typecheck", "fmt"];

/// Go subcommands allowed. `go fmt` writes files and is therefore denied.
const GO_ALLOWED_SUBCOMMANDS: &[&str] = &["test", "build", "vet", "mod"];

/// Make targets allowed. Running an arbitrary Make target could execute
/// anything, so only the conventional validation targets are permitted.
const MAKE_ALLOWED_TARGETS: &[&str] = &["build", "test", "check", "lint"];

/// A project-aware command policy for one workspace.
#[derive(Debug, Clone)]
pub struct TestingCommandPolicy {
    workspace_root: std::path::PathBuf,
    /// Whether the workspace looks like a Cargo project.
    is_cargo: bool,
    /// Whether the workspace looks like a Node project.
    is_node: bool,
    /// Whether the workspace looks like a Go project.
    is_go: bool,
    /// Whether the workspace has a Makefile.
    has_makefile: bool,
}

impl TestingCommandPolicy {
    /// Build the policy for a workspace, deriving the permitted validation
    /// surface from the project metadata present in the workspace.
    pub fn for_workspace(workspace_root: &Path) -> Self {
        TestingCommandPolicy {
            workspace_root: workspace_root.to_path_buf(),
            is_cargo: workspace_root.join("Cargo.toml").exists(),
            is_node: workspace_root.join("package.json").exists(),
            is_go: workspace_root.join("go.mod").exists(),
            has_makefile: workspace_root.join("Makefile").exists()
                || workspace_root.join("makefile").exists(),
        }
    }

    /// Whether the workspace has any recognised build/test metadata.
    pub fn has_validation_surface(&self) -> bool {
        self.is_cargo || self.is_node || self.is_go || self.has_makefile
    }

    /// A one-line description of the permitted surface for the prompt.
    pub fn describe(&self) -> String {
        let mut allowed = Vec::new();
        if self.is_cargo {
            allowed.push("cargo check/test/build/clippy/fmt -- --check");
        }
        if self.is_node {
            allowed.push("npm test / npm run build / npx tsc --noEmit");
        }
        if self.is_go {
            allowed.push("go test / go vet / go build");
        }
        if self.has_makefile {
            allowed.push("make build/test/check/lint");
        }
        allowed.push("git status / git diff");
        if allowed.is_empty() {
            "no validation commands detected for this project type".to_string()
        } else {
            allowed.join(", ")
        }
    }

    /// Check a raw command string against the policy.
    ///
    /// Returns `Allowed` only when the command is a single, metacharacter-free
    /// invocation of an allowed program with an allowed subcommand/args.
    pub fn check(&self, command: &str) -> CommandDecision {
        let normalized = normalize(command);
        if normalized.is_empty() {
            return CommandDecision::Denied {
                reason: "empty command".to_string(),
            };
        }

        // Structural boundary: a single command may never contain shell
        // metacharacters. This is checked before argument parsing so nothing
        // can smuggle a second command or a redirection into `sh -c`.
        if let Some(bad) = normalized
            .chars()
            .find(|c| SHELL_METACHARACTERS.contains(c))
        {
            return CommandDecision::Denied {
                reason: format!(
                    "shell metacharacter '{bad}' is not allowed in a single validation command"
                ),
            };
        }

        let tokens: Vec<&str> = normalized.split(' ').collect();
        let program = tokens[0];

        // Diagnostic primitives (used by deterministic tests) are safe by
        // construction; `sleep` requires a positive integer argument so it can
        // only ever burn bounded budget.
        if ALLOWED_DIAGNOSTIC_PROGRAMS.contains(&program) {
            return self.check_diagnostic(program, &tokens[1..]);
        }

        if !ALLOWED_PROGRAMS.contains(&program) {
            return CommandDecision::Denied {
                reason: format!(
                    "program '{program}' is not allowed by the Testing policy (allowed: {}, {}; mutation and arbitrary binaries are denied)",
                    ALLOWED_PROGRAMS.join(", "),
                    ALLOWED_DIAGNOSTIC_PROGRAMS.join(", "),
                ),
            };
        }

        match program {
            "cargo" => self.check_cargo(&tokens[1..]),
            "git" => self.check_git(&tokens[1..]),
            "npm" | "pnpm" | "yarn" => self.check_npm(&tokens[1..]),
            "npx" => self.check_npx(&tokens[1..]),
            "go" => self.check_go(&tokens[1..]),
            "make" => self.check_make(&tokens[1..]),
            _ => CommandDecision::Denied {
                reason: format!("program '{program}' is not handled by the Testing policy"),
            },
        }
    }

    /// `true`, `false`, `echo`, `printf` are always safe; `sleep` needs a
    /// positive integer argument (bounded by the PTY timeout and session
    /// deadline).
    fn check_diagnostic(&self, program: &str, args: &[&str]) -> CommandDecision {
        match program {
            "true" | "false" if args.is_empty() => CommandDecision::Allowed,
            "echo" | "printf" => CommandDecision::Allowed,
            "sleep" => {
                if args.len() == 1 && args[0].parse::<u64>().map(|n| n > 0).unwrap_or(false) {
                    CommandDecision::Allowed
                } else {
                    CommandDecision::Denied {
                        reason: "sleep requires exactly one positive integer argument".to_string(),
                    }
                }
            }
            _ => CommandDecision::Denied {
                reason: format!("'{program}' requires no arguments under the Testing policy"),
            },
        }
    }

    /// Cargo: subcommand must be allowed, flags must be allowed, `fmt` needs
    /// `--check`, and no mutating arg token may appear.
    fn check_cargo(&self, args: &[&str]) -> CommandDecision {
        if !self.is_cargo {
            return CommandDecision::Denied {
                reason: "no Cargo.toml detected in this workspace".to_string(),
            };
        }
        let Some(sub) = args.first() else {
            return CommandDecision::Denied {
                reason: "cargo requires a subcommand".to_string(),
            };
        };
        if !CARGO_SUBCOMMANDS.contains(sub) {
            return CommandDecision::Denied {
                reason: format!("cargo {sub} is not an allowed validation subcommand"),
            };
        }
        if *sub == "fmt" && !args.iter().any(|a| *a == "--check") {
            return CommandDecision::Denied {
                reason: "cargo fmt rewrites files; only 'cargo fmt -- --check' (or 'cargo fmt --check') is allowed".to_string(),
            };
        }
        for arg in &args[1..] {
            if MUTATING_ARG_TOKENS.contains(arg) {
                return CommandDecision::Denied {
                    reason: format!("'{arg}' is not allowed for cargo validation commands"),
                };
            }
            if arg.starts_with('-') && !CARGO_ALLOWED_FLAGS.contains(arg) {
                return CommandDecision::Denied {
                    reason: format!("cargo flag '{arg}' is not allowed"),
                };
            }
        }
        CommandDecision::Allowed
    }

    /// Git: subcommand must be on the read-only allowlist. Every mutation
    /// (commit/add/checkout/...) is denied by both the allowlist and the
    /// mutating-arg list.
    fn check_git(&self, args: &[&str]) -> CommandDecision {
        let Some(sub) = args.first() else {
            return CommandDecision::Denied {
                reason: "git requires a subcommand".to_string(),
            };
        };
        if !GIT_READ_ONLY_SUBCOMMANDS.contains(sub) {
            return CommandDecision::Denied {
                reason: format!("git {sub} is not a read-only git subcommand"),
            };
        }
        if MUTATING_ARG_TOKENS
            .iter()
            .any(|m| args.iter().skip(1).any(|a| a == m))
        {
            return CommandDecision::Denied {
                reason: "git read-only commands must not carry mutating arguments".to_string(),
            };
        }
        CommandDecision::Allowed
    }

    /// npm/pnpm/yarn: `test`, or `run` restricted to a safe script allowlist.
    fn check_npm(&self, args: &[&str]) -> CommandDecision {
        if !self.is_node {
            return CommandDecision::Denied {
                reason: "no package.json detected in this workspace".to_string(),
            };
        }
        let Some(sub) = args.first() else {
            return CommandDecision::Denied {
                reason: "npm requires a subcommand".to_string(),
            };
        };
        if *sub == "test" {
            return CommandDecision::Allowed;
        }
        if *sub == "run" {
            let Some(script) = args.get(1) else {
                return CommandDecision::Denied {
                    reason: "npm run requires a script name".to_string(),
                };
            };
            if NPM_ALLOWED_RUN_SCRIPTS.contains(script) {
                return CommandDecision::Allowed;
            }
            return CommandDecision::Denied {
                reason: format!("npm run {script} is not an allowed validation script"),
            };
        }
        CommandDecision::Denied {
            reason: format!("npm {sub} is not an allowed validation subcommand"),
        }
    }

    /// npx: only `tsc --noEmit`, `eslint` (no fix), `vitest run`, `jest`.
    fn check_npx(&self, args: &[&str]) -> CommandDecision {
        if !self.is_node {
            return CommandDecision::Denied {
                reason: "no package.json detected in this workspace".to_string(),
            };
        }
        let Some(tool) = args.first() else {
            return CommandDecision::Denied {
                reason: "npx requires a tool".to_string(),
            };
        };
        match *tool {
            "tsc" => {
                if args.iter().any(|a| *a == "--noEmit") {
                    CommandDecision::Allowed
                } else {
                    CommandDecision::Denied {
                        reason: "npx tsc requires --noEmit (compilation output is not allowed)"
                            .to_string(),
                    }
                }
            }
            "eslint" | "vitest" | "jest" => {
                if args.iter().any(|a| MUTATING_ARG_TOKENS.contains(a)) {
                    CommandDecision::Denied {
                        reason: format!("npx {tool} must not carry mutating flags"),
                    }
                } else {
                    CommandDecision::Allowed
                }
            }
            _ => CommandDecision::Denied {
                reason: format!("npx {tool} is not an allowed validation tool"),
            },
        }
    }

    /// go: test/build/vet only. `go fmt` writes files and is denied.
    fn check_go(&self, args: &[&str]) -> CommandDecision {
        if !self.is_go {
            return CommandDecision::Denied {
                reason: "no go.mod detected in this workspace".to_string(),
            };
        }
        let Some(sub) = args.first() else {
            return CommandDecision::Denied {
                reason: "go requires a subcommand".to_string(),
            };
        };
        if !GO_ALLOWED_SUBCOMMANDS.contains(sub) {
            return CommandDecision::Denied {
                reason: format!("go {sub} is not an allowed validation subcommand"),
            };
        }
        if args.iter().any(|a| MUTATING_ARG_TOKENS.contains(a)) {
            return CommandDecision::Denied {
                reason: "go validation commands must not carry mutating arguments".to_string(),
            };
        }
        CommandDecision::Allowed
    }

    /// make: only the conventional validation targets.
    fn check_make(&self, args: &[&str]) -> CommandDecision {
        if !self.has_makefile {
            return CommandDecision::Denied {
                reason: "no Makefile detected in this workspace".to_string(),
            };
        }
        let Some(target) = args.first() else {
            return CommandDecision::Denied {
                reason: "make requires a target".to_string(),
            };
        };
        if MAKE_ALLOWED_TARGETS.contains(target) && args.len() == 1 {
            CommandDecision::Allowed
        } else {
            CommandDecision::Denied {
                reason: format!("make {target} is not an allowed validation target"),
            }
        }
    }
}

/// Collapse runs of whitespace to single spaces and trim, so tokenization is
/// stable regardless of how the model formatted the command.
fn normalize(command: &str) -> String {
    command.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(root: &Path) -> TestingCommandPolicy {
        TestingCommandPolicy::for_workspace(root)
    }

    fn cargo_workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        dir
    }

    fn node_workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        dir
    }

    #[test]
    fn test_allows_cargo_validation_commands() {
        let dir = cargo_workspace();
        let p = policy(dir.path());
        for cmd in [
            "cargo check",
            "cargo test",
            "cargo test parser",
            "cargo test --all-targets",
            "cargo test -- --nocapture",
            "cargo build",
            "cargo clippy",
            "cargo clippy --all-targets --all-features",
            "cargo fmt -- --check",
            "cargo fmt --check",
            "cargo doc --no-deps",
        ] {
            assert!(
                p.check(cmd).is_allowed(),
                "'{cmd}' must be allowed, got: {:?}",
                p.check(cmd)
            );
        }
    }

    #[test]
    fn test_denies_cargo_mutation_commands() {
        let dir = cargo_workspace();
        let p = policy(dir.path());
        for cmd in [
            "cargo fmt",
            "cargo fmt --all",
            "cargo clippy --fix",
            "cargo clean",
            "cargo remove serde",
            "cargo add tokio",
            "cargo install ripgrep",
            "cargo new foo",
            "cargo run",
        ] {
            let decision = p.check(cmd);
            assert!(
                decision.is_denied(),
                "'{cmd}' must be denied, got: {:?}",
                decision
            );
        }
    }

    #[test]
    fn test_denies_all_git_mutations_and_allows_read_only() {
        let dir = cargo_workspace();
        let p = policy(dir.path());
        for cmd in [
            "git status",
            "git status --short",
            "git diff",
            "git diff --check",
            "git log --oneline -5",
            "git rev-parse --show-toplevel",
        ] {
            assert!(
                p.check(cmd).is_allowed(),
                "'{cmd}' must be allowed, got: {:?}",
                p.check(cmd)
            );
        }
        for cmd in [
            "git commit -m x",
            "git add .",
            "git checkout main",
            "git reset --hard",
            "git clean -fd",
            "git apply patch",
            "git push",
            "git pull",
            "git stash",
            "git rebase main",
            "git merge main",
            "git fetch",
            "git config user.email x",
        ] {
            let decision = p.check(cmd);
            assert!(
                decision.is_denied(),
                "'{cmd}' must be denied, got: {:?}",
                decision
            );
        }
    }

    #[test]
    fn test_denies_destructive_and_arbitrary_programs() {
        let dir = cargo_workspace();
        let p = policy(dir.path());
        for cmd in [
            "rm -rf /",
            "rm src/main.rs",
            "mv a b",
            "cp a b",
            "mkdir newdir",
            "touch src/main.rs",
            "chmod 777 x",
            "chown root x",
            "sed -i s/x/y/ file",
            "perl -i -pe 's/x/y/' file",
            "sh -c 'rm -rf /'",
            "bash -c 'echo hi'",
            "python3 -c 'open(\"f\",\"w\")'",
            "cat Cargo.toml",
            "ls -la",
            "grep foo src",
            "cargo test; rm -rf /",
            "cargo check > /dev/null",
            "cargo check | tee log",
            "cargo check && cargo test",
            "echo hi > file.txt",
            "$(rm -rf /)",
            "cargo test `touch evil`",
            "cargo check |& grep err",
        ] {
            let decision = p.check(cmd);
            assert!(
                decision.is_denied(),
                "'{cmd}' must be denied, got: {:?}",
                decision
            );
        }
    }

    #[test]
    fn test_project_aware_surface() {
        // Cargo commands are denied in a node-only workspace.
        let node = node_workspace();
        let p = policy(node.path());
        assert!(!p.check("cargo test").is_allowed());
        assert!(p.check("npm test").is_allowed());
        assert!(p.check("npm run build").is_allowed());
        assert!(!p.check("npm install").is_allowed());
        assert!(!p.check("npm run deploy").is_allowed());

        // No project metadata => no cargo/npm surface, but git read-only and
        // diagnostics still work.
        let bare = tempfile::tempdir().unwrap();
        let p = policy(bare.path());
        assert!(!p.check("cargo test").is_allowed());
        assert!(!p.check("npm test").is_allowed());
        assert!(p.check("git status").is_allowed());
        assert!(!p.has_validation_surface());
    }

    #[test]
    fn test_diagnostic_primitives() {
        let bare = tempfile::tempdir().unwrap();
        let p = policy(bare.path());
        assert!(p.check("true").is_allowed());
        assert!(p.check("false").is_allowed());
        assert!(p.check("echo hello").is_allowed());
        assert!(p.check("printf hi").is_allowed());
        assert!(p.check("sleep 1").is_allowed());
        assert!(!p.check("sleep").is_allowed());
        assert!(!p.check("sleep 0").is_allowed());
        assert!(!p.check("sleep abc").is_allowed());
        assert!(!p.check("sleep 1; rm -rf /").is_allowed());
    }

    #[test]
    fn test_normalization_tolerates_extra_whitespace() {
        let dir = cargo_workspace();
        let p = policy(dir.path());
        assert!(p.check("  cargo    test   ").is_allowed());
        assert!(!p.check("  rm    -rf   /  ").is_allowed());
    }
}
