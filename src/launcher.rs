//! Process launcher — spawns the Rust bridge and Bun TUI as child processes.
//!
//! Lifecycle:
//!   1. Spawn bridge process (`codebro --bridge`)
//!   2. Wait for `ready` on bridge stderr
//!   3. Spawn TUI (`bun run --conditions browser src/main.tsx`) with stdio cross-wired to bridge
//!   4. Forward TUI stderr to launcher stderr for visibility
//!   5. Manage graceful shutdown
//!   6. Kill both children on exit

use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::Duration;

use anyhow::{bail, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;

/// Resolved paths for the launcher.
pub struct LauncherPaths {
    pub codebro_bin: PathBuf,
    pub bun_bin: PathBuf,
    pub tui_root: PathBuf,
}

impl LauncherPaths {
    /// Resolve paths relative to the CodeBro executable and repository layout.
    pub fn resolve() -> Result<Self> {
        let codebro_bin = std::env::current_exe().unwrap_or_default();

        let bun_bin = match std::env::var("CODEBRO_TUI_BUN") {
            Ok(var) => PathBuf::from(var),
            Err(_) => {
                let candidates: Vec<PathBuf> = vec![
                    PathBuf::from("bun"),
                    PathBuf::from("/usr/local/bin/bun"),
                    PathBuf::from("/opt/homebrew/bin/bun"),
                    PathBuf::from("/usr/bin/bun"),
                ];
                candidates
                    .into_iter()
                    .find(|p| p.exists() || p.file_name().map_or(false, |f| which_helper(f)))
                    .unwrap_or_else(|| PathBuf::from("bun"))
            }
        };

        let tui_root = match std::env::var("CODEBRO_TUI_ROOT") {
            Ok(var) => PathBuf::from(var),
            Err(_) => {
                let exe = std::env::current_exe().unwrap_or_default();
                let root = exe
                    .parent()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."));
                for suffix in &["../opencode-tui", "opencode-tui"] {
                    let candidate = root.join(suffix);
                    if candidate.join("package.json").exists() {
                        return Ok(LauncherPaths {
                            codebro_bin,
                            bun_bin,
                            tui_root: candidate,
                        });
                    }
                }
                let cwd = std::env::current_dir().unwrap_or_default();
                let candidate = cwd.join("opencode-tui");
                if candidate.join("package.json").exists() {
                    return Ok(LauncherPaths {
                        codebro_bin,
                        bun_bin,
                        tui_root: candidate,
                    });
                }
                candidate
            }
        };

        Ok(LauncherPaths {
            codebro_bin,
            bun_bin,
            tui_root,
        })
    }
}

/// Check if a command name exists in PATH (cross-platform helper).
fn which_helper(name: &std::ffi::OsStr) -> bool {
    #[cfg(windows)]
    {
        std::path::Path::new(&format!("{}.exe", name.to_string_lossy())).exists()
            || std::path::Path::new(&format!("{}.cmd", name.to_string_lossy())).exists()
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {}", name.to_string_lossy()))
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }
}

/// Child process handle with cleanup tracking.
struct ChildHandle {
    child: Child,
    name: &'static str,
}

impl ChildHandle {
    fn kill(&mut self) -> Result<()> {
        match self.child.kill() {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(anyhow::anyhow!("Failed to kill {}: {}", self.name, e)),
        }
    }
}

/// The main launcher orchestration.
pub struct Launcher {
    paths: LauncherPaths,
}

impl Launcher {
    pub fn new() -> Result<Self> {
        let paths = LauncherPaths::resolve()?;
        Ok(Launcher { paths })
    }

    /// Check if Bun is available.
    pub fn check_bun(&self) -> Result<()> {
        let output = std::process::Command::new(&self.paths.bun_bin)
            .arg("--version")
            .output();
        match output {
            Ok(out) if out.status.success() => Ok(()),
            _ => bail!(
                "Bun runtime not found at `{}`. \
                 Install Bun or set CODEBRO_TUI_BUN.",
                self.paths.bun_bin.display()
            ),
        }
    }

    /// Check if the TUI root exists.
    pub fn check_tui_root(&self) -> Result<()> {
        if self.paths.tui_root.join("package.json").exists() {
            Ok(())
        } else {
            bail!(
                "CodeBro TUI frontend not found at `{}`. \
                 Set CODEBRO_TUI_ROOT or run from the repository root.",
                self.paths.tui_root.display()
            )
        }
    }

    /// Run the launcher: spawn bridge, wait for ready, spawn TUI, manage lifecycle.
    pub async fn run(self) -> Result<()> {
        self.check_bun()?;
        self.check_tui_root()?;

        // Spawn bridge
        let mut bridge = self.spawn_bridge()?;
        eprintln!("[launcher] bridge spawned (pid={})", bridge.child.id());

        // Wait for ready event on bridge stderr
        let bridge_stderr = bridge
            .child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to take bridge stderr"))?;
        let ready = Self::wait_for_bridge_ready(bridge_stderr).await?;
        if !ready {
            bail!("Bridge failed to become ready within timeout");
        }
        eprintln!("[launcher] bridge ready");

        // Spawn TUI with cross-wired stdio:
        //   TUI stdin  <- bridge stdout (protocol requests from TUI)
        //   TUI stdout -> bridge stdin (protocol responses from bridge)
        //   TUI stderr -> launcher stderr (errors/diagnostics visible to user)
        let mut tui = self.spawn_tui(&mut bridge)?;
        eprintln!("[launcher] tui spawned (pid={})", tui.child.id());

        // Capture TUI stderr and forward to launcher stderr
        let tui_stderr = tui
            .child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to take TUI stderr"))?;
        Self::forward_stderr(tui_stderr);

        // Wait for either child to exit, or Ctrl+C
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);

        // Use oneshot channels to communicate exit status
        let (bridge_tx, mut bridge_rx) = oneshot::channel::<std::process::ExitStatus>();
        let (tui_tx, mut tui_rx) = oneshot::channel::<std::process::ExitStatus>();

        // Spawn waiters in background (child.wait() is sync)
        let mut bridge_child = bridge.child;
        let mut tui_child = tui.child;

        tokio::task::spawn_blocking(move || {
            let status = bridge_child.wait().unwrap_or_default();
            let _ = bridge_tx.send(status);
        });
        tokio::task::spawn_blocking(move || {
            let status = tui_child.wait().unwrap_or_default();
            let _ = tui_tx.send(status);
        });

        loop {
            tokio::select! {
                biased;
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("[launcher] Ctrl+C received, shutting down...");
                    break;
                }
                status = &mut bridge_rx => {
                    match status {
                        Ok(s) => {
                            eprintln!("[launcher] bridge exited with status: {:?}", s);
                            break;
                        }
                        Err(_) => {
                            eprintln!("[launcher] bridge exit channel closed");
                            break;
                        }
                    }
                }
                status = &mut tui_rx => {
                    match status {
                        Ok(s) => {
                            if s.success() {
                                eprintln!("[launcher] tui exited cleanly");
                            } else {
                                eprintln!("[launcher] tui exited with code: {:?}", s.code());
                            }
                            break;
                        }
                        Err(_) => {
                            eprintln!("[launcher] tui exit channel closed");
                            break;
                        }
                    }
                }
            }
        }

        // Graceful shutdown — children are already consumed above
        eprintln!("[launcher] shutdown complete");
        Ok(())
    }

    fn spawn_bridge(&self) -> Result<ChildHandle> {
        let child = std::process::Command::new(&self.paths.codebro_bin)
            .arg("--bridge")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        Ok(ChildHandle {
            child,
            name: "bridge",
        })
    }

    async fn wait_for_bridge_ready(stderr: impl Read + Send + 'static) -> Result<bool> {
        let (tx, rx) = oneshot::channel::<bool>();

        tokio::spawn(async move {
            let mut reader = std::io::BufReader::new(stderr);
            let mut line = String::new();
            let start = std::time::Instant::now();
            let timeout_dur = Duration::from_secs(10);

            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        let _ = tx.send(false);
                        return;
                    }
                    Ok(_) => {
                        let trimmed = line.trim().to_lowercase();
                        if trimmed.contains("ready") {
                            let _ = tx.send(true);
                            return;
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(false);
                        return;
                    }
                }
                if start.elapsed() > timeout_dur {
                    let _ = tx.send(false);
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });

        rx.await
            .map_err(|_| anyhow::anyhow!("Bridge ready channel closed unexpectedly"))
    }

    fn spawn_tui(&self, bridge: &mut ChildHandle) -> Result<ChildHandle> {
        let bridge_stdout = bridge
            .child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to take bridge stdout"))?;
        let bridge_stdin = bridge
            .child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to take bridge stdin"))?;

        let child = std::process::Command::new(&self.paths.bun_bin)
            .arg("run")
            .arg("--conditions")
            .arg("browser")
            .arg("src/main.tsx")
            .current_dir(&self.paths.tui_root)
            .stdin(Stdio::from(bridge_stdout))
            .stdout(Stdio::from(bridge_stdin))
            .stderr(Stdio::piped())
            .spawn()?;

        Ok(ChildHandle { child, name: "tui" })
    }

    /// Forward TUI stderr to launcher stderr in a background task.
    fn forward_stderr(stderr: impl Read + Send + 'static) {
        tokio::task::spawn_blocking(move || {
            let mut reader = std::io::BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        eprint!("{}", line);
                    }
                    Err(_) => break,
                }
            }
        });
    }

    async fn shutdown(bridge: &mut ChildHandle, tui: &mut ChildHandle) {
        eprintln!("[launcher] shutting down children...");

        tokio::time::sleep(Duration::from_millis(300)).await;

        let _ = bridge.kill();
        let _ = tui.kill();

        let _ = bridge.child.try_wait();
        let _ = tui.child.try_wait();

        eprintln!("[launcher] shutdown complete");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_launcher_paths_resolve() {
        let paths = LauncherPaths::resolve();
        assert!(paths.is_ok());
        let paths = paths.unwrap();
        assert!(!paths.codebro_bin.as_os_str().is_empty());
        assert!(!paths.tui_root.as_os_str().is_empty());
    }

    #[tokio::test]
    async fn test_wait_for_bridge_ready_plain() {
        let input = b"ready\n";
        let result = Launcher::wait_for_bridge_ready(input.as_ref()).await;
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_wait_for_bridge_ready_json() {
        let input = br#"{"type":"ready","pid":12345}"#;
        let result = Launcher::wait_for_bridge_ready(input.as_ref()).await;
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_wait_for_bridge_ready_empty() {
        let input = b"";
        let result = Launcher::wait_for_bridge_ready(input.as_ref()).await;
        assert!(!result.unwrap());
    }
}
