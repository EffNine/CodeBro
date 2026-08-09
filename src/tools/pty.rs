//! Real PTY-backed process execution.
//!
//! Every task that runs a shell/process operation gets a pseudo-terminal.
//! Output is surfaced live, token by token, while the process is still
//! running — the runtime never waits for completion before emitting output.
//! ANSI color/control sequences are preserved in the stream (they are
//! rendered, not stripped). The PTY is read-only from the user's perspective:
//! input travels through the main input field, never into the console.
//!
//! ```text
//! task
//!   ↓
//! PtyTask (portable-pty session)
//!   ↓
//! PtyEvent stream (Output / Exited / TimedOut / Cancelled / Error)
//!   ↓
//! runtime event bus → TUI terminal console
//! ```
//!
//! Safety behavior is preserved: timeouts, output caps, and secret redaction
//! are enforced here (see [`PtyConfig`]). Cancellation follows terminal
//! semantics: a `Ctrl+C` cancels the task by sending `SIGINT` to the child's
//! process group (the PTY session leader), then escalates to `SIGKILL` if the
//! process does not exit within a grace period.

use anyhow::{Context, Result};
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};

use crate::cancellation::CancellationToken;

/// Maximum characters of a single task's PTY output that flow into the tool
/// result. The console buffer may be larger; this bounds the context/LLM path.
pub const MAX_PTY_OUTPUT: usize = 32_768;

/// The PTY column width used when creating the pseudo-terminal.
pub const PTY_COLS: u16 = 120;
/// The PTY row height used when creating the pseudo-terminal.
pub const PTY_ROWS: u16 = 40;

/// Grace period after `SIGINT` before escalating to `SIGKILL` on
/// cancel/timeout.
const KILL_GRACE: Duration = Duration::from_millis(1_000);

/// Configuration for one PTY-backed process run.
#[derive(Debug, Clone)]
pub struct PtyConfig {
    /// The shell command line to execute (`sh -c <command>`).
    pub command: String,
    /// Optional working directory for the child.
    pub working_directory: Option<PathBuf>,
    /// Extra environment variables for the child.
    pub environment: Vec<(String, String)>,
    /// Optional hard timeout; the process is terminated when exceeded.
    pub timeout: Option<Duration>,
    /// Maximum characters forwarded to the result stream.
    pub max_output: usize,
}

impl Default for PtyConfig {
    fn default() -> Self {
        PtyConfig {
            command: String::new(),
            working_directory: None,
            environment: Vec::new(),
            timeout: Some(Duration::from_secs(300)),
            max_output: MAX_PTY_OUTPUT,
        }
    }
}

impl PtyConfig {
    /// A ready-to-use config for a single command line.
    pub fn for_command(command: impl Into<String>) -> Self {
        PtyConfig {
            command: command.into(),
            ..Default::default()
        }
    }
}

/// One event from a running PTY task.
#[derive(Debug, Clone, PartialEq)]
pub enum PtyEvent {
    /// Live output chunk (ANSI sequences preserved).
    Output(String),
    /// The process exited with the given code.
    Exited { exit_code: i32 },
    /// The process was terminated because the task was cancelled (Ctrl+C).
    Cancelled,
    /// The process was terminated because it exceeded the configured timeout.
    TimedOut,
    /// The task failed to start or encountered a fatal error.
    Error(String),
}

impl PtyEvent {
    /// Whether the task has finished (terminal event).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            PtyEvent::Exited { .. } | PtyEvent::Cancelled | PtyEvent::TimedOut | PtyEvent::Error(_)
        )
    }

    /// A short one-line summary for the activity stream.
    pub fn summary(&self) -> String {
        match self {
            PtyEvent::Output(_) => "output".to_string(),
            PtyEvent::Exited { exit_code } => {
                if *exit_code == 0 {
                    "completed".to_string()
                } else {
                    format!("failed (exit {})", exit_code)
                }
            }
            PtyEvent::Cancelled => "cancelled".to_string(),
            PtyEvent::TimedOut => "timed out".to_string(),
            PtyEvent::Error(e) => format!("error: {}", e),
        }
    }
}

