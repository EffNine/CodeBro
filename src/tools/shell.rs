#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const SHELL_HISTORY_PATH: &str = ".codebro/shell_history.json";
const MAX_HISTORY: usize = 200;
const DEFAULT_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellCommandRecord {
    pub command: String,
    pub working_directory: String,
    pub timestamp: String,
    pub success: bool,
    pub duration_ms: u128,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShellHistory {
    pub commands: VecDeque<ShellCommandRecord>,
}

impl ShellHistory {
    pub fn new() -> Self {
        ShellHistory {
            commands: VecDeque::new(),
        }
    }

    pub fn add(&mut self, record: ShellCommandRecord) {
        self.commands.push_back(record);
        if self.commands.len() > MAX_HISTORY {
            self.commands.pop_front();
        }
    }

    pub fn recent(&self, count: usize) -> Vec<&ShellCommandRecord> {
        self.commands.iter().rev().take(count).collect()
    }

    pub fn save(&self, path: &PathBuf) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn load(path: &PathBuf) -> Result<Self> {
        if path.exists() {
            let content = fs::read_to_string(path)?;
            let history: ShellHistory = serde_json::from_str(&content)?;
            Ok(history)
        } else {
            Ok(ShellHistory::new())
        }
    }
}

pub struct RunCommand {
    pub timeout_secs: u64,
    pub working_directory: Option<String>,
    pub environment: Vec<(String, String)>,
    shell_history_path: Option<PathBuf>,
}

impl RunCommand {
    pub fn new() -> Self {
        RunCommand {
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            working_directory: None,
            environment: Vec::new(),
            shell_history_path: None,
        }
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    pub fn with_working_directory(mut self, dir: String) -> Self {
        self.working_directory = Some(dir);
        self
    }

    pub fn with_environment(mut self, key: String, value: String) -> Self {
        self.environment.push((key, value));
        self
    }

    pub fn with_history_path(mut self, path: PathBuf) -> Self {
        self.shell_history_path = Some(path);
        self
    }

    fn execute_with_timeout(&self, args: &str) -> Result<(String, String, i32, u128)> {
        self.execute_child(args, self.timeout_secs)
    }

    fn execute_with_timeout_async(&self, args: &str) -> Result<(String, String, i32, u128)> {
        self.execute_child(args, self.timeout_secs)
    }

    /// Spawns `sh -c​(args)`, polls for completion up to `timeout_secs`, then
    /// kills the process tree on timeout. This actually enforces the stated
    /// timeout (previously `wait_with_output`/`output` blocked indefinitely).
    fn execute_child(&self, args: &str, timeout_secs: u64) -> Result<(String, String, i32, u128)> {
        let start = std::time::Instant::now();

        let mut child = self
            .build_command(args)
            .spawn()
            .with_context(|| format!("Failed to spawn command: {}", args))?;

        let deadline = start + Duration::from_secs(timeout_secs);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(anyhow::anyhow!(
                            "Command timed out after {}s: {}",
                            timeout_secs,
                            args
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => {
                    let _ = child.kill();
                    return Err(anyhow::anyhow!(
                        "Failed to wait for command {}: {}",
                        args,
                        e
                    ));
                }
            }
        };

        // Collect remaining pipe output now that the process has exited.
        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut out) = child.stdout.take() {
            use std::io::Read;
            let _ = out.read_to_string(&mut stdout);
        }
        if let Some(mut err) = child.stderr.take() {
            use std::io::Read;
            let _ = err.read_to_string(&mut stderr);
        }

        // Cap runaway output so a chatty command can't blow up the UI/context.
        let duration = start.elapsed().as_millis();
        let exit_code = status.code().unwrap_or(-1);
        let (stdout, stderr) = cap_output(&stdout, &stderr);

        Ok((
            stdout.trim().to_string(),
            stderr.trim().to_string(),
            exit_code,
            duration,
        ))
    }

    fn build_command(&self, args: &str) -> Command {
        use std::process::Stdio;
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(ref wd) = self.working_directory {
            cmd.current_dir(wd);
        }

        for (key, value) in &self.environment {
            cmd.env(key, value);
        }

        cmd
    }
}

/// Maximum bytes of a single tool's output that flow into the UI/context.
const MAX_TOOL_OUTPUT: usize = 32_768;

/// Caps runaway command output so a chatty process can't blow up the UI/context,
/// and redacts obvious secrets (API keys/tokens) from the surfaced output.
fn cap_output(stdout: &str, stderr: &str) -> (String, String) {
    let redact = |s: &str| -> String {
        let s = redact_secrets(s);
        if s.chars().count() > MAX_TOOL_OUTPUT {
            let mut out: String = s.chars().take(MAX_TOOL_OUTPUT).collect();
            out.push_str("\n…[output truncated]");
            out
        } else {
            s
        }
    };
    (redact(stdout), redact(stderr))
}

/// Redacts bearer tokens and API keys so tool output never leaks credentials
/// into the conversation, context, or session logs.
fn redact_secrets(s: &str) -> String {
    use regex::Regex;
    // sk-..., common API key headers, and bearer tokens.
    let patterns: &[&str] = &[
        r"(?i)sk-[A-Za-z0-9_-]{16,}",
        r"(?i)bearer\s+[A-Za-z0-9._~+/=-]{20,}",
        r#"(?i)api[_-]?key["'=:\s]+[A-Za-z0-9._-]{16,}"#,
        r#"(?i)authorization["'=:\s]+[A-Za-z0-9._~+/=-]{16,}"#,
    ];
    let mut out = s.to_string();
    for pat in patterns {
        if let Ok(re) = Regex::new(pat) {
            out = re.replace_all(&out, "[REDACTED]").into_owned();
        }
    }
    out
}

impl RunCommand {
    fn record_command(
        &self,
        command: &str,
        working_directory: &str,
        success: bool,
        exit_code: Option<i32>,
        duration_ms: u128,
    ) {
        if let Some(ref history_path) = self.shell_history_path {
            let mut history =
                ShellHistory::load(history_path).unwrap_or_else(|_| ShellHistory::new());

            let record = ShellCommandRecord {
                command: command.to_string(),
                working_directory: working_directory.to_string(),
                timestamp: chrono::Local::now().to_rfc3339(),
                success,
                duration_ms,
                exit_code,
            };

            history.add(record);
            let _ = history.save(history_path);
        }
    }
}

impl super::Tool for RunCommand {
    fn name(&self) -> &str {
        "run_command"
    }

    fn description(&self) -> &str {
        "Execute a shell command with timeout and history tracking"
    }

    fn execute(&self, args: &str) -> Result<String> {
        let working_directory = self
            .working_directory
            .clone()
            .unwrap_or_else(|| ".".to_string());

        let (stdout, stderr, exit_code, duration) = self.execute_with_timeout(args)?;

        let success = exit_code == 0;
        self.record_command(args, &working_directory, success, Some(exit_code), duration);

        if success {
            Ok(stdout)
        } else {
            Err(anyhow::anyhow!(
                "Command failed (exit {}): {}\n{}",
                exit_code,
                stderr,
                stdout
            ))
        }
    }
}

impl Default for RunCommand {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;

    #[test]
    fn test_run_command_success() {
        let tool = RunCommand::new();
        let result = tool.execute("echo hello").expect("run_command should work");
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_run_command_with_timeout() {
        let tool = RunCommand::new().with_timeout(10);
        let result = tool
            .execute("echo timed_out_test")
            .expect("run_command should work");
        assert_eq!(result, "timed_out_test");
    }

    #[test]
    fn test_run_command_with_working_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tool =
            RunCommand::new().with_working_directory(dir.path().to_string_lossy().to_string());
        let result = tool.execute("pwd").expect("run_command should work");
        let expected = std::fs::canonicalize(dir.path())
            .expect("canonicalize path")
            .to_string_lossy()
            .to_string();
        assert_eq!(result, expected);
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
    fn test_run_command_enforces_timeout() {
        let tool = RunCommand::new().with_timeout(1);
        let err = tool
            .execute("sleep 10 && echo 'should_not_happen'")
            .expect_err("a 10s sleep must time out under 1s");
        assert!(
            err.to_string().contains("timed out"),
            "expected timeout error, got: {}",
            err
        );
    }

    #[test]
    fn test_cap_output_truncates_and_redacts() {
        // Secret redaction.
        let (out, _) = cap_output("sk-abcdefghijklmnopqrstuvwxyz0123456 hello", "");
        assert!(
            !out.contains("sk-abcdefghijklmnopqrst"),
            "sk- key not redacted"
        );

        // Truncation.
        let big = "x".repeat(100_000);
        let (out, _) = cap_output(&big, "");
        assert!(out.len() < big.len(), "long output was not truncated");
        assert!(out.contains("truncated"));
    }
}
