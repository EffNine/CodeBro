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
use std::sync::atomic::{AtomicBool, Ordering};
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
///
/// Thread creation failure is reported as an error instead of being silently
/// dropped, so a caller can never receive a receiver that waits forever
/// because the producer failed to start.
pub fn spawn_pty(
    config: PtyConfig,
    cancel: CancellationToken,
) -> Result<tokio::sync::mpsc::Receiver<PtyEvent>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<PtyEvent>(64);

    std::thread::Builder::new()
        .name("codebro-pty".to_string())
        .spawn(move || {
            pty_worker(config, cancel, tx);
        })
        .with_context(|| "Failed to start the PTY worker thread")?;

    Ok(rx)
}

/// The body of the PTY worker thread. Any [`run_pty`] failure is turned into a
/// single terminal [`PtyEvent::Error`], so the receiver always observes a
/// terminal event — never a channel that closes silently because the producer
/// failed.
fn pty_worker(
    config: PtyConfig,
    cancel: CancellationToken,
    tx: tokio::sync::mpsc::Sender<PtyEvent>,
) {
    run_pty_worker(&config, &cancel, &tx, run_pty);
}

/// The signature of a PTY runtime body (see [`run_pty`]). This is a thin,
/// test-only-visible seam that lets the worker's error-propagation contract be
/// validated deterministically without forcing an OS-level PTY allocation
/// failure. Production always passes [`run_pty`].
type PtyRun = fn(
    &PtyConfig,
    &CancellationToken,
    &tokio::sync::mpsc::Sender<PtyEvent>,
    &Arc<AtomicBool>,
) -> Result<()>;

/// Drive a [`PtyRun`] and enforce the terminal-event invariant: if the body
/// returns an error without already having emitted a terminal event, exactly
/// one terminal [`PtyEvent::Error`] is sent. A terminal event emitted by the
/// body is never duplicated.
fn run_pty_worker(
    config: &PtyConfig,
    cancel: &CancellationToken,
    tx: &tokio::sync::mpsc::Sender<PtyEvent>,
    run: PtyRun,
) {
    let terminal_sent = Arc::new(AtomicBool::new(false));
    if let Err(e) = run(config, cancel, tx, &terminal_sent) {
        // Only surface an Error if the body did not already emit a terminal
        // event (e.g. a setup path that failed after sending one).
        if !terminal_sent.load(Ordering::Relaxed) {
            let _ = send_event(tx, PtyEvent::Error(format!("PTY runtime failed: {}", e)));
        }
    }
}

fn run_pty(
    config: &PtyConfig,
    cancel: &CancellationToken,
    tx: &tokio::sync::mpsc::Sender<PtyEvent>,
    terminal_sent: &Arc<AtomicBool>,
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

    // Fatal setup failures below return an error *without* emitting an event;
    // the worker body emits the single terminal `Error`. No failure is ever
    // silently discarded.
    let mut child = pair
        .slave
        .spawn_command(builder)
        .context("Failed to spawn command")?;

    // Read-only console: the master writer is kept alive (dropping it would
    // send EOF) but the user never types into the PTY.
    let _master_writer = pair.master.take_writer().ok();
    let mut reader = pair
        .master
        .try_clone_reader()
        .context("Failed to clone PTY reader")?;

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
    // every byte has been surfaced. It stops forwarding as soon as a terminal
    // event has been emitted. A reader-thread failure is observable: the
    // child is terminated and the error is propagated to the worker.
    let reader_tx = tx.clone();
    let max_output = config.max_output;
    let reader_exit_code = exit_code.clone();
    let reader_done_flag = reader_done.clone();
    let reader_terminal = Arc::clone(terminal_sent);
    if let Err(e) = std::thread::Builder::new()
        .name("codebro-pty-reader".to_string())
        .spawn(move || {
            read_loop(
                &mut reader,
                &reader_tx,
                max_output,
                &reader_exit_code,
                &reader_done_flag,
                &reader_terminal,
            );
        })
    {
        signal_group(group_leader, libc::SIGKILL);
        let _ = child.wait();
        return Err(e).context("Failed to start the PTY reader thread");
    }

    // Waiter: polls exit / cancellation / timeout and terminates the group.
    let started = Instant::now();
    let deadline = config.timeout.map(|t| started + t);

    waiter_loop(
        &mut child,
        group_leader,
        cancel,
        deadline,
        &exit_code,
        &reader_done,
        terminal_sent,
        tx,
    )
}

