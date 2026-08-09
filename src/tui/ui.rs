#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
use ratatui::Terminal;
use std::io::stdout;
use std::sync::Arc;
use std::time::Duration;

use crate::agent::events::AgentEvent;
use crate::agent::status::AgentStatus;
use crate::agent::task_graph::TaskStatus;
use crate::config::Config;
use crate::tui::animation::progress_bar;
use crate::tui::app::{MessageRole, PendingAction, TuiApp};
use crate::tui::commands::{self, CommandNamespace};
use crate::tui::console::PtyConsole;
use crate::tui::dashboard::Dashboard;
use crate::tui::events::{self, Shortcut};

const FRAME_INTERVAL: Duration = Duration::from_millis(50);

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
            app.insert_text(&text);
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
                    // Context-aware completion for `/`, `//`, `!`.
                    if app.input.starts_with('/') || app.input.starts_with('!') {
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
                KeyCode::End => app.cursor_end(),
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
                KeyCode::Esc => {
                    if !app.dashboard.autocomplete.is_empty() {
                        app.dashboard.autocomplete.clear();
                        app.dashboard.autocomplete_index = 0;
                    } else if app.dashboard.show_command_palette {
                        app.dashboard.toggle_command_palette();
                    } else if app.dashboard.model_picker.is_open() {
                        app.dashboard.model_picker.close();
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
            app.add_message(MessageRole::User, input.clone());
            app.is_loading = true;
            app.begin_task(input.clone());
            app.dashboard
                .animation
                .start_activity(crate::tui::animation::ActivityType::Thinking);
            let token = app.begin_cancellable_task();
            let config = app.config.clone();
            let tx = app.tx.clone();
            let conversation = conversation_from(app);
            tokio::spawn(async move {
                run_chat_pipeline(&config, &input, conversation, &tx, token).await;
            });
        }
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
    if let Some((_, action)) = app.pending_confirmation.take() {
        if confirmed {
            confirm_action(action, app);
        } else {
            app.add_message(MessageRole::System, "Action cancelled".to_string());
        }
    } else if cancelled {
        // Nothing pending; Esc is a no-op here.
    }
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
        "  Shortcuts: Ctrl+P commands · Ctrl+C cancel · Ctrl+L clear · Esc dismiss".to_string(),
    );
    lines.join("\n")
}

/// Submit a natural-language engineering task through the canonical runtime.
fn submit_task(app: &mut TuiApp, task: String) {
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
    tokio::spawn(async move {
        run_chat_pipeline(&config, &task, conversation, &tx, token).await;
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
    let options = crate::canonical_runtime::TaskOptions {
        cancel: Some(token),
        on_pty: Some(on_pty),
    };

    let result = runtime.run_task_with_options(&request, options).await;

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

// ─── Layout ───────────────────────────────────────────────────────────────

struct PanelLayout {
    title_h: u16,
    op_h: u16,
    output_h: u16,
    activity_h: u16,
    agents_h: u16,
    graph_h: u16,
    metrics_h: u16,
    coord_h: u16,
    input_h: u16,
}

const MIN_OUTPUT: u16 = 4;

/// The default view is the task output: title, current operation, output,
/// a concise live-activity line, and input. Everything else (agents, task
/// graph, metrics, coordination) is an overlay opened on demand.
fn compute_layout(app: &TuiApp, total_h: u16) -> PanelLayout {
    let title_h: u16 = 1;
    let op_h: u16 = 1;
    let input_h: u16 = 3;

    let activity_h: u16 = if app.dashboard.compact {
        1
    } else if app.dashboard.verbose {
        8
    } else {
        4
    };

    let agents_h = if app.dashboard.show_agents { 5 } else { 0 };
    let graph_h = if app.dashboard.show_task_graph { 5 } else { 0 };
    let metrics_h = if app.dashboard.show_metrics { 6 } else { 0 };
    let coord_h = if app.dashboard.show_coordination {
        6
    } else {
        0
    };

    let fixed: u16 = title_h + op_h + input_h;
    let optional: [u16; 5] = [agents_h, graph_h, metrics_h, coord_h, activity_h];

    let mut output_h = total_h.saturating_sub(fixed + optional.iter().sum::<u16>());
    // Shrink optional panels (largest first) until the output has room.
    let mut optional = optional;
    while output_h < MIN_OUTPUT {
        let mut max_i: Option<usize> = None;
        let mut max_v: u16 = 0;
        for (i, &h) in optional.iter().enumerate() {
            if h > 0 && h > max_v {
                max_v = h;
                max_i = Some(i);
            }
        }
        match max_i {
            Some(i) => optional[i] = optional[i].saturating_sub(1),
            None => break,
        }
        output_h = total_h.saturating_sub(fixed + optional.iter().sum::<u16>());
    }

    PanelLayout {
        title_h,
        op_h,
        output_h,
        activity_h: optional[4],
        agents_h: optional[0],
        graph_h: optional[1],
        metrics_h: optional[2],
        coord_h: optional[3],
        input_h,
    }
}

fn split_panels(area: Rect, layout: &PanelLayout) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(layout.title_h),
            Constraint::Length(layout.op_h),
            Constraint::Min(MIN_OUTPUT),
            Constraint::Length(layout.activity_h),
            Constraint::Length(layout.agents_h),
            Constraint::Length(layout.graph_h),
            Constraint::Length(layout.metrics_h),
            Constraint::Length(layout.coord_h),
            Constraint::Length(layout.input_h),
        ])
        .split(area)
}

