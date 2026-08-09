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
use std::time::Duration;

use crate::agent::events::AgentEvent;
use crate::agent::status::AgentStatus;
use crate::config::Config;
use crate::tui::animation::progress_bar;
use crate::tui::app::{MessageRole, TuiApp};
use crate::tui::dashboard::Dashboard;
use crate::tui::events::{self, Shortcut};

const FRAME_INTERVAL: Duration = Duration::from_millis(50);

/// Every slash command and its description. Source of truth for /help, the
/// command palette, and TAB autocompletion.
const SLASH_COMMANDS: &[(&str, &str, &str)] = &[
    ("/help", "Show help", "List all commands"),
    (
        "/model",
        "Pick a model",
        "Open the interactive model picker",
    ),
    ("/agents", "Show agent status", "Agent status + progress"),
    ("/tasks", "Show task graph", "Current task graph"),
    (
        "/memory",
        "Show memory changes",
        "Recent memory notifications",
    ),
    (
        "/skills",
        "Show skill changes",
        "Recent skill confidence changes",
    ),
    ("/sessions", "List sessions", "Recent session history"),
    (
        "/replay <id>",
        "Replay a session",
        "Replay a session timeline",
    ),
    ("/config", "Open config", "View/edit configuration"),
    (
        "/status",
        "Show status",
        "Pipeline, workspace and tool state",
    ),
    ("/metrics", "Task metrics", "Toggle metrics panel"),
    (
        "/apply <file>",
        "Propose a change",
        "Stage a reviewed file change (no writes)",
    ),
    (
        "/approve [verify-cmd]",
        "Apply pending change",
        "Apply + optionally verify the staged change",
    ),
    (
        "/copy",
        "Copy conversation",
        "Copy the conversation to the clipboard",
    ),
    // P5: Developer Experience
    (
        "/settings",
        "Open settings",
        "View and edit all settings interactively",
    ),
    (
        "/settings:apply",
        "Apply settings",
        "Save pending settings changes",
    ),
    (
        "/settings:discard",
        "Discard settings",
        "Revert all pending settings changes",
    ),
    (
        "/providers",
        "Show providers",
        "View and manage AI providers",
    ),
    ("/health", "Check health", "Test provider connections"),
    (
        "/discover",
        "Discover workspace",
        "Scan workspace for integrations and capabilities",
    ),
    (
        "/workspace",
        "Show workspace",
        "Display workspace detection results",
    ),
    (
        "/onboard",
        "Re-run onboarding",
        "Run the first-run setup wizard",
    ),
];