/// Poll the child until it exits, the task is cancelled, the deadline passes,
/// or `try_wait` starts failing. Exactly one terminal event is emitted (the
/// terminal-event invariant). A `try_wait` error is fatal and bounded: the
/// process group is terminated, the reader is drained within its grace, and an
/// error is returned for the worker to surface as a terminal `Error` — there
/// is no infinite polling loop, no orphan, and no hanging receiver.
fn waiter_loop(
    child: &mut Box<dyn Child + Send + Sync>,
    group_leader: Option<i32>,
    cancel: &CancellationToken,
    deadline: Option<Instant>,
    exit_code: &Arc<std::sync::Mutex<Option<i32>>>,
    reader_done: &Arc<std::sync::Mutex<bool>>,
    terminal_sent: &AtomicBool,
    tx: &tokio::sync::mpsc::Sender<PtyEvent>,
) -> Result<()> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status.exit_code() as i32;
                *exit_code.lock().unwrap_or_else(|p| p.into_inner()) = Some(code);
                wait_reader_done(reader_done, Duration::from_secs(2));
                let _ = emit_terminal(tx, terminal_sent, PtyEvent::Exited { exit_code: code });
                return Ok(());
            }
            Ok(None) => {
                if cancel.is_cancelled() {
                    signal_group(group_leader, libc::SIGINT);
                    let _ = wait_for_exit(child, KILL_GRACE);
                    if child.try_wait().ok().flatten().is_none() {
                        signal_group(group_leader, libc::SIGKILL);
                        let _ = child.wait();
                    }
                    if let Ok(Some(status)) = child.try_wait() {
                        *exit_code.lock().unwrap_or_else(|p| p.into_inner()) =
                            Some(status.exit_code() as i32);
                    }
                    wait_reader_done(reader_done, Duration::from_secs(1));
                    let _ = emit_terminal(tx, terminal_sent, PtyEvent::Cancelled);
                    return Ok(());
                }

                if let Some(deadline) = deadline {
                    if Instant::now() >= deadline {
                        signal_group(group_leader, libc::SIGINT);
                        let _ = wait_for_exit(child, KILL_GRACE);
                        if child.try_wait().ok().flatten().is_none() {
                            signal_group(group_leader, libc::SIGKILL);
                            let _ = child.wait();
                        }
                        if let Ok(Some(status)) = child.try_wait() {
                            *exit_code.lock().unwrap_or_else(|p| p.into_inner()) =
                                Some(status.exit_code() as i32);
                        }
                        wait_reader_done(reader_done, Duration::from_secs(1));
                        let _ = emit_terminal(tx, terminal_sent, PtyEvent::TimedOut);
                        return Ok(());
                    }
                }

                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                // `try_wait` is failing: terminate the process group so no
                // orphan survives, drain the reader within its grace period,
                // then return an error. The worker emits exactly one terminal
                // `Error`. This path can never loop indefinitely.
                signal_group(group_leader, libc::SIGKILL);
                let _ = wait_for_exit(child, KILL_GRACE);
                wait_reader_done(reader_done, Duration::from_secs(1));
                return Err(e).context("Failed to wait for PTY child");
            }
        }
    }
}

/// Send a terminal event exactly once. A second terminal event is refused, so
/// the "exactly one terminal event per invocation" invariant holds on every
/// path (exit, cancel, timeout, error).
fn emit_terminal(
    tx: &tokio::sync::mpsc::Sender<PtyEvent>,
    terminal_sent: &AtomicBool,
    event: PtyEvent,
) -> Result<()> {
    debug_assert!(event.is_terminal());
    if terminal_sent.load(Ordering::Relaxed) {
        return Ok(());
    }
    terminal_sent.store(true, Ordering::Relaxed);
    let _ = tx.blocking_send(event);
    Ok(())
}

