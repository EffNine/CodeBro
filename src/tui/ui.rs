#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use ratatui::Terminal;
use std::io::stdout;
use std::sync::Arc;
use std::time::Duration;

use crate::agent::events::AgentEvent;
use crate::agent::status::AgentStatus;
use crate::agent::task_graph::TaskStatus;
use crate::config::Config;
use crate::tui::actions::{ActionStream, UiActionGroup, UiActionKind, UiActionStatus};
use crate::tui::animation::SPINNER_FRAMES;
use crate::tui::app::{MessageRole, PendingAction, TuiApp};
use crate::tui::commands::{self, CommandNamespace};
use crate::tui::console::PtyConsole;
use crate::tui::dashboard::Dashboard;
use crate::tui::events::{self, Shortcut};
use crate::tui::theme::{Phase, StatusGlyph, THEME};

const FRAME_INTERVAL: Duration = Duration::from_millis(50);
const ANIM_FRAME_MS: u128 = 80;

/// Minimum terminal width below which the rail collapses automatically.
const COMPACT_MIN_WIDTH: u16 = 100;
/// Minimum terminal height below which the UI sheds chrome automatically.
const COMPACT_MIN_HEIGHT: u16 = 26;
/// Rail width is ~22% of the terminal width, at least this many columns and
/// never more than 25%.
const RAIL_MIN_WIDTH: u16 = 24;

/// Truncates a string to `width` Unicode characters (never panics on long text).
pub fn truncate_to(s: &str, width: usize) -> String {
    let count = s.chars().count();
    if count <= width {
        s.to_string()
    } else if width == 0 {
        String::new()
    } else {
        let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

pub fn run(mut app: TuiApp) -> Result<()> {
    use crossterm::event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    };
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };
    use crossterm::ExecutableCommand;
    use std::io::stdout as io_stdout;

    enable_raw_mode()?;
    let mut out = io_stdout();
    out.execute(EnterAlternateScreen)?;
    out.execute(EnableBracketedPaste)?;
    out.execute(EnableMouseCapture)?;

    let result = run_loop(&mut app);

    let _ = out.execute(DisableMouseCapture);
    let _ = out.execute(DisableBracketedPaste);
    let _ = out.execute(LeaveAlternateScreen);
    let _ = disable_raw_mode();
    result
}

