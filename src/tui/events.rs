#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::Result;
use crossterm::event::{Event, KeyEvent, MouseEvent};
use futures::StreamExt;
use std::sync::mpsc;
use tokio::task;

#[derive(Debug, Clone)]
pub enum AppEvent {
    Input(crossterm::event::KeyEvent),
    Quit,
    Response(String),
    StreamChunk(String),
    AgentEvent(crate::agent::events::AgentEvent),
    Resize(u16, u16),
    /// Model discovery completed with provenance (discovered vs fallback).
    ModelsFetched {
        models: Vec<crate::provider_manager::ModelInfo>,
        /// Optional note shown in the chat (e.g. fallback usage).
        note: Option<String>,
    },
    ModelsFetchFailed(String),
    /// Provider health/model check completed right after an API key was
    /// stored. `message` is the sanitized, actionable summary.
    ProviderCheckResult {
        provider: String,
        message: String,
    },
    /// A bracketed-paste block from the terminal (may contain newlines).
    Paste(String),
    Mouse(MouseEvent),
    /// The runtime finished the current task with a real success flag. Sent
    /// immediately before `Response` so the UI can finalize action groups
    /// honestly (completed vs failed).
    TaskFinished {
        success: bool,
    },
    /// P5: Provider health check results
    ProviderHealthResults(Vec<(String, crate::provider_manager::HealthStatus, Option<u64>)>),
    /// P5: Workspace discovery results
    WorkspaceDiscovered {
        discovery: crate::workspace_discovery::WorkspaceDiscovery,
        capabilities: crate::capability_discovery::CapabilityDiscovery,
        mcp_servers: Vec<crate::workspace_discovery::McpServerInfo>,
    },
}

pub fn start_event_loop(tx: mpsc::Sender<AppEvent>) -> Result<()> {
    task::spawn_blocking(move || {
        let mut reader = crossterm::event::EventStream::new();

        loop {
            let event = futures::executor::block_on(reader.next());
            match event {
                Some(Ok(Event::Key(key))) => {
                    let _ = tx.send(AppEvent::Input(key));
                }
                Some(Ok(Event::Paste(text))) => {
                    let _ = tx.send(AppEvent::Paste(text));
                }
                Some(Ok(Event::Mouse(mouse))) => {
                    let _ = tx.send(AppEvent::Mouse(mouse));
                }
                Some(Ok(Event::Resize(w, h))) => {
                    let _ = tx.send(AppEvent::Resize(w, h));
                }
                Some(Err(_)) => {
                    let _ = tx.send(AppEvent::Quit);
                }
                None => {
                    let _ = tx.send(AppEvent::Quit);
                }
                _ => {}
            }
        }
    });

    Ok(())
}