/// Continuously read the PTY master and forward output chunks. Stops on EOF /
/// error (which happens when the child and its session close) or as soon as a
/// terminal event has been emitted, so no `Output` ever follows the terminal
/// event. Output is capped at `max_output` characters for the result stream,
/// then discarded (but still drained) so long-running processes never grow
/// memory unboundedly and never block on a full PTY buffer.
fn read_loop(
    reader: &mut Box<dyn Read + Send>,
    tx: &tokio::sync::mpsc::Sender<PtyEvent>,
    max_output: usize,
    _exit_code: &Arc<std::sync::Mutex<Option<i32>>>,
    reader_done: &Arc<std::sync::Mutex<bool>>,
    terminal_sent: &AtomicBool,
) {
    let mut buf = [0u8; 4096];
    let mut output_sent: usize = 0;
    let mut truncated = false;

    loop {
        if terminal_sent.load(Ordering::Relaxed) {
            break;
        }
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if terminal_sent.load(Ordering::Relaxed) {
                    break;
                }
                if output_sent < max_output {
                    let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let chunk = crate::tools::shell::redact_secrets_public(&chunk);
                    let remaining = max_output - output_sent;
                    if chunk.chars().count() > remaining {
                        truncated = true;
                        let out: String = chunk.chars().take(remaining).collect();
                        output_sent = max_output;
                        if !terminal_sent.load(Ordering::Relaxed) {
                            let _ = send_event(tx, PtyEvent::Output(out));
                        }
                    } else {
                        output_sent += chunk.chars().count();
                        if !terminal_sent.load(Ordering::Relaxed) {
                            let _ = send_event(tx, PtyEvent::Output(chunk));
                        }
                    }
                }
            }
        }
        if truncated && !terminal_sent.load(Ordering::Relaxed) {
            let _ = send_event(tx, PtyEvent::Output("…[output truncated]".to_string()));
            truncated = false;
        }
    }
    *reader_done.lock().unwrap_or_else(|p| p.into_inner()) = true;
}