fn run_loop(app: &mut TuiApp) -> Result<()> {
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let (tx, rx) = std::sync::mpsc::channel();
    app.tx = tx.clone();
    events::start_event_loop(tx)?;

    let mut needs_redraw = true;

    loop {
        if needs_redraw {
            terminal.draw(|f| ui(f, app))?;
            needs_redraw = false;
        }

        if app.should_quit {
            break;
        }

        if app.should_clear {
            app.clear_screen();
            app.dashboard.clear_logs();
            app.should_clear = false;
            needs_redraw = true;
        }

        match rx.recv_timeout(FRAME_INTERVAL) {
            Ok(msg) => {
                let handled = handle_event(msg, app);
                if handled || app.dashboard.tick() {
                    needs_redraw = true;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if app.dashboard.tick() {
                    needs_redraw = true;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                app.should_quit = true;
            }
        }
    }

    terminal.clear()?;
    Ok(())
}

fn handle_event(msg: events::AppEvent, app: &mut TuiApp) -> bool {
    match msg {
        events::AppEvent::Input(key) => {
            handle_key(key, app);
            true
        }
        events::AppEvent::Quit => {
            app.should_quit = true;
            true
        }
        events::AppEvent::TaskFinished { success } => {
            app.action_stream.finalize_response(success);
            // Keep groups live under the current turn until the next user
            // message seals them (turn-scoped timeline).
            true
        }
        events::AppEvent::Response(content) => {
            app.is_loading = false;
            app.dashboard.dismiss_welcome();
            app.dashboard.end_streaming();
            app.dashboard.clear_error();
            app.add_message(MessageRole::Assistant, content);
            app.end_task();
            app.cancel_token = None;
            true
        }
        events::AppEvent::StreamChunk(content) => {
            app.dashboard.push_stream_chunk(&content);
            true
        }
        events::AppEvent::AgentEvent(event) => {
            app.handle_agent_event(event.clone());
            if matches!(event, AgentEvent::AgentStarted { .. }) {
                app.dashboard.dismiss_welcome();
            }
            true
        }
        events::AppEvent::Resize(_, _) => {
            app.scroll_to_bottom();
            true
        }
        events::AppEvent::ModelsFetched(models) => {
            app.handle_models_fetched(models);
            true
        }
        events::AppEvent::ModelsFetchFailed(err) => {
            app.handle_models_failed(err);
            true
        }
        events::AppEvent::ProviderHealthResults(results) => {
            app.handle_provider_health_results(results);
            true
        }
        events::AppEvent::WorkspaceDiscovered {
            discovery,
            capabilities,
            mcp_servers,
        } => {
            app.workspace_panel.discovery = Some(discovery);
            app.workspace_panel.capability_discovery = Some(capabilities);
            app.workspace_panel.mcp_servers = mcp_servers;
            app.add_message(
                MessageRole::System,
                "Workspace discovery complete".to_string(),
            );
            true
        }
        events::AppEvent::Paste(text) => {
            // Masked API-key mode must receive paste into the secure buffer,
            // never into the ordinary input (where secrets could leak).
            if let Some(ref mut secure) = app.secure_input {
                secure.buffer.push_str(&text);
            } else {
                app.insert_text(&text);
            }
            true
        }
        events::AppEvent::Mouse(mouse) => {
            use crossterm::event::MouseEventKind;
            match mouse.kind {
                MouseEventKind::ScrollDown => app.mouse_scroll(-3),
                MouseEventKind::ScrollUp => app.mouse_scroll(3),
                _ => {}
            }
            true
        }
    }
}

fn handle_key(key: crossterm::event::KeyEvent, app: &mut TuiApp) {
    match key.kind {
        KeyEventKind::Press => {
            if app.dashboard.model_picker.is_open() {
                handle_model_picker_key(key, app);
                return;
            }
            if app.dashboard.show_command_palette {
                handle_palette_key(key, app);
                return;
            }
            if app.pending_confirmation.is_some() {
                handle_confirmation_key(key, app);
                return;
            }
            // Masked secret input has the highest priority: keys are consumed
            // by the secure buffer, never by the main input field.
            if app.secure_input.is_some() {
                handle_secure_input_key(key, app);
                return;
            }

            if let Some(shortcut) = events::check_key_shortcuts(&key) {
                handle_shortcut(shortcut, app);
                return;
            }

            match key.code {
                KeyCode::Tab => {
                    // Empty input: cycle the focused action group. Otherwise
                    // context-aware completion for `/`, `//`, `!`.
                    if app.input.is_empty() {
                        if app.action_stream.cycle_focus(true) {
                            app.dashboard
                                .log("info", "focused action group".to_string());
                        }
                    } else if app.input.starts_with('/') || app.input.starts_with('!') {
                        let candidates = commands::completion_candidates(&app.input, app);
                        let names: Vec<String> =
                            candidates.iter().map(|c| c.command.to_string()).collect();
                        if names.is_empty() {
                            app.dashboard.autocomplete.clear();
                        } else {
                            app.dashboard.autocomplete_command(&mut app.input, names);
                        }
                    }
                }
                KeyCode::Up => {
                    if app.input.is_empty() {
                        app.scroll_up();
                    } else {
                        app.history_previous();
                    }
                }
                KeyCode::Down => {
                    if app.input.is_empty() {
                        app.scroll_down();
                    } else {
                        app.history_next();
                    }
                }
                KeyCode::Left => app.cursor_left(),
                KeyCode::Right => app.cursor_right(),
                KeyCode::Home => app.cursor_home(),
                KeyCode::End => app.scroll_to_bottom(),
                KeyCode::PageUp => {
                    for _ in 0..10 {
                        app.scroll_up();
                    }
                }
                KeyCode::PageDown => {
                    for _ in 0..10 {
                        app.scroll_down();
                    }
                }
                KeyCode::Enter => {
                    if key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::SHIFT)
                    {
                        app.insert_char('\n');
                        return;
                    }
                    if app.input.is_empty() {
                        // Expand/collapse the focused action group.
                        if !app.action_stream.toggle_focused() {
                            app.scroll_to_bottom();
                        }
                        return;
                    }

                    let input = app.input.trim().to_string();
                    if !input.is_empty() {
                        // Inline `//apikey <provider> <key>` must never be
                        // stored in history; the masked path is the only
                        // accepted way to set a key.
                        if !is_inline_apikey(&input) {
                            app.push_history(input.clone());
                        }
                        app.clear_input();
                        submit_input(input, app);
                    }
                }
                KeyCode::Backspace => app.backspace(),
                KeyCode::Esc => {
                    dismiss_top_overlay(app);
                }
                KeyCode::Char('?') if app.input.is_empty() => {
                    app.add_message(MessageRole::System, help_text(app));
                }
                KeyCode::Char(c) => {
                    app.insert_char(c);
                    // Live-filter the completion list while typing a command.
                    if app.input.starts_with('/') || app.input.starts_with('!') {
                        let candidates = commands::completion_candidates(&app.input, app);
                        app.dashboard.autocomplete =
                            candidates.iter().map(|c| c.command.to_string()).collect();
                        if app.dashboard.autocomplete.is_empty() {
                            app.dashboard.autocomplete_index = 0;
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// Submit an input line: `/`, `//`, `!` commands or a natural-language task.
fn submit_input(input: String, app: &mut TuiApp) {
    match commands::namespace_of(&input) {
        Some(CommandNamespace::Shell) => {
            let command = input.trim_start_matches('!').trim().to_string();
            if command.is_empty() {
                return;
            }
            if is_dangerous_shell(&command) {
                app.pending_confirmation = Some((
                    format!(
                        "This command may be destructive. Proceed? `{}` (y/n)",
                        crate::tools::shell::redact_secrets_public(&command)
                    ),
                    PendingAction::RunShell(command),
                ));
                return;
            }
            run_command_task(app, "shell", command);
        }
        Some(CommandNamespace::Engineering) => {
            handle_engineering_command(&input, app);
        }
        Some(CommandNamespace::Runtime) => {
            handle_runtime_command(&input, app);
        }
        None => {
            submit_task(app, input);
        }
    }
}

/// Close the topmost overlay / transient UI. Design-spec: Esc dismisses.
fn dismiss_top_overlay(app: &mut TuiApp) {
    if !app.dashboard.autocomplete.is_empty() {
        app.dashboard.autocomplete.clear();
        app.dashboard.autocomplete_index = 0;
        return;
    }
    if app.dashboard.show_command_palette {
        app.dashboard.toggle_command_palette();
        return;
    }
    if app.dashboard.model_picker.is_open() {
        app.dashboard.model_picker.close();
        return;
    }
    if app.show_console {
        app.show_console = false;
        return;
    }
    if app.dashboard.show_agents {
        app.dashboard.show_agents = false;
        return;
    }
    if app.dashboard.show_task_graph {
        app.dashboard.show_task_graph = false;
        return;
    }
    if app.dashboard.show_metrics {
        app.dashboard.show_metrics = false;
        return;
    }
    if app.dashboard.show_coordination {
        app.dashboard.show_coordination = false;
        return;
    }
    if app.dashboard.show_memory {
        app.dashboard.show_memory = false;
        return;
    }
    if app.dashboard.show_trace {
        app.dashboard.show_trace = false;
    }
}

/// Whether an input line is an inline `//apikey <provider> <key>` (which must
/// be rejected and never stored in history).
fn is_inline_apikey(input: &str) -> bool {
    let trimmed = input.trim();
    if !trimmed.starts_with("//apikey") {
        return false;
    }
    trimmed.split_whitespace().count() >= 3
}

/// Keys for the masked secret-input mode: printable chars append to the
/// buffer, Backspace removes, Enter stores the secret securely, Esc cancels.
fn handle_secure_input_key(key: crossterm::event::KeyEvent, app: &mut TuiApp) {
    let Some(state) = app.secure_input.as_mut() else {
        return;
    };
    match key.code {
        KeyCode::Char(c) => {
            state.buffer.push(c);
        }
        KeyCode::Backspace => {
            state.buffer.pop();
        }
        KeyCode::Enter => {
            let provider = state.provider.clone();
            let secret = std::mem::take(&mut state.buffer);
            app.secure_input = None;
            if secret.is_empty() {
                app.add_message(
                    MessageRole::System,
                    "API key not set: input was empty. Run `//apikey <provider>` to retry."
                        .to_string(),
                );
                return;
            }
            match app.set_provider_api_key(&provider, &secret) {
                Ok(()) => {
                    // set_provider_api_key already reports success without
                    // echoing the value.
                }
                Err(e) => {
                    app.add_message(MessageRole::System, format!("API key error: {}", e));
                }
            }
        }
        KeyCode::Esc => {
            app.secure_input = None;
            app.add_message(MessageRole::System, "API key input cancelled".to_string());
        }
        _ => {}
    }
}

fn handle_confirmation_key(key: crossterm::event::KeyEvent, app: &mut TuiApp) {
    let confirmed = matches!(
        key.code,
        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y')
    );
    let cancelled = matches!(
        key.code,
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N')
    );
    if confirmed {
        if let Some((_, action)) = app.pending_confirmation.take() {
            confirm_action(action, app);
        }
    } else if cancelled {
        if app.pending_confirmation.take().is_some() {
            app.add_message(MessageRole::System, "Action cancelled".to_string());
        }
    }
    // Any other key leaves the confirmation pending (no silent dismiss).
}

fn confirm_action(action: PendingAction, app: &mut TuiApp) {
    match action {
        PendingAction::RunShell(command) => {
            app.add_message(
                MessageRole::System,
                format!(
                    "Running `{}`",
                    crate::tools::shell::redact_secrets_public(&command)
                ),
            );
            run_command_task(app, "shell", command);
        }
        PendingAction::ApproveChange => execute_approve(app),
        PendingAction::RejectChange => execute_reject(app),
    }
}

/// Apply the staged change, with an optional verification gate.
fn execute_approve(app: &mut TuiApp) {
    let mut plan = match app.pending_change.take() {
        Some(plan) => plan,
        None => {
            app.add_message(
                MessageRole::System,
                "No pending change to approve.".to_string(),
            );
            return;
        }
    };
    let verify = crate::tools::detect_workspace_root()
        .join(".git")
        .exists()
        .then(|| "git status --porcelain".to_string());
    match plan.apply_and_verify(verify.as_deref()) {
        Ok(msg) => {
            app.add_message(MessageRole::System, msg);
            if let Err(e) = save_session(app) {
                app.add_message(MessageRole::System, format!("Save error: {}", e));
            }
        }
        Err(e) => {
            app.add_message(
                MessageRole::System,
                format!("Approval rejected the change: {}", e),
            );
        }
    }
}

/// Discard the staged change without writing anything.
fn execute_reject(app: &mut TuiApp) {
    if app.pending_change.take().is_some() {
        app.add_message(
            MessageRole::System,
            "Pending change rejected; files were not modified.".to_string(),
        );
    }
}

/// Dangerous shell patterns that always require confirmation.
fn is_dangerous_shell(command: &str) -> bool {
    let c = command.trim();
    let patterns: &[&str] = &[
        "rm -rf",
        "rm -fr",
        "rm -r -f",
        "git push --force",
        "git push -f",
        "chmod -R 777",
        "chmod 777",
        "mkfs",
        "dd if=",
        "shutdown",
        "reboot",
        ":(){",
    ];
    patterns.iter().any(|p| c.contains(p))
}

// ─── Command dispatch ─────────────────────────────────────────────────────

fn handle_engineering_command(input: &str, app: &mut TuiApp) {
    let parts: Vec<&str> = input.split_whitespace().collect();
    let command = parts.first().copied().unwrap_or("");
    let args = parts.get(1..).map(|p| p.join(" ")).unwrap_or_default();

    match command {
        "/help" => {
            app.add_message(MessageRole::System, help_text(app));
        }
        "/status" => {
            let root = crate::tools::detect_workspace_root();
            let model = if app.config.model.trim().is_empty() {
                "auto".to_string()
            } else {
                app.config.model.clone()
            };
            app.add_message(
                MessageRole::System,
                format!(
                    "Status:\n  workspace: {}\n  model: {}\n  provider: {}\n  task: {}",
                    root.display(),
                    model,
                    app.config.provider,
                    if app.has_active_task() {
                        "running"
                    } else {
                        "idle"
                    }
                ),
            );
        }
        "/build" => {
            app.add_message(MessageRole::System, "Building project…".to_string());
            let cmd = build_command(app);
            run_command_task(app, "build", cmd);
        }
        "/test" => {
            app.add_message(MessageRole::System, "Running tests…".to_string());
            let cmd = test_command(app);
            run_command_task(app, "test", cmd);
        }
        "/benchmark" => {
            app.add_message(MessageRole::System, "Running benchmarks…".to_string());
            let cmd = if app.workspace_has("Cargo.toml") {
                "cargo bench".to_string()
            } else {
                "npm run benchmark".to_string()
            };
            run_command_task(app, "benchmark", cmd);
        }
        "/doctor" => {
            app.add_message(
                MessageRole::System,
                "Running project health checks…".to_string(),
            );
            let cmd = if app.workspace_has("Cargo.toml") {
                "cargo check".to_string()
            } else if app.workspace_has("package.json") {
                "npm run lint".to_string()
            } else {
                "git status --short".to_string()
            };
            run_command_task(app, "doctor", cmd);
        }
        "/playwright" => {
            app.add_message(MessageRole::System, "Running Playwright tests…".to_string());
            run_playwright_task(app, &args);
        }
        "/review" => {
            let cmd = if app.workspace_has(".git") {
                "git diff --stat".to_string()
            } else {
                "git status".to_string()
            };
            run_command_task(app, "review", cmd);
        }
        "/search" => {
            let pattern = args.trim();
            if pattern.is_empty() {
                app.add_message(MessageRole::System, "Usage: /search <pattern>".to_string());
                return;
            }
            let cmd = format!(
                "grep -rn --exclude-dir=node_modules --exclude-dir=target --exclude-dir=.git \"{}\" .",
                pattern.replace('"', "\\\"")
            );
            run_command_task(app, "search", cmd);
        }
        "/refactor" | "/fix" | "/explain" => {
            // Task-based commands: route through the canonical runtime so the
            // full engineering interaction loop applies.
            let task = if args.trim().is_empty() {
                format!(
                    "{}",
                    match command {
                        "/refactor" => "Refactor the project",
                        "/fix" => "Fix the last error or failing test in the project",
                        _ => "Explain the project",
                    }
                )
            } else {
                format!(
                    "{} {}",
                    match command {
                        "/refactor" => "Refactor",
                        "/fix" => "Fix",
                        _ => "Explain",
                    },
                    args.trim()
                )
            };
            submit_task(app, task);
        }
        "/apply" => {
            let file = parts.get(1).cloned().unwrap_or("");
            let new_content = parts.get(2..).map(|p| p.join(" ")).unwrap_or_default();
            if file.is_empty() || new_content.trim().is_empty() {
                app.add_message(
                    MessageRole::System,
                    "Usage: /apply <file> <new content>".to_string(),
                );
                return;
            }
            let path = crate::tools::detect_workspace_root().join(file);
            if !path.exists() {
                app.add_message(
                    MessageRole::System,
                    format!("Target not found: {}", path.display()),
                );
                return;
            }
            match crate::tools::ChangePlan::propose(&path, &new_content) {
                Ok(plan) => {
                    app.add_message(
                        MessageRole::System,
                        format!(
                            "Staged change for {} (not applied). Review, then run //approve:\n{}",
                            path.display(),
                            plan.preview()
                        ),
                    );
                    app.pending_change = Some(plan);
                }
                Err(e) => {
                    app.add_message(MessageRole::System, format!("Could not stage change: {e}"));
                }
            }
        }
        "/copy" => {
            if app.copy_to_clipboard(&app.conversation_text()) {
                app.add_message(
                    MessageRole::System,
                    "Conversation copied to clipboard".to_string(),
                );
            } else {
                app.add_message(
                    MessageRole::System,
                    "Copy failed (no clipboard tool found)".to_string(),
                );
            }
        }
        _ => {
            app.add_message(
                MessageRole::System,
                format!("Unknown command: {}. Type /help for commands.", command),
            );
        }
    }
}

fn handle_runtime_command(input: &str, app: &mut TuiApp) {
    let parts: Vec<&str> = input.split_whitespace().collect();
    let command = parts.first().copied().unwrap_or("");
    let arg = parts.get(1).copied().unwrap_or("");

    match command {
        "//rail" => {
            app.toggle_rail();
            app.add_message(
                MessageRole::System,
                format!(
                    "Intelligence rail: {}",
                    if app.rail_visible {
                        "shown"
                    } else {
                        "collapsed"
                    }
                ),
            );
        }
        "//console" => {
            app.toggle_console();
            app.add_message(
                MessageRole::System,
                format!(
                    "PTY console: {}",
                    if app.show_console { "open" } else { "closed" }
                ),
            );
        }
        "//model" => {
            if arg.is_empty() {
                app.add_message(
                    MessageRole::System,
                    format!("Current model: {}", {
                        if app.config.model.trim().is_empty() {
                            "auto".to_string()
                        } else {
                            app.config.model.clone()
                        }
                    }),
                );
                app.open_model_picker();
            } else {
                app.apply_model(arg.to_string());
            }
        }
        "//provider" => {
            if arg.is_empty() {
                let status = app.provider_status_text();
                app.add_message(MessageRole::System, format!("Providers:\n{}", status));
            } else if let Err(e) = app.switch_provider(arg) {
                app.add_message(MessageRole::System, format!("Provider error: {}", e));
            }
        }
        "//apikey" => {
            if parts.len() >= 3 {
                // The key must never travel as a normal command argument (it
                // would land in input history and be echoed into context).
                app.add_message(
                    MessageRole::System,
                    "Inline API keys are not accepted.\nRun `//apikey <provider>` and enter the key in the masked prompt."
                        .to_string(),
                );
                return;
            }
            let provider = if arg.is_empty() {
                app.provider_manager
                    .as_ref()
                    .and_then(|pm| pm.active_provider().cloned())
                    .unwrap_or_else(|| "openai".to_string())
            } else {
                arg.to_string()
            };
            // Validate the provider so the masked prompt never targets an
            // unknown provider.
            let known = app
                .provider_manager
                .as_ref()
                .map(|pm| pm.list_provider_ids().contains(&provider))
                .unwrap_or(false);
            if !known {
                app.add_message(
                    MessageRole::System,
                    format!(
                        "Unknown provider '{}'. Use //provider to list providers.",
                        provider
                    ),
                );
                return;
            }
            app.secure_input = Some(crate::tui::app::SecureInputState {
                provider,
                buffer: String::new(),
            });
            app.add_message(
                MessageRole::System,
                "Enter API key (masked). Enter to save securely, Esc to cancel.".to_string(),
            );
        }
        "//settings" => {
            app.toggle_settings();
            if let Some(ref sm) = app.settings {
                app.add_message(MessageRole::System, sm.summary());
            }
        }
        "//preferences" => {
            app.add_message(
                MessageRole::System,
                format!(
                    "Preferences:\n  verbosity: {}\n  compact: {}",
                    if app.dashboard.verbose {
                        "verbose"
                    } else {
                        "minimal"
                    },
                    app.dashboard.compact
                ),
            );
        }
        "//profile" => {
            if arg.is_empty() {
                app.add_message(
                    MessageRole::System,
                    format!("Profile directory: {:?}", Config::config_dir()),
                );
            } else {
                app.add_message(
                    MessageRole::System,
                    format!(
                        "Profile switching for '{}' is not implemented; using default.",
                        arg
                    ),
                );
            }
        }
        "//session" | "//resume" => {
            if command == "//resume" {
                let sessions = app.list_sessions();
                if let Some(last) = sessions.first() {
                    app.add_message(
                        MessageRole::System,
                        format!("Most recent session:\n{}", last),
                    );
                } else {
                    app.add_message(MessageRole::System, "No sessions found".to_string());
                }
                return;
            }
            let sessions = app.list_sessions();
            if sessions.is_empty() {
                app.add_message(MessageRole::System, "No sessions found".to_string());
            } else {
                app.add_message(
                    MessageRole::System,
                    format!("Recent sessions:\n{}", sessions.join("\n")),
                );
            }
        }
        "//theme" => {
            app.add_message(
                MessageRole::System,
                format!(
                    "Theme: {} (muted). //theme <name> not yet supported.",
                    if app.dashboard.compact {
                        "compact"
                    } else {
                        "default"
                    }
                ),
            );
        }
        "//verbose" => {
            app.dashboard.verbose = !app.dashboard.verbose;
            app.add_message(
                MessageRole::System,
                format!(
                    "Verbose mode: {}",
                    if app.dashboard.verbose { "on" } else { "off" }
                ),
            );
        }
        "//compact" => {
            app.dashboard.compact = !app.dashboard.compact;
            app.add_message(
                MessageRole::System,
                format!(
                    "Compact mode: {}",
                    if app.dashboard.compact { "on" } else { "off" }
                ),
            );
        }
        "//memory" => {
            let notifications: Vec<String> = app
                .dashboard
                .memory_notifications
                .iter()
                .take(10)
                .map(|n| format!("[{}] {}", n.timestamp, n.message))
                .collect();
            if notifications.is_empty() {
                app.add_message(MessageRole::System, "No memory changes".to_string());
            } else {
                app.add_message(
                    MessageRole::System,
                    format!("Memory changes:\n{}", notifications.join("\n")),
                );
            }
        }
        "//mcp" => {
            if app.workspace_panel.mcp_servers.is_empty() {
                app.add_message(
                    MessageRole::System,
                    "Scanning for MCP servers… (run //mcp again to list)".to_string(),
                );
                trigger_workspace_discovery(app);
                return;
            }
            let servers: Vec<String> = app
                .workspace_panel
                .mcp_servers
                .iter()
                .map(|s| {
                    format!(
                        "  {} ({})",
                        s.name,
                        if s.available {
                            "available"
                        } else {
                            "unavailable"
                        }
                    )
                })
                .collect();
            app.add_message(
                MessageRole::System,
                format!("MCP servers:\n{}", servers.join("\n")),
            );
        }
        "//skills" => {
            let notifications: Vec<String> = app
                .dashboard
                .skill_notifications
                .iter()
                .take(10)
                .map(|n| {
                    format!(
                        "{} - {:.2} -> {:.2}",
                        n.skill, n.confidence_before, n.confidence_after
                    )
                })
                .collect();
            if notifications.is_empty() {
                app.add_message(MessageRole::System, "No skill changes".to_string());
            } else {
                app.add_message(
                    MessageRole::System,
                    format!("Skill changes:\n{}", notifications.join("\n")),
                );
            }
        }
        "//plugins" => {
            app.add_message(
                MessageRole::System,
                "Plugins: loaded via the plugin SDK. None installed in this session.".to_string(),
            );
        }
        "//update" => {
            app.add_message(
                MessageRole::System,
                format!(
                    "CodeBro v{} — the latest installed build. Auto-update is not enabled.",
                    env!("CARGO_PKG_VERSION")
                ),
            );
        }
        "//version" => {
            app.add_message(
                MessageRole::System,
                format!(
                    "CodeBro v{}\n  workspace: {}\n  provider: {}\n  model: {}",
                    env!("CARGO_PKG_VERSION"),
                    crate::tools::detect_workspace_root().display(),
                    app.config.provider,
                    app.config.model
                ),
            );
        }
        "//export" => {
            let path = if arg.is_empty() {
                "codebro-export.json".to_string()
            } else {
                arg.to_string()
            };
            match app.export_state(&path) {
                Ok(p) => app.add_message(
                    MessageRole::System,
                    format!("Exported session/config to {}", p.display()),
                ),
                Err(e) => app.add_message(MessageRole::System, format!("Export error: {}", e)),
            }
        }
        "//import" => {
            if arg.is_empty() {
                app.add_message(MessageRole::System, "Usage: //import <file>".to_string());
                return;
            }
            match app.import_state(arg) {
                Ok(()) => app.add_message(MessageRole::System, format!("Imported from {}", arg)),
                Err(e) => app.add_message(MessageRole::System, format!("Import error: {}", e)),
            }
        }
        "//clear" => {
            app.should_clear = true;
        }
        "//tasks" => {
            let entries = app.dashboard.graph_entries();
            if entries.is_empty() {
                app.add_message(MessageRole::System, "No active task graph".to_string());
            } else {
                let text: Vec<String> = entries
                    .iter()
                    .map(|(desc, agent, status)| format!("[{}] {} - {}", status, agent, desc))
                    .collect();
                app.add_message(MessageRole::System, format!("Tasks:\n{}", text.join("\n")));
            }
        }
        "//agents" => {
            let entries = app.dashboard.agent_entries();
            if entries.is_empty() {
                app.add_message(MessageRole::System, "No agents active".to_string());
            } else {
                let text: Vec<String> = entries
                    .iter()
                    .map(|e| format!("{} - {} ({:.0}%)", e.name, e.status, e.progress * 100.0))
                    .collect();
                app.add_message(MessageRole::System, format!("Agents:\n{}", text.join("\n")));
            }
        }
        "//metrics" => {
            app.dashboard.toggle_metrics();
        }
        "//approve" => {
            if let Some(plan) = &app.pending_change {
                let path = plan.path().display().to_string();
                app.pending_confirmation = Some((
                    format!(
                        "This will apply the staged change to {}. Proceed? (y/n)",
                        path
                    ),
                    PendingAction::ApproveChange,
                ));
            } else {
                app.add_message(
                    MessageRole::System,
                    "No pending change. Stage one with /apply <file> <new content>.".to_string(),
                );
            }
        }
        "//reject" => {
            if let Some(plan) = &app.pending_change {
                let path = plan.path().display().to_string();
                app.pending_confirmation = Some((
                    format!(
                        "This will discard the staged change to {}. Proceed? (y/n)",
                        path
                    ),
                    PendingAction::RejectChange,
                ));
            } else {
                app.add_message(
                    MessageRole::System,
                    "No pending change to reject.".to_string(),
                );
            }
        }
        "//save" => {
            if let Err(e) = save_session(app) {
                app.add_message(MessageRole::System, format!("Save error: {}", e));
            } else {
                app.add_message(MessageRole::System, "Session saved".to_string());
            }
        }
        _ => {
            app.add_message(
                MessageRole::System,
                format!("Unknown runtime command: {}. Type /help.", command),
            );
        }
    }
}

fn help_text(app: &TuiApp) -> String {
    let mut lines = vec![
        format!(
            "CodeBro v{} — engineering commands, runtime commands, shell.",
            env!("CARGO_PKG_VERSION")
        ),
        "".to_string(),
        "  /  engineering   operate on the project".to_string(),
    ];
    for spec in commands::ENGINEERING_COMMANDS {
        lines.push(format!("    {}  — {}", spec.usage, spec.description));
    }
    lines.push("".to_string());
    lines.push("  //  runtime      operate on CodeBro".to_string());
    for spec in commands::RUNTIME_COMMANDS {
        if commands::is_applicable(spec, app) {
            lines.push(format!("    {}  — {}", spec.usage, spec.description));
        }
    }
    lines.push("".to_string());
    lines.push("  !  shell        execute directly in the shell".to_string());
    lines.push("    e.g. !git status, !cargo test, !ls".to_string());
    lines.push("".to_string());
    lines.push(
        "  Shortcuts: Ctrl+P palette · Ctrl+O rail · Ctrl+D diff · Ctrl+M memory · Ctrl+Enter send · Esc dismiss · ? help".to_string(),
    );
    lines.push(
        "  Chat: Tab focuses an action group · Enter expands/collapses it · End returns to live"
            .to_string(),
    );
    lines.join("\n")
}

/// Submit a natural-language engineering task through the canonical runtime.
fn submit_task(app: &mut TuiApp, task: String) {
    // Seal the previous turn's timeline onto its user message first.
    app.seal_actions_to_last_user();
    app.add_message(MessageRole::User, task.clone());
    app.is_loading = true;
    app.begin_task(task.clone());
    app.dashboard
        .animation
        .start_activity(crate::tui::animation::ActivityType::Thinking);
    let token = app.begin_cancellable_task();
    let config = app.config.clone();
    let tx = app.tx.clone();
    let conversation = conversation_from(app);
    // The TUI submission runs Assist today (Research + Main): research is
    // enabled, no autonomous specialists. The header and rail derive their
    // claims from this explicit mode — never from "a task is running".
    let mode = crate::canonical_runtime::TaskMode::Assist;
    app.task_mode = mode;
    tokio::spawn(async move {
        run_chat_pipeline(&config, &task, conversation, &tx, token, mode).await;
    });
}

/// Kick off an async workspace discovery scan; results arrive as
/// `AppEvent::WorkspaceDiscovered`.
fn trigger_workspace_discovery(app: &TuiApp) {
    let tx = app.tx.clone();
    tokio::spawn(async move {
        let root = crate::tools::detect_workspace_root();
        let engine = crate::workspace_discovery::DiscoveryEngine::new(root.clone());
        let discovery = engine.discover();
        let scanner = crate::capability_discovery::CapabilityScanner::new(root.clone());
        let capabilities = scanner.scan();
        let mcp_servers = crate::workspace_discovery::discover_mcp_servers(&root);
        let _ = tx.send(events::AppEvent::WorkspaceDiscovered {
            discovery,
            capabilities,
            mcp_servers,
        });
    });
}

/// Run a real shell/build/test command through the canonical PTY tool path.
fn run_command_task(app: &mut TuiApp, label: &str, command: String) {
    app.is_loading = true;
    let token = app.begin_cancellable_task();
    let tx = app.tx.clone();
    let config = app.config.clone();
    let workspace = crate::tools::detect_workspace_root();
    // The echoed command is a conversation/persistence surface: redact obvious
    // secrets so they never reach history, context, or exports. Execution uses
    // the raw `command`.
    app.add_message(
        MessageRole::System,
        format!(
            "[{}] {}",
            label,
            crate::tools::shell::redact_secrets_public(&command)
        ),
    );
    tokio::spawn(async move {
        let emit_tx = tx.clone();
        let emit = move |event: AgentEvent| {
            let _ = emit_tx.send(events::AppEvent::AgentEvent(event));
        };
        let mut runtime = match crate::canonical_runtime::CanonicalRuntime::new(config) {
            Ok(runtime) => runtime,
            Err(e) => {
                let _ = tx.send(events::AppEvent::TaskFinished { success: false });
                let _ = tx.send(events::AppEvent::Response(format!(
                    "Runtime initialization failed: {e}"
                )));
                return;
            }
        };
        let outcome = runtime.run_shell(&command, &emit, token).await;
        let status = if outcome.cancelled {
            "Task cancelled"
        } else if outcome.success {
            "Completed"
        } else {
            "Failed"
        };
        let _ = tx.send(events::AppEvent::TaskFinished {
            success: outcome.success,
        });
        let _ = tx.send(events::AppEvent::Response(format!(
            "[{}] {}\n{}",
            status,
            crate::tools::shell::redact_secrets_public(&command),
            outcome.output.trim_end()
        )));
        let _ = workspace;
    });
}

/// Run the Playwright tool through the canonical tool path.
fn run_playwright_task(app: &mut TuiApp, args: &str) {
    app.is_loading = true;
    let token = app.begin_cancellable_task();
    let tx = app.tx.clone();
    let config = app.config.clone();
    let args = args.to_string();
    tokio::spawn(async move {
        let emit_tx = tx.clone();
        let emit = move |event: AgentEvent| {
            let _ = emit_tx.send(events::AppEvent::AgentEvent(event));
        };
        let mut runtime = match crate::canonical_runtime::CanonicalRuntime::new(config) {
            Ok(runtime) => runtime,
            Err(e) => {
                let _ = tx.send(events::AppEvent::TaskFinished { success: false });
                let _ = tx.send(events::AppEvent::Response(format!(
                    "Runtime initialization failed: {e}"
                )));
                return;
            }
        };
        let console_id = uuid::Uuid::new_v4().to_string();
        let outcome = runtime
            .run_tool_streaming("playwright_test", &console_id, &args, &emit, token)
            .await;
        let status = if outcome.cancelled {
            "cancelled"
        } else if outcome.success {
            "passed"
        } else {
            "failed"
        };
        let _ = tx.send(events::AppEvent::TaskFinished {
            success: outcome.success,
        });
        let _ = tx.send(events::AppEvent::Response(format!(
            "[playwright {}]\n{}",
            status,
            outcome.output.trim_end()
        )));
    });
}

fn build_command(app: &TuiApp) -> String {
    if app.workspace_has("Cargo.toml") {
        "cargo build".to_string()
    } else if app.workspace_has("package.json") {
        "npm run build".to_string()
    } else {
        "make".to_string()
    }
}

fn test_command(app: &TuiApp) -> String {
    if app.workspace_has("Cargo.toml") {
        "cargo test".to_string()
    } else if app.workspace_has("package.json") {
        "npm test".to_string()
    } else {
        "make test".to_string()
    }
}

/// Build the conversation history from the session messages for the
/// engineering context (recent user/assistant turns only).
fn conversation_from(app: &TuiApp) -> Vec<crate::engineering_context::ConversationMessage> {
    let mut out = Vec::new();
    for msg in app.messages.iter().rev().take(20) {
        let role = match msg.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
        };
        out.push(crate::engineering_context::ConversationMessage {
            role: role.to_string(),
            content: msg.content.clone(),
        });
    }
    out.reverse();
    out
}

/// Wires a chat submission into the canonical runtime:
/// identity → memory → context assembly → EngineeringContext → PromptBuilder →
/// IntelligentProviderRouter → ProviderRuntime → provider, streaming to the TUI.
async fn run_chat_pipeline(
    config: &Config,
    task: &str,
    conversation: Vec<crate::engineering_context::ConversationMessage>,
    tx: &std::sync::mpsc::Sender<events::AppEvent>,
    token: crate::cancellation::CancellationToken,
    mode: crate::canonical_runtime::TaskMode,
) {
    let emit_tx = tx.clone();
    let emit = move |event: AgentEvent| {
        let _ = emit_tx.send(events::AppEvent::AgentEvent(event));
    };
    let chunk_tx = tx.clone();
    let on_chunk = move |chunk: &str| {
        let _ = chunk_tx.send(events::AppEvent::StreamChunk(chunk.to_string()));
    };
    let pty_tx = tx.clone();
    let on_pty = Arc::new(move |_console: &str, content: &str| {
        let _ = pty_tx.send(events::AppEvent::AgentEvent(AgentEvent::PtyOutput {
            console: "task".to_string(),
            content: content.to_string(),
        }));
    });

    let mut runtime = match crate::canonical_runtime::CanonicalRuntime::new(config.clone()) {
        Ok(runtime) => runtime,
        Err(e) => {
            let _ = tx.send(events::AppEvent::TaskFinished { success: false });
            let _ = tx.send(events::AppEvent::Response(format!(
                "Runtime initialization failed: {e}"
            )));
            return;
        }
    };

    let request = crate::canonical_runtime::TaskRequest {
        task,
        conversation,
        emit: &emit,
        on_chunk: &on_chunk,
    };
    // The canonical per-mode option set: the same flags `phase_flags()`
    // defines, so the header/progress claims always match the runtime.
    let mut options = crate::canonical_runtime::TaskOptions::for_mode(mode);
    options.cancel = Some(token);
    options.on_pty = Some(on_pty);

    let result = runtime.run_task_with_options(&request, options).await;

    let _ = tx.send(events::AppEvent::TaskFinished {
        success: result.success,
    });
    if result.success {
        let _ = tx.send(events::AppEvent::Response(result.response));
    } else {
        let msg = result
            .error
            .clone()
            .unwrap_or_else(|| "Task failed".to_string());
        let _ = tx.send(events::AppEvent::Response(format!("Task failed: {msg}")));
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Layout
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayoutMode {
    /// Chat ~78% + right intelligence rail ~22% (the default).
    Expanded,
    /// Full-width chat, rail hidden.
    Collapsed,
    /// Narrow/short terminal: rail off, chrome reduced.
    Compact,
}

#[derive(Debug, Clone, Copy)]
struct UiLayout {
    mode: LayoutMode,
    header_h: u16,
    chat_h: u16,
    input_h: u16,
    footer_h: u16,
    rail_w: u16,
    chat_w: u16,
}

impl UiLayout {
    fn header_area(self, size: Rect) -> Rect {
        Rect::new(size.x, size.y, size.width, self.header_h)
    }

    fn chat_area(self, size: Rect) -> Rect {
        Rect::new(size.x, size.y + self.header_h, self.chat_w, self.chat_h)
    }

    fn rail_area(self, size: Rect) -> Rect {
        Rect::new(
            size.x + self.chat_w,
            size.y + self.header_h,
            self.rail_w,
            self.chat_h,
        )
    }

    fn input_area(self, size: Rect) -> Rect {
        Rect::new(
            size.x,
            size.y + self.header_h + self.chat_h,
            size.width,
            self.input_h,
        )
    }

    fn footer_area(self, size: Rect) -> Rect {
        Rect::new(
            size.x,
            size.y + size.height.saturating_sub(self.footer_h),
            size.width,
            self.footer_h,
        )
    }
}

/// Decide the layout mode from terminal size and user preference. Narrow or
/// short terminals always collapse to compact so the chat stays usable.
fn layout_mode(app: &TuiApp, size: Rect) -> LayoutMode {
    if size.width < COMPACT_MIN_WIDTH || size.height < COMPACT_MIN_HEIGHT {
        LayoutMode::Compact
    } else if app.rail_visible {
        LayoutMode::Expanded
    } else {
        LayoutMode::Collapsed
    }
}

/// The rail is 22% of width, at least `RAIL_MIN_WIDTH` columns (so it stays
/// readable) and never more than 25%.
fn rail_width(total: u16) -> u16 {
    let pct = (total as f32 * 0.22) as u16;
    let max = (total as f32 * 0.25) as u16;
    pct.max(RAIL_MIN_WIDTH).min(max).min(total / 2).max(4)
}

fn compute_ui_layout(app: &TuiApp, size: Rect) -> UiLayout {
    let mode = layout_mode(app, size);
    let compact = mode == LayoutMode::Compact || app.dashboard.compact;
    let header_h: u16 = 1;
    let footer_h: u16 = if compact { 0 } else { 1 };
    let input_h: u16 = if compact { 2 } else { 3 };
    let rail_w = if mode == LayoutMode::Expanded {
        rail_width(size.width)
    } else {
        0
    };
    let chat_w = size.width.saturating_sub(rail_w);
    let chat_h = size
        .height
        .saturating_sub(header_h + input_h + footer_h)
        .max(1);
    UiLayout {
        mode,
        header_h,
        chat_h,
        input_h,
        footer_h,
        rail_w,
        chat_w,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Top-level render
// ═══════════════════════════════════════════════════════════════════════════

fn ui(f: &mut Frame, app: &TuiApp) {
    let size = f.size();
    let layout = compute_ui_layout(app, size);

    render_header(f, app, layout.header_area(size));
    render_chat(f, app, layout.chat_area(size));
    if layout.rail_w > 0 {
        render_rail(f, app, layout.rail_area(size));
    }
    render_input(f, app, layout.input_area(size));
    if layout.footer_h > 0 {
        render_footer(f, app, layout.footer_area(size));
    }

    // Overlays (rendered over the chat when open).
    if app.dashboard.show_command_palette {
        render_command_palette(f, app, layout.chat_area(size));
    }
    if !app.dashboard.autocomplete.is_empty() && !app.dashboard.model_picker.is_open() {
        render_autocomplete(f, app, layout.input_area(size));
    }
    if app.dashboard.model_picker.is_open() {
        render_model_picker(f, app);
    }
    if let Some((message, _)) = &app.pending_confirmation {
        render_confirmation(f, message, layout.input_area(size));
    }
    if app.show_console {
        render_console_popup(f, app, layout.chat_area(size));
    }
    if app.dashboard.show_agents {
        render_agents_popup(f, app, layout.chat_area(size));
    }
    if app.dashboard.show_task_graph {
        render_task_graph_popup(f, app, layout.chat_area(size));
    }
    if app.dashboard.show_metrics {
        render_metrics_popup(f, app, layout.chat_area(size));
    }
    if app.dashboard.show_coordination {
        render_coordination_popup(f, app, layout.chat_area(size));
    }
    if app.dashboard.show_memory {
        render_memory_popup(f, app, layout.chat_area(size));
    }
    if app.dashboard.show_trace {
        render_activity_popup(f, app, layout.chat_area(size));
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Header
// ═══════════════════════════════════════════════════════════════════════════

/// The header's mode label. Idle always renders READY; a running task renders
/// the actual [`TaskMode`] the TUI submission is using. The mode is explicit
/// state — it is NEVER inferred from "a task is running" or from whichever
/// agent events happened to arrive.
pub fn header_mode_label(mode: crate::canonical_runtime::TaskMode, working: bool) -> &'static str {
    if !working {
        return "READY";
    }
    match mode {
        crate::canonical_runtime::TaskMode::Assist => "ASSIST",
        crate::canonical_runtime::TaskMode::Validate => "VALIDATE",
        crate::canonical_runtime::TaskMode::Plan => "PLAN",
        crate::canonical_runtime::TaskMode::Autonomous => "AUTONOMOUS",
    }
}

fn render_header(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.height < 1 || area.width < 4 {
        return;
    }
    let model = if app.config.model.trim().is_empty() {
        "auto".to_string()
    } else {
        app.config.model.clone()
    };

    let working = app.has_active_task() || app.action_stream.has_running();
    let mode_label = header_mode_label(app.task_mode, working);
    let mut spans = vec![
        Span::styled(
            "CodeBro",
            Style::default()
                .fg(THEME.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(THEME.muted),
        ),
        Span::styled("   ", Style::default()),
        Span::styled(
            if working { "●" } else { "○" },
            Style::default().fg(if working { THEME.green } else { THEME.muted }),
        ),
        Span::styled(
            format!(" {}", mode_label),
            Style::default()
                .fg(if working {
                    THEME.green
                } else {
                    THEME.secondary
                })
                .add_modifier(Modifier::BOLD),
        ),
    ];

    if working {
        spans.push(Span::styled(
            format!(" {}", spinner_char_now()),
            Style::default().fg(THEME.purple),
        ));
    }

    // Right-side metadata: model · tokens · tools (real values only).
    let mut right: Vec<Span<'static>> = Vec::new();
    right.push(Span::styled(
        truncate_to(&model, 24),
        Style::default().fg(THEME.secondary),
    ));
    if let Some(metrics) = &app.dashboard.metrics {
        let tokens = metrics.total_tokens();
        if tokens > 0 {
            right.push(Span::styled("  ·  ", Style::default().fg(THEME.muted)));
            right.push(Span::styled(
                crate::metrics::format_token_count(tokens),
                Style::default().fg(THEME.secondary),
            ));
        }
    }
    if app.action_stream.tool_calls > 0 {
        right.push(Span::styled("  ·  ", Style::default().fg(THEME.muted)));
        right.push(Span::styled(
            format!("{} tools", app.action_stream.tool_calls),
            Style::default().fg(THEME.secondary),
        ));
    }

    let left_line = Line::from(spans);
    let right_line = Line::from(right);
    let right_width = right_line
        .spans
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>() as u16;
    let left_budget = area.width.saturating_sub(right_width.saturating_add(2));

    f.render_widget(
        Paragraph::new(truncate_line_to(&left_line, left_budget as usize))
            .style(Style::default().bg(THEME.bg)),
        Rect::new(area.x, area.y, left_budget, 1),
    );
    if right_width > 0 && area.width > right_width {
        f.render_widget(
            Paragraph::new(right_line).style(Style::default().bg(THEME.bg)),
            Rect::new(
                area.x + area.width.saturating_sub(right_width),
                area.y,
                right_width,
                1,
            ),
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Chat
// ═══════════════════════════════════════════════════════════════════════════

fn render_chat(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.height < 2 || area.width < 4 {
        return;
    }
    let inner_w = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    let has_actions = !app.action_stream.groups.is_empty();
    let has_console = app.has_console_content();

    if app.dashboard.show_welcome && app.messages.is_empty() && !has_actions && !has_console {
        lines.push(Line::from(Span::styled(
            "CodeBro",
            Style::default()
                .fg(THEME.purple)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Ask me to inspect, modify, test, or review your project.",
            Style::default().fg(THEME.secondary),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Type a task · / commands · // runtime · ! shell · Ctrl+P palette · ? help",
            Style::default().fg(THEME.muted),
        )));
        lines.push(Line::from(Span::styled(
            "Tab focuses an action group · Enter expands/collapses · Ctrl+O rail · Esc dismiss",
            Style::default().fg(THEME.muted),
        )));
        lines.push(Line::from(""));
    } else {
        if let Some(ref err) = app.dashboard.last_error {
            lines.push(Line::from(vec![
                Span::styled("✗ ", Style::default().fg(THEME.red)),
                Span::styled(
                    truncate_to(err, inner_w.saturating_sub(2)),
                    Style::default().fg(THEME.red),
                ),
            ]));
            lines.push(Line::from(""));
        }

        // Turn-scoped timeline: sealed actions render under their user
        // message; the live stream renders after the latest user message.
        let last_user = app
            .messages
            .iter()
            .rposition(|m| m.role == MessageRole::User);

        for (idx, msg) in app.messages.iter().enumerate() {
            match msg.role {
                MessageRole::User => {
                    lines.extend(message_header_lines(
                        "●",
                        THEME.purple,
                        "You",
                        THEME.purple,
                        &msg.timestamp,
                        inner_w,
                    ));
                    for content_line in msg.content.lines() {
                        lines.push(Line::from(Span::styled(
                            truncate_to(content_line, inner_w),
                            Style::default().fg(THEME.primary),
                        )));
                    }
                    lines.push(Line::from(""));
                    // Historical sealed groups for this turn.
                    if let Some(groups) = &msg.sealed_actions {
                        lines.extend(render_action_groups(
                            groups.iter(),
                            app.action_stream.focused_from_back,
                            groups.len(),
                            inner_w,
                            false,
                        ));
                    }
                    // Live stream belongs to the latest user turn only.
                    if Some(idx) == last_user && !app.action_stream.groups.is_empty() {
                        lines.extend(action_group_lines(app, inner_w));
                    }
                }
                MessageRole::Assistant => {
                    lines.extend(message_header_lines(
                        "●",
                        THEME.blue,
                        "CodeBro",
                        THEME.blue,
                        &msg.timestamp,
                        inner_w,
                    ));
                    for md_line in crate::tui::markdown::render_markdown(&msg.content, inner_w) {
                        lines.push(md_line);
                    }
                    lines.push(Line::from(""));
                }
                MessageRole::System => {
                    for content_line in msg.content.lines() {
                        lines.push(Line::from(Span::styled(
                            truncate_to(content_line, inner_w),
                            Style::default().fg(THEME.yellow),
                        )));
                    }
                    lines.push(Line::from(""));
                }
            }
        }

        // Shell-only sessions (no user chat turn): show the live stream at end.
        if last_user.is_none() && !app.action_stream.groups.is_empty() {
            lines.extend(action_group_lines(app, inner_w));
        }

        // Streaming response.
        if app.dashboard.is_streaming && !app.dashboard.streaming_buffer.is_empty() {
            lines.push(Line::from(""));
            lines.extend(message_header_lines(
                spinner_char_now(),
                THEME.blue,
                "CodeBro",
                THEME.blue,
                "",
                inner_w,
            ));
            for md_line in
                crate::tui::markdown::render_markdown(&app.dashboard.streaming_buffer, inner_w)
            {
                lines.push(md_line);
            }
            lines.push(Line::from(""));
        }

        // Task-complete banner when the stream finished successfully.
        if !app.has_active_task()
            && !app.action_stream.has_running()
            && app
                .action_stream
                .groups
                .iter()
                .any(|g| g.status == crate::tui::actions::UiActionStatus::Completed)
            && last_user.is_some()
        {
            lines.push(Line::from(vec![
                Span::styled("━", Style::default().fg(THEME.green)),
                Span::styled(
                    " ✓ Complete ",
                    Style::default()
                        .fg(THEME.green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "─".repeat(inner_w.saturating_sub(14).min(40)),
                    Style::default().fg(THEME.border),
                ),
            ]));
            lines.push(Line::from(""));
        }

        if app.has_active_task() && app.has_console_content() {
            lines.push(Line::from(Span::styled(
                "  [PTY output live — Ctrl+K to view]",
                Style::default().fg(THEME.muted),
            )));
        }
    }

    let total_lines = lines.len() as u16;
    let view_h = area.height.saturating_sub(1).max(1);
    let max_scroll = total_lines.saturating_sub(view_h);
    let scroll = max_scroll.saturating_sub(app.scroll_from_bottom as u16);

    let paragraph = Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: true })
        .scroll((scroll, 0))
        .style(Style::default().bg(THEME.bg));
    f.render_widget(paragraph, area);

    // "New activity" indicator when the user scrolled away from the live view.
    if app.scroll_from_bottom > 0 && max_scroll > 0 {
        let hint = "↓ New activity · End to return";
        let hint_w = hint.len() as u16;
        let x = area.x + area.width.saturating_sub(hint_w + 1);
        let y = area.y + area.height.saturating_sub(1);
        let popup = Rect::new(x, y, hint_w + 1, 1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                hint,
                Style::default()
                    .fg(THEME.bg)
                    .bg(THEME.blue)
                    .add_modifier(Modifier::BOLD),
            ))),
            popup,
        );
    }
}

/// Message avatar + name + optional right-aligned timestamp.
fn message_header_lines(
    avatar: &str,
    avatar_color: Color,
    name: &str,
    name_color: Color,
    timestamp: &str,
    inner_w: usize,
) -> Vec<Line<'static>> {
    let mut spans = vec![
        Span::styled(
            format!("{} ", avatar),
            Style::default()
                .fg(avatar_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            name.to_string(),
            Style::default().fg(name_color).add_modifier(Modifier::BOLD),
        ),
    ];
    if !timestamp.is_empty() {
        let used = avatar.chars().count() + 1 + name.chars().count();
        let pad = inner_w.saturating_sub(used + timestamp.chars().count());
        spans.push(Span::styled(
            format!("{}{}", " ".repeat(pad), timestamp),
            Style::default().fg(THEME.muted),
        ));
    }
    vec![Line::from(spans)]
}

/// Build the chat lines for the phase-grouped action timeline.
fn action_group_lines(app: &TuiApp, inner_w: usize) -> Vec<Line<'static>> {
    let stream = &app.action_stream;
    render_action_groups(
        stream.groups.iter(),
        stream.focused_from_back,
        stream.groups.len(),
        inner_w,
        true,
    )
}

fn render_action_groups<'a, I>(
    groups: I,
    focused_from_back: Option<usize>,
    group_count: usize,
    inner_w: usize,
    focusable: bool,
) -> Vec<Line<'static>>
where
    I: Iterator<Item = &'a UiActionGroup>,
{
    let mut out: Vec<Line<'static>> = Vec::new();
    for (offset, group) in groups.enumerate() {
        let from_back = group_count.saturating_sub(1).saturating_sub(offset);
        let focused = focusable && focused_from_back == Some(from_back);
        out.extend(group_lines(group, inner_w, focused));
    }
    out
}

fn group_lines(group: &UiActionGroup, inner_w: usize, focused: bool) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let phase_color = THEME.phase_color(group.phase);

    // Accent bar — design-spec phase color strip above the card.
    out.push(Line::from(Span::styled(
        "━".repeat(inner_w.min(48).max(8)),
        Style::default().fg(phase_color),
    )));

    // Title: » 📚 Research  ✓  4.1s
    let mut title = String::new();
    if focused {
        title.push_str("» ");
    }
    title.push_str(group.phase.emoji());
    title.push(' ');
    // Gerund while live ("Researching"); stable label once terminal ("Research").
    let phase_name = if group.status.is_terminal() {
        group.phase.label()
    } else {
        phase_gerund(group.phase)
    };
    title.push_str(phase_name);
    title.push(' ');
    title.push_str(group.status.glyph().glyph());
    let duration = group.duration_ms.unwrap_or_else(|| group.elapsed_ms());
    title.push_str(&format!("  {}", format_duration(duration)));

    let mut title_spans = vec![Span::styled(
        truncate_to(&title, inner_w.saturating_sub(10)),
        Style::default()
            .fg(phase_color)
            .add_modifier(Modifier::BOLD),
    )];
    if group.status.is_terminal() {
        let ts = format_timestamp(group.started_at);
        let pad = inner_w.saturating_sub(title.chars().count() + ts.chars().count());
        title_spans.push(Span::styled(
            format!("{}{}", " ".repeat(pad.max(2)), ts),
            Style::default().fg(THEME.muted),
        ));
    }
    out.push(Line::from(title_spans));

    let expanded = group.expanded || group.is_running();
    if expanded {
        for action in group.actions.iter() {
            out.extend(action_card_lines(action, inner_w, phase_color));
        }
        if let Some(v) = &group.verification {
            if let Some(s) = v.summary() {
                out.push(Line::from(vec![
                    Span::styled(
                        format!("  {} ", StatusGlyph::Completed.glyph()),
                        Style::default().fg(THEME.green),
                    ),
                    Span::styled(
                        format!(
                            "Verification · {}",
                            truncate_to(&s, inner_w.saturating_sub(6))
                        ),
                        Style::default().fg(THEME.green),
                    ),
                ]));
            }
        }
    } else {
        let summary = group.summary_line();
        out.push(Line::from(Span::styled(
            format!("  └─ {}", truncate_to(&summary, inner_w.saturating_sub(4))),
            Style::default().fg(THEME.secondary),
        )));
    }

    out.push(Line::from(""));
    out
}

fn phase_gerund(phase: Phase) -> &'static str {
    match phase {
        Phase::Research => "Researching",
        Phase::Testing => "Testing",
        Phase::Planning => "Planning",
        Phase::Coding => "Coding",
        Phase::Review => "Reviewing",
        Phase::Verification => "Verifying",
        Phase::Main => "Thinking",
    }
}

/// Multi-line action card: status row plus nested command/file detail box.
fn action_card_lines(
    action: &crate::tui::actions::UiAction,
    inner_w: usize,
    phase_color: Color,
) -> Vec<Line<'static>> {
    let glyph = action.status.glyph();
    let color = THEME.status_color(glyph);
    let status_mark = if action.status == UiActionStatus::Running {
        spinner_char_now().to_string()
    } else {
        glyph.glyph().to_string()
    };

    let mut lines = Vec::new();
    let mut head = format!(
        "  {} {} {}",
        status_mark,
        action.kind.emoji(),
        action.kind.label()
    );
    if !action.detail.is_empty()
        && !matches!(
            action.kind,
            UiActionKind::RunningCommand | UiActionKind::Testing
        )
    {
        head.push_str(" · ");
        head.push_str(&action.detail);
    }
    if let Some(summary) = &action.result_summary {
        if !matches!(
            action.kind,
            UiActionKind::RunningCommand | UiActionKind::Testing
        ) {
            head.push_str("  ");
            head.push_str(summary);
        }
    }
    lines.push(Line::from(Span::styled(
        truncate_to(&head, inner_w),
        Style::default().fg(color),
    )));

    // Nested terminal / file detail box for commands and edits.
    let box_w = inner_w.saturating_sub(4).max(8);
    if matches!(
        action.kind,
        UiActionKind::RunningCommand | UiActionKind::Testing
    ) && (!action.detail.is_empty() || !action.live_output().is_empty())
    {
        lines.push(Line::from(Span::styled(
            format!("  ┌{}┐", "─".repeat(box_w.saturating_sub(2))),
            Style::default().fg(THEME.border),
        )));
        if !action.detail.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  │ ", Style::default().fg(THEME.border)),
                Span::styled(
                    truncate_to(&action.detail, box_w.saturating_sub(2)),
                    Style::default().fg(phase_color),
                ),
            ]));
        }
        let tail = action
            .live_output()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .last()
            .map(|s| s.trim().to_string())
            .or_else(|| action.result_summary.clone());
        if let Some(t) = tail {
            lines.push(Line::from(vec![
                Span::styled("  │ ", Style::default().fg(THEME.border)),
                Span::styled(
                    truncate_to(&t, box_w.saturating_sub(2)),
                    Style::default().fg(THEME.secondary),
                ),
            ]));
        }
        lines.push(Line::from(Span::styled(
            format!("  └{}┘", "─".repeat(box_w.saturating_sub(2))),
            Style::default().fg(THEME.border),
        )));
    } else if matches!(action.kind, UiActionKind::Editing) && !action.detail.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("  ├─ ", Style::default().fg(THEME.border)),
            Span::styled(
                truncate_to(&action.detail, inner_w.saturating_sub(4)),
                Style::default().fg(THEME.secondary),
            ),
        ]));
        if let Some(summary) = &action.result_summary {
            lines.push(Line::from(vec![
                Span::styled("  └─ ", Style::default().fg(THEME.border)),
                Span::styled(
                    truncate_to(summary, inner_w.saturating_sub(4)),
                    Style::default().fg(THEME.muted),
                ),
            ]));
        }
    }

    lines
}