/// Spawn a command in a real PTY and stream its output.
///
/// The PTY runs in dedicated OS threads so the async runtime and TUI event
/// loop are never blocked. Returns a channel that yields [`PtyEvent`]s until a
/// terminal event. The caller shares the [`CancellationToken`]: setting it
/// sends `SIGINT` to the child's process group, exactly like Ctrl+C in a
/// terminal.
pub fn spawn_pty(
    config: PtyConfig,
    cancel: CancellationToken,
) -> tokio::sync::mpsc::Receiver<PtyEvent> {
    let (tx, rx) = tokio::sync::mpsc::channel::<PtyEvent>(64);

    std::thread::Builder::new()
        .name("codebro-pty".to_string())
        .spawn(move || {
            let _ = run_pty(&config, &cancel, &tx);
        })
        .ok();

    rx
}

fn run_pty(
    config: &PtyConfig,
    cancel: &CancellationToken,
    tx: &tokio::sync::mpsc::Sender<PtyEvent>,
) -> Result<()> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: PTY_ROWS,
            cols: PTY_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("Failed to allocate PTY")?;

    let mut builder = CommandBuilder::new("sh");
    builder.arg("-c");
    builder.arg(config.command.clone());
    if let Some(ref wd) = config.working_directory {
        builder.cwd(wd);
    }
    for (key, value) in &config.environment {
        builder.env(key, value);
    }

    let mut child = match pair.slave.spawn_command(builder) {
        Ok(child) => child,
        Err(e) => {
            let _ = send_event(tx, PtyEvent::Error(format!("Failed to spawn: {}", e)));
            return Err(e).context("Failed to spawn command");
        }
    };

    // Read-only console: the master writer is kept alive (dropping it would
    // send EOF) but the user never types into the PTY.
    let _master_writer = pair.master.take_writer().ok();
    let mut reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            let _ = send_event(tx, PtyEvent::Error(format!("PTY read failed: {}", e)));
            return Err(e).context("Failed to clone PTY reader");
        }
    };

    // The child is a session leader; its pgid equals its pid, so signaling the
    // group terminates the whole tree (sh + descendants).
    let group_leader = pair.master.process_group_leader();

    // Shared state between the reader and waiter threads so the terminal
    // event is never emitted before all pending output has been drained.
    let exit_code: Arc<std::sync::Mutex<Option<i32>>> = Arc::new(std::sync::Mutex::new(None));
    let reader_done: Arc<std::sync::Mutex<bool>> = Arc::new(std::sync::Mutex::new(false));

    // Reader thread: forwards output continuously so streaming is live, and
    // keeps draining the PTY even after the cap is hit so the child never
    // blocks on a full buffer. Sets `reader_done` at EOF so the waiter knows
    // every byte has been surfaced.
    let reader_tx = tx.clone();
    let max_output = config.max_output;
    let reader_exit_code = exit_code.clone();
    let reader_done_flag = reader_done.clone();
    std::thread::Builder::new()
        .name("codebro-pty-reader".to_string())
        .spawn(move || {
            read_loop(
                &mut reader,
                &reader_tx,
                max_output,
                &reader_exit_code,
                &reader_done_flag,
            );
        })
        .ok();

    // Waiter: polls exit / cancellation / timeout and terminates the group.
    let started = Instant::now();
    let deadline = config.timeout.map(|t| started + t);

    loop {
        if let Ok(Some(status)) = child.try_wait() {
            let code = status.exit_code() as i32;
            *exit_code.lock().unwrap() = Some(code);
            wait_reader_done(&reader_done, Duration::from_secs(2));
            let _ = send_event(tx, PtyEvent::Exited { exit_code: code });
            return Ok(());
        }

        if cancel.is_cancelled() {
            signal_group(group_leader, libc::SIGINT);
            let _ = wait_for_exit(&mut child, KILL_GRACE);
            if child.try_wait().ok().flatten().is_none() {
                signal_group(group_leader, libc::SIGKILL);
                let _ = child.wait();
            }
            if let Ok(Some(status)) = child.try_wait() {
                *exit_code.lock().unwrap() = Some(status.exit_code() as i32);
            }
            wait_reader_done(&reader_done, Duration::from_secs(1));
            let _ = send_event(tx, PtyEvent::Cancelled);
            return Ok(());
        }

        if let Some(deadline) = deadline {
            if Instant::now() >= deadline {
                signal_group(group_leader, libc::SIGINT);
                let _ = wait_for_exit(&mut child, KILL_GRACE);
                if child.try_wait().ok().flatten().is_none() {
                    signal_group(group_leader, libc::SIGKILL);
                    let _ = child.wait();
                }
                if let Ok(Some(status)) = child.try_wait() {
                    *exit_code.lock().unwrap() = Some(status.exit_code() as i32);
                }
                wait_reader_done(&reader_done, Duration::from_secs(1));
                let _ = send_event(tx, PtyEvent::TimedOut);
                return Ok(());
            }
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Continuously read the PTY master and forward output chunks. Stops on EOF /
/// error (which happens when the child and its session close). Output is
/// capped at `max_output` characters for the result stream, then discarded
/// (but still drained) so long-running processes never grow memory unboundedly
/// and never block on a full PTY buffer.
fn read_loop(
    reader: &mut Box<dyn Read + Send>,
    tx: &tokio::sync::mpsc::Sender<PtyEvent>,
    max_output: usize,
    _exit_code: &Arc<std::sync::Mutex<Option<i32>>>,
    reader_done: &Arc<std::sync::Mutex<bool>>,
) {
    let mut buf = [0u8; 4096];
    let mut output_sent: usize = 0;
    let mut truncated = false;

    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if output_sent < max_output {
                    let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let chunk = crate::tools::shell::redact_secrets_public(&chunk);
                    let remaining = max_output - output_sent;
                    if chunk.chars().count() > remaining {
                        truncated = true;
                        let out: String = chunk.chars().take(remaining).collect();
                        output_sent = max_output;
                        let _ = send_event(tx, PtyEvent::Output(out));
                    } else {
                        output_sent += chunk.chars().count();
                        let _ = send_event(tx, PtyEvent::Output(chunk));
                    }
                }
            }
        }
        if truncated {
            let _ = send_event(tx, PtyEvent::Output("…[output truncated]".to_string()));
            truncated = false;
        }
    }
    *reader_done.lock().unwrap() = true;
}