/// Wait up to `grace` for the reader thread to drain the PTY.
fn wait_reader_done(reader_done: &Arc<std::sync::Mutex<bool>>, grace: Duration) {
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if *reader_done.lock().unwrap_or_else(|p| p.into_inner()) {
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
        let mut rx = spawn_pty(config, cancel).unwrap();
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
        )
        .unwrap();
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
        let mut rx = spawn_pty(PtyConfig::for_command("sleep 30"), cancel.clone()).unwrap();
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

    // ─── P2: Process-group / cancellation / EOF-ordering tests ─────────────

    #[test]
    fn test_pty_cancellation_terminates_grandchildren() {
        // `sh -c 'sleep 30 & sleep 30'` leaves two independent grandchildren.
        // Cancel must kill the whole process group (not just sh), so the PTY
        // closes and the Cancelled event arrives promptly.
        let rt = Runtime::new().unwrap();
        let cancel = CancellationToken::new();
        let mut rx = spawn_pty(
            PtyConfig::for_command("sleep 30 & sleep 30"),
            cancel.clone(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let start = Instant::now();
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
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "process group must be killed promptly so the session closes"
        );
    }

    #[test]
    fn test_pty_cancellation_escalates_when_sigint_is_ignored() {
        // `trap '' INT; sleep 30` ignores SIGINT; cancellation must escalate
        // from SIGINT to SIGKILL after the grace period and still complete.
        let rt = Runtime::new().unwrap();
        let cancel = CancellationToken::new();
        let mut rx = spawn_pty(
            PtyConfig::for_command("trap '' INT; sleep 30"),
            cancel.clone(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let start = Instant::now();
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
        assert!(
            cancelled,
            "expected Cancelled event even with SIGINT ignored"
        );
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "SIGKILL escalation must terminate the SIGINT-ignoring process"
        );
    }

    #[test]
    fn test_pty_all_output_delivered_before_exited() {
        // EOF ordering: the Exited event must never arrive before the final
        // output chunk has been surfaced (reader completion is ordered first).
        let rt = Runtime::new().unwrap();
        let mut rx = spawn_pty(
            PtyConfig::for_command("printf 'tail-marker-xyz\\n'; exit 0"),
            CancellationToken::new(),
        )
        .unwrap();
        let mut saw_tail = false;
        let mut saw_exit_before_tail = false;
        rt.block_on(async {
            while let Some(ev) = rx.recv().await {
                match &ev {
                    PtyEvent::Output(c) if c.contains("tail-marker-xyz") => saw_tail = true,
                    PtyEvent::Exited { .. } if !saw_tail => {
                        saw_exit_before_tail = true;
                        break;
                    }
                    PtyEvent::Exited { .. } => break,
                    _ => {}
                }
            }
        });
        assert!(
            !saw_exit_before_tail,
            "Exited was emitted before the final output chunk"
        );
        assert!(saw_tail, "final output chunk must be surfaced");
    }

    #[test]
    fn test_pty_large_output_still_delivers_exit_ordering() {
        // A large burst must still deliver Exited only after output is drained.
        let rt = Runtime::new().unwrap();
        let mut config = PtyConfig::for_command("yes z | head -c 500000; exit 0");
        config.max_output = 4096;
        let mut rx = spawn_pty(config, CancellationToken::new()).unwrap();
        let mut exit_code = None;
        rt.block_on(async {
            while let Some(ev) = rx.recv().await {
                match &ev {
                    PtyEvent::Exited { exit_code: code } => {
                        exit_code = Some(*code);
                        break;
                    }
                    _ => {}
                }
            }
        });
        assert_eq!(exit_code, Some(0), "exit code must remain authoritative");
    }

    // ─── Sprint 28.1: worker-failure / terminal-event-invariant tests ───────

    /// A fake portable-pty child whose `try_wait` always fails. Lets the
    /// waiter's error branch be tested deterministically (no timing races).
    #[derive(Debug)]
    struct ErrTryWaitChild;

    impl portable_pty::ChildKiller for ErrTryWaitChild {
        fn kill(&mut self) -> std::io::Result<()> {
            Ok(())
        }
        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
            Box::new(ErrTryWaitChild)
        }
    }

    impl portable_pty::Child for ErrTryWaitChild {
        fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "forced try_wait failure",
            ))
        }
        fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
            Ok(portable_pty::ExitStatus::with_exit_code(1))
        }
        fn process_id(&self) -> Option<u32> {
            None
        }
    }

    /// Collect events until a terminal event or until `max_events`, so a
    /// broken invariant fails fast instead of hanging the test runner.
    fn drain_events(
        rt: &Runtime,
        rx: tokio::sync::mpsc::Receiver<PtyEvent>,
        max_events: usize,
    ) -> Vec<PtyEvent> {
        let mut events = Vec::new();
        rt.block_on(async {
            let mut rx = rx;
            while let Some(ev) = rx.recv().await {
                events.push(ev.clone());
                if ev.is_terminal() {
                    break;
                }
                if events.len() >= max_events {
                    break;
                }
            }
        });
        events
    }

    #[test]
    fn test_worker_runtime_failure_emits_single_terminal_error() {
        // A PTY runtime body that fails without emitting a terminal event must
        // surface exactly one terminal Error to the receiver — the receiver
        // can never hang because the producer failed.
        let rt = Runtime::new().unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel::<PtyEvent>(64);
        run_pty_worker(
            &PtyConfig::for_command("echo hi"),
            &CancellationToken::new(),
            &tx,
            |_, _, _, _| Err(anyhow::anyhow!("forced PTY runtime failure")),
        );
        drop(tx);
        let events = drain_events(&rt, rx, 16);
        assert_eq!(
            events.len(),
            1,
            "expected exactly one terminal event, got {:?}",
            events
        );
        assert!(events[0].is_terminal());
        assert!(
            matches!(&events[0], PtyEvent::Error(m) if m.contains("forced PTY runtime failure")),
            "expected an Error event, got {:?}",
            events[0]
        );
    }

    #[test]
    fn test_worker_does_not_duplicate_terminal_event() {
        // If the runtime body already emitted a terminal event before failing,
        // the worker must not emit a second terminal event.
        let rt = Runtime::new().unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel::<PtyEvent>(64);
        run_pty_worker(
            &PtyConfig::for_command("echo hi"),
            &CancellationToken::new(),
            &tx,
            |_, _, tx, terminal_sent| {
                emit_terminal(
                    tx,
                    terminal_sent,
                    PtyEvent::Error("already failed".to_string()),
                )?;
                Err(anyhow::anyhow!("body also returns an error"))
            },
        );
        drop(tx);
        let events = drain_events(&rt, rx, 16);
        assert_eq!(
            events.len(),
            1,
            "expected exactly one terminal event, got {:?}",
            events
        );
        assert!(
            matches!(&events[0], PtyEvent::Error(m) if m == "already failed"),
            "the body's own terminal event must be preserved, got {:?}",
            events[0]
        );
    }

    #[test]
    fn test_waiter_try_wait_error_is_bounded_and_clean() {
        // The waiter must not spin forever when try_wait starts failing: it
        // terminates the group, drains bounded, and returns an error.
        let mut child: Box<dyn portable_pty::Child + Send + Sync> = Box::new(ErrTryWaitChild);
        let exit_code = Arc::new(std::sync::Mutex::new(None));
        let reader_done = Arc::new(std::sync::Mutex::new(true)); // already drained
        let terminal_sent = AtomicBool::new(false);
        let (tx, rx) = tokio::sync::mpsc::channel::<PtyEvent>(64);

        let start = Instant::now();
        let err = waiter_loop(
            &mut child,
            None,
            &CancellationToken::new(),
            None,
            &exit_code,
            &reader_done,
            &terminal_sent,
            &tx,
        )
        .expect_err("try_wait failure must surface as an error");
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "try_wait failure must not loop forever"
        );
        assert!(
            err.to_string().contains("Failed to wait for PTY child"),
            "got: {}",
            err
        );
        // The waiter emits no terminal event on the error path (the worker
        // does); the channel must close cleanly.
        drop(tx);
        let events = drain_events(&Runtime::new().unwrap(), rx, 16);
        assert!(events.is_empty());
    }

    #[test]
    fn test_terminal_event_uniqueness_across_timeout() {
        let rt = Runtime::new().unwrap();
        let mut config = PtyConfig::for_command("sleep 30");
        config.timeout = Some(Duration::from_millis(200));
        let rx = spawn_pty(config, CancellationToken::new()).unwrap();
        let events = drain_events(&rt, rx, 32);
        let terminals: Vec<_> = events.iter().filter(|e| e.is_terminal()).collect();
        assert_eq!(
            terminals.len(),
            1,
            "expected exactly one terminal event, got {:?}",
            events
        );
        assert!(matches!(terminals[0], PtyEvent::TimedOut));
    }

    #[test]
    fn test_terminal_event_uniqueness_across_cancellation() {
        let rt = Runtime::new().unwrap();
        let cancel = CancellationToken::new();
        let rx = spawn_pty(PtyConfig::for_command("sleep 30"), cancel.clone()).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        cancel.cancel();
        let events = drain_events(&rt, rx, 32);
        let terminals: Vec<_> = events.iter().filter(|e| e.is_terminal()).collect();
        assert_eq!(
            terminals.len(),
            1,
            "expected exactly one terminal event, got {:?}",
            events
        );
        assert!(matches!(terminals[0], PtyEvent::Cancelled));
    }

    #[test]
    fn test_terminal_event_uniqueness_across_normal_exit() {
        let rt = Runtime::new().unwrap();
        let rx = spawn_pty(
            PtyConfig::for_command("printf 'x'; exit 0"),
            CancellationToken::new(),
        )
        .unwrap();
        let events = drain_events(&rt, rx, 32);
        let terminals: Vec<_> = events.iter().filter(|e| e.is_terminal()).collect();
        assert_eq!(
            terminals.len(),
            1,
            "expected exactly one terminal event, got {:?}",
            events
        );
        assert!(matches!(terminals[0], PtyEvent::Exited { exit_code: 0 }));
    }

    #[test]
    fn test_no_output_after_terminal_event() {
        // Once a terminal event has been emitted, no further Output events may
        // follow (the reader stops forwarding on terminal_sent). Drain the
        // whole channel (until the worker drops its sender) to prove it.
        let rt = Runtime::new().unwrap();
        let mut rx = spawn_pty(
            PtyConfig::for_command("printf 'before\\n'; exit 0"),
            CancellationToken::new(),
        )
        .unwrap();
        let events = rt.block_on(async {
            let mut all = Vec::new();
            while let Some(ev) = rx.recv().await {
                all.push(ev);
            }
            all
        });
        let terminal_count = events.iter().filter(|e| e.is_terminal()).count();
        assert_eq!(
            terminal_count, 1,
            "expected exactly one terminal event, got {:?}",
            events
        );
        let mut seen_terminal = false;
        for ev in &events {
            if seen_terminal {
                assert!(
                    !matches!(ev, PtyEvent::Output(_)),
                    "no Output may follow a terminal event: {:?}",
                    events
                );
            }
            if ev.is_terminal() {
                seen_terminal = true;
            }
        }
    }
}