// ═══════════════════════════════════════════════════════════════════════════
// Right intelligence rail
// ═══════════════════════════════════════════════════════════════════════════

/// Section heights for [Agents, Progress, Context, Activity, Session], in that
/// visual order, shrunk (least important first) to fit the available height.
fn rail_section_heights(total: u16) -> [u16; 5] {
    let mut heights = [5u16, 5, 4, 5, 4];
    let drop_order = [1usize, 2, 4, 3, 0];
    loop {
        let sum: u16 = heights.iter().sum();
        if sum <= total {
            break;
        }
        let mut removed = false;
        for &idx in &drop_order {
            if heights[idx] > 0 {
                heights[idx] -= 1;
                removed = true;
                break;
            }
        }
        if !removed {
            break;
        }
    }
    heights
}

fn render_rail(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.width < 12 || area.height < 4 {
        return;
    }
    let mut heights = rail_section_heights(area.height);
    // The progress section only makes sense while something is actually
    // running (or has run); the empty state shows agents only.
    if app.dashboard.task_graph.is_none() && app.action_stream.groups.is_empty() {
        heights[1] = 0;
    }
    let constraints: Vec<Constraint> = heights.iter().map(|h| Constraint::Length(*h)).collect();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    render_rail_agents(f, app, chunks[0]);
    if heights[1] > 0 {
        render_rail_progress(f, app, chunks[1]);
    }
    if heights[2] > 0 {
        render_rail_context(f, app, chunks[2]);
    }
    if heights[3] > 0 {
        render_rail_activity(f, app, chunks[3]);
    }
    if heights[4] > 0 {
        render_rail_session(f, app, chunks[4]);
    }
}