fn ui(f: &mut Frame, app: &TuiApp) {
    let size = f.size();
    let layout = compute_layout(app, size.height);
    let chunks = split_panels(size, &layout);

    render_title(f, app, chunks[0]);
    render_operation(f, app, chunks[1]);
    render_output(f, app, chunks[2]);

    if layout.activity_h > 0 {
        render_activity(f, &app.dashboard, chunks[3], layout.activity_h as usize);
    }
    if layout.agents_h > 0 {
        render_agents(f, app, chunks[4]);
    }
    if layout.graph_h > 0 {
        render_task_graph(f, app, chunks[5]);
    }
    if layout.metrics_h > 0 {
        render_metrics(f, app, chunks[6]);
    }
    if layout.coord_h > 0 {
        render_coordination(f, app, chunks[7]);
    }

    render_input(f, app, chunks[8]);

    // Command palette overlay.
    if app.dashboard.show_command_palette {
        render_command_palette(f, app, chunks[8]);
    }

    // Autocomplete overlay.
    if !app.dashboard.autocomplete.is_empty() && !app.dashboard.model_picker.is_open() {
        render_autocomplete(f, app, chunks[8]);
    }

    if app.dashboard.model_picker.is_open() {
        render_model_picker(f, app);
    }

    if let Some((message, _)) = &app.pending_confirmation {
        render_confirmation(f, message, chunks[8]);
    }
}

fn render_title(f: &mut Frame, app: &TuiApp, area: Rect) {
    let root = crate::tools::detect_workspace_root();
    let workspace = root.file_name().and_then(|n| n.to_str()).unwrap_or(".");
    let model = if app.config.model.trim().is_empty() {
        "auto".to_string()
    } else {
        app.config.model.clone()
    };

    let mut spans = vec![
        Span::styled(
            "codebro",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::default().fg(Color::DarkGray)),
        Span::styled(workspace, Style::default().fg(Color::Green)),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled(model, Style::default().fg(Color::DarkGray)),
    ];

    // Restrained status indicators: a single spinner while working.
    if app.dashboard.animation.is_active() {
        spans.push(Span::styled(
            format!(" {}", app.dashboard.animation.spinner_char()),
            Style::default().fg(Color::Cyan),
        ));
    }

    let title = Paragraph::new(Line::from(spans));
    f.render_widget(title, area);
}