/// Wait up to `grace` for the reader thread to drain the PTY.
fn wait_reader_done(reader_done: &Arc<std::sync::Mutex<bool>>, grace: Duration) {
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if *reader_done.lock().unwrap() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Block up to `grace` for the child to exit.
fn wait_for_exit(child: &mut Box<dyn Child + Send + Sync>, grace: Duration) -> bool {
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Send a signal to the child's process group (whole session), if known.
fn signal_group(leader: Option<i32>, signal: i32) {
    if let Some(pid) = leader {
        unsafe {
            libc::killpg(pid, signal);
        }
    }
}

/// Send an event, ignoring send failures (the receiver may be gone).
fn send_event(tx: &tokio::sync::mpsc::Sender<PtyEvent>, event: PtyEvent) -> Result<()> {
    let _ = tx.blocking_send(event);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;

    fn collect(
        rt: &Runtime,
        config: PtyConfig,
        cancel: CancellationToken,
    ) -> (Vec<PtyEvent>, String) {
        let mut rx = spawn_pty(config, cancel);
        let mut events = Vec::new();
        let mut out = String::new();
        rt.block_on(async {
            while let Some(ev) = rx.recv().await {
                match &ev {
                    PtyEvent::Output(c) => out.push_str(c),
                    _ => {}
                }
                events.push(ev.clone());
                if ev.is_terminal() {
                    break;
                }
            }
        });
        (events, out)
    }

    #[test]
    fn test_pty_streams_output_live_and_exits() {
        let rt = Runtime::new().unwrap();
        let (events, out) = collect(
            &rt,
            PtyConfig::for_command("printf 'hello\\nworld\\n'"),
            CancellationToken::new(),
        );
        assert!(out.contains("hello"));
        assert!(out.contains("world"));
        let exit = events
            .iter()
            .find_map(|e| match e {
                PtyEvent::Exited { exit_code } => Some(*exit_code),
                _ => None,
            })
            .expect("must emit Exited");
        assert_eq!(exit, 0);
    }

    #[test]
    fn test_pty_output_is_streamed_not_batched() {
        let rt = Runtime::new().unwrap();
        let mut rx = spawn_pty(
            PtyConfig::for_command(
                "printf 'one\\n'; sleep 0.2; printf 'two\\n'; sleep 0.2; printf 'three\\n'",
            ),
            CancellationToken::new(),
        );
        let mut saw_first = false;
        rt.block_on(async {
            while let Some(ev) = rx.recv().await {
                match &ev {
                    PtyEvent::Output(c) if c.contains("one") => saw_first = true,
                    PtyEvent::Exited { .. } => break,
                    _ => {}
                }
                if saw_first {
                    break;
                }
            }
        });
        assert!(saw_first, "first line must arrive before the process exits");
    }

    #[test]
    fn test_pty_failure_exit_code() {
        let rt = Runtime::new().unwrap();
        let (events, out) = collect(
            &rt,
            PtyConfig::for_command("echo before; exit 3"),
            CancellationToken::new(),
        );
        assert!(out.contains("before"));
        let exit = events
            .iter()
            .find_map(|e| match e {
                PtyEvent::Exited { exit_code } => Some(*exit_code),
                _ => None,
            })
            .expect("must emit Exited");
        assert_eq!(exit, 3);
    }

    #[test]
    fn test_pty_timeout() {
        let rt = Runtime::new().unwrap();
        let mut config = PtyConfig::for_command("sleep 30");
        config.timeout = Some(Duration::from_millis(300));
        let (events, _) = collect(&rt, config, CancellationToken::new());
        assert!(
            events.iter().any(|e| matches!(e, PtyEvent::TimedOut)),
            "expected TimedOut event, got {:?}",
            events
        );
    }

    #[test]
    fn test_pty_cancellation() {
        let rt = Runtime::new().unwrap();
        let cancel = CancellationToken::new();
        let mut rx = spawn_pty(PtyConfig::for_command("sleep 30"), cancel.clone());
        std::thread::sleep(Duration::from_millis(200));
        cancel.cancel();
        let mut cancelled = false;
        rt.block_on(async {
            while let Some(ev) = rx.recv().await {
                if matches!(ev, PtyEvent::Cancelled) {
                    cancelled = true;
                    break;
                }
                if ev.is_terminal() {
                    break;
                }
            }
        });
        assert!(cancelled, "expected Cancelled event");
    }

    #[test]
    fn test_pty_preserves_ansi() {
        let rt = Runtime::new().unwrap();
        let (_, out) = collect(
            &rt,
            PtyConfig::for_command("printf '\\033[31mred\\033[0m'"),
            CancellationToken::new(),
        );
        assert!(out.contains("\x1b[31m"), "ANSI escape must be preserved");
        assert!(out.contains("red"));
    }

    #[test]
    fn test_pty_large_output_is_capped() {
        let rt = Runtime::new().unwrap();
        let mut config = PtyConfig::for_command("yes x | head -c 200000");
        config.max_output = 1000;
        let (_, out) = collect(&rt, config, CancellationToken::new());
        assert!(
            out.chars().count() <= 1000 + 64,
            "output exceeded cap: {}",
            out.chars().count()
        );
    }

    #[test]
    fn test_pty_redacts_secrets() {
        let rt = Runtime::new().unwrap();
        let (_, out) = collect(
            &rt,
            PtyConfig::for_command("echo 'sk-abcdefghijklmnopqrstuvwxyz012345'"),
            CancellationToken::new(),
        );
        assert!(
            !out.contains("sk-abcdefghijklmnopqrstuvwxyz012345"),
            "secret leaked through PTY output"
        );
        assert!(out.contains("REDACTED"));
    }

    #[test]
    fn test_pty_working_directory() {
        let rt = Runtime::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut config = PtyConfig::for_command("pwd");
        config.working_directory = Some(dir.path().to_path_buf());
        let (_, out) = collect(&rt, config, CancellationToken::new());
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        assert!(out.contains(&canonical.to_string_lossy().to_string()));
    }
}