fn specialist_status(app: &TuiApp, name: &str) -> (StatusGlyph, String, Option<u64>) {
    // Real state comes from the agent status monitor; duration from metrics.
    let status = app
        .dashboard
        .status_monitor
        .get(name)
        .map(|s| s.status.clone());
    let duration = app
        .dashboard
        .metrics
        .as_ref()
        .and_then(|m| m.agent_durations.get(name).copied());

    let (glyph, label) = match status.as_ref() {
        Some(AgentStatus::Completed) => (StatusGlyph::Completed, "Done"),
        Some(AgentStatus::Failed) => (StatusGlyph::Failed, "Failed"),
        Some(AgentStatus::Cancelled) => (StatusGlyph::Cancelled, "Cancelled"),
        Some(AgentStatus::Idle) => (StatusGlyph::Ready, "Ready"),
        Some(_) => (StatusGlyph::Running, "Running"),
        None => (StatusGlyph::Ready, "Ready"),
    };
    (glyph, label.to_string(), duration)
}

fn render_rail_agents(f: &mut Frame, app: &TuiApp, area: Rect) {
    let rows: [(Phase, &str); 5] = [
        (Phase::Research, "research"),
        (Phase::Testing, "testing"),
        (Phase::Planning, "planning"),
        (Phase::Coding, "coding"),
        (Phase::Review, "review"),
    ];
    let inner = area.width.saturating_sub(2) as usize;
    let mut lines = Vec::new();
    for (phase, name) in rows {
        let (glyph, label, duration) = specialist_status(app, name);
        let color = THEME.status_color(glyph);
        let mark = if glyph == StatusGlyph::Running {
            spinner_char_now().to_string()
        } else {
            glyph.glyph().to_string()
        };
        let mut left = format!("{} {} {}", mark, phase.emoji(), phase.label());
        let right = if let Some(ms) = duration {
            format!("{} {}", label, format_duration(ms))
        } else {
            label
        };
        let pad = inner
            .saturating_sub(left.chars().count() + right.chars().count() + 1)
            .max(1);
        left = format!("{}{}{}", left, " ".repeat(pad), right);
        lines.push(Line::from(Span::styled(
            truncate_to(&left, inner),
            Style::default().fg(color),
        )));
    }
    render_rail_section(f, area, "▾ AGENTS", lines);
}