fn render_operation(f: &mut Frame, app: &TuiApp, area: Rect) {
    let op = app
        .dashboard
        .current_operation
        .as_deref()
        .unwrap_or("ready");
    let line = Paragraph::new(Line::from(Span::styled(
        truncate_to(op, area.width.saturating_sub(2) as usize),
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(line, area);
}

fn render_output(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.height < 2 {
        return;
    }
    let inner_w = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    if app.dashboard.show_welcome && app.messages.is_empty() && !app.has_console_content() {
        // A single-line hint, not a full-screen welcome.
        lines.push(Line::from(Span::styled(
            "  Type a task, or / for commands · // for runtime · ! for shell · Ctrl+P palette",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        if let Some(ref err) = app.dashboard.last_error {
            lines.push(Line::from(Span::styled(
                format!("  ! {}", truncate_to(err, inner_w)),
                Style::default().fg(Color::Red),
            )));
            lines.push(Line::from(""));
        }

        if app.dashboard.is_streaming && !app.dashboard.streaming_buffer.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("{} ", app.dashboard.animation.spinner_char()),
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            )));
            for md_line in
                crate::tui::markdown::render_markdown(&app.dashboard.streaming_buffer, inner_w)
            {
                lines.push(md_line);
            }
            lines.push(Line::from(""));
        }

        for msg in app.messages.iter() {
            let (label, color): (&str, Color) = match msg.role {
                MessageRole::User => ("you", Color::Green),
                MessageRole::Assistant => ("codebro", Color::Blue),
                MessageRole::System => ("•", Color::Yellow),
            };
            lines.push(Line::from(Span::styled(
                format!("{}", label),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )));

            if msg.role == MessageRole::Assistant {
                for md_line in crate::tui::markdown::render_markdown(&msg.content, inner_w) {
                    lines.push(md_line);
                }
            } else {
                for (i, content_line) in msg.content.lines().enumerate() {
                    let style = match msg.role {
                        MessageRole::User => Style::default().fg(Color::Green),
                        MessageRole::System => Style::default().fg(Color::Yellow),
                        _ => Style::default(),
                    };
                    lines.push(Line::from(Span::styled(
                        if i == 0 {
                            content_line.to_string()
                        } else {
                            format!("  {}", content_line)
                        },
                        style,
                    )));
                }
            }
            lines.push(Line::from(""));
        }

        // The live task console: appended, never replaced.
        if let Some(console) = app.active_console_ref() {
            if !console.is_empty() {
                lines.push(Line::from(""));
                lines.extend(console.render_lines(inner_w));
            }
        }
    }

    let total_lines = lines.len() as u16;
    let view_h = area.height;
    let max_scroll = total_lines.saturating_sub(view_h);
    let scroll = max_scroll.saturating_sub(app.scroll_from_bottom as u16);

    let output = Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: true })
        .scroll((scroll, 0));
    f.render_widget(output, area);
}

fn render_activity(f: &mut Frame, dashboard: &Dashboard, area: Rect, max_lines: usize) {
    if area.height < 1 {
        return;
    }
    let mut lines = Vec::new();
    let entries: Vec<_> = dashboard.activity_log.iter().take(max_lines).collect();
    for entry in entries {
        let color = match entry.level.as_str() {
            "error" => Color::Red,
            "tool" => Color::Cyan,
            "task" => Color::Green,
            "console" => Color::Blue,
            _ => Color::DarkGray,
        };
        let msg = truncate_to(&entry.message, area.width.saturating_sub(2) as usize);
        lines.push(Line::from(Span::styled(
            format!("  {}", msg),
            Style::default().fg(color),
        )));
    }
    let paragraph = Paragraph::new(Text::from(lines));
    f.render_widget(paragraph, area);
}