fn match_slash_command(input: &str) -> Vec<String> {
    let token = input.split_whitespace().next().unwrap_or("");
    if !token.starts_with('/') || token.len() < 2 {
        return Vec::new();
    }
    SLASH_COMMANDS
        .iter()
        .map(|c| c.0.split_whitespace().next().unwrap().to_string())
        .filter(|name| name.starts_with(token))
        .collect()
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
    // Bracketed paste lets multi-line pastes arrive as one Paste event instead
    // of being split into Enter presses (the prompt stays together).
    out.execute(EnableBracketedPaste)?;
    // Mouse capture keeps the wheel scrolling the app (conversation) instead of
    // the whole terminal.
    out.execute(EnableMouseCapture)?;

    let result = run_loop(&mut app);

    // Always restore the terminal, even if the loop errored.
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
    // Reuse the same sender the event loop reads so responses / stream chunks /
    // agent events sent by spawned tasks actually reach the UI.
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

        // Non-blocking poll with a fixed frame interval so the spinner
        // animates smoothly without spinning the CPU at 100%.
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
            true
        }
        events::AppEvent::StreamChunk(content) => {
            app.dashboard.streaming_buffer.push_str(&content);
            app.dashboard.is_streaming = true;
            true
        }
        events::AppEvent::AgentEvent(event) => {
            app.handle_agent_event(event.clone());
            // Dismiss welcome on first meaningful activity.
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
            // Model picker takes over keyboard input while open.
            if app.dashboard.model_picker.is_open() {
                handle_model_picker_key(key, app);
                return;
            }

            // Command palette takes over keyboard input while open.
            if app.dashboard.show_command_palette {
                handle_palette_key(key, app);
                return;
            }

            if let Some(shortcut) = events::check_key_shortcuts(&key) {
                handle_shortcut(shortcut, app);
                return;
            }

            match key.code {
                KeyCode::Tab => {
                    // Slash-command autocompletion while typing a command.
                    if app.input.starts_with('/') {
                        let candidates = match_slash_command(&app.input);
                        app.dashboard
                            .autocomplete_command(&mut app.input, candidates);
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
                    // Shift+Enter inserts a newline for multi-line input.
                    if key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::SHIFT)
                    {
                        app.insert_char('\n');
                        return;
                    }

                    let input = app.input.trim().to_string();
                    if !input.is_empty() {
                        app.push_history(input.clone());
                        if input.starts_with('/') {
                            handle_command(&input, app);
                            app.clear_input();
                            return;
                        }
                        app.add_message(MessageRole::User, input.clone());
                        app.clear_input();
                        app.is_loading = true;
                        app.begin_task(input.clone());
                        app.dashboard
                            .animation
                            .start_activity(crate::tui::animation::ActivityType::Thinking);
                        let config = app.config.clone();
                        let tx = app.tx.clone();
                        tokio::spawn(async move {
                            run_chat_pipeline(&config, &input, &tx).await;
                        });
                    }
                }
                KeyCode::Backspace => app.backspace(),
                KeyCode::Char(c) => app.insert_char(c),
                KeyCode::Esc => {
                    app.dashboard.toggle_command_palette();
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn handle_model_picker_key(key: crossterm::event::KeyEvent, app: &mut TuiApp) {
    use crossterm::event::KeyCode;

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
    use crossterm::event::KeyCode;
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
                .map(|c| c.0.to_string())
            {
                app.dashboard.toggle_command_palette();
                app.add_message(MessageRole::User, cmd.clone());
                handle_command(&cmd, app);
                app.is_loading = false;
                app.dashboard.show_command_palette = false;
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

fn palette_entries(filter: &str) -> Vec<(&'static str, &'static str)> {
    let f = filter.to_lowercase();
    SLASH_COMMANDS
        .iter()
        .filter(|(name, short, desc)| {
            f.is_empty()
                || name.to_lowercase().contains(&f)
                || short.to_lowercase().contains(&f)
                || desc.to_lowercase().contains(&f)
        })
        .map(|(name, _short, desc)| (*name, *desc))
        .collect()
}

fn handle_command(cmd: &str, app: &mut TuiApp) {
    let cmd_parts: Vec<&str> = cmd.split_whitespace().collect();
    let command = cmd_parts.first().copied().unwrap_or("");

    match command {
        "/help" => {
            let lines: Vec<String> = SLASH_COMMANDS
                .iter()
                .map(|(name, short, desc)| format!("{} - {} ({})", name, short, desc))
                .collect();
            app.add_message(
                MessageRole::System,
                format!("Commands:\n{}", lines.join("\n")),
            );
        }
        "/config" => {
            let cfg = &app.config;
            let api = if cfg.api_key.is_some() {
                "set (hidden)"
            } else {
                "unset"
            };
            let model = if cfg.model.trim().is_empty() {
                "(auto-detect)".to_string()
            } else {
                cfg.model.clone()
            };
            let root = crate::tools::detect_workspace_root();
            app.add_message(
                MessageRole::System,
                format!(
                    "Config:\n  provider: {}\n  base_url: {}\n  model: {}\n  api_key: {}\n  workspace: {}",
                    cfg.provider, cfg.base_url, model, api, root.display()
                ),
            );
        }
        "/status" => {
            let root = crate::tools::detect_workspace_root();
            let toolable =
                crate::tools::is_toolable(&cmd_parts.get(1).copied().unwrap_or("status"));
            app.add_message(
                MessageRole::System,
                format!(
                    "Status:\n  workspace: {}\n  tool pipeline: {}\n  streaming: {}",
                    root.display(),
                    if toolable { "enabled" } else { "idle" },
                    app.dashboard.is_streaming
                ),
            );
        }
        "/sessions" => {
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
        "/replay" => {
            let id = cmd_parts.get(1).copied().unwrap_or("");
            if let Some(tracker) = app.session_tracker.as_ref() {
                if let Ok(session) = tracker.store().load_session(id) {
                    let timeline = session.replay_timeline();
                    app.add_message(
                        MessageRole::System,
                        format!(
                            "Session {} - {}\n{}",
                            session.id,
                            session.task,
                            timeline.join("\n")
                        ),
                    );
                } else {
                    app.add_message(MessageRole::System, format!("Session not found: {}", id));
                }
            }
        }
        "/agents" => {
            let entries = app.dashboard.agent_entries();
            let text: Vec<String> = entries
                .iter()
                .map(|e| format!("{} - {} ({:.0}%)", e.name, e.status, e.progress * 100.0))
                .collect();
            app.add_message(MessageRole::System, format!("Agents:\n{}", text.join("\n")));
        }
        "/tasks" => {
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
        "/memory" => {
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
        "/skills" => {
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
        "/metrics" => {
            app.dashboard.toggle_metrics();
        }
        "/apply" => {
            // Code-change workflow step 1: propose + preview. NO writes happen.
            let root = crate::tools::detect_workspace_root();
            let file = cmd_parts.get(1).cloned().unwrap_or("");
            let new_content = cmd_parts.get(2..).map(|p| p.join(" ")).unwrap_or_default();
            if file.is_empty() || new_content.trim().is_empty() {
                app.add_message(
                    MessageRole::System,
                    "Usage: /apply <file> <new content>".to_string(),
                );
                return;
            }
            let path = root.join(file);
            if !path.exists() {
                app.add_message(
                    MessageRole::System,
                    format!("Target not found: {}", root.join(file).display()),
                );
                return;
            }
            match crate::tools::ChangePlan::propose(&path, &new_content) {
                Ok(plan) => {
                    app.add_message(
                        MessageRole::System,
                        format!(
                            "Staged change for {} (not applied). Review, then run /approve [verify-cmd]:\n{}",
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
        "/approve" => {
            // Code-change workflow step 2: explicit approval -> apply -> verify.
            if app.pending_change.is_none() {
                app.add_message(
                    MessageRole::System,
                    "No pending change. Stage one with /apply <file> <new content>.".to_string(),
                );
                return;
            }
            let verify = cmd_parts.get(1).map(|c| c.to_string()).or_else(|| {
                // Default verification gate when the workspace is a git repo.
                crate::tools::detect_workspace_root()
                    .join(".git")
                    .exists()
                    .then(|| "git status --porcelain".to_string())
            });
            let mut plan = app.pending_change.take().unwrap();
            match plan.apply_and_verify(verify.as_deref()) {
                Ok(msg) => {
                    app.add_message(MessageRole::System, msg);
                    if let Err(e) = save_session(app) {
                        app.add_message(MessageRole::System, format!("Save error: {e}"));
                    }
                }
                Err(e) => {
                    app.add_message(
                        MessageRole::System,
                        format!("Approval rejected the change: {e}"),
                    );
                }
            }
        }
        "/model" => {
            app.open_model_picker();
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
        _ if cmd.starts_with("/settings")
            || cmd.starts_with("/providers")
            || cmd.starts_with("/health")
            || cmd.starts_with("/discover")
            || cmd.starts_with("/workspace")
            || cmd.starts_with("/onboard") =>
        {
            app.handle_settings_command(cmd);
        }
        _ => {
            app.add_message(MessageRole::System, format!("Unknown command: {}", command));
        }
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
        Shortcut::ClearLogs => app.dashboard.clear_logs(),
        Shortcut::CancelTask => {
            app.is_loading = false;
            app.dashboard.end_streaming();
            app.dashboard
                .log("info", "Task cancelled by user".to_string());
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

/// Wires a chat submission into the canonical runtime:
/// identity → memory → context assembly → EngineeringContext → PromptBuilder →
/// IntelligentProviderRouter → ProviderRuntime → provider, streaming to the TUI.
///
/// The TUI remains responsible for input, rendering and diagnostics visibility;
/// all execution concerns are owned by the canonical runtime.
async fn run_chat_pipeline(
    config: &Config,
    task: &str,
    tx: &std::sync::mpsc::Sender<events::AppEvent>,
) {
    let emit_tx = tx.clone();
    let emit = move |event: AgentEvent| {
        let _ = emit_tx.send(events::AppEvent::AgentEvent(event));
    };
    let chunk_tx = tx.clone();
    let on_chunk = move |chunk: &str| {
        let _ = chunk_tx.send(events::AppEvent::StreamChunk(chunk.to_string()));
    };

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
        conversation: Vec::new(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&request).await;

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

struct PanelLayout {
    title_h: u16,
    conv_h: u16,
    agents_h: u16,
    activity_h: u16,
    graph_h: u16,
    metrics_h: u16,
    coord_h: u16,
    shortcuts_h: u16,
    palette_h: u16,
    input_h: u16,
}

const MIN_CONV: u16 = 4;
const BORDER: u16 = 2;

fn compute_layout(app: &TuiApp, total_h: u16) -> PanelLayout {
    let title_h: u16 = 1;
    let shortcuts_h: u16 = 1;
    let input_h: u16 = 3;
    let fixed: u16 = title_h + shortcuts_h + input_h;

    let agent_count = app.dashboard.status_monitor.count() as u16;
    let agents_h = if app.dashboard.show_agents {
        (agent_count + BORDER).min(10)
    } else {
        0
    };
    let activity_h: u16 = 6;
    let graph_len = app.dashboard.graph_entries().len() as u16;
    let graph_h = if app.dashboard.show_task_graph {
        (graph_len + BORDER).min(10)
    } else {
        0
    };
    let metrics_h = if app.dashboard.show_metrics { 6 } else { 0 };
    let coord_h = if app.dashboard.show_coordination {
        8
    } else {
        0
    };
    let palette_h = if app.dashboard.show_command_palette {
        12
    } else {
        0
    };

    let mut optional: [u16; 6] = [agents_h, activity_h, graph_h, metrics_h, coord_h, palette_h];
    let mut conv_h = total_h.saturating_sub(fixed + optional.iter().sum::<u16>());

    // Shrink optional panels (largest first) until the conversation has room.
    while conv_h < MIN_CONV {
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
        conv_h = total_h.saturating_sub(fixed + optional.iter().sum::<u16>());
    }

    PanelLayout {
        title_h,
        conv_h,
        agents_h: optional[0],
        activity_h: optional[1],
        graph_h: optional[2],
        metrics_h: optional[3],
        coord_h: optional[4],
        palette_h: optional[5],
        shortcuts_h,
        input_h,
    }
}

fn split_panels(area: Rect, layout: &PanelLayout) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(layout.title_h),
            Constraint::Length(layout.conv_h),
            Constraint::Length(layout.agents_h),
            Constraint::Length(layout.activity_h),
            Constraint::Length(layout.graph_h),
            Constraint::Length(layout.metrics_h),
            Constraint::Length(layout.coord_h),
            Constraint::Length(layout.shortcuts_h),
            Constraint::Length(layout.palette_h),
            Constraint::Length(layout.input_h),
        ])
        .split(area)
}

fn ui(f: &mut Frame, app: &TuiApp) {
    let size = f.size();
    let layout = compute_layout(app, size.height);

    let chunks = split_panels(size, &layout);

    render_title(f, app, chunks[0]);

    render_conversation(f, app, chunks[1]);

    if layout.agents_h > 0 {
        render_agents(f, app, chunks[2]);
    }

    if layout.activity_h > 0 {
        render_activity_log(f, &app.dashboard, chunks[3]);
    }

    if layout.graph_h > 0 {
        render_task_graph(f, app, chunks[4]);
    }

    if layout.metrics_h > 0 {
        render_metrics(f, app, chunks[5]);
    }

    if layout.coord_h > 0 {
        render_coordination(f, app, chunks[6]);
    }

    render_shortcuts(f, chunks[7]);

    if layout.palette_h > 0 {
        render_command_palette(f, app, chunks[8]);
    }

    render_input(f, app, chunks[9]);

    if app.dashboard.model_picker.is_open() {
        render_model_picker(f, app);
    }

    // Slash-command autocomplete popup sits just above the input row.
    if !app.dashboard.autocomplete.is_empty() && !app.dashboard.model_picker.is_open() {
        render_autocomplete(f, app, chunks[9]);
    }
}

fn render_autocomplete(f: &mut Frame, app: &TuiApp, input_area: Rect) {
    let mut entries: Vec<String> = app
        .dashboard
        .autocomplete
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let marker = if i == app.dashboard.autocomplete_index {
                "▶ "
            } else {
                "  "
            };
            let desc = SLASH_COMMANDS
                .iter()
                .find(|d| d.0.split_whitespace().next().unwrap_or("") == c)
                .map(|d| d.2)
                .unwrap_or("");
            format!("{}{}  {}", marker, c, desc)
        })
        .collect();
    entries.truncate(6);

    let width = input_area.width.min(input_area.width.max(40));
    let height = (entries.len() as u16 + BORDER).min(input_area.y.saturating_sub(2));
    if height < 2 {
        return;
    }
    let top = input_area.y.saturating_sub(height);
    let popup = Rect::new(input_area.x, top, width, height);

    let lines: Vec<Line> = entries
        .iter()
        .map(|e| {
            let sel = e.starts_with("▶");
            Line::from(Span::styled(
                e.clone(),
                if sel {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                },
            ))
        })
        .collect();

    let para = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" Complete (Tab cycles) "),
    );
    f.render_widget(para, popup);
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
            "CODEBRO",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "WS:",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(workspace, style_if(workspace, Color::Green)),
        Span::styled(" ", Style::default()),
        Span::styled(
            "Model:",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(model, Style::default().fg(Color::Yellow)),
        Span::styled(" ", Style::default()),
        Span::styled(
            "Tools:",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if app.dashboard.is_streaming {
                "running"
            } else {
                "armed"
            },
            Style::default().fg(Color::Magenta),
        ),
    ];
    if app.dashboard.is_streaming {
        spans.push(Span::styled(
            format!(" {}", app.dashboard.animation.spinner_char()),
            Style::default().fg(Color::Cyan),
        ));
    }
    let title = Paragraph::new(Line::from(spans))
        .style(Style::default().fg(Color::Cyan))
        .alignment(ratatui::layout::Alignment::Left);
    f.render_widget(title, area);
}

fn style_if(_text: &str, color: Color) -> Style {
    Style::default().fg(color)
}

fn render_conversation(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.height < 2 {
        return;
    }
    let inner_w = area.width.saturating_sub(BORDER) as usize;
    let mut lines: Vec<Line> = Vec::new();

    // Startup/welcome banner shown until the first response arrives.
    if app.dashboard.show_welcome && app.messages.is_empty() {
        let root = crate::tools::detect_workspace_root();
        let project = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace");
        let model = if app.config.model.trim().is_empty() {
            "auto-detect".to_string()
        } else {
            app.config.model.clone()
        };
        let ready_text = format!(
            "CODEBRO v{}\n\n  Workspace: {}    Model: {}\n  Tools: armed    Status: ready\n\n  Enter a task to begin (Ctrl+P for commands)",
            env!("CARGO_PKG_VERSION"),
            project,
            model,
        );
        for line in ready_text.lines() {
            let color = if line.starts_with("CODEBRO") {
                Color::Cyan
            } else if line.starts_with("  ") {
                Color::DarkGray
            } else {
                Color::White
            };
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(color),
            )));
        }
    } else {
        // Error banner if a recent error is pending.
        if let Some(ref err) = app.dashboard.last_error {
            lines.push(Line::from(Span::styled(
                format!(" ! Error: {}", truncate(err, inner_w)),
                Style::default().fg(Color::Red),
            )));
            lines.push(Line::from(""));
        }

        // Streaming assistant output renders live with a spinner.
        if app.dashboard.is_streaming && !app.dashboard.streaming_buffer.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("[AI] {} ", app.dashboard.animation.spinner_char()),
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
            let (role_label, _role_color, border_color): (_, Color, Color) = match msg.role {
                MessageRole::User => ("YOU", Color::Green, Color::Green),
                MessageRole::Assistant => ("AI", Color::Blue, Color::Blue),
                MessageRole::System => ("SYS", Color::Yellow, Color::Yellow),
            };

            // Distinct header bar for each message so roles are visually separated.
            lines.push(Line::from(Span::styled(
                format!("{} {} {}", "───", role_label, "─".repeat(20)),
                Style::default()
                    .fg(border_color)
                    .add_modifier(Modifier::BOLD),
            )));

            // Content: LLM messages go through the markdown renderer; user and
            // system messages render verbatim but still wrapped.
            if msg.role == MessageRole::Assistant {
                for md_line in crate::tui::markdown::render_markdown(&msg.content, inner_w) {
                    lines.push(md_line);
                }
            } else {
                let content = msg.content.lines().next().unwrap_or("");
                let rest: Vec<_> = msg.content.lines().skip(1).collect();
                let style = match msg.role {
                    MessageRole::User => Style::default().fg(Color::Green),
                    MessageRole::System => Style::default().fg(Color::Yellow),
                    _ => Style::default(),
                };
                lines.push(Line::from(Span::styled(content.to_string(), style)));
                for r in rest {
                    lines.push(Line::from(Span::styled(r.to_string(), style)));
                }
            }
            lines.push(Line::from(""));
        }
    }

    if app.messages.is_empty() && !app.dashboard.show_welcome {
        lines.push(Line::from(Span::styled(
            "  No conversation yet — enter a task to begin.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let total_lines = lines.len() as u16;
    let view_h = area.height.saturating_sub(BORDER);
    let max_scroll = total_lines.saturating_sub(view_h);
    let scroll = max_scroll.saturating_sub(app.scroll_from_bottom as u16);

    let conversation = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(" Conversation ")
                .title_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
        )
        .wrap(Wrap { trim: true })
        .scroll((scroll, 0));
    f.render_widget(conversation, area);
}

fn render_agents(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.height < 2 {
        return;
    }
    let mut lines = Vec::new();
    let entries = app.dashboard.agent_entries();
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No active agents",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for entry in entries {
            let (icon, color) = match entry.status {
                AgentStatus::Completed => ("✓", Color::Green),
                AgentStatus::Failed => ("✗", Color::Red),
                AgentStatus::Idle => ("○", Color::DarkGray),
                AgentStatus::Executing => ("⟳", Color::Yellow),
                AgentStatus::Thinking
                | AgentStatus::Searching
                | AgentStatus::Analysing
                | AgentStatus::Planning
                | AgentStatus::Testing
                | AgentStatus::Reviewing => {
                    let spinner = app.dashboard.animation.spinner_char();
                    (spinner, Color::Cyan)
                }
            };

            let bar = progress_bar(entry.progress, 10);
            let action = entry.action.as_deref().unwrap_or("");
            let name = truncate(&entry.name, 10);
            let status_str = truncate(entry.status.as_str(), 12);
            let action = truncate(action, 30);

            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {} ", icon),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:10}", name),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {:12}", status_str), Style::default().fg(color)),
                Span::styled(format!(" [{}]", bar), Style::default().fg(Color::Magenta)),
                Span::raw(format!(" {}", action)),
            ]));
        }
    }

    let agents_block = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Agents (Ctrl+A)"),
    );
    f.render_widget(agents_block, area);
}

fn render_activity_log(f: &mut Frame, dashboard: &Dashboard, area: Rect) {
    if area.height < 2 {
        return;
    }
    let mut lines = Vec::new();
    let entries: Vec<_> = dashboard.activity_log.iter().take(8).collect();
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Waiting for task",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for entry in entries {
            let color = match entry.level.as_str() {
                "error" => Color::Red,
                "tool" => Color::Cyan,
                "memory" => Color::Green,
                "skill" => Color::Magenta,
                "task" => Color::Yellow,
                _ => Color::Gray,
            };
            let ts = truncate(&entry.timestamp, 9);
            let msg = truncate(&entry.message, area.width.saturating_sub(14) as usize);
            lines.push(Line::from(vec![
                Span::styled(format!("[{}] ", ts), Style::default().fg(Color::DarkGray)),
                Span::styled(msg, Style::default().fg(color)),
            ]));
        }
    }

    let log_block = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title("Activity"))
        .wrap(Wrap { trim: true });
    f.render_widget(log_block, area);
}

fn render_task_graph(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.height < 2 {
        return;
    }
    let mut lines = Vec::new();
    let entries = app.dashboard.graph_entries();
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No task running",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        let mut prev_level = 0;

        for (desc, agent, status) in entries {
            let icon = match status {
                crate::agent::task_graph::TaskStatus::Completed => "✓",
                crate::agent::task_graph::TaskStatus::Failed => "✗",
                crate::agent::task_graph::TaskStatus::Running => "⟳",
                _ => "○",
            };
            let color = match status {
                crate::agent::task_graph::TaskStatus::Completed => Color::Green,
                crate::agent::task_graph::TaskStatus::Failed => Color::Red,
                crate::agent::task_graph::TaskStatus::Running => Color::Yellow,
                _ => Color::DarkGray,
            };

            let connector = if prev_level > 0 { "  |" } else { "" };
            let agent = truncate(&agent, 10);
            let desc = truncate(&desc, area.width.saturating_sub(20) as usize);
            lines.push(Line::from(vec![
                Span::styled(connector, Style::default().fg(Color::DarkGray)),
                Span::styled(format!(" {} ", icon), Style::default().fg(color)),
                Span::styled(format!("{:10}", agent), Style::default().fg(Color::Cyan)),
                Span::styled(desc, Style::default().fg(Color::White)),
            ]));
            prev_level = 1;
        }
    }

    let graph_block = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Task Graph (Ctrl+G)"),
    );
    f.render_widget(graph_block, area);
}

