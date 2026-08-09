//! Playwright browser-test capability.
//!
//! A real, extensible Playwright integration through the existing Tool
//! Platform. The tool discovers and runs Playwright test suites in the
//! workspace, surfaces live output through the PTY console path, and reports
//! failures. It never fakes browser execution: it invokes the actual
//! Playwright test runner (`npx playwright test`) against the workspace.

use anyhow::Result;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use super::context::ToolContext;
use super::shell::RunCommand;
use super::streaming::AsyncTool;
use super::{StreamChunk, StreamResult, Tool};

const PLAYWRIGHT_TIMEOUT_SECS: u64 = 600;

/// Run Playwright tests in a workspace.
pub struct PlaywrightTool {
    runner: RunCommand,
}

impl PlaywrightTool {
    /// Create a Playwright tool bound to a workspace root. The runner executes
    /// `npx --no-install playwright test` inside that root, so local
    /// installations are honored without auto-installing packages.
    pub fn new(workspace_root: PathBuf) -> Self {
        let runner = RunCommand::new()
            .with_working_directory(workspace_root.to_string_lossy().to_string())
            .with_timeout(PLAYWRIGHT_TIMEOUT_SECS);
        PlaywrightTool { runner }
    }

    /// The shell command invoked for the given user arguments.
    pub fn build_command(&self, args: &str) -> String {
        let args = args.trim();
        if args.is_empty() {
            "npx --no-install playwright test".to_string()
        } else {
            format!("npx --no-install playwright test {}", args)
        }
    }
}

impl Tool for PlaywrightTool {
    fn name(&self) -> &str {
        "playwright_test"
    }

    fn description(&self) -> &str {
        "Run Playwright browser tests in the workspace and report failures"
    }

    fn as_async(&self) -> Option<&dyn AsyncTool> {
        Some(self)
    }

    fn execute(&self, args: &str) -> Result<String> {
        let cmd = self.build_command(args);
        // A failing test suite is a result, not a tool crash: surface the
        // output and a concise status line instead of an error.
        match self.runner.run(&cmd) {
            Ok(run) => Ok(format!(
                "[playwright {}]\n{}",
                if run.exit_code == 0 {
                    "passed"
                } else {
                    "failed"
                },
                run.stdout
            )),
            Err(e) => Err(e),
        }
    }
}

impl AsyncTool for PlaywrightTool {
    fn name(&self) -> &str {
        "playwright_test"
    }

    fn execute_stream(
        &self,
        args: &str,
        context: &ToolContext,
    ) -> Pin<Box<dyn Future<Output = Result<StreamResult>> + Send>> {
        let cmd = self.build_command(args);
        let runner = self.runner.clone();
        let cancel = context.cancellation.clone();
        Box::pin(async move {
            let config = super::pty::PtyConfig {
                command: cmd,
                working_directory: runner.working_directory.clone().map(PathBuf::from),
                environment: runner.environment.clone(),
                timeout: Some(Duration::from_secs(runner.timeout_secs)),
                max_output: super::pty::MAX_PTY_OUTPUT,
            };
            let cancel = cancel.unwrap_or_default();
            let mut rx = super::pty::spawn_pty(config, cancel);
            let stream =
                super::streaming::channel_stream_factory("playwright_test", move |tx| loop {
                    match rx.blocking_recv() {
                        Some(super::pty::PtyEvent::Output(content)) => {
                            if tx.blocking_send(StreamChunk::new(&content, false)).is_err() {
                                break;
                            }
                        }
                        Some(super::pty::PtyEvent::Exited { exit_code }) => {
                            let _ = tx.blocking_send(StreamChunk {
                                text: String::new(),
                                is_final: true,
                                metadata: Some(format!("exit:{}", exit_code)),
                            });
                            break;
                        }
                        Some(super::pty::PtyEvent::Cancelled) => {
                            let _ = tx.blocking_send(StreamChunk {
                                text: "\n[Cancelled by user]".to_string(),
                                is_final: true,
                                metadata: Some("cancelled".to_string()),
                            });
                            break;
                        }
                        Some(super::pty::PtyEvent::TimedOut) => {
                            let _ = tx.blocking_send(StreamChunk {
                                text: "\n[Playwright timed out]".to_string(),
                                is_final: true,
                                metadata: Some("timeout".to_string()),
                            });
                            break;
                        }
                        Some(super::pty::PtyEvent::Error(e)) => {
                            let _ = tx.blocking_send(StreamChunk {
                                text: format!("\n[Error: {}]", e),
                                is_final: true,
                                metadata: Some("error".to_string()),
                            });
                            break;
                        }
                        None => break,
                    }
                });
            Ok(StreamResult::new(stream, "playwright_test"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_command_with_args() {
        let tool = PlaywrightTool::new(PathBuf::from("/tmp"));
        assert_eq!(
            tool.build_command("tests/e2e --project=chromium"),
            "npx --no-install playwright test tests/e2e --project=chromium"
        );
        assert_eq!(tool.build_command(""), "npx --no-install playwright test");
        assert_eq!(tool.build_command("  "), "npx --no-install playwright test");
    }

    #[test]
    fn test_playwright_name_and_description() {
        let tool = PlaywrightTool::new(PathBuf::from("/tmp"));
        assert_eq!(Tool::name(&tool), "playwright_test");
        assert!(tool.description().to_lowercase().contains("playwright"));
        assert!(tool.as_async().is_some());
    }
}