fn render_agents(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.height < 1 {
        return;
    }
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "  agents  (Ctrl+A to close)",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    let entries = app.dashboard.agent_entries();
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no active agents",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for entry in entries {
            let (icon, color) = match entry.status {
                AgentStatus::Completed => ("✓", Color::Green),
                AgentStatus::Failed => ("✗", Color::Red),
                AgentStatus::Idle => ("○", Color::DarkGray),
                _ => ("⟳", Color::Yellow),
            };
            let bar = progress_bar(entry.progress, 8);
            let name = truncate_to(&entry.name, 12);
            let action = entry.action.as_deref().unwrap_or("");
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                Span::styled(name, Style::default().fg(Color::White)),
                Span::styled(
                    format!(" [{}] {}", bar, truncate_to(action, 30)),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }
    f.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_task_graph(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.height < 1 {
        return;
    }
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "  task graph  (Ctrl+G to close)",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    let entries = app.dashboard.graph_entries();
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no task running",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (desc, agent, status) in entries.iter().take(4) {
            let icon = match status {
                TaskStatus::Completed => "✓",
                TaskStatus::Failed => "✗",
                TaskStatus::Running => "⟳",
                _ => "○",
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "  {} {}: {}",
                    icon,
                    truncate_to(agent, 10),
                    truncate_to(desc, 50)
                ),
                Style::default().fg(Color::White),
            )));
        }
    }
    f.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_metrics(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.height < 1 {
        return;
    }
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "  metrics  (Ctrl+V to close)",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));

    let agent_count = app.dashboard.status_monitor.count();
    let active_count = app.dashboard.status_monitor.active_count();
    lines.push(Line::from(Span::styled(
        format!("  agents {} ({} active)", agent_count, active_count),
        Style::default().fg(Color::DarkGray),
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
        Style::default().fg(Color::DarkGray),
    )));

    f.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_coordination(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.height < 1 {
        return;
    }
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "  coordination  (Ctrl+O to close)",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    for msg in app.dashboard.recent_messages.iter().take(3) {
        lines.push(Line::from(Span::styled(
            format!(
                "  {}",
                truncate_to(msg, area.width.saturating_sub(4) as usize)
            ),
            Style::default().fg(Color::DarkGray),
        )));
    }
    f.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_input(f: &mut Frame, app: &TuiApp, area: Rect) {
    let prefix = if let Some(s) = &app.secure_input {
        format!("API key for {}: ", s.provider)
    } else if app.dashboard.animation.is_active() {
        format!("{}> ", app.dashboard.animation.spinner_char())
    } else {
        "> ".to_string()
    };

    let inner_h = area.height.saturating_sub(2) as usize;
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

    let paragraph = Paragraph::new(Text::from(text_lines)).style(Style::default().fg(Color::White));
    f.render_widget(paragraph, area);

    if area.width >= 4 && area.height >= 2 {
        let x_off = if cursor_line_rel == 0 {
            prefix.chars().count() + cursor_col
        } else {
            cursor_col
        };
        let x = (area.x + x_off as u16).min(area.x + area.width.saturating_sub(2));
        let y = (area.y + cursor_line_rel as u16).min(area.y + area.height.saturating_sub(2));
        f.set_cursor(x, y);
    }
}

fn render_confirmation(f: &mut Frame, message: &str, input_area: Rect) {
    let width = input_area.width.min(80);
    let height = 3u16;
    let x = input_area.x;
    let y = input_area.y.saturating_sub(height);
    let popup = Rect::new(x, y, width, height);
    let lines = vec![
        Line::from(Span::styled("  ", Style::default().fg(Color::Yellow))),
        Line::from(Span::styled(
            format!("  {}", truncate_to(message, width as usize)),
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            "  y/Enter = proceed · n/Esc = cancel",
            Style::default().fg(Color::DarkGray),
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
        lines.push(Line::from(vec![
            Span::styled(
                if selected { "> " } else { "  " },
                if selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                },
            ),
            Span::styled(
                entry.clone(),
                if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                },
            ),
            Span::styled(
                format!(
                    "  {}",
                    truncate_to(desc, width.saturating_sub(entry.len() as u16 + 4) as usize)
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    f.render_widget(Paragraph::new(Text::from(lines)), popup);
}

fn render_command_palette(f: &mut Frame, app: &TuiApp, input_area: Rect) {
    let query = app.dashboard.palette_query.clone();
    let entries = palette_entries(&query);
    let width = input_area.width.min(70);
    let height = (entries.len() as u16 + 2).min(input_area.y.saturating_sub(2));
    if height < 3 {
        return;
    }
    let top = input_area.y.saturating_sub(height);
    let popup = Rect::new(input_area.x, top, width, height);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            "search> ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(query.clone(), Style::default().fg(Color::White)),
        Span::styled(
            format!("  ({} matches)", entries.len()),
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    for (i, (cmd, desc)) in entries.iter().enumerate() {
        let selected = i == app.dashboard.palette_index;
        let style = if selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        lines.push(Line::from(vec![
            Span::styled(if selected { "> " } else { "  " }, style),
            Span::styled(cmd.clone(), style),
            Span::styled(
                format!("  {}", truncate_to(desc, width as usize)),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    f.render_widget(Paragraph::new(Text::from(lines)), popup);
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

    let picker = &app.dashboard.model_picker;
    let mut lines: Vec<Line> = Vec::new();

    let header = format!(
        "  Model picker - {} models (type to filter, Enter=select, Esc=cancel)",
        picker.count()
    );
    lines.push(Line::from(Span::styled(
        header,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));

    if picker.loading {
        lines.push(Line::from(Span::styled(
            "  Loading models...",
            Style::default().fg(Color::Yellow),
        )));
    } else if let Some(ref err) = picker.error {
        lines.push(Line::from(Span::styled(
            format!("  Error: {}", err),
            Style::default().fg(Color::Red),
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
                    Style::default().fg(Color::Green),
                ));
            } else {
                spans.push(Span::styled(
                    truncate_to(model, width.saturating_sub(6)),
                    Style::default().fg(if selected {
                        Color::Yellow
                    } else {
                        Color::White
                    }),
                ));
            }
            lines.push(Line::from(spans));
        }
    }

    if !picker.filter.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  filter: {}", picker.filter),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black))
        .title("Model");
    let widget = Paragraph::new(Text::from(lines))
        .block(block)
        .style(Style::default().bg(Color::Black));
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

    fn make_app() -> TuiApp {
        TuiApp::new().expect("app creation")
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

    #[test]
    fn test_layout_default_is_task_focused() {
        let app = make_app();
        let layout = compute_layout(&app, 40);
        // Default: no overlay panels open.
        assert_eq!(layout.agents_h, 0);
        assert_eq!(layout.graph_h, 0);
        assert_eq!(layout.metrics_h, 0);
        assert_eq!(layout.coord_h, 0);
        // Output dominates.
        assert!(layout.output_h >= MIN_OUTPUT);
        assert!(layout.activity_h >= 1);
        assert!(layout.input_h >= 3);
    }

    #[test]
    fn test_layout_overlays_are_optional() {
        let mut app = make_app();
        app.dashboard.show_agents = true;
        app.dashboard.show_task_graph = true;
        app.dashboard.show_metrics = true;
        let layout = compute_layout(&app, 40);
        assert!(layout.agents_h > 0);
        assert!(layout.graph_h > 0);
        assert!(layout.metrics_h > 0);
        assert!(layout.output_h >= MIN_OUTPUT);
        let total = layout.title_h
            + layout.op_h
            + layout.output_h
            + layout.activity_h
            + layout.agents_h
            + layout.graph_h
            + layout.metrics_h
            + layout.coord_h
            + layout.input_h;
        assert!(total <= 40);
    }

    #[test]
    fn test_layout_extreme_small() {
        let app = make_app();
        let layout = compute_layout(&app, 10);
        assert!(layout.title_h >= 1);
        assert!(layout.input_h >= 3);
    }

    #[test]
    fn test_dangerous_shell_detection() {
        assert!(is_dangerous_shell("rm -rf /tmp/foo"));
        assert!(is_dangerous_shell("git push --force origin main"));
        assert!(!is_dangerous_shell("git status"));
        assert!(!is_dangerous_shell("cargo test"));
        assert!(!is_dangerous_shell("rm file.txt"));
    }
}