fn render_metrics(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.height < 2 {
        return;
    }
    let mut lines = Vec::new();

    let agent_count = app.dashboard.status_monitor.count();
    let active_count = app.dashboard.status_monitor.active_count();
    let progress = if agent_count > 0 {
        app.dashboard
            .status_monitor
            .list()
            .iter()
            .map(|s| s.progress)
            .sum::<f32>()
            / agent_count as f32
    } else {
        0.0
    };

    lines.push(Line::from(vec![
        Span::styled(
            "Agents:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" {} ({} active)", agent_count, active_count)),
        Span::styled(
            "   Progress:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" {:.0}%", progress * 100.0)),
    ]));

    let bar = progress_bar(progress, 20);
    lines.push(Line::from(Span::styled(
        format!("[{}]", bar),
        Style::default().fg(Color::Magenta),
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
    let duration_ms = app
        .dashboard
        .metrics
        .as_ref()
        .map(|m| m.total_duration_ms)
        .unwrap_or(0);

    lines.push(Line::from(vec![
        Span::styled(
            "Tokens:",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            " {}",
            crate::metrics::format_token_count(total_tokens)
        )),
        Span::styled(
            "   Cost:",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" {}", crate::metrics::format_cost_usd(cost))),
    ]));

    lines.push(Line::from(vec![
        Span::styled(
            "Time:",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            " {}",
            crate::session::format_duration_ms(duration_ms)
        )),
    ]));

    let metrics_block = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Task Metrics (Ctrl+V)"),
    );
    f.render_widget(metrics_block, area);
}

fn render_command_palette(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.height < 2 {
        return;
    }
    let entries = palette_entries(&app.dashboard.palette_query);

    let mut lines = Vec::new();
    // Filter line at the top.
    lines.push(Line::from(vec![
        Span::styled(
            "search> ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(app.dashboard.palette_query.clone()),
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
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(
                if selected {
                    "▶ ".to_string()
                } else {
                    "  ".to_string()
                },
                style,
            ),
            Span::styled(cmd.to_string(), style),
            Span::styled(
                format!("  {}", desc),
                style.fg(if selected {
                    Color::Black
                } else {
                    Color::DarkGray
                }),
            ),
        ]));
    }

    let palette = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" Command Palette (Ctrl+P) "),
    );
    f.render_widget(palette, area);
}