/// (done, total) of the rail's specialist progress. `total` is the number of
/// specialist phases ENABLED by the task's actual [`TaskMode`] (from the
/// canonical `enabled_phase_names`), and `done` counts those with real
/// Completed runtime status. Disabled phases never appear — a mode with only
/// Research enabled shows 0/1, never 0/5. Main is never counted: the rail
/// tracks specialists.
pub fn rail_progress_counts(app: &TuiApp) -> (usize, usize) {
    let names = app.task_mode.enabled_phase_names();
    let done = names
        .iter()
        .filter(|name| matches!(specialist_status(app, name).0, StatusGlyph::Completed))
        .count();
    (done, names.len())
}

fn render_rail_progress(f: &mut Frame, app: &TuiApp, area: Rect) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let inner = area.width.saturating_sub(2) as usize;
    let graph = app.dashboard.graph_entries();
    let phases: Vec<(Phase, &str)> = app
        .task_mode
        .enabled_phase_names()
        .iter()
        .filter_map(|name| ActionStream::phase_for(name).map(|phase| (phase, *name)))
        .collect();
    let (done, total) = rail_progress_counts(app);
    let bar_w = inner.saturating_sub(8).max(4);
    let filled = if total == 0 {
        0
    } else {
        (done * bar_w) / total
    };
    let bar = format!(
        "{}{}",
        "█".repeat(filled),
        "░".repeat(bar_w.saturating_sub(filled))
    );
    lines.push(Line::from(vec![
        Span::styled(
            format!(" {}/{} ", done, total),
            Style::default().fg(THEME.secondary),
        ),
        Span::styled(bar, Style::default().fg(THEME.purple)),
    ]));

    if !graph.is_empty() {
        for (desc, agent, status) in graph.iter().take(area.height.saturating_sub(3) as usize) {
            let (icon, color) = match status {
                TaskStatus::Completed => ("✓", THEME.green),
                TaskStatus::Failed => ("✗", THEME.red),
                TaskStatus::Running => ("●", THEME.purple),
                TaskStatus::Cancelled => ("⏸", THEME.yellow),
                _ => ("○", THEME.muted),
            };
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", icon), Style::default().fg(color)),
                Span::styled(
                    truncate_to(&format!("{} {}", agent, desc), inner.saturating_sub(3)),
                    Style::default().fg(THEME.primary),
                ),
            ]));
        }
    } else {
        for (phase, name) in phases {
            let (glyph, _, _) = specialist_status(app, name);
            let color = THEME.status_color(glyph);
            lines.push(Line::from(Span::styled(
                format!(" {} {}", glyph.glyph(), phase.label()),
                Style::default().fg(color),
            )));
        }
    }
    render_rail_section(f, area, "▾ TASK PROGRESS", lines);
}

fn render_rail_context(f: &mut Frame, app: &TuiApp, area: Rect) {
    let s = &app.action_stream;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let inner = area.width.saturating_sub(2) as usize;
    let col = (inner / 2).max(8);

    // 2-column grid of real counters (design-spec Context Summary).
    let cells: Vec<(&str, String, Color)> = {
        let mut c = Vec::new();
        let files = s.files_inspected.len();
        if files > 0 {
            c.push(("Files", format!("{}", files), THEME.primary));
        }
        if !s.tools_used.is_empty() {
            c.push(("Tools", format!("{}", s.tools_used.len()), THEME.primary));
        }
        if s.tests_total > 0 {
            c.push((
                "Tests",
                format!("{}/{}", s.tests_passed, s.tests_total),
                if s.tests_passed == s.tests_total {
                    THEME.green
                } else {
                    THEME.yellow
                },
            ));
        }
        if s.tool_calls > 0 {
            c.push(("Calls", format!("{}", s.tool_calls), THEME.primary));
        }
        c
    };

    if cells.is_empty() {
        lines.push(Line::from(Span::styled(
            "  —",
            Style::default().fg(THEME.muted),
        )));
    } else {
        for pair in cells.chunks(2) {
            let mut spans = Vec::new();
            for (i, (label, value, color)) in pair.iter().enumerate() {
                let cell = format!("{} {}", label, value);
                let padded = if i == 0 {
                    format!(" {:<width$}", cell, width = col.saturating_sub(1))
                } else {
                    cell
                };
                spans.push(Span::styled(
                    truncate_to(&padded, col),
                    Style::default().fg(*color),
                ));
            }
            lines.push(Line::from(spans));
        }
    }

    if s.failures > 0 {
        lines.push(Line::from(vec![
            Span::styled(" ⚠ Risks ", Style::default().fg(THEME.yellow)),
            Span::styled(format!("{}", s.failures), Style::default().fg(THEME.yellow)),
        ]));
    }
    let verified = s
        .groups
        .iter()
        .filter(|g| g.verification.as_ref().and_then(|v| v.summary()).is_some())
        .count();
    if verified > 0 {
        lines.push(Line::from(vec![
            Span::styled(" ✓ Verified ", Style::default().fg(THEME.green)),
            Span::styled(format!("{}", verified), Style::default().fg(THEME.green)),
        ]));
    }
    render_rail_section(f, area, "▾ CONTEXT", lines);
}

fn render_rail_activity(f: &mut Frame, app: &TuiApp, area: Rect) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    match app.action_stream.current_activity() {
        Some((group, action)) => {
            let color = THEME.phase_color(group.phase);
            lines.push(Line::from(vec![Span::styled(
                format!(" {} {}", group.phase.emoji(), phase_gerund(group.phase)),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )]));
            let detail = if !action.detail.is_empty() {
                action.detail.clone()
            } else if !action.title.is_empty() {
                action.title.clone()
            } else {
                action.kind.label().to_string()
            };
            lines.push(Line::from(Span::styled(
                format!(
                    " {}",
                    truncate_to(&detail, area.width.saturating_sub(3) as usize)
                ),
                Style::default().fg(THEME.primary),
            )));
        }
        None => {
            lines.push(Line::from(Span::styled(
                " ○ Idle",
                Style::default().fg(THEME.muted),
            )));
        }
    }
    render_rail_section(f, area, "▾ CURRENT ACTIVITY", lines);
}

/// Session panel rows, built from real machine state only. No "Started" row
/// exists: there is no authoritative wall-clock session-start timestamp in
/// the app (only an elapsed duration), so a start time would have to be
/// reconstructed — it is never shown. Duration, Tokens (when available) and
/// Tools are real.
pub fn session_panel_rows(app: &TuiApp) -> Vec<(String, String)> {
    let mut rows = vec![(
        "Duration".to_string(),
        format_duration(app.session_duration_secs() * 1000),
    )];
    if let Some(metrics) = &app.dashboard.metrics {
        let tokens = metrics.total_tokens();
        if tokens > 0 {
            rows.push((
                "Tokens".to_string(),
                crate::metrics::format_token_count(tokens),
            ));
        }
    }
    rows.push((
        "Tools".to_string(),
        format!("{}", app.action_stream.tool_calls),
    ));
    rows
}

fn render_rail_session(f: &mut Frame, app: &TuiApp, area: Rect) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let rows = session_panel_rows(app);
    for (label, value) in rows {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {:<9}", label),
                Style::default().fg(THEME.secondary),
            ),
            Span::styled(value, Style::default().fg(THEME.primary)),
        ]));
    }
    render_rail_section(f, area, "▾ SESSION", lines);
}

fn render_rail_section(f: &mut Frame, area: Rect, title: &str, lines: Vec<Line<'static>>) {
    if area.height < 2 {
        return;
    }
    let inner_h = area.height.saturating_sub(2) as usize;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(THEME.border_style())
        .style(Style::default().bg(THEME.surface))
        .title(Span::styled(title, THEME.title_style()));
    let mut visible = lines;
    visible.truncate(inner_h);
    let widget = Paragraph::new(Text::from(visible))
        .block(block)
        .style(Style::default().bg(THEME.bg));
    f.render_widget(widget, area);
}

// ═══════════════════════════════════════════════════════════════════════════
// Input + footer
// ═══════════════════════════════════════════════════════════════════════════

fn render_input(f: &mut Frame, app: &TuiApp, area: Rect) {
    let prefix = if let Some(s) = &app.secure_input {
        format!("API key for {}: ", s.provider)
    } else if app.dashboard.animation.is_active() {
        format!("{}❯ ", spinner_char_now())
    } else {
        "❯ ".to_string()
    };

    let inner_h = area.height.saturating_sub(1) as usize;
    let inner_w = area.width.saturating_sub(2) as usize;
    if inner_h == 0 {
        return;
    }

    let (display_lines, cursor_line_rel, cursor_col) = if let Some(s) = &app.secure_input {
        // Masked secret display: the raw buffer is never rendered.
        let masked: String = "•".repeat(s.buffer.chars().count());
        let line = if masked.is_empty() {
            "(secret)".to_string()
        } else {
            masked
        };
        (vec![line], 0usize, s.buffer.chars().count())
    } else {
        input_display_lines(&app.input, app.input_cursor, inner_h, inner_w)
    };

    let mut text_lines = Vec::new();
    for (i, line) in display_lines.iter().enumerate() {
        let full = if i == 0 {
            format!("{}{}", prefix, line)
        } else {
            line.clone()
        };
        text_lines.push(Line::from(truncate_to(&full, area.width as usize)));
    }
    if app.input.is_empty() && app.secure_input.is_none() {
        text_lines[0] = Line::from(Span::styled(
            format!("{}Ask CodeBro anything... (Ctrl+Enter to send)", prefix),
            Style::default().fg(THEME.muted),
        ));
    }

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(THEME.border_style())
        .style(Style::default().bg(THEME.surface));
    let paragraph = Paragraph::new(Text::from(text_lines))
        .block(block)
        .style(Style::default().fg(THEME.primary).bg(THEME.surface));
    f.render_widget(paragraph, area);

    // Right-aligned action chips matching the design-spec input chrome.
    if area.height >= 2 && area.width >= 28 && app.secure_input.is_none() {
        let hints: Vec<(&str, &str, Color)> = if app.has_active_task() {
            vec![("Ctrl+C", "Cancel", THEME.yellow)]
        } else {
            vec![
                ("Ctrl+P", "Actions", THEME.secondary),
                ("Enter", "Send", THEME.purple),
            ]
        };
        let mut x = area.x + area.width;
        for (key, label, color) in hints.into_iter().rev() {
            let text = format!(" {} {} ", key, label);
            let w = text.chars().count() as u16;
            x = x.saturating_sub(w + 1);
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!(" {} ", key),
                        Style::default().fg(THEME.muted).bg(THEME.border),
                    ),
                    Span::styled(
                        format!("{} ", label),
                        Style::default().fg(color).bg(THEME.border),
                    ),
                ])),
                Rect::new(x, area.y, w, 1),
            );
        }
    }

    if area.width >= 4 && area.height >= 2 {
        let x_off = if cursor_line_rel == 0 {
            prefix.chars().count() + cursor_col
        } else {
            cursor_col
        };
        let x = (area.x + x_off as u16).min(area.x + area.width.saturating_sub(2));
        let y = (area.y + 1 + cursor_line_rel as u16).min(area.y + area.height.saturating_sub(2));
        f.set_cursor(x, y);
    }
}

