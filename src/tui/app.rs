#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use crate::config::Config;
use crate::tui::dashboard::Dashboard;
use anyhow::Result;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::mpsc;

use crate::agent::events::AgentEvent;
use crate::tui::events::{self, AppEvent};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
}

pub struct TuiApp {
    pub messages: VecDeque<Message>,
    pub input: String,
    pub input_cursor: usize,
    pub is_loading: bool,
    /// Lines scrolled up from the bottom of the conversation (0 = at bottom).
    pub scroll_from_bottom: usize,
    pub config: Config,
    pub should_quit: bool,
    pub should_clear: bool,
    pub session_id: String,
    pub tx: mpsc::Sender<AppEvent>,
    pub dashboard: Dashboard,
    pub streaming_message_id: Option<usize>,
    pub session_tracker: Option<crate::session::SessionTracker>,
    pub metrics_registry: Option<crate::metrics::MetricsRegistry>,
    pub input_history: Vec<String>,
    pub history_index: Option<usize>,
    /// A proposed code change awaiting explicit user approval. Files are not
    /// modified until `/approve` runs. `None` = no pending change.
    pub pending_change: Option<crate::tools::ChangePlan>,
    /// P5: Interactive settings manager
    pub settings: Option<crate::settings::SettingsManager>,
    /// P5: Provider manager
    pub provider_manager: Option<crate::provider_manager::ProviderManager>,
    /// P5: Settings panel UI state
    pub settings_panel: crate::settings::SettingsPanel,
    /// P5: Provider manager panel UI state
    pub provider_panel: ProviderPanel,
    /// P5: Workspace discovery panel
    pub workspace_panel: WorkspacePanel,
}

/// UI state for the provider management panel
#[derive(Debug, Clone)]
pub struct ProviderPanel {
    pub active_provider: Option<String>,
    pub selected_provider_index: usize,
    pub health_results: Vec<(String, crate::provider_manager::HealthStatus, Option<u64>)>,
    pub loading_health: bool,
    pub show_api_key_input: bool,
    pub api_key_input: String,
}

impl ProviderPanel {
    pub fn new() -> Self {
        ProviderPanel {
            active_provider: None,
            selected_provider_index: 0,
            health_results: Vec::new(),
            loading_health: false,
            show_api_key_input: false,
            api_key_input: String::new(),
        }
    }
}

impl Default for ProviderPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// UI state for the workspace discovery panel
#[derive(Debug, Clone)]
pub struct WorkspacePanel {
    pub discovery: Option<crate::workspace_discovery::WorkspaceDiscovery>,
    pub capability_discovery: Option<crate::capability_discovery::CapabilityDiscovery>,
    pub selected_proposal_index: usize,
    pub selected_capability_index: usize,
    pub mcp_servers: Vec<crate::workspace_discovery::McpServerInfo>,
}

impl WorkspacePanel {
    pub fn new() -> Self {
        WorkspacePanel {
            discovery: None,
            capability_discovery: None,
            selected_proposal_index: 0,
            selected_capability_index: 0,
            mcp_servers: Vec::new(),
        }
    }
}

impl Default for WorkspacePanel {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiApp {
    pub fn new() -> Result<Self> {
        let config = Config::load()?;
        Self::new_with_config(config)
    }

    pub fn new_with_config(config: Config) -> Result<Self> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let (tx, _rx) = std::sync::mpsc::channel();

        let session_tracker = crate::session::SessionTracker::new(Config::config_dir()).ok();
        let metrics_registry = crate::metrics::MetricsRegistry::new().ok();

        let config_dir = Config::config_dir();
        let mut provider_manager =
            crate::provider_manager::ProviderManager::new(config_dir.clone());
        provider_manager.register_builtin();
        let _ = provider_manager.load();

        let settings = crate::settings::SettingsManager::new(config.clone(), config_dir.clone());