pub fn check_key_shortcuts(key: &KeyEvent) -> Option<Shortcut> {
    use crossterm::event::KeyCode;
    use crossterm::event::KeyModifiers;

    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }

    match key.code {
        KeyCode::Char('a') => Some(Shortcut::ToggleAgents),
        KeyCode::Char('g') => Some(Shortcut::ToggleTaskGraph),
        KeyCode::Char('m') => Some(Shortcut::ToggleMemory),
        KeyCode::Char('s') => Some(Shortcut::SaveSession),
        KeyCode::Char('t') => Some(Shortcut::ToggleTrace),
        KeyCode::Char('l') => Some(Shortcut::ClearLogs),
        KeyCode::Char('c') => Some(Shortcut::CancelTask),
        KeyCode::Char('q') => Some(Shortcut::Quit),
        KeyCode::Char('p') => Some(Shortcut::OpenCommandPalette),
        KeyCode::Char('e') => Some(Shortcut::ToggleMetrics),
        // Design-spec: Ctrl+O toggles the intelligence rail.
        KeyCode::Char('o') => Some(Shortcut::ToggleRail),
        // Ctrl+U keeps coordination (swapped off Ctrl+O).
        KeyCode::Char('u') => Some(Shortcut::ToggleCoordination),
        KeyCode::Char('k') => Some(Shortcut::ToggleConsole),
        KeyCode::Char('d') => Some(Shortcut::ViewDiff),
        // Design-spec: Ctrl+Enter sends the current input.
        KeyCode::Enter => Some(Shortcut::SendInput),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shortcut {
    ToggleAgents,
    ToggleTaskGraph,
    ToggleMemory,
    SaveSession,
    ToggleTrace,
    ClearLogs,
    CancelTask,
    Quit,
    OpenCommandPalette,
    ToggleMetrics,
    ToggleCoordination,
    /// Collapse/expand the right intelligence rail.
    ToggleRail,
    /// Open/close the live PTY console overlay.
    ToggleConsole,
    /// Show the staged change preview (the existing `/apply` diff flow).
    ViewDiff,
    /// Submit the current input (Ctrl+Enter).
    SendInput,
}

impl Shortcut {
    pub fn label(&self) -> &'static str {
        match self {
            Shortcut::ToggleAgents => "Ctrl+A Agents",
            Shortcut::ToggleTaskGraph => "Ctrl+G Graph",
            Shortcut::ToggleMemory => "Ctrl+M Memory",
            Shortcut::SaveSession => "Ctrl+S Save",
            Shortcut::ToggleTrace => "Ctrl+T Trace",
            Shortcut::ClearLogs => "Ctrl+L Clear",
            Shortcut::CancelTask => "Ctrl+C Cancel",
            Shortcut::Quit => "Ctrl+Q Quit",
            Shortcut::OpenCommandPalette => "Ctrl+P Palette",
            Shortcut::ToggleMetrics => "Ctrl+E Metrics",
            Shortcut::ToggleCoordination => "Ctrl+U Coord",
            Shortcut::ToggleRail => "Ctrl+O Rail",
            Shortcut::ToggleConsole => "Ctrl+K Console",
            Shortcut::ViewDiff => "Ctrl+D Diff",
            Shortcut::SendInput => "Ctrl+Enter Send",
        }
    }
}

pub fn spawn_response_task<F>(sender: mpsc::Sender<AppEvent>, f: F)
where
    F: futures::Future<Output = ()> + Send + 'static,
{
    task::spawn(async move {
        f.await;
        let _ = sender;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn test_ctrl_d_maps_to_view_diff() {
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Char('d'))),
            Some(Shortcut::ViewDiff)
        );
        assert_eq!(Shortcut::ViewDiff.label(), "Ctrl+D Diff");
    }

    #[test]
    fn test_ctrl_o_toggles_rail() {
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Char('o'))),
            Some(Shortcut::ToggleRail)
        );
        assert_eq!(Shortcut::ToggleRail.label(), "Ctrl+O Rail");
    }

    #[test]
    fn test_ctrl_u_toggles_coordination() {
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Char('u'))),
            Some(Shortcut::ToggleCoordination)
        );
    }

    #[test]
    fn test_ctrl_enter_sends_input() {
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Enter)),
            Some(Shortcut::SendInput)
        );
    }

    #[test]
    fn test_all_existing_shortcuts_preserved() {
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Char('a'))),
            Some(Shortcut::ToggleAgents)
        );
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Char('g'))),
            Some(Shortcut::ToggleTaskGraph)
        );
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Char('m'))),
            Some(Shortcut::ToggleMemory)
        );
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Char('s'))),
            Some(Shortcut::SaveSession)
        );
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Char('t'))),
            Some(Shortcut::ToggleTrace)
        );
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Char('l'))),
            Some(Shortcut::ClearLogs)
        );
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Char('c'))),
            Some(Shortcut::CancelTask)
        );
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Char('q'))),
            Some(Shortcut::Quit)
        );
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Char('p'))),
            Some(Shortcut::OpenCommandPalette)
        );
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Char('e'))),
            Some(Shortcut::ToggleMetrics)
        );
        assert_eq!(
            check_key_shortcuts(&ctrl(KeyCode::Char('k'))),
            Some(Shortcut::ToggleConsole)
        );
    }
}