fn render_footer(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.height < 1 || area.width < 8 {
        return;
    }

    let mut left: Vec<Span<'static>> = Vec::new();
    let entries: [(&str, &str); 5] = [
        ("Ctrl+M", "Memory"),
        ("Ctrl+P", "Palette"),
        ("Ctrl+D", "Diff"),
        ("Ctrl+O", "Rail"),
        ("?", "Help"),
    ];
    for (i, (key, rest)) in entries.iter().enumerate() {
        if i > 0 {
            left.push(Span::styled("  ", Style::default().fg(THEME.muted)));
        }
        left.push(Span::styled(
            format!("{} ", key),
            Style::default().fg(THEME.secondary),
        ));
        left.push(Span::styled(*rest, Style::default().fg(THEME.muted)));
    }

    let mut right: Vec<Span<'static>> = Vec::new();
    let (dot, conn_color, conn_label) = {
        let active = app
            .provider_manager
            .as_ref()
            .and_then(|pm| pm.active_provider().cloned());
        let health = active.as_ref().and_then(|id| {
            app.provider_panel
                .health_results
                .iter()
                .find(|(pid, _, _)| pid == id)
                .map(|(_, h, _)| h)
        });
        match health {
            Some(crate::provider_manager::HealthStatus::Healthy) => ("●", THEME.green, "Connected"),
            Some(crate::provider_manager::HealthStatus::Unhealthy { .. }) => {
                ("⚠", THEME.yellow, "Unavailable")
            }
            _ => ("○", THEME.muted, "Ready"),
        }
    };
    right.push(Span::styled(
        format!("{} {}", dot, conn_label),
        Style::default().fg(conn_color),
    ));
    if !app.workspace_path_display.is_empty() {
        right.push(Span::styled("  ", Style::default()));
        right.push(Span::styled(
            truncate_to(&app.workspace_path_display, 28),
            Style::default().fg(THEME.secondary),
        ));
    }
    if !app.git_branch.is_empty() {
        right.push(Span::styled("  ⎇ ", Style::default().fg(THEME.muted)));
        right.push(Span::styled(
            app.git_branch.clone(),
            Style::default().fg(THEME.green),
        ));
    }

    let right_w = right
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>() as u16;
    let left_budget = area.width.saturating_sub(right_w.saturating_add(2));
    f.render_widget(
        Paragraph::new(truncate_line_to(&Line::from(left), left_budget as usize))
            .style(Style::default().bg(THEME.bg)),
        Rect::new(area.x, area.y, left_budget.max(1), 1),
    );
    if right_w > 0 && area.width > right_w {
        f.render_widget(
            Paragraph::new(Line::from(right)).style(Style::default().bg(THEME.bg)),
            Rect::new(
                area.x + area.width.saturating_sub(right_w),
                area.y,
                right_w,
                1,
            ),
        );
    }
}

/// Truncate a line of spans to `width` columns, dropping trailing spans.
fn truncate_line_to(line: &Line<'static>, width: usize) -> Line<'static> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for span in &line.spans {
        let len = span.content.chars().count();
        if used + len <= width {
            out.push(span.clone());
            used += len;
        } else if used < width {
            let room = width - used;
            let s = truncate_to(&span.content, room);
            out.push(Span::styled(s, span.style));
            used = width;
        } else {
            break;
        }
    }
    Line::from(out)
}

// ═══════════════════════════════════════════════════════════════════════════
// Overlays
// ═══════════════════════════════════════════════════════════════════════════

fn centered_popup(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.max(4), height.max(2))
}

fn render_console_popup(f: &mut Frame, app: &TuiApp, area: Rect) {
    let width = area.width.min(area.width / 2 + area.width / 4).max(40);
    let height = area.height.min(16).max(6);
    let popup = centered_popup(area, width, height);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(THEME.border_style())
        .title(Span::styled(
            format!(
                "PTY Console (Ctrl+K to close){}",
                if app.has_console_content() {
                    " · Esc close"
                } else {
                    ""
                }
            ),
            THEME.title_style(),
        ));
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(console) = app.active_console_ref() {
        let inner_w = popup.width.saturating_sub(2) as usize;
        let inner_h = popup.height.saturating_sub(2) as usize;
        let rendered = console.render_lines(inner_w);
        let status = console.status;
        let status_line = format!(
            "  {} {}",
            status.label(),
            match status {
                crate::tui::console::ConsoleStatus::Exited { exit_code } => {
                    format!("exit {}", exit_code)
                }
                _ => String::new(),
            }
        );
        lines.push(Line::from(Span::styled(
            truncate_to(&status_line, inner_w),
            Style::default().fg(status.color()),
        )));
        let start = rendered.len().saturating_sub(inner_h.saturating_sub(1));
        for line in rendered.iter().skip(start).take(inner_h.saturating_sub(1)) {
            lines.push(line.clone());
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  No PTY output yet.",
            Style::default().fg(THEME.muted),
        )));
    }
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .style(Style::default().bg(THEME.bg)),
        popup,
    );
}

fn render_agents_popup(f: &mut Frame, app: &TuiApp, area: Rect) {
    let popup = centered_popup(area, 60, 12);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(THEME.border_style())
        .title(Span::styled(
            "AGENTS (Ctrl+A to close)",
            THEME.title_style(),
        ));
    let mut lines = Vec::new();
    let entries = app.dashboard.agent_entries();
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no active agents",
            Style::default().fg(THEME.muted),
        )));
    } else {
        for entry in entries {
            let (icon, color) = match entry.status {
                AgentStatus::Completed => ("✓", THEME.green),
                AgentStatus::Failed => ("✗", THEME.red),
                AgentStatus::Cancelled => ("⏸", THEME.yellow),
                AgentStatus::Idle => ("○", THEME.muted),
                _ => ("●", THEME.purple),
            };
            let name = truncate_to(&entry.name, 14);
            let action = entry.action.as_deref().unwrap_or("");
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                Span::styled(name, Style::default().fg(THEME.primary)),
                Span::styled(
                    format!("  {}", truncate_to(action, 34)),
                    Style::default().fg(THEME.muted),
                ),
            ]));
        }
    }
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .style(Style::default().bg(THEME.bg)),
        popup,
    );
}

fn render_task_graph_popup(f: &mut Frame, app: &TuiApp, area: Rect) {
    let popup = centered_popup(area, 64, 12);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(THEME.border_style())
        .title(Span::styled(
            "TASK GRAPH (Ctrl+G to close)",
            THEME.title_style(),
        ));
    let mut lines = Vec::new();
    let entries = app.dashboard.graph_entries();
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no task running",
            Style::default().fg(THEME.muted),
        )));
    } else {
        for (desc, agent, status) in entries.iter().take(8) {
            let (icon, color) = match status {
                TaskStatus::Completed => ("✓", THEME.green),
                TaskStatus::Failed => ("✗", THEME.red),
                TaskStatus::Running => ("●", THEME.purple),
                TaskStatus::Cancelled => ("⏸", THEME.yellow),
                _ => ("○", THEME.muted),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                Span::styled(truncate_to(agent, 10), Style::default().fg(THEME.secondary)),
                Span::styled(
                    format!(" {}", truncate_to(desc, 42)),
                    Style::default().fg(THEME.primary),
                ),
            ]));
        }
    }
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .style(Style::default().bg(THEME.bg)),
        popup,
    );
}

fn render_metrics_popup(f: &mut Frame, app: &TuiApp, area: Rect) {
    let popup = centered_popup(area, 56, 10);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(THEME.border_style())
        .title(Span::styled(
            "METRICS (Ctrl+V to close)",
            THEME.title_style(),
        ));
    let mut lines = Vec::new();
    let agent_count = app.dashboard.status_monitor.count();
    let active_count = app.dashboard.status_monitor.active_count();
    lines.push(Line::from(Span::styled(
        format!("  agents {} ({} active)", agent_count, active_count),
        Style::default().fg(THEME.secondary),
    )));
    let total_tokens = app
        .dashboard
        .metrics
        .as_ref()
        .map(|m| m.total_tokens())
        .unwrap_or(0);
    let cost = app
        .dashboard
        .metrics
        .as_ref()
        .map(|m| m.estimated_cost_usd(&app.config.model))
        .unwrap_or(0.0);
    lines.push(Line::from(Span::styled(
        format!(
            "  tokens {} · cost {}",
            crate::metrics::format_token_count(total_tokens),
            crate::metrics::format_cost_usd(cost)
        ),
        Style::default().fg(THEME.secondary),
    )));
    lines.push(Line::from(Span::styled(
        format!("  tool calls {}", app.action_stream.tool_calls),
        Style::default().fg(THEME.secondary),
    )));
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .style(Style::default().bg(THEME.bg)),
        popup,
    );
}

fn render_coordination_popup(f: &mut Frame, app: &TuiApp, area: Rect) {
    let popup = centered_popup(area, 64, 12);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(THEME.border_style())
        .title(Span::styled(
            "COORDINATION (Ctrl+O to close)",
            THEME.title_style(),
        ));
    let mut lines = Vec::new();
    for msg in app.dashboard.recent_messages.iter().take(8) {
        lines.push(Line::from(Span::styled(
            format!(
                "  {}",
                truncate_to(msg, popup.width.saturating_sub(4) as usize)
            ),
            Style::default().fg(THEME.secondary),
        )));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no coordination activity",
            Style::default().fg(THEME.muted),
        )));
    }
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .style(Style::default().bg(THEME.bg)),
        popup,
    );
}

fn render_memory_popup(f: &mut Frame, app: &TuiApp, area: Rect) {
    let popup = centered_popup(area, 60, 12);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(THEME.border_style())
        .title(Span::styled(
            "MEMORY (Ctrl+M to close)",
            THEME.title_style(),
        ));
    let mut lines = Vec::new();
    for note in app.dashboard.memory_notifications.iter().take(8) {
        lines.push(Line::from(Span::styled(
            format!(
                "  {}  {}",
                truncate_to(&note.timestamp, 19),
                truncate_to(&note.message, popup.width.saturating_sub(24) as usize)
            ),
            Style::default().fg(THEME.secondary),
        )));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no memory changes",
            Style::default().fg(THEME.muted),
        )));
    }
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .style(Style::default().bg(THEME.bg)),
        popup,
    );
}

fn render_activity_popup(f: &mut Frame, app: &TuiApp, area: Rect) {
    let popup = centered_popup(area, 66, 14);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(THEME.border_style())
        .title(Span::styled(
            "ACTIVITY (Ctrl+T to close)",
            THEME.title_style(),
        ));
    let mut lines = Vec::new();
    for entry in app.dashboard.activity_log.iter().take(12) {
        let color = match entry.level.as_str() {
            "error" => THEME.red,
            "tool" => THEME.purple,
            "task" => THEME.green,
            "console" => THEME.blue,
            _ => THEME.muted,
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}  ", truncate_to(&entry.timestamp, 8)),
                Style::default().fg(THEME.muted),
            ),
            Span::styled(
                truncate_to(&entry.message, popup.width.saturating_sub(16) as usize),
                Style::default().fg(color),
            ),
        ]));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no activity",
            Style::default().fg(THEME.muted),
        )));
    }
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .style(Style::default().bg(THEME.bg)),
        popup,
    );
}

fn render_confirmation(f: &mut Frame, message: &str, input_area: Rect) {
    let width = input_area.width.min(80);
    let height = 3u16;
    let x = input_area.x;
    let y = input_area.y.saturating_sub(height);
    let popup = Rect::new(x, y, width, height);
    f.render_widget(Clear, popup);
    let lines = vec![
        Line::from(Span::styled(
            format!("  🛡 {}", truncate_to(message, width as usize)),
            Style::default().fg(THEME.yellow),
        )),
        Line::from(Span::styled(
            "  y/Enter = proceed · n/Esc = cancel",
            Style::default().fg(THEME.muted),
        )),
    ];
    f.render_widget(Paragraph::new(Text::from(lines)), popup);
}

fn render_autocomplete(f: &mut Frame, app: &TuiApp, input_area: Rect) {
    let entries: Vec<String> = app.dashboard.autocomplete.iter().take(6).cloned().collect();
    if entries.is_empty() {
        return;
    }
    let width = input_area.width.min(60);
    let height = (entries.len() as u16 + 1).min(input_area.y.saturating_sub(2));
    if height < 2 {
        return;
    }
    let top = input_area.y.saturating_sub(height);
    let popup = Rect::new(input_area.x, top, width, height);

    let mut lines: Vec<Line> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let selected = i == app.dashboard.autocomplete_index;
        let spec = commands::all_commands().find(|s| s.command == entry.as_str());
        let desc = spec.map(|s| s.description).unwrap_or("");
        let (marker_style, entry_style) = if selected {
            (
                Style::default()
                    .fg(THEME.bg)
                    .bg(THEME.purple)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(THEME.bg)
                    .bg(THEME.purple)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (
                Style::default().fg(THEME.muted),
                Style::default().fg(THEME.primary),
            )
        };
        lines.push(Line::from(vec![
            Span::styled(if selected { "> " } else { "  " }, marker_style),
            Span::styled(entry.clone(), entry_style),
            Span::styled(
                format!(
                    "  {}",
                    truncate_to(desc, width.saturating_sub(entry.len() as u16 + 4) as usize)
                ),
                Style::default().fg(THEME.muted),
            ),
        ]));
    }
    f.render_widget(Paragraph::new(Text::from(lines)), popup);
}

fn render_command_palette(f: &mut Frame, app: &TuiApp, chat_area: Rect) {
    let query = app.dashboard.palette_query.clone();
    let entries = palette_entries(&query);
    let width = chat_area.width.min(72);
    let height = (entries.len() as u16 + 2).min(chat_area.height.min(18));
    if height < 3 {
        return;
    }
    let popup = centered_popup(chat_area, width, height);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(THEME.border_style())
        .title(Span::styled("COMMAND PALETTE", THEME.title_style()));

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            "  search> ",
            Style::default()
                .fg(THEME.purple)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(query.clone(), Style::default().fg(THEME.primary)),
        Span::styled(
            format!("  ({} matches)", entries.len()),
            Style::default().fg(THEME.muted),
        ),
    ]));

    let inner_h = height.saturating_sub(3) as usize;
    for (i, (cmd, desc)) in entries.iter().take(inner_h).enumerate() {
        let selected = i == app.dashboard.palette_index;
        let style = if selected {
            Style::default()
                .fg(THEME.bg)
                .bg(THEME.purple)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(THEME.primary)
        };
        lines.push(Line::from(vec![
            Span::styled(if selected { "> " } else { "  " }, style),
            Span::styled(cmd.clone(), style),
            Span::styled(
                format!("  {}", truncate_to(desc, width as usize)),
                Style::default().fg(THEME.muted),
            ),
        ]));
    }

    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .style(Style::default().bg(THEME.bg)),
        popup,
    );
}

fn palette_entries(filter: &str) -> Vec<(String, &'static str)> {
    let f = filter.to_lowercase();
    commands::all_commands()
        .filter(|spec| {
            f.is_empty()
                || spec.command.to_lowercase().contains(&f)
                || spec.description.to_lowercase().contains(&f)
        })
        .map(|spec| (spec.command.to_string(), spec.description))
        .collect()
}

fn render_model_picker(f: &mut Frame, app: &TuiApp) {
    let size = f.size();
    let width = (size.width as usize).min(70);
    let height = (size.height as usize).min(24);

    let x = ((size.width as usize).saturating_sub(width)) / 2;
    let y = ((size.height as usize).saturating_sub(height)) / 2;
    if x + width > size.width as usize || y + height > size.height as usize {
        return;
    }
    let area = Rect::new(x as u16, y as u16, width as u16, height as u16);
    f.render_widget(Clear, area);

    let picker = &app.dashboard.model_picker;
    let mut lines: Vec<Line> = Vec::new();

    let header = format!(
        "  Model picker - {} models (type to filter, Enter=select, Esc=cancel)",
        picker.count()
    );
    lines.push(Line::from(Span::styled(
        header,
        Style::default()
            .fg(THEME.purple)
            .add_modifier(Modifier::BOLD),
    )));

    if picker.loading {
        lines.push(Line::from(Span::styled(
            "  Loading models...",
            Style::default().fg(THEME.yellow),
        )));
    } else if let Some(ref err) = picker.error {
        lines.push(Line::from(Span::styled(
            format!("  Error: {}", err),
            Style::default().fg(THEME.red),
        )));
    } else {
        let visible = picker.visible_models();
        let current = app.config.model.clone();
        let start = picker
            .index
            .saturating_sub(height.saturating_sub(4) as usize / 2);
        for (i, model) in visible
            .iter()
            .enumerate()
            .skip(start)
            .take(height as usize - 3)
        {
            let selected = i == picker.index;
            let is_current = model == &current;
            let marker = if selected { ">" } else { " " };
            let mut spans = vec![Span::raw(format!("{} ", marker))];
            if is_current {
                spans.push(Span::styled(
                    format!("{}  (current)", model),
                    Style::default().fg(THEME.green),
                ));
            } else {
                spans.push(Span::styled(
                    truncate_to(model, width.saturating_sub(6)),
                    Style::default().fg(if selected {
                        THEME.yellow
                    } else {
                        THEME.primary
                    }),
                ));
            }
            lines.push(Line::from(spans));
        }
    }

    if !picker.filter.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  filter: {}", picker.filter),
            Style::default().fg(THEME.muted),
        )));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(THEME.border_style())
        .title("Model");
    let widget = Paragraph::new(Text::from(lines))
        .block(block)
        .style(Style::default().bg(THEME.bg));
    f.render_widget(widget, area);
}