fn render_coordination(f: &mut Frame, app: &TuiApp, area: Rect) {
    if area.height < 2 {
        return;
    }
    let mut lines = Vec::new();

    let status = app.dashboard.status_monitor.get_all_status();
    lines.push(Line::from(vec![Span::styled(
        "Agent Communication:",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )]));

    for (agent, status) in &status {
        let icon = match status.as_str() {
            "completed" => "✓",
            "failed" => "✗",
            "working" => "⟳",
            _ => "○",
        };
        let agent = truncate(agent, 10);
        let status = truncate(status, 20);
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", icon), Style::default().fg(Color::Green)),
            Span::styled(format!("{:10}", agent), Style::default().fg(Color::White)),
            Span::styled(status, Style::default().fg(Color::DarkGray)),
        ]));
    }

    if !app.dashboard.recent_messages.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Recent Messages:",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )]));
        for msg in app.dashboard.recent_messages.iter().take(3) {
            let msg = truncate(msg, area.width.saturating_sub(4) as usize);
            lines.push(Line::from(Span::raw(format!("  {}", msg))));
        }
    }

    let coordination = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Coordination (Ctrl+O)"),
    );
    f.render_widget(coordination, area);
}

fn render_shortcuts(f: &mut Frame, area: Rect) {
    let shortcuts = [
        Shortcut::ToggleAgents,
        Shortcut::ToggleTaskGraph,
        Shortcut::ToggleMemory,
        Shortcut::ToggleMetrics,
        Shortcut::OpenCommandPalette,
        Shortcut::ToggleCoordination,
        Shortcut::ClearLogs,
        Shortcut::CancelTask,
        Shortcut::Quit,
    ];

    let mut spans = Vec::new();
    for (i, shortcut) in shortcuts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        }
        spans.push(Span::styled(
            shortcut.label(),
            Style::default().fg(Color::Blue),
        ));
    }

    let hint = Paragraph::new(Line::from(spans))
        .style(Style::default().fg(Color::DarkGray))
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(hint, area);
}