        Ok(TuiApp {
            messages: VecDeque::new(),
            input: String::new(),
            input_cursor: 0,
            is_loading: false,
            scroll_from_bottom: 0,
            config,
            should_quit: false,
            should_clear: false,
            session_id,
            tx,
            dashboard: Dashboard::new(),
            streaming_message_id: None,
            session_tracker,
            metrics_registry,
            input_history: Vec::new(),
            history_index: None,
            pending_change: None,
            settings: Some(settings),
            provider_manager: Some(provider_manager),
            settings_panel: crate::settings::SettingsPanel::new(),
            provider_panel: ProviderPanel::new(),
            workspace_panel: WorkspacePanel::new(),
        })
    }

    pub fn add_message(&mut self, role: MessageRole, content: String) {
        self.messages.push_back(Message { role, content });
        self.scroll_to_bottom();
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_from_bottom = 0;
    }

    pub fn scroll_up(&mut self) {
        self.scroll_from_bottom += 1;
    }

    pub fn scroll_down(&mut self) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(1);
    }

    pub fn clear_screen(&mut self) {
        self.messages.clear();
        self.scroll_from_bottom = 0;
    }

    // ---- Input cursor handling ----

    pub fn insert_char(&mut self, c: char) {
        let idx = self.input_char_boundary(self.input_cursor);
        self.input.insert(idx, c);
        self.input_cursor += c.len_utf8();
    }

    /// Inserts a block of text (from a paste) at the cursor. Newlines are kept
    /// so multi-line prompts stay together in the input buffer.
    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let idx = self.input_char_boundary(self.input_cursor);
        self.input.insert_str(idx, text);
        self.input_cursor = idx + text.len();
    }

    /// Returns the (line index, column) of the cursor within the input buffer.
    pub fn input_cursor_line_col(&self) -> (usize, usize) {
        let up_to = &self.input[..self.input_char_boundary(self.input_cursor)];
        let line_idx = up_to.matches('\n').count();
        let col = up_to
            .rsplit('\n')
            .next()
            .map(|s| s.chars().count())
            .unwrap_or(0);
        (line_idx, col)
    }

    /// Scrolls the conversation using mouse wheel deltas.
    /// `lines > 0` means the user scrolled up (see older content).
    pub fn mouse_scroll(&mut self, lines: isize) {
        if lines > 0 {
            self.scroll_from_bottom = self.scroll_from_bottom.saturating_add(lines as usize);
        } else {
            self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub((-lines) as usize);
        }
    }

    /// Copies text to the system clipboard (macOS: pbcopy, Linux: xclip/wl-copy).
    pub fn copy_to_clipboard(&self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        for cmd in ["pbcopy", "xclip", "wl-copy"] {
            let mut c = std::process::Command::new(cmd);
            if cmd == "xclip" {
                c.arg("-selection").arg("clipboard");
            }
            let mut child = match c.stdin(std::process::Stdio::piped()).spawn() {
                Ok(ch) => ch,
                Err(_) => continue,
            };
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if child.wait().map(|s| s.success()).unwrap_or(false) {
                return true;
            }
        }
        false
    }

    /// Returns the full conversation as plain text (for copying / sessions).
    pub fn conversation_text(&self) -> String {
        let mut out = String::new();
        for msg in &self.messages {
            let role = match msg.role {
                MessageRole::User => "USER",
                MessageRole::Assistant => "AI",
                MessageRole::System => "SYSTEM",
            };
            out.push_str(&format!("[{}] {}\n", role, msg.content));
            out.push('\n');
        }
        out
    }

    pub fn backspace(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let idx = self.input_char_boundary(self.input_cursor);
        if let Some(prev) = self.input[..idx].char_indices().next_back() {
            self.input.remove(prev.0);
            self.input_cursor = prev.0;
        } else {
            self.input.remove(0);
            self.input_cursor = 0;
        }
    }

    pub fn cursor_left(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let idx = self.input_char_boundary(self.input_cursor);
        if let Some((pos, _)) = self.input[..idx].char_indices().next_back() {
            self.input_cursor = pos;
        } else {
            self.input_cursor = 0;
        }
    }

    pub fn cursor_right(&mut self) {
        let len = self.input.len();
        if self.input_cursor >= len {
            self.input_cursor = len;
            return;
        }
        let idx = self.input_char_boundary(self.input_cursor);
        if let Some((pos, ch)) = self.input[idx..].char_indices().next() {
            self.input_cursor = idx + pos + ch.len_utf8();
        } else {
            self.input_cursor = len;
        }
    }

    pub fn cursor_home(&mut self) {
        self.input_cursor = 0;
    }

    pub fn cursor_end(&mut self) {
        self.input_cursor = self.input.len();
    }

    fn input_char_boundary(&self, byte_idx: usize) -> usize {
        let len = self.input.len();
        if byte_idx >= len {
            return len;
        }
        let mut i = byte_idx;
        while i > 0 && !self.input.is_char_boundary(i) {
            i -= 1;
        }
        i
    }

    // ---- Input history navigation ----

    pub fn push_history(&mut self, cmd: String) {
        if cmd.is_empty() {
            return;
        }
        if self.input_history.last() != Some(&cmd) {
            self.input_history.push(cmd);
        }
        self.history_index = None;
    }

    pub fn history_previous(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        let idx = match self.history_index {
            Some(i) if i > 0 => i - 1,
            Some(_) => return,
            None => self.input_history.len() - 1,
        };
        self.history_index = Some(idx);
        self.input = self.input_history[idx].clone();
        self.input_cursor = self.input.len();
    }

    pub fn history_next(&mut self) {
        match self.history_index {
            Some(i) if i + 1 < self.input_history.len() => {
                self.history_index = Some(i + 1);
                self.input = self.input_history[i + 1].clone();
                self.input_cursor = self.input.len();
            }
            Some(_) => {
                self.history_index = None;
                self.input.clear();
                self.input_cursor = 0;
            }
            None => {}
        }
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
        self.input_cursor = 0;
        self.history_index = None;
    }

    pub fn begin_task(&mut self, task: impl Into<String>) {
        let task = task.into();
        if let Some(tracker) = self.session_tracker.as_mut() {
            let _ = tracker.start_session(task.clone());
        }
        if let Some(registry) = self.metrics_registry.as_mut() {
            registry.begin_task(task);
        }
        self.dashboard.metrics = Some(crate::metrics::TaskMetrics::new("current task"));
    }

    pub fn end_task(&mut self) {
        if let Some(tracker) = self.session_tracker.as_mut() {
            let _ = tracker.end_session();
        }
        if let Some(registry) = self.metrics_registry.as_mut() {
            registry.end_task(&self.config.model);
        }
        self.dashboard.metrics = None;
    }

    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        self.dashboard.handle_event(event.clone());

        if let Some(tracker) = self.session_tracker.as_mut() {
            let _ = tracker.record_event(&event);
        }

        match event.clone() {
            AgentEvent::AgentStarted { agent, .. } => {
                self.dashboard.log("info", format!("{} started", agent));
            }
            AgentEvent::AgentProgress { agent, .. } => {
                if let Some(metrics) = self.dashboard.metrics.as_mut() {
                    metrics.record_agent_duration(&agent, 0);
                }
            }
            AgentEvent::ToolCompleted { tool, .. } => {
                if let Some(metrics) = self.dashboard.metrics.as_mut() {
                    metrics.record_tool_duration(&tool, 0);
                }
            }
            AgentEvent::AgentCompleted { agent, duration_ms } => {
                self.dashboard
                    .log("info", format!("{} completed in {}ms", agent, duration_ms));
                self.dashboard.dismiss_welcome();
                if let Some(metrics) = self.dashboard.metrics.as_mut() {
                    metrics.record_agent_duration(&agent, duration_ms);
                }
            }
            AgentEvent::AgentFailed { agent, error } => {
                self.dashboard
                    .log("error", format!("{} failed: {}", agent, error));
                self.dashboard.set_error(error.clone());
                if let Some(metrics) = self.dashboard.metrics.as_mut() {
                    metrics.increment_retries();
                }
            }
            AgentEvent::MemoryUpdated { summary } => {
                self.add_message(MessageRole::System, format!("Memory: {}", summary));
            }
            AgentEvent::SkillUpdated {
                skill,
                confidence_before,
                confidence_after,
            } => {
                self.add_message(
                    MessageRole::System,
                    format!(
                        "Skill '{}' confidence: {:.2} -> {:.2}",
                        skill, confidence_before, confidence_after
                    ),
                );
            }
            _ => {}
        }
    }

    /// Opens the interactive model picker and fetches the provider's models.
    pub fn open_model_picker(&mut self) {
        self.dashboard.model_picker.open();

        let base_url = self.config.base_url.clone();
        let api_key = self.config.api_key.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let key = api_key
                .clone()
                .or_else(|| std::env::var("CODEBRO_API_KEY").ok());
            // We're already inside the app's tokio runtime — just await.
            match crate::providers::fetch_models(&base_url, key.as_deref()).await {
                Ok(models) if !models.is_empty() => {
                    let _ = tx.send(events::AppEvent::ModelsFetched(models));
                }
                Ok(_) => {
                    let _ = tx.send(events::AppEvent::ModelsFetchFailed(
                        "Provider returned no models".to_string(),
                    ));
                }
                Err(e) => {
                    let _ = tx.send(events::AppEvent::ModelsFetchFailed(e.to_string()));
                }
            }
        });
    }

    pub fn handle_models_fetched(&mut self, models: Vec<String>) {
        self.dashboard.model_picker.set_models(models);
        self.dashboard.log("info", "Model list loaded".to_string());
    }

    pub fn handle_models_failed(&mut self, error: String) {
        self.dashboard.model_picker.loading = false;
        self.dashboard.model_picker.error = Some(error.clone());
        self.dashboard
            .log("error", format!("Model fetch failed: {}", error));
        self.add_message(
            MessageRole::System,
            format!("Failed to load models: {}", error),
        );
    }

    /// Applies a selected model: updates config, persists it, closes the picker.
    pub fn apply_model(&mut self, model: String) {
        if model.trim().is_empty() {
            return;
        }
        self.config.model = model.clone();
        let _ = self.config.persist_model();
        self.dashboard.model_picker.close();
        self.add_message(MessageRole::System, format!("Model set to {}", model));
        self.dashboard
            .log("info", format!("Model set to {}", model));
    }

    pub fn list_sessions(&self) -> Vec<String> {
        if let Some(tracker) = self.session_tracker.as_ref() {
            if let Ok(sessions) = tracker.store().list_sessions() {
                return sessions
                    .into_iter()
                    .take(20)
                    .map(|s| format!("{} - {} ({} events)", s.id, s.task, s.timeline.len()))
                    .collect();
            }
        }
        Vec::new()
    }

    // ─── P5: Settings Management ─────────────────────────────────────────

    pub fn open_settings(&mut self) {
        self.settings_panel.open();
    }

    pub fn close_settings(&mut self) {
        self.settings_panel.close();
    }

    pub fn toggle_settings(&mut self) {
        if self.settings_panel.view == crate::settings::SettingsView::List {
            self.open_settings();
        } else {
            self.close_settings();
        }
    }

    pub fn settings_summary(&self) -> String {
        if let Some(ref sm) = self.settings {
            sm.summary()
        } else {
            "Settings not initialized".to_string()
        }
    }

    pub fn apply_settings_changes(&mut self) -> Result<()> {
        if let Some(ref mut sm) = self.settings {
            sm.apply_changes()?;
            // Update config from settings
            if let Some(model) = sm.get_setting("model") {
                if let crate::settings::SettingKind::String(v) = &model.value {
                    self.config.model = v.clone();
                    let _ = self.config.persist_model();
                }
            }
            if let Some(provider) = sm.get_setting("provider") {
                if let crate::settings::SettingKind::String(v) = &provider.value {
                    self.config.provider = v.clone();
                }
            }
            if let Some(base_url) = sm.get_setting("base_url") {
                if let crate::settings::SettingKind::String(v) = &base_url.value {
                    self.config.base_url = v.clone();
                }
            }
            self.add_message(
                MessageRole::System,
                "Settings applied successfully".to_string(),
            );
        }
        Ok(())
    }

    pub fn discard_settings_changes(&mut self) {
        if let Some(ref mut sm) = self.settings {
            sm.discard_changes();
        }
    }

    // ─── P5: Provider Management ─────────────────────────────────────────

    pub fn open_provider_manager(&mut self) {
        self.provider_panel = ProviderPanel::new();
        if let Some(ref pm) = self.provider_manager {
            self.provider_panel.active_provider = pm.active_provider().cloned();
        }
    }

    pub fn close_provider_manager(&mut self) {
        self.provider_panel = ProviderPanel::new();
    }

    pub fn toggle_provider_manager(&mut self) {
        if self.provider_panel.active_provider.is_none()
            && self.provider_panel.health_results.is_empty()
        {
            self.open_provider_manager();
        } else {
            self.close_provider_manager();
        }
    }

    pub fn check_provider_health(&mut self) {
        if let Some(ref mut pm) = self.provider_manager {
            self.provider_panel.loading_health = true;
            let mut pm_clone = pm.clone();
            let tx = self.tx.clone();
            tokio::spawn(async move {
                let results = pm_clone.check_all_health().await;
                let _ = tx.send(events::AppEvent::ProviderHealthResults(results));
            });
        }
    }

    pub fn handle_provider_health_results(
        &mut self,
        results: Vec<(String, crate::provider_manager::HealthStatus, Option<u64>)>,
    ) {
        self.provider_panel.health_results = results;
        self.provider_panel.loading_health = false;
    }

    pub fn set_provider_api_key(&mut self, provider_id: &str, key: &str) -> Result<()> {
        if let Some(ref mut pm) = self.provider_manager {
            pm.set_api_key(provider_id, key)?;
            pm.persist()?;
            self.add_message(
                MessageRole::System,
                format!("API key set for {}", provider_id),
            );
        }
        Ok(())
    }

    pub fn switch_provider(&mut self, provider_id: &str) -> Result<()> {
        if let Some(ref mut pm) = self.provider_manager {
            pm.set_active(provider_id)?;
            pm.persist()?;
            self.provider_panel.active_provider = Some(provider_id.to_string());
            self.add_message(
                MessageRole::System,
                format!("Switched to provider: {}", provider_id),
            );
        }
        Ok(())
    }

    pub fn provider_status_text(&self) -> String {
        let mut lines = Vec::new();
        if let Some(ref pm) = self.provider_manager {
            let active = pm.active_provider().cloned();
            for (id, entry) in pm.list_providers() {
                let key_display = match pm.api_key_masked(id) {
                    Some(k) => format!("key: {}", k),
                    None => "key: (unset)".to_string(),
                };
                let health = match &entry.health {
                    crate::provider_manager::HealthStatus::Healthy => "✓ healthy".to_string(),
                    crate::provider_manager::HealthStatus::Unhealthy { reason } => {
                        format!("✗ {}", reason)
                    }
                    _ => "○ unknown".to_string(),
                };
                let active_mark = if active.as_deref() == Some(id) {
                    " [active]"
                } else {
                    ""
                };
                lines.push(format!("  {} {}{}{}", id, key_display, health, active_mark));
            }
        }
        lines.join("\n")
    }

    // ─── P5: Workspace Discovery ─────────────────────────────────────────

    pub async fn discover_workspace(&mut self) {
        let root = crate::tools::detect_workspace_root();
        let engine = crate::workspace_discovery::DiscoveryEngine::new(root.clone());
        let discovery = engine.discover();
        self.workspace_panel.discovery = Some(discovery);

        let scanner = crate::capability_discovery::CapabilityScanner::new(root.clone());
        let cap_discovery = scanner.scan();
        self.workspace_panel.capability_discovery = Some(cap_discovery);

        let mcp = crate::workspace_discovery::discover_mcp_servers(&root);
        self.workspace_panel.mcp_servers = mcp;

        self.dashboard
            .log("info", "Workspace discovery complete".to_string());
    }

    pub fn workspace_summary(&self) -> String {
        let mut lines = Vec::new();
        if let Some(ref wd) = self.workspace_panel.discovery {
            lines.push(format!("Workspace: {}", wd.root.display()));
            lines.push(format!("Language: {}", wd.language));
            if let Some(ref fw) = wd.framework {
                lines.push(format!("Framework: {}", fw));
            }
            if let Some(ref bs) = wd.build_system {
                lines.push(format!("Build: {}", bs));
            }
            if let Some(ref pm) = wd.package_manager {
                lines.push(format!("Package manager: {}", pm));
            }
            lines.push(format!(
                "Integrations: {}/{} enabled",
                wd.enabled_count(),
                wd.proposals.len()
            ));
        }
        lines.join("\n")
    }

    pub fn toggle_integration(&mut self, index: usize) {
        if let Some(ref mut wd) = self.workspace_panel.discovery {
            if index < wd.proposals.len() {
                wd.proposals[index].enabled = !wd.proposals[index].enabled;
                wd.proposals[index].approved = wd.proposals[index].enabled;
            }
        }
    }

    // ─── P5: Slash Commands ──────────────────────────────────────────────

    pub fn handle_settings_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        match parts.first().copied() {
            Some("/settings") => {
                self.toggle_settings();
                if let Some(ref sm) = self.settings {
                    self.add_message(MessageRole::System, sm.summary());
                }
            }
            Some("/settings:apply") => {
                if let Err(e) = self.apply_settings_changes() {
                    self.add_message(MessageRole::System, format!("Settings error: {}", e));
                }
            }
            Some("/settings:discard") => {
                self.discard_settings_changes();
                self.add_message(
                    MessageRole::System,
                    "Settings changes discarded".to_string(),
                );
            }
            Some("/providers") => {
                self.toggle_provider_manager();
                let status = self.provider_status_text();
                self.add_message(MessageRole::System, format!("Providers:\n{}", status));
            }
            Some("/health") => {
                self.check_provider_health();
                self.add_message(
                    MessageRole::System,
                    "Checking provider health...".to_string(),
                );
            }
            Some("/discover") => {
                self.add_message(MessageRole::System, "Discovering workspace...".to_string());
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    // Discovery runs in a detached context
                    let root = crate::tools::detect_workspace_root();
                    let engine = crate::workspace_discovery::DiscoveryEngine::new(root.clone());
                    let discovery = engine.discover();
                    let scanner = crate::capability_discovery::CapabilityScanner::new(root.clone());
                    let cap_discovery = scanner.scan();
                    let mcp = crate::workspace_discovery::discover_mcp_servers(&root);
                    let _ = tx.send(events::AppEvent::WorkspaceDiscovered {
                        discovery,
                        capabilities: cap_discovery,
                        mcp_servers: mcp,
                    });
                });
            }
            Some("/workspace") => {
                let summary = self.workspace_summary();
                self.add_message(MessageRole::System, format!("Workspace:\n{}", summary));
            }
            Some("/onboard") => {
                self.add_message(
                    MessageRole::System,
                    "Run `codebro onboard` from the command line to re-run onboarding.".to_string(),
                );
            }
            _ => {
                self.add_message(
                    MessageRole::System,
                    format!("Unknown settings command: {}", cmd),
                );
            }
        }
    }
}