fn handle_model_picker_key(key: crossterm::event::KeyEvent, app: &mut TuiApp) {
    match key.code {
        KeyCode::Up => app.dashboard.model_picker.prev(),
        KeyCode::Down => app.dashboard.model_picker.next(),
        KeyCode::PageUp => {
            for _ in 0..5 {
                app.dashboard.model_picker.prev();
            }
        }
        KeyCode::PageDown => {
            for _ in 0..5 {
                app.dashboard.model_picker.next();
            }
        }
        KeyCode::Enter => {
            if let Some(model) = app.dashboard.model_picker.selected() {
                app.apply_model(model);
            }
        }
        KeyCode::Esc => {
            app.dashboard.model_picker.close();
        }
        KeyCode::Backspace => {
            app.dashboard.model_picker.filter.pop();
            app.dashboard.model_picker.index = 0;
        }
        KeyCode::Char(c) => {
            if app.dashboard.model_picker.loading {
                return;
            }
            app.dashboard.model_picker.filter.push(c);
            app.dashboard.model_picker.index = 0;
        }
        _ => {}
    }
}

fn handle_palette_key(key: crossterm::event::KeyEvent, app: &mut TuiApp) {
    let filtered = palette_entries(&app.dashboard.palette_query);
    match key.code {
        KeyCode::Char(c) => {
            app.dashboard.palette_query.push(c);
            app.dashboard.palette_index = 0;
        }
        KeyCode::Backspace => {
            app.dashboard.palette_query.pop();
            app.dashboard.palette_index = 0;
        }
        KeyCode::Up => {
            if !filtered.is_empty() {
                app.dashboard.palette_index =
                    (app.dashboard.palette_index + filtered.len() - 1) % filtered.len();
            }
        }
        KeyCode::Down | KeyCode::Tab => {
            if !filtered.is_empty() {
                app.dashboard.palette_index = (app.dashboard.palette_index + 1) % filtered.len();
            }
        }
        KeyCode::Enter => {
            if let Some(cmd) = filtered
                .get(app.dashboard.palette_index)
                .map(|c| c.0.clone())
            {
                app.dashboard.toggle_command_palette();
                app.add_message(MessageRole::User, cmd.clone());
                app.dashboard.show_command_palette = false;
                app.is_loading = false;
                submit_input(cmd, app);
            } else {
                app.dashboard.toggle_command_palette();
            }
        }
        KeyCode::Esc => {
            app.dashboard.toggle_command_palette();
        }
        _ => {}
    }
}

fn handle_shortcut(shortcut: Shortcut, app: &mut TuiApp) {
    match shortcut {
        Shortcut::ToggleAgents => app.dashboard.toggle_agents(),
        Shortcut::ToggleTaskGraph => app.dashboard.toggle_task_graph(),
        Shortcut::ToggleMemory => app.dashboard.toggle_memory(),
        Shortcut::SaveSession => {
            if let Err(e) = save_session(app) {
                app.add_message(MessageRole::System, format!("Save error: {}", e));
            }
        }
        Shortcut::ToggleTrace => app.dashboard.toggle_trace(),
        Shortcut::ClearLogs => {
            app.dashboard.clear_logs();
            app.clear_screen();
        }
        Shortcut::CancelTask => {
            app.cancel_current_task();
        }
        Shortcut::Quit => {
            app.should_quit = true;
        }
        Shortcut::OpenCommandPalette => {
            app.dashboard.toggle_command_palette();
        }
        Shortcut::ToggleMetrics => {
            app.dashboard.toggle_metrics();
        }
        Shortcut::ToggleCoordination => {
            app.dashboard.toggle_coordination();
        }
        Shortcut::ToggleRail => {
            app.toggle_rail();
        }
        Shortcut::ToggleConsole => {
            app.toggle_console();
        }
        Shortcut::SendInput => {
            if app.secure_input.is_some() || app.pending_confirmation.is_some() {
                return;
            }
            let input = app.input.trim().to_string();
            if input.is_empty() {
                return;
            }
            if !is_inline_apikey(&input) {
                app.push_history(input.clone());
            }
            app.clear_input();
            submit_input(input, app);
        }
        Shortcut::ViewDiff => {
            // Reuses the existing staged-change preview flow (`/apply` →
            // `//approve`); no new diff engine.
            match &app.pending_change {
                Some(plan) => {
                    let path = plan.path().display().to_string();
                    let preview = plan.preview();
                    app.add_message(
                        MessageRole::System,
                        format!("Diff for {} (not applied):\n{}", path, preview),
                    );
                }
                None => {
                    app.add_message(
                        MessageRole::System,
                        "No pending change to show. Stage one with /apply <file> <new content>."
                            .to_string(),
                    );
                }
            }
        }
    }
}

fn save_session(app: &TuiApp) -> Result<()> {
    let session_dir = std::path::Path::new(".codebro");
    if !session_dir.exists() {
        std::fs::create_dir_all(session_dir)?;
    }
    let session_path = session_dir.join(format!("session_{}.json", app.session_id));
    let json = serde_json::to_string_pretty(&app.messages)?;
    std::fs::write(&session_path, json)?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Formatting helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Spinner frame derived from wall time; cheap and stateless so it can be used
/// anywhere without owning animation state.
fn spinner_char_now() -> &'static str {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let frame = ((millis / ANIM_FRAME_MS) as usize) % SPINNER_FRAMES.len();
    SPINNER_FRAMES[frame]
}

fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let secs = ms / 1000;
        let mins = secs / 60;
        format!("{}m {:02}s", mins, secs % 60)
    }
}

/// Real local timestamp from an `Instant`.
fn format_timestamp(instant: std::time::Instant) -> String {
    let elapsed = instant.elapsed();
    let now = chrono::Local::now();
    let ts = now - chrono::Duration::from_std(elapsed).unwrap_or_default();
    ts.format("%H:%M:%S").to_string()
}