fn render_input(f: &mut Frame, app: &TuiApp, area: Rect) {
    let prefix = if app.dashboard.animation.is_active() {
        format!("{} > ", app.dashboard.animation.spinner_char())
    } else {
        "> ".to_string()
    };

    let inner_h = area.height.saturating_sub(BORDER) as usize;
    let inner_w = area.width.saturating_sub(BORDER) as usize;
    if inner_h == 0 {
        return;
    }

    let (display_lines, cursor_line_rel, cursor_col) =
        input_display_lines(&app.input, app.input_cursor, inner_h, inner_w);

    let mut text_lines = Vec::new();
    for (i, line) in display_lines.iter().enumerate() {
        // Prefix only on the first visible line.
        let full = if i == 0 {
            format!("{}{}", prefix, line)
        } else {
            line.clone()
        };
        text_lines.push(Line::from(truncate(&full, area.width as usize)));
    }

    let paragraph = Paragraph::new(Text::from(text_lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Input (Enter=send, Shift+Enter=newline)"),
        )
        .style(Style::default().fg(Color::Yellow));
    f.render_widget(paragraph, area);

    // Place the cursor on the correct line and column.
    if area.width >= 4 && area.height >= 3 {
        let x_off = if cursor_line_rel == 0 {
            prefix.chars().count() + cursor_col
        } else {
            cursor_col
        };
        let x = (area.x + 1 + x_off as u16).min(area.x + area.width.saturating_sub(2));
        let y = (area.y + 1 + cursor_line_rel as u16).min(area.y + area.height.saturating_sub(2));
        f.set_cursor(x, y);
    }
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
                    truncate(model, width.saturating_sub(6)),
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

/// Truncates a line to fit `width` Unicode characters (never panics on long text).
fn truncate(s: &str, width: usize) -> String {
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

fn truncate_line(s: &str, width: u16) -> String {
    truncate(s, width as usize)
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
        display.push(truncate(line, inner_w));
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
        // cursor at end
        let (lines, l, c) = input_display_lines(input, input.len(), 2, 50);
        assert_eq!(lines, vec!["line two", "line three"]);
        assert_eq!((l, c), (1, 10));
    }

    #[test]
    fn test_input_display_lines_empty() {
        let (lines, l, c) = input_display_lines("", 0, 3, 50);
        assert_eq!(lines, vec![""]);
        assert_eq!((l, c), (0, 0));
    }

    #[test]
    fn test_input_display_lines_truncates() {
        let long = "a".repeat(100);
        let (lines, _, _) = input_display_lines(&long, 100, 3, 10);
        assert!(lines[0].chars().count() <= 10);
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_long() {
        let out = truncate("a very long line of text", 10);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn test_truncate_zero() {
        assert_eq!(truncate("hello", 0), "");
    }

    #[test]
    fn test_compute_layout_small_terminal() {
        let mut app = make_app();
        app.dashboard.show_agents = true;
        app.dashboard.show_task_graph = true;
        app.dashboard.show_metrics = true;
        app.dashboard.show_coordination = true;
        app.dashboard.show_command_palette = true;

        let layout = compute_layout(&app, 24);
        // Conversation must keep a usable minimum even with all panels open.
        assert!(layout.conv_h >= MIN_CONV);
        assert!(layout.input_h >= 3);
        // Total allocated must never exceed the terminal height.
        let total = layout.title_h
            + layout.conv_h
            + layout.agents_h
            + layout.activity_h
            + layout.graph_h
            + layout.metrics_h
            + layout.coord_h
            + layout.shortcuts_h
            + layout.palette_h
            + layout.input_h;
        assert!(total <= 24);
    }

    #[test]
    fn test_compute_layout_large_terminal() {
        let mut app = make_app();
        app.dashboard.show_agents = true;
        app.dashboard.show_task_graph = true;

        let layout = compute_layout(&app, 40);
        assert!(layout.conv_h >= MIN_CONV);
        assert!(layout.agents_h > 0);
        assert!(layout.graph_h > 0);
    }

    #[test]
    fn test_compute_layout_no_panels() {
        let mut app = make_app();
        app.dashboard.show_agents = false;
        app.dashboard.show_task_graph = false;
        app.dashboard.show_metrics = false;
        app.dashboard.show_coordination = false;
        app.dashboard.show_command_palette = false;

        let layout = compute_layout(&app, 24);
        assert_eq!(layout.agents_h, 0);
        assert_eq!(layout.graph_h, 0);
        assert_eq!(layout.metrics_h, 0);
        assert!(layout.conv_h >= MIN_CONV);
    }

    #[test]
    fn test_compute_layout_extreme_small() {
        let app = make_app();
        // A 10-row terminal must not panic and must keep input + title.
        let layout = compute_layout(&app, 10);
        assert!(layout.title_h >= 1);
        assert!(layout.input_h >= 3);
    }

    #[test]
    fn test_compute_layout_default_panels() {
        let app = make_app();
        // Agents panel is on by default; verify it gets space on a normal terminal.
        let layout = compute_layout(&app, 24);
        assert!(layout.agents_h > 0);
    }

    #[test]
    fn test_match_slash_command_completes() {
        let matches = match_slash_command("/mo");
        assert!(matches.iter().any(|m| m == "/model"));
    }

    #[test]
    fn test_match_slash_command_empty_missing_slash() {
        assert!(match_slash_command("model").is_empty());
        assert!(match_slash_command("/").is_empty());
    }

    #[test]
    fn test_palette_filters_substring() {
        let entries = palette_entries("copy");
        assert!(entries.iter().any(|(name, _)| *name == "/copy"));
        let all = palette_entries("");
        assert!(all.iter().any(|(name, _)| *name == "/help"));
    }

    #[test]
    fn test_autocomplete_replaces_input() {
        let mut app = make_app();
        app.input = "/mo".to_string();
        app.dashboard
            .autocomplete_command(&mut app.input, vec!["/model".to_string()]);
        assert_eq!(app.input, "/model");
    }

    #[test]
    fn test_slash_commands_include_required() {
        let required = [
            "/help",
            "/model",
            "/agents",
            "/memory",
            "/skills",
            "/sessions",
            "/tasks",
            "/replay",
            "/config",
            "/status",
        ];
        for r in required {
            assert!(
                SLASH_COMMANDS
                    .iter()
                    .any(|(n, _, _)| n.split_whitespace().next().unwrap_or("") == r),
                "missing {}",
                r
            );
        }
    }
}