/// Returns the lines of the input buffer that should be rendered and the
/// visible cursor position, keeping the cursor line in view.
fn input_display_lines(
    input: &str,
    cursor_byte: usize,
    inner_h: usize,
    inner_w: usize,
) -> (Vec<String>, usize, usize) {
    let lines: Vec<&str> = input.split('\n').collect();
    let cursor_byte = cursor_byte.min(input.len());
    let up_to = &input[..cursor_byte];
    let cursor_line = up_to.matches('\n').count();
    let cursor_col = up_to
        .rsplit('\n')
        .next()
        .map(|s| s.chars().count())
        .unwrap_or(0);

    let total = lines.len();
    let first = if total > inner_h {
        if cursor_line + 1 > inner_h {
            cursor_line + 1 - inner_h
        } else {
            0
        }
    } else {
        0
    };
    let last = (first + inner_h).min(total);

    let mut display = Vec::new();
    for line in lines.iter().skip(first).take(last - first) {
        display.push(truncate_to(line, inner_w));
    }

    (display, cursor_line.saturating_sub(first), cursor_col)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_runtime::TaskMode;

    fn make_app() -> TuiApp {
        TuiApp::new().expect("app creation")
    }

    fn rect(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    #[test]
    fn test_input_display_lines_single() {
        let (lines, l, c) = input_display_lines("hello", 3, 3, 50);
        assert_eq!(lines, vec!["hello"]);
        assert_eq!((l, c), (0, 3));
    }

    #[test]
    fn test_input_display_lines_multiline() {
        let input = "line one\nline two\nline three";
        let (lines, l, c) = input_display_lines(input, input.len(), 2, 50);
        assert_eq!(lines, vec!["line two", "line three"]);
        assert_eq!((l, c), (1, 10));
    }

    #[test]
    fn test_input_display_lines_truncates() {
        let long = "a".repeat(100);
        let (lines, _, _) = input_display_lines(&long, 100, 3, 10);
        assert!(lines[0].chars().count() <= 10);
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate_to("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_long() {
        let out = truncate_to("a very long line of text", 10);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn test_truncate_zero() {
        assert_eq!(truncate_to("hello", 0), "");
    }

    // ─── Layout ─────────────────────────────────────────────────────────

    #[test]
    fn test_layout_default_expanded_rail() {
        let app = make_app();
        // 120x40: rail on, ~22% width, chat dominates.
        let layout = compute_ui_layout(&app, rect(120, 40));
        assert_eq!(layout.mode, LayoutMode::Expanded);
        assert!(layout.rail_w > 0);
        let pct = layout.rail_w as f32 / 120.0;
        assert!((0.18..=0.26).contains(&pct), "rail is {}%", pct * 100.0);
        assert!(layout.chat_w >= layout.rail_w * 2);
        assert!(layout.footer_h == 1);
        assert!(layout.input_h == 3);
        assert_eq!(layout.header_h, 1);
        assert!(layout.chat_h >= 20);
    }

    #[test]
    fn test_layout_rail_never_exceeds_quarter() {
        let app = make_app();
        for w in [120u16, 140, 160, 200, 240] {
            let layout = compute_ui_layout(&app, rect(w, 40));
            assert!(
                layout.rail_w as f32 <= w as f32 * 0.25 + 0.5,
                "rail too wide at {}: {}",
                w,
                layout.rail_w
            );
            assert!(
                layout.rail_w as f32 >= w as f32 * 0.18,
                "rail too narrow at {}: {}",
                w,
                layout.rail_w
            );
        }
    }

    #[test]
    fn test_layout_collapsed_rail_full_chat() {
        let mut app = make_app();
        app.rail_visible = false;
        let layout = compute_ui_layout(&app, rect(120, 40));
        assert_eq!(layout.mode, LayoutMode::Collapsed);
        assert_eq!(layout.rail_w, 0);
        assert_eq!(layout.chat_w, 120);
    }

    #[test]
    fn test_layout_compact_narrow_terminal() {
        let app = make_app();
        // Narrow: rail forced off, chrome reduced.
        let layout = compute_ui_layout(&app, rect(80, 30));
        assert_eq!(layout.mode, LayoutMode::Compact);
        assert_eq!(layout.rail_w, 0);
        assert_eq!(layout.chat_w, 80);
        assert_eq!(layout.footer_h, 0);
        assert_eq!(layout.input_h, 2);
    }

    #[test]
    fn test_layout_compact_short_terminal() {
        let app = make_app();
        let layout = compute_ui_layout(&app, rect(140, 18));
        assert_eq!(layout.mode, LayoutMode::Compact);
        assert_eq!(layout.rail_w, 0);
        assert!(layout.chat_h >= 12, "chat too small: {}", layout.chat_h);
    }

    #[test]
    fn test_layout_priority_input_and_chat_preserved() {
        let app = make_app();
        let layout = compute_ui_layout(&app, rect(120, 22));
        assert!(layout.input_h >= 2);
        assert!(layout.chat_h >= 10);
    }

    #[test]
    fn test_rail_width_bounds() {
        assert!(rail_width(120) >= RAIL_MIN_WIDTH);
        assert!(rail_width(120) <= (120.0 * 0.26) as u16);
        let w = rail_width(200);
        let pct = w as f32 / 200.0;
        assert!((0.18..=0.26).contains(&pct));
    }

    #[test]
    fn test_rail_section_heights_fit() {
        let heights = rail_section_heights(20);
        assert_eq!(heights.iter().sum::<u16>(), 20);
        // Agents are the last to be cut.
        assert!(heights[0] >= 3);
        // More height than requested: sections stay at their defaults.
        let heights = rail_section_heights(30);
        assert_eq!(heights, [5, 5, 4, 5, 4]);
        assert!(heights.iter().sum::<u16>() <= 30);
        // Extreme squeeze still keeps at least one row for the rail.
        let heights = rail_section_heights(3);
        assert!(heights.iter().sum::<u16>() <= 3);
    }

    #[test]
    fn test_layout_area_math() {
        let app = make_app();
        let layout = compute_ui_layout(&app, rect(120, 40));
        let size = rect(120, 40);
        let header = layout.header_area(size);
        let chat = layout.chat_area(size);
        let input = layout.input_area(size);
        let footer = layout.footer_area(size);
        assert_eq!(header.height, 1);
        assert_eq!(footer.y + footer.height, 40);
        assert_eq!(input.y + input.height, footer.y);
        assert_eq!(chat.y + chat.height, input.y);
        assert!(chat.x + chat.width <= 120);
    }

    // ─── Action rendering helpers ───────────────────────────────────────

    #[test]
    fn test_format_duration() {
        assert!(format_duration(500).contains("ms"));
        assert!(format_duration(3200).contains("s"));
        assert!(format_duration(120_000).contains("m"));
    }

    #[test]
    fn test_spinner_is_stable() {
        let a = spinner_char_now();
        let b = spinner_char_now();
        assert!(SPINNER_FRAMES.contains(&a));
        assert!(SPINNER_FRAMES.contains(&b));
    }

    #[test]
    fn test_dangerous_shell_detection() {
        assert!(is_dangerous_shell("rm -rf /tmp/foo"));
        assert!(is_dangerous_shell("git push --force origin main"));
        assert!(!is_dangerous_shell("git status"));
        assert!(!is_dangerous_shell("cargo test"));
        assert!(!is_dangerous_shell("rm file.txt"));
    }

    #[test]
    fn test_group_lines_collapsed_shows_summary() {
        let mut app = make_app();
        app.action_stream.handle_event(&AgentEvent::ToolStarted {
            tool: "read_file".to_string(),
            args: "src/main.rs".to_string(),
        });
        app.action_stream.handle_event(&AgentEvent::ToolCompleted {
            tool: "read_file".to_string(),
            result: "ok".to_string(),
            success: true,
        });
        if let Some(group) = app.action_stream.groups.back_mut() {
            group.status = UiActionStatus::Completed;
            group.expanded = false;
        }
        let lines = action_group_lines(&app, 80);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect::<Vec<_>>()
            .join("|");
        assert!(joined.contains("└─"), "collapsed summary: {}", joined);
        assert!(joined.contains("read"), "keeps real counts");
    }

    #[test]
    fn test_group_lines_running_expanded() {
        let mut app = make_app();
        app.action_stream.handle_event(&AgentEvent::ToolStarted {
            tool: "run_command".to_string(),
            args: "cargo test".to_string(),
        });
        let lines = action_group_lines(&app, 80);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect::<Vec<_>>()
            .join("|");
        assert!(joined.contains("⚙"), "command emoji present");
        assert!(joined.contains("cargo test"));
    }

    // ─── Render smoke tests (no panic across sizes and states) ─────────

    fn draw(app: &TuiApp, width: u16, height: u16) {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|f| ui(f, app))
            .expect("render must not panic");
    }

    fn app_with_task() -> TuiApp {
        let mut app = make_app();
        app.add_message(
            MessageRole::User,
            "Fix the parser bug and add regression tests.".to_string(),
        );
        app.action_stream.handle_event(&AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: "Fix parser".to_string(),
        });
        app.action_stream
            .handle_event(&AgentEvent::AgentStatusChanged {
                agent: "main".to_string(),
                status: AgentStatus::Searching,
            });
        app.action_stream.handle_event(&AgentEvent::ToolStarted {
            tool: "read_file".to_string(),
            args: "src/agent/tool_parser.rs".to_string(),
        });
        app.action_stream.handle_event(&AgentEvent::ToolCompleted {
            tool: "read_file".to_string(),
            result: "fn parse() {}".to_string(),
            success: true,
        });
        app.action_stream.handle_event(&AgentEvent::ToolStarted {
            tool: "run_command".to_string(),
            args: "cargo test".to_string(),
        });
        app.action_stream.handle_event(&AgentEvent::PtyOutput {
            console: "c1".to_string(),
            content: "running 2 tests\n".to_string(),
        });
        app.is_loading = true;
        app
    }

    #[test]
    fn test_render_idle_expanded() {
        let app = make_app();
        draw(&app, 120, 40);
        draw(&app, 140, 50);
    }

    #[test]
    fn test_render_idle_compact_narrow() {
        let app = make_app();
        draw(&app, 80, 24);
        draw(&app, 60, 16);
    }

    #[test]
    fn test_render_active_task_all_sizes() {
        let app = app_with_task();
        for (w, h) in [
            (160u16, 48u16),
            (140, 40),
            (120, 32),
            (120, 40),
            (100, 26),
            (80, 24),
            (60, 16),
            (140, 45),
            (110, 20),
        ] {
            draw(&app, w, h);
        }
    }

    #[test]
    fn test_render_active_task_rail_collapsed() {
        let mut app = app_with_task();
        app.rail_visible = false;
        draw(&app, 120, 40);
    }

    #[test]
    fn test_render_with_console_content() {
        let mut app = app_with_task();
        app.route_pty_output("c1", "compiling...\n");
        app.route_pty_exit("c1", 0, "completed");
        draw(&app, 120, 40);
    }

    #[test]
    fn test_render_console_popup() {
        let mut app = app_with_task();
        app.route_pty_output("c1", "line one\nline two\n");
        app.show_console = true;
        draw(&app, 120, 40);
        draw(&app, 80, 24);
    }

    #[test]
    fn test_render_palette_and_picker() {
        let mut app = make_app();
        app.dashboard.show_command_palette = true;
        draw(&app, 120, 40);
        app.dashboard.show_command_palette = false;
        app.dashboard.model_picker.open();
        app.dashboard.model_picker.set_models(vec![
            "agnes-2.5-flash".to_string(),
            "gpt-4o".to_string(),
            "qwen2.5-coder".to_string(),
        ]);
        draw(&app, 120, 40);
    }

    #[test]
    fn test_render_overlay_panels() {
        let mut app = app_with_task();
        app.dashboard.show_agents = true;
        draw(&app, 120, 40);
        app.dashboard.show_agents = false;
        app.dashboard.show_task_graph = true;
        draw(&app, 120, 40);
        app.dashboard.show_task_graph = false;
        app.dashboard.show_metrics = true;
        draw(&app, 120, 40);
        app.dashboard.show_metrics = false;
        app.dashboard.show_coordination = true;
        draw(&app, 120, 40);
        app.dashboard.show_coordination = false;
        app.dashboard.show_memory = true;
        draw(&app, 120, 40);
        app.dashboard.show_memory = false;
        app.dashboard.show_trace = true;
        draw(&app, 120, 40);
    }

    #[test]
    fn test_render_confirmation_and_secure_input() {
        let mut app = make_app();
        app.pending_confirmation = Some((
            "Run dangerous command? (y/n)".to_string(),
            PendingAction::RunShell("rm -rf /tmp/x".to_string()),
        ));
        draw(&app, 120, 40);
        app.pending_confirmation = None;
        app.secure_input = Some(crate::tui::app::SecureInputState {
            provider: "openai".to_string(),
            buffer: "sk-secret".to_string(),
        });
        draw(&app, 120, 40);
    }

    #[test]
    fn test_render_scrolled_with_indicator() {
        let mut app = make_app();
        for i in 0..60 {
            app.add_message(
                MessageRole::Assistant,
                format!("Response line {} with some markdown **bold** text here.", i),
            );
        }
        app.scroll_up();
        draw(&app, 120, 40);
        draw(&app, 80, 24);
    }

    #[test]
    fn test_render_terminal_smoke_all_panels_at_once() {
        let mut app = app_with_task();
        app.dashboard.show_agents = true;
        app.dashboard.show_task_graph = true;
        app.dashboard.show_metrics = true;
        app.dashboard.show_coordination = true;
        app.show_console = true;
        app.route_pty_output("c1", "output\n");
        app.add_message(
            MessageRole::Assistant,
            "The parser now handles nested structured arguments.".to_string(),
        );
        draw(&app, 160, 50);
        draw(&app, 100, 26);
    }

    fn buffer_text(app: &TuiApp, width: u16, height: u16) -> String {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|f| ui(f, app))
            .expect("render must not panic");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn test_render_expanded_layout_shows_header_chat_rail_footer() {
        let app = app_with_task();
        let text = buffer_text(&app, 120, 40);
        assert!(text.contains("CodeBro"), "header identity");
        assert!(text.contains("You"), "user message");
        assert!(text.contains("AGENTS"), "rail: agents section");
        assert!(text.contains("CURRENT ACTIVITY"), "rail: activity section");
        assert!(text.contains("SESSION"), "rail: session section");
        assert!(text.contains("Send"), "input send hint");
        assert!(text.contains("Ctrl+P"), "footer hints");
        assert!(text.contains("Ctrl+O"), "footer rail hint");
        assert!(
            text.contains("Ready") || text.contains("Connected"),
            "footer status"
        );
    }

    #[test]
    fn test_render_collapsed_hides_rail() {
        let mut app = app_with_task();
        app.rail_visible = false;
        let text = buffer_text(&app, 120, 40);
        assert!(!text.contains("AGENTS"), "rail hidden when collapsed");
        assert!(text.contains("CodeBro"));
        assert!(text.contains("Send"));
    }

    #[test]
    fn test_render_compact_hides_footer_and_rail() {
        let app = app_with_task();
        let text = buffer_text(&app, 80, 24);
        assert!(!text.contains("AGENTS"), "rail off in compact");
        assert!(text.contains("CodeBro"), "header kept");
        assert!(
            text.contains("Ask CodeBro") || text.contains("Send"),
            "input kept"
        );
    }

    #[test]
    fn test_render_action_timeline_visible_in_chat() {
        let app = app_with_task();
        let text = buffer_text(&app, 120, 40);
        assert!(text.contains("Research"), "phase label in chat");
        assert!(text.contains("Read File"), "reading action line");
        assert!(text.contains("src/agent/tool_parser.rs"), "read path");
        assert!(text.contains("cargo test"), "command action line");
    }

    #[test]
    fn test_render_new_activity_indicator_when_scrolled() {
        let mut app = make_app();
        for i in 0..80 {
            app.add_message(
                MessageRole::Assistant,
                format!("Long response number {} with plenty of text to scroll.", i),
            );
        }
        app.scroll_up();
        let text = buffer_text(&app, 120, 40);
        assert!(
            text.contains("New activity"),
            "scroll indicator shown when not at bottom"
        );
    }

    // ─── F3: chronological order user → activity → response ───────────

    #[test]
    fn test_render_chat_order_is_user_timeline_response() {
        let mut app = make_app();
        app.dashboard.show_welcome = false;
        app.add_message(MessageRole::User, "Order check task".to_string());
        app.action_stream.handle_event(&AgentEvent::ToolStarted {
            tool: "read_file".to_string(),
            args: "src/order.rs".to_string(),
        });
        app.action_stream.handle_event(&AgentEvent::ToolCompleted {
            tool: "read_file".to_string(),
            result: "ok".to_string(),
            success: true,
        });
        app.add_message(MessageRole::Assistant, "FINAL-RESPONSE-MARKER".to_string());

        let text = buffer_text(&app, 120, 40);
        let user_at = text.find("Order check task").expect("user message");
        let activity_at = text.find("Read File").expect("action timeline");
        let response_at = text.find("FINAL-RESPONSE-MARKER").expect("final response");
        assert!(
            user_at < activity_at && activity_at < response_at,
            "expected user < activity < response, got user={} activity={} response={}",
            user_at,
            activity_at,
            response_at
        );
    }

    #[test]
    fn test_render_chat_timeline_after_user_without_response() {
        // No assistant response yet: the timeline still follows the user
        // message and no ordering regression occurs.
        let mut app = make_app();
        app.dashboard.show_welcome = false;
        app.add_message(MessageRole::User, "No response yet".to_string());
        app.action_stream.handle_event(&AgentEvent::ToolStarted {
            tool: "run_command".to_string(),
            args: "cargo test".to_string(),
        });
        let text = buffer_text(&app, 120, 40);
        let user_at = text.find("No response yet").expect("user message");
        let activity_at = text.find("cargo test").expect("action timeline");
        assert!(user_at < activity_at);
    }

    // ─── F1: renderer shows no permanent spinner after finalization ───

    #[test]
    fn test_render_completed_group_has_no_spinner() {
        let mut app = make_app();
        app.dashboard.show_welcome = false;
        app.add_message(MessageRole::User, "Spinner check".to_string());
        app.action_stream.handle_event(&AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: "spinner".to_string(),
        });
        app.action_stream
            .handle_event(&AgentEvent::AgentStatusChanged {
                agent: "main".to_string(),
                status: AgentStatus::Thinking,
            });
        app.action_stream.handle_event(&AgentEvent::ToolStarted {
            tool: "read_file".to_string(),
            args: "a.rs".to_string(),
        });
        app.action_stream.finalize_response(true);
        let lines = action_group_lines(&app, 80);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect::<Vec<_>>()
            .join("|");
        assert!(
            !joined.contains("Thinking"),
            "finalized group must not render a stale action: {}",
            joined
        );
        assert!(joined.contains("✓ Main"), "group completed: {}", joined);
    }

    // ─── Sprint 30UI.2 F1: header claims the REAL task mode ───────────

    #[test]
    fn test_header_assist_mode() {
        assert_eq!(header_mode_label(TaskMode::Assist, true), "ASSIST");
    }

    #[test]
    fn test_header_validate_mode() {
        assert_eq!(header_mode_label(TaskMode::Validate, true), "VALIDATE");
    }

    #[test]
    fn test_header_plan_mode() {
        assert_eq!(header_mode_label(TaskMode::Plan, true), "PLAN");
    }

    #[test]
    fn test_header_autonomous_mode() {
        assert_eq!(header_mode_label(TaskMode::Autonomous, true), "AUTONOMOUS");
    }

    #[test]
    fn test_header_idle_state() {
        // Idle renders READY for every mode — never a mode claim.
        assert_eq!(header_mode_label(TaskMode::Assist, false), "READY");
        assert_eq!(header_mode_label(TaskMode::Autonomous, false), "READY");
    }

    #[test]
    fn test_task_mode_matches_header() {
        let mut app = make_app();
        // Production submission runs Assist: a running task claims ASSIST,
        // never a mode inferred from "a task is running" or from events.
        app.task_mode = TaskMode::Assist;
        app.action_stream.handle_event(&AgentEvent::ToolStarted {
            tool: "run_command".to_string(),
            args: "cargo test".to_string(),
        });
        assert!(app.action_stream.has_running(), "task is running");
        let working = app.has_active_task() || app.action_stream.has_running();
        assert_eq!(header_mode_label(app.task_mode, working), "ASSIST");
        // Changing the mode changes the claim with it.
        app.task_mode = TaskMode::Autonomous;
        assert_eq!(
            header_mode_label(app.task_mode, working),
            "AUTONOMOUS",
            "header follows the explicit task mode"
        );
        // Idle: no mode claim at all.
        let idle = make_app();
        assert_eq!(
            header_mode_label(idle.task_mode, false),
            "READY",
            "idle app never claims a running mode"
        );
    }

    // ─── Sprint 30UI.2 F3: rail progress uses enabled specialist phases

    fn set_agent_completed(app: &mut TuiApp, name: &str) {
        app.dashboard.status_monitor.register_agent(name);
        app.dashboard
            .status_monitor
            .update_status(name, AgentStatus::Completed);
    }

    #[test]
    fn test_progress_assist_mode() {
        let app = make_app();
        assert_eq!(app.task_mode, TaskMode::Assist);
        assert_eq!(rail_progress_counts(&app), (0, 1));
        let mut app = make_app();
        set_agent_completed(&mut app, "research");
        assert_eq!(rail_progress_counts(&app), (1, 1));
    }

    #[test]
    fn test_progress_validate_mode() {
        let mut app = make_app();
        app.task_mode = TaskMode::Validate;
        assert_eq!(rail_progress_counts(&app), (0, 2));
        set_agent_completed(&mut app, "research");
        assert_eq!(rail_progress_counts(&app), (1, 2));
        set_agent_completed(&mut app, "testing");
        assert_eq!(rail_progress_counts(&app), (2, 2));
    }

    #[test]
    fn test_progress_plan_mode() {
        let mut app = make_app();
        app.task_mode = TaskMode::Plan;
        assert_eq!(rail_progress_counts(&app), (0, 3));
        set_agent_completed(&mut app, "research");
        set_agent_completed(&mut app, "testing");
        assert_eq!(rail_progress_counts(&app), (2, 3));
        set_agent_completed(&mut app, "planning");
        assert_eq!(rail_progress_counts(&app), (3, 3));
    }

    #[test]
    fn test_progress_autonomous_mode() {
        let mut app = make_app();
        app.task_mode = TaskMode::Autonomous;
        assert_eq!(rail_progress_counts(&app), (0, 5));
        for name in ["research", "testing", "planning", "coding", "review"] {
            set_agent_completed(&mut app, name);
        }
        assert_eq!(rail_progress_counts(&app), (5, 5));
    }

    #[test]
    fn test_progress_partial_autonomous() {
        let mut app = make_app();
        app.task_mode = TaskMode::Autonomous;
        set_agent_completed(&mut app, "research");
        set_agent_completed(&mut app, "testing");
        app.dashboard.status_monitor.register_agent("planning");
        app.dashboard
            .status_monitor
            .update_status("planning", AgentStatus::Planning);
        // Coding + Review pending must not render as unfinished phases that
        // were never started — they simply stay in the 2/5 denominator.
        assert_eq!(
            rail_progress_counts(&app),
            (2, 5),
            "research+testing done, planning running, coding/review pending"
        );
    }

    #[test]
    fn test_task_mode_matches_progress() {
        for mode in [
            TaskMode::Assist,
            TaskMode::Validate,
            TaskMode::Plan,
            TaskMode::Autonomous,
        ] {
            let mut app = make_app();
            app.task_mode = mode;
            let (_, total) = rail_progress_counts(&app);
            assert_eq!(
                total,
                mode.enabled_phase_names().len(),
                "progress denominator comes from the mode's canonical phases"
            );
        }
    }

    // ─── Sprint 30UI.2 F4: no fabricated session start time ───────────

    #[test]
    fn test_session_panel_omits_started_without_authoritative_timestamp() {
        let app = make_app();
        let rows = session_panel_rows(&app);
        assert!(
            rows.iter().all(|(label, _)| label != "Started"),
            "no reconstructed start time without an authoritative wall-clock timestamp"
        );
        assert!(rows.iter().any(|(label, _)| label == "Duration"));
        assert!(rows.iter().any(|(label, _)| label == "Tools"));
        // Render smoke: the session panel still draws its real rows.
        let text = buffer_text(&app, 120, 40);
        assert!(text.contains("SESSION"));
        assert!(text.contains("Duration"));
        assert!(text.contains("Tools"));
    }
}
