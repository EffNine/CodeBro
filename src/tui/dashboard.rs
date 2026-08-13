#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use crate::agent::events::AgentEvent;
use crate::agent::status::{AgentStatus, AgentStatusMonitor};
use crate::agent::task_graph::{TaskGraph, TaskStatus};
use crate::provider_manager::ModelInfo;
use crate::tui::animation::{ActivityType, AnimationState};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Maximum characters retained in the live streaming buffer. A pathological
/// provider response (or PTY burst) cannot grow memory without bound.
pub const MAX_STREAMING_BUFFER_CHARS: usize = 1_000_000;

/// Wrap-around list navigation (Up/Down). Moving past the last item returns
/// to the first, and past the first wraps to the last.
pub fn nav_wrap(index: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let len_i = len as isize;
    let mut i = index as isize + delta;
    i %= len_i;
    if i < 0 {
        i += len_i;
    }
    i as usize
}

/// Page navigation (PageUp/PageDown). Moves by `page` rows and clamps at the
/// ends; it never wraps around.
pub fn nav_page(index: usize, len: usize, delta: isize, page: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let step = page as isize * delta;
    let target = index as isize + step;
    target.clamp(0, len as isize - 1) as usize
}

/// Clamp an index into a list (used when a filter shrinks the list).
pub fn nav_clamp(index: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        index.min(len - 1)
    }
}

/// The default page size used by PageUp/PageDown in overlay selectors.
pub const NAV_PAGE_SIZE: usize = 6;

/// Shared list-navigation state for every selectable overlay: command palette,
/// slash-autocomplete, provider picker, model picker, settings list.
///
/// The contract every caller must honour:
///   - `selected` is always clamped to `[0, item_count)`.
///   - When items are added/removed, the caller re-renders after calling
///     `clamp_selection()` or the constructor reset.
///   - `scroll_offset` tracks how many rows above the selected row are visible;
///     it is adjusted automatically when the selection moves so the selected
///     row never leaves the viewport.
#[derive(Debug, Clone, Default)]
pub struct SelectableListState {
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub viewport_height: usize,
}

impl SelectableListState {
    pub fn new(viewport_height: usize) -> Self {
        SelectableListState {
            selected_index: 0,
            scroll_offset: 0,
            viewport_height,
        }
    }

    /// Move selection up by one (wraps to last).
    pub fn move_up(&mut self, item_count: usize) {
        if item_count == 0 {
            return;
        }
        self.selected_index = nav_wrap(self.selected_index, item_count, -1);
        self.ensure_visible(item_count);
    }

    /// Move selection down by one (wraps to first).
    pub fn move_down(&mut self, item_count: usize) {
        if item_count == 0 {
            return;
        }
        self.selected_index = nav_wrap(self.selected_index, item_count, 1);
        self.ensure_visible(item_count);
    }

    /// Page up (clamps, no wrap).
    pub fn page_up(&mut self, item_count: usize) {
        if item_count == 0 {
            return;
        }
        self.selected_index = nav_page(self.selected_index, item_count, -1, NAV_PAGE_SIZE);
        self.ensure_visible(item_count);
    }

    /// Page down (clamps, no wrap).
    pub fn page_down(&mut self, item_count: usize) {
        if item_count == 0 {
            return;
        }
        self.selected_index = nav_page(self.selected_index, item_count, 1, NAV_PAGE_SIZE);
        self.ensure_visible(item_count);
    }

    /// Jump to the first item.
    pub fn move_home(&mut self) {
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Jump to the last item.
    pub fn move_end(&mut self, item_count: usize) {
        if item_count == 0 {
            return;
        }
        self.selected_index = item_count - 1;
        self.ensure_visible(item_count);
    }

    /// Clamp the selection into the valid range and adjust scroll so the
    /// selected row stays visible.
    pub fn clamp_selection(&mut self, item_count: usize) {
        if item_count == 0 {
            self.selected_index = 0;
            self.scroll_offset = 0;
            return;
        }
        self.selected_index = self.selected_index.min(item_count - 1);
        self.ensure_visible(item_count);
    }

    /// Adjust `scroll_offset` so that `selected_index` is always within the
    /// currently visible window `[scroll_offset, scroll_offset + viewport_height)`.
    pub fn ensure_visible(&mut self, item_count: usize) {
        if item_count == 0 || self.viewport_height == 0 {
            return;
        }
        let idx = self.selected_index.min(item_count - 1);
        if idx < self.scroll_offset {
            self.scroll_offset = idx;
        } else if idx >= self.scroll_offset + self.viewport_height {
            self.scroll_offset = idx.saturating_sub(self.viewport_height - 1);
        }
    }

    /// Visible range for rendering: `(start_index, end_index)`.
    pub fn visible_range(&self, item_count: usize) -> (usize, usize) {
        let start = self.scroll_offset.min(item_count);
        let end = (start + self.viewport_height).min(item_count);
        (start, end)
    }

    /// Refresh viewport height (e.g. after resize). Clamps scroll if needed.
    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = height;
        self.scroll_offset = self
            .scroll_offset
            .min(self.selected_index.saturating_sub(height.saturating_sub(1)));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub message: String,
    pub level: String,
}

#[derive(Debug, Clone)]
pub struct ToolView {
    pub name: String,
    pub args_summary: String,
    pub status: ToolStatus,
    pub result: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct MemoryNotification {
    pub message: String,
    pub timestamp: String,
}

#[derive(Debug, Clone)]
pub struct SkillNotification {
    pub skill: String,
    pub confidence_before: f32,
    pub confidence_after: f32,
    pub timestamp: String,
}

#[derive(Debug, Clone)]
pub struct AgentPanelEntry {
    pub name: String,
    pub status: AgentStatus,
    pub progress: f32,
    pub action: Option<String>,
    pub task: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ModelPicker {
    pub models: Vec<ModelInfo>,
    pub list_state: SelectableListState,
    pub open: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub filter: String,
}

impl ModelPicker {
    pub fn new() -> Self {
        ModelPicker {
            models: Vec::new(),
            list_state: SelectableListState::new(NAV_PAGE_SIZE * 2),
            open: false,
            loading: false,
            error: None,
            filter: String::new(),
        }
    }

    pub fn open(&mut self) {
        self.open = true;
        self.loading = true;
        self.list_state = SelectableListState::new(NAV_PAGE_SIZE * 2);
        self.error = None;
        self.filter.clear();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.loading = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_models(&mut self, models: Vec<ModelInfo>) {
        self.models = models;
        self.loading = false;
        self.list_state = SelectableListState::new(NAV_PAGE_SIZE * 2);
    }

    /// Visible model count used for navigation calculations.
    pub fn visible_count(&self) -> usize {
        self.visible_models().len()
    }

    pub fn next(&mut self) {
        self.list_state.move_down(self.visible_count());
    }

    pub fn prev(&mut self) {
        self.list_state.move_up(self.visible_count());
    }

    pub fn page_next(&mut self) {
        self.list_state.page_down(self.visible_count());
    }

    pub fn page_prev(&mut self) {
        self.list_state.page_up(self.visible_count());
    }

    pub fn home(&mut self) {
        self.list_state.move_home();
    }

    pub fn end(&mut self) {
        self.list_state.move_end(self.visible_count());
    }

    pub fn selected(&self) -> Option<ModelInfo> {
        let visible = self.visible_models();
        let i = nav_clamp(self.list_state.selected_index, visible.len());
        visible.get(i).map(|m| (*m).clone())
    }

    /// Push a filter character and reset the selection (the filtered window
    /// may be shorter than the previous one).
    pub fn push_filter(&mut self, c: char) {
        self.filter.push(c);
        self.list_state = SelectableListState::new(NAV_PAGE_SIZE * 2);
    }

    /// Pop a filter character and reset the selection.
    pub fn pop_filter(&mut self) {
        self.filter.pop();
        self.list_state = SelectableListState::new(NAV_PAGE_SIZE * 2);
    }

    /// Models matching the filter, in list order.
    pub fn visible_models(&self) -> Vec<&ModelInfo> {
        let f = self.filter.to_lowercase();
        if f.is_empty() {
            self.models.iter().collect()
        } else {
            self.models
                .iter()
                .filter(|m| m.id.to_lowercase().contains(&f))
                .collect()
        }
    }

    pub fn count(&self) -> usize {
        self.models.len()
    }
}

impl Default for ModelPicker {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Dashboard {
    pub status_monitor: AgentStatusMonitor,
    pub activity_log: VecDeque<LogEntry>,
    pub active_tools: VecDeque<ToolView>,
    pub memory_notifications: VecDeque<MemoryNotification>,
    pub skill_notifications: VecDeque<SkillNotification>,
    pub task_graph: Option<TaskGraph>,
    pub animation: AnimationState,
    pub max_log_entries: usize,
    pub show_agents: bool,
    pub show_task_graph: bool,
    pub show_memory: bool,
    pub show_trace: bool,
    pub show_metrics: bool,
    pub show_command_palette: bool,
    pub show_coordination: bool,
    pub streaming_buffer: String,
    pub is_streaming: bool,
    pub auto_scroll: bool,
    pub metrics: Option<crate::metrics::TaskMetrics>,
    pub recent_messages: Vec<String>,
    pub model_picker: ModelPicker,
    /// Slash-command autocomplete candidates (from TAB) and the selection index.
    pub autocomplete: Vec<String>,
    pub autocomplete_index: usize,
    /// Command palette filter and selected row index.
    pub palette_query: String,
    pub palette_index: usize,
    /// Startup/welcome banner shown on first render until a task completes.
    pub show_welcome: bool,
    /// Most recent provider/tool error surfaced to the user.
    pub last_error: Option<String>,
    /// Verbose mode: reveal tool calls, routing decisions, provider details,
    /// and internal diagnostics. Off by default (minimal).
    pub verbose: bool,
    /// Compact mode: reduce secondary activity further.
    pub compact: bool,
    /// Current operation being performed (shown as the "current operation"
    /// line under the title).
    pub current_operation: Option<String>,
}

impl Dashboard {
    pub fn new() -> Self {
        Dashboard {
            status_monitor: AgentStatusMonitor::new(),
            activity_log: VecDeque::new(),
            active_tools: VecDeque::new(),
            memory_notifications: VecDeque::new(),
            skill_notifications: VecDeque::new(),
            task_graph: None,
            animation: AnimationState::new(),
            max_log_entries: 200,
            // Default view is task-focused: panels are overlays, not fixtures.
            show_agents: false,
            show_task_graph: false,
            show_memory: false,
            show_trace: false,
            show_metrics: false,
            show_command_palette: false,
            show_coordination: false,
            streaming_buffer: String::new(),
            is_streaming: false,
            auto_scroll: true,
            metrics: None,
            recent_messages: Vec::new(),
            model_picker: ModelPicker::new(),
            autocomplete: Vec::new(),
            autocomplete_index: 0,
            palette_query: String::new(),
            palette_index: 0,
            show_welcome: true,
            last_error: None,
            verbose: false,
            compact: false,
            current_operation: None,
        }
    }

    pub fn handle_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::AgentStarted { agent, task } => {
                // Agents appear only when they become active.
                self.status_monitor.register_agent(&agent);
                self.status_monitor
                    .update_status(&agent, AgentStatus::Thinking);
                self.status_monitor.update_task(&agent, task.clone());
                self.animation.start_activity(ActivityType::Thinking);
                self.set_operation(format!("{}: {}", agent, task));
                self.log("info", format!("Agent {} started: {}", agent, task));
            }
            AgentEvent::AgentProgress {
                agent,
                progress,
                action,
            } => {
                self.status_monitor.register_agent(&agent);
                self.status_monitor.update_progress(&agent, progress);
                self.status_monitor.update_action(&agent, action.clone());
                self.log("info", format!("{}  {}", agent, action));
            }
            AgentEvent::AgentStatusChanged { agent, status } => {
                self.status_monitor.register_agent(&agent);
                self.status_monitor.update_status(&agent, status.clone());
                self.apply_activity(&status);
                self.set_operation(format!("{}: {}", agent, status));
                self.log_verbose("info", format!("Agent {} -> {}", agent, status));
            }
            AgentEvent::ToolStarted { tool, args } => {
                self.active_tools.push_front(ToolView {
                    name: tool.clone(),
                    args_summary: sanitize_args(&args),
                    status: ToolStatus::Running,
                    result: None,
                });
                self.set_operation(format!("{} {}", tool, sanitize_args(&args)));
                self.log("tool", format!("{} {}", tool, sanitize_args(&args)));
            }
            AgentEvent::ToolCompleted {
                tool,
                result,
                success,
            } => {
                if let Some(view) = self.active_tools.iter_mut().find(|t| t.name == tool) {
                    view.status = if success {
                        ToolStatus::Completed
                    } else {
                        ToolStatus::Failed
                    };
                    view.result = Some(sanitize_result(&result));
                }
                self.log(
                    "tool",
                    format!("{} {}", tool, if success { "completed" } else { "failed" }),
                );
            }
            AgentEvent::TaskUpdated {
                description,
                status,
                ..
            } => {
                self.log("task", format!("Task {}: {}", status, description));
            }
            AgentEvent::MemoryUpdated { summary } => {
                self.memory_notifications.push_front(MemoryNotification {
                    message: summary.clone(),
                    timestamp: chrono::Local::now().to_rfc3339(),
                });
                self.log_verbose("memory", format!("Memory updated: {}", summary));
            }
            AgentEvent::SkillUpdated {
                skill,
                confidence_before,
                confidence_after,
            } => {
                self.skill_notifications.push_front(SkillNotification {
                    skill: skill.clone(),
                    confidence_before,
                    confidence_after,
                    timestamp: chrono::Local::now().to_rfc3339(),
                });
                self.log_verbose(
                    "skill",
                    format!(
                        "Skill updated: {} confidence {:.2} -> {:.2}",
                        skill, confidence_before, confidence_after
                    ),
                );
            }
            AgentEvent::AgentCompleted { agent, .. } => {
                self.status_monitor.register_agent(&agent);
                self.status_monitor
                    .update_status(&agent, AgentStatus::Completed);
                self.log("info", format!("Agent {} completed", agent));
            }
            AgentEvent::AgentFailed { agent, error } => {
                self.status_monitor.register_agent(&agent);
                self.status_monitor
                    .update_status(&agent, AgentStatus::Failed);
                self.last_error = Some(error.clone());
                self.log("error", format!("Agent {} failed: {}", agent, error));
            }
            AgentEvent::AgentCancelled { agent } => {
                self.status_monitor.register_agent(&agent);
                self.status_monitor
                    .update_status(&agent, AgentStatus::Cancelled);
                self.log("info", format!("Agent {} cancelled", agent));
            }
            AgentEvent::TaskGraphUpdated { graph } => {
                self.set_task_graph(graph);
            }
            AgentEvent::StreamChunk { content } => {
                self.push_stream_chunk(&content);
            }
            AgentEvent::PtyOutput { console, content } => {
                self.log_verbose("console", format!("{}: {}", console, content.trim_end()));
            }
            AgentEvent::PtyExited {
                console,
                exit_code,
                status,
            } => {
                self.log(
                    "console",
                    format!("{} {} (exit {})", console, status, exit_code),
                );
            }
            AgentEvent::Log { level, message } => {
                if level == "coordination" {
                    self.add_recent_message(&message);
                }
                self.log(&level, message);
            }
        }
    }

    /// Append a live stream chunk to the streaming buffer. The buffer is capped
    /// so a pathological provider response (or PTY burst) can never grow memory
    /// without bound; overflow is dropped once the cap is reached.
    pub fn push_stream_chunk(&mut self, content: &str) {
        if content.is_empty() {
            return;
        }
        let used = self.streaming_buffer.chars().count();
        if used >= MAX_STREAMING_BUFFER_CHARS {
            self.is_streaming = true;
            return;
        }
        let room = MAX_STREAMING_BUFFER_CHARS.saturating_sub(used);
        let keep: String = content.chars().take(room).collect();
        self.streaming_buffer.push_str(&keep);
        self.is_streaming = true;
    }

    /// Set the current operation (shown under the title bar).
    fn set_operation(&mut self, op: String) {
        self.current_operation = Some(op);
    }

    /// Log only when verbose mode is enabled (internal noise stays hidden in
    /// the default minimal mode).
    fn log_verbose(&mut self, level: &str, message: String) {
        if self.verbose {
            self.log(level, message);
        }
    }

    fn apply_activity(&mut self, status: &AgentStatus) {
        let activity = match status {
            AgentStatus::Searching => Some(ActivityType::Searching),
            AgentStatus::Analysing => Some(ActivityType::Analysing),
            AgentStatus::Planning => Some(ActivityType::Planning),
            AgentStatus::Executing => Some(ActivityType::Executing),
            AgentStatus::Testing => Some(ActivityType::Testing),
            AgentStatus::Reviewing => Some(ActivityType::Reviewing),
            AgentStatus::Thinking => Some(ActivityType::Thinking),
            _ => None,
        };

        match activity {
            Some(a) => self.animation.start_activity(a),
            None => self.animation.stop_activity(),
        }
    }

    pub fn log(&mut self, level: &str, message: String) {
        self.activity_log.push_front(LogEntry {
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            message,
            level: level.to_string(),
        });
        while self.activity_log.len() > self.max_log_entries {
            self.activity_log.pop_back();
        }
    }

    pub fn clear_logs(&mut self) {
        self.activity_log.clear();
    }

    pub fn set_task_graph(&mut self, graph: TaskGraph) {
        self.task_graph = Some(graph);
    }

    pub fn end_streaming(&mut self) {
        self.is_streaming = false;
        self.streaming_buffer.clear();
        self.animation.stop_activity();
    }

    pub fn set_error(&mut self, err: String) {
        self.last_error = Some(err);
    }

    pub fn clear_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    pub fn dismiss_welcome(&mut self) {
        self.show_welcome = false;
    }

    pub fn tick(&mut self) -> bool {
        self.animation.tick_if_due()
    }

    pub fn agent_entries(&self) -> Vec<AgentPanelEntry> {
        self.status_monitor
            .list()
            .into_iter()
            .map(|state| AgentPanelEntry {
                name: state.name.clone(),
                status: state.status.clone(),
                progress: state.progress,
                action: state.latest_action.clone(),
                task: state.current_task.clone(),
            })
            .collect()
    }

    pub fn graph_entries(&self) -> Vec<(String, String, TaskStatus)> {
        match &self.task_graph {
            Some(graph) => {
                let order = graph.execution_order();
                order
                    .iter()
                    .filter_map(|id| graph.get_task(id))
                    .map(|node| {
                        (
                            node.description.clone(),
                            node.agent.clone(),
                            node.status.clone(),
                        )
                    })
                    .collect()
            }
            None => Vec::new(),
        }
    }

    pub fn toggle_agents(&mut self) {
        self.show_agents = !self.show_agents;
    }

    pub fn toggle_task_graph(&mut self) {
        self.show_task_graph = !self.show_task_graph;
    }

    pub fn toggle_memory(&mut self) {
        self.show_memory = !self.show_memory;
    }

    pub fn toggle_trace(&mut self) {
        self.show_trace = !self.show_trace;
    }

    pub fn toggle_metrics(&mut self) {
        self.show_metrics = !self.show_metrics;
    }

    pub fn toggle_command_palette(&mut self) {
        self.show_command_palette = !self.show_command_palette;
        if self.show_command_palette {
            self.palette_query.clear();
            self.palette_index = 0;
        }
    }

    /// Slash-command autocomplete: pops the current completion and replaces
    /// `input` with the next matching command. Returns true if a completion
    /// was applied (so the caller can skip normal TAB behaviour).
    pub fn autocomplete_command(&mut self, input: &mut String, candidates: Vec<String>) {
        if candidates.is_empty() {
            self.autocomplete.clear();
            self.autocomplete_index = 0;
            return;
        }
        if self.autocomplete != candidates {
            self.autocomplete = candidates;
            self.autocomplete_index = 0;
        }
        if let Some(cmd) = self.autocomplete.get(self.autocomplete_index) {
            // Replace the command token, preserving any typed argument.
            let rest = match input.split_once(' ') {
                Some((_, args)) => format!("{} {}", cmd, args),
                None => cmd.to_string(),
            };
            *input = rest;
            self.autocomplete_index = (self.autocomplete_index + 1) % self.autocomplete.len();
        }
    }

    /// Apply the currently selected autocomplete entry to the input line.
    /// Returns the new input; the selection is reset.
    pub fn autocomplete_apply(&mut self, input: &str) -> Option<String> {
        let cmd = self.autocomplete.get(self.autocomplete_index)?.to_string();
        let rest = match input.split_once(' ') {
            Some((_, args)) => format!("{} {}", cmd, args),
            None => cmd,
        };
        self.autocomplete.clear();
        self.autocomplete_index = 0;
        Some(rest)
    }

    pub fn autocomplete_nav(&mut self, delta: isize) {
        let len = self.autocomplete.len();
        self.autocomplete_index = nav_wrap(self.autocomplete_index, len, delta);
    }

    pub fn autocomplete_page(&mut self, delta: isize) {
        let len = self.autocomplete.len();
        self.autocomplete_index = nav_page(self.autocomplete_index, len, delta, NAV_PAGE_SIZE);
    }

    pub fn autocomplete_home(&mut self) {
        self.autocomplete_index = 0;
    }

    pub fn autocomplete_end(&mut self) {
        self.autocomplete_index = self.autocomplete.len().saturating_sub(1);
    }

    pub fn palette_nav(&mut self, delta: isize, len: usize) {
        self.palette_index = nav_wrap(self.palette_index, len, delta);
    }

    pub fn palette_page(&mut self, delta: isize, len: usize) {
        self.palette_index = nav_page(self.palette_index, len, delta, NAV_PAGE_SIZE);
    }

    pub fn palette_home(&mut self) {
        self.palette_index = 0;
    }

    pub fn palette_end(&mut self, len: usize) {
        self.palette_index = len.saturating_sub(1);
    }

    /// Keep the palette selection valid after a filter change. The selection
    /// index is preserved when the previously selected entry still matches;
    /// otherwise it clamps to the new list length.
    pub fn palette_filter(&mut self, query: &str, entries: &[(String, &'static str)]) {
        self.palette_query = query.to_string();
        self.palette_index = nav_clamp(self.palette_index, entries.len());
    }

    pub fn toggle_coordination(&mut self) {
        self.show_coordination = !self.show_coordination;
    }

    pub fn add_recent_message(&mut self, message: &str) {
        self.recent_messages.push(message.to_string());
        if self.recent_messages.len() > 20 {
            self.recent_messages.remove(0);
        }
    }
}

impl Default for Dashboard {
    fn default() -> Self {
        Self::new()
    }
}

/// Sanitize tool arguments for the activity log: redact obvious secrets using
/// the shared authority, then truncate the summary.
fn sanitize_args(args: &str) -> String {
    crate::tools::shell::redact_secrets_public(args)
        .chars()
        .take(80)
        .collect()
}

fn sanitize_result(result: &str) -> String {
    crate::tools::shell::redact_secrets_public(result)
        .chars()
        .take(200)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::events::AgentEvent;
    use crate::agent::status::AgentStatus;

    #[test]
    fn test_dashboard_new() {
        let dashboard = Dashboard::new();
        // No agents are pre-registered: agents appear only when active.
        assert_eq!(dashboard.status_monitor.count(), 0);
        assert!(dashboard.activity_log.is_empty());
        assert!(!dashboard.show_agents);
        assert!(!dashboard.show_task_graph);
        assert!(!dashboard.verbose);
    }

    #[test]
    fn test_dashboard_handle_agent_event() {
        let mut dashboard = Dashboard::new();
        dashboard.handle_event(AgentEvent::AgentStarted {
            agent: "research".to_string(),
            task: "Find auth code".to_string(),
        });
        assert!(dashboard.animation.is_active());
        assert_eq!(dashboard.status_monitor.count(), 1);
        assert!(!dashboard.activity_log.is_empty());
    }

    #[test]
    fn test_dashboard_status_update() {
        let mut dashboard = Dashboard::new();
        dashboard.handle_event(AgentEvent::AgentStatusChanged {
            agent: "coding".to_string(),
            status: AgentStatus::Executing,
        });
        // Status change registers the agent dynamically.
        let state = dashboard.status_monitor.get("coding").unwrap();
        assert_eq!(state.status, AgentStatus::Executing);
    }

    #[test]
    fn test_dashboard_tool_tracking() {
        let mut dashboard = Dashboard::new();
        dashboard.handle_event(AgentEvent::ToolStarted {
            tool: "cargo_test".to_string(),
            args: "cargo test".to_string(),
        });
        assert_eq!(dashboard.active_tools.len(), 1);
        dashboard.handle_event(AgentEvent::ToolCompleted {
            tool: "cargo_test".to_string(),
            result: "all passed".to_string(),
            success: true,
        });
        assert_eq!(dashboard.active_tools[0].status, ToolStatus::Completed);
    }

    #[test]
    fn test_dashboard_memory_notification() {
        let mut dashboard = Dashboard::new();
        dashboard.handle_event(AgentEvent::MemoryUpdated {
            summary: "Added repo pattern".to_string(),
        });
        assert_eq!(dashboard.memory_notifications.len(), 1);
    }

    #[test]
    fn test_dashboard_skill_notification() {
        let mut dashboard = Dashboard::new();
        dashboard.handle_event(AgentEvent::SkillUpdated {
            skill: "rust_api".to_string(),
            confidence_before: 0.5,
            confidence_after: 0.8,
        });
        assert_eq!(dashboard.skill_notifications.len(), 1);
    }

    #[test]
    fn test_dashboard_streaming() {
        let mut dashboard = Dashboard::new();
        dashboard.handle_event(AgentEvent::StreamChunk {
            content: "I found the auth module".to_string(),
        });
        assert!(dashboard.is_streaming);
        assert_eq!(dashboard.streaming_buffer, "I found the auth module");
    }

    #[test]
    fn test_streaming_buffer_is_bounded() {
        let mut dashboard = Dashboard::new();
        let big = "x".repeat(200_000);
        for _ in 0..30 {
            dashboard.push_stream_chunk(&big);
        }
        assert!(
            dashboard.streaming_buffer.chars().count() <= MAX_STREAMING_BUFFER_CHARS,
            "streaming buffer grew unbounded: {}",
            dashboard.streaming_buffer.chars().count()
        );
    }

    #[test]
    fn test_dashboard_log_limit() {
        let mut dashboard = Dashboard::new();
        dashboard.max_log_entries = 5;
        for i in 0..10 {
            dashboard.log("info", format!("message {}", i));
        }
        assert_eq!(dashboard.activity_log.len(), 5);
    }

    #[test]
    fn test_dashboard_toggle() {
        let mut dashboard = Dashboard::new();
        dashboard.toggle_task_graph();
        assert!(dashboard.show_task_graph);
        dashboard.toggle_task_graph();
        assert!(!dashboard.show_task_graph);
        dashboard.toggle_memory();
        assert!(dashboard.show_memory);
    }

    #[test]
    fn test_dashboard_agent_entries() {
        let mut dashboard = Dashboard::new();
        dashboard.handle_event(AgentEvent::AgentStatusChanged {
            agent: "research".to_string(),
            status: AgentStatus::Searching,
        });
        let entries = dashboard.agent_entries();
        assert!(!entries.is_empty());
        let research = entries.iter().find(|e| e.name == "research").unwrap();
        assert_eq!(research.status, AgentStatus::Searching);
    }

    #[test]
    fn test_dashboard_clear_logs() {
        let mut dashboard = Dashboard::new();
        dashboard.log("info", "test".to_string());
        dashboard.clear_logs();
        assert!(dashboard.activity_log.is_empty());
    }

    #[test]
    fn test_dashboard_full_lifecycle() {
        // Simulate a complete task lifecycle to verify state consistency.
        let mut dashboard = Dashboard::new();

        // Task start
        dashboard.handle_event(AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: "explain repo".to_string(),
        });
        assert_eq!(
            dashboard.status_monitor.get("main").unwrap().status,
            AgentStatus::Thinking
        );

        // Progress updates
        dashboard.handle_event(AgentEvent::AgentProgress {
            agent: "main".to_string(),
            progress: 0.5,
            action: "searching".to_string(),
        });
        let state = dashboard.status_monitor.get("main").unwrap();
        assert_eq!(state.progress, 0.5);
        assert_eq!(state.latest_action.as_deref(), Some("searching"));

        // Tool execution
        dashboard.handle_event(AgentEvent::ToolStarted {
            tool: "run_command".to_string(),
            args: "cargo test".to_string(),
        });
        assert_eq!(dashboard.active_tools.len(), 1);
        assert_eq!(dashboard.active_tools[0].status, ToolStatus::Running);

        dashboard.handle_event(AgentEvent::ToolCompleted {
            tool: "run_command".to_string(),
            result: "test passed".to_string(),
            success: true,
        });
        assert_eq!(dashboard.active_tools[0].status, ToolStatus::Completed);

        // Completion
        dashboard.handle_event(AgentEvent::AgentCompleted {
            agent: "main".to_string(),
            duration_ms: 1500,
        });
        assert_eq!(
            dashboard.status_monitor.get("main").unwrap().status,
            AgentStatus::Completed
        );
    }

    #[test]
    fn test_dashboard_error_state() {
        let mut dashboard = Dashboard::new();
        dashboard.handle_event(AgentEvent::AgentFailed {
            agent: "main".to_string(),
            error: "timeout".to_string(),
        });
        assert_eq!(
            dashboard.status_monitor.get("main").unwrap().status,
            AgentStatus::Failed
        );
        assert!(dashboard.last_error.is_some());

        // Error should be clearable
        let err = dashboard.clear_error();
        assert_eq!(err, Some("timeout".to_string()));
        assert!(dashboard.last_error.is_none());
    }

    #[test]
    fn test_dashboard_welcome_dismiss() {
        let mut dashboard = Dashboard::new();
        assert!(dashboard.show_welcome);

        dashboard.dismiss_welcome();
        assert!(!dashboard.show_welcome);
    }

    #[test]
    fn test_dashboard_verbose_gates_internal_logs() {
        let mut dashboard = Dashboard::new();
        dashboard.handle_event(AgentEvent::AgentStatusChanged {
            agent: "main".to_string(),
            status: AgentStatus::Thinking,
        });
        // Minimal mode: status transitions are not logged as activity.
        assert!(dashboard.activity_log.is_empty());

        dashboard.verbose = true;
        dashboard.handle_event(AgentEvent::AgentStatusChanged {
            agent: "main".to_string(),
            status: AgentStatus::Planning,
        });
        assert!(!dashboard.activity_log.is_empty());
    }

    #[test]
    fn test_dashboard_current_operation() {
        let mut dashboard = Dashboard::new();
        assert!(dashboard.current_operation.is_none());
        dashboard.handle_event(AgentEvent::ToolStarted {
            tool: "run_command".to_string(),
            args: "cargo test".to_string(),
        });
        assert_eq!(
            dashboard.current_operation.as_deref(),
            Some("run_command cargo test")
        );
    }

    #[test]
    fn test_dashboard_pty_events() {
        let mut dashboard = Dashboard::new();
        dashboard.handle_event(AgentEvent::PtyOutput {
            console: "c1".to_string(),
            content: "building…\n".to_string(),
        });
        assert!(
            dashboard.activity_log.is_empty(),
            "PTY output is verbose-only"
        );

        dashboard.handle_event(AgentEvent::PtyExited {
            console: "c1".to_string(),
            exit_code: 0,
            status: "completed".to_string(),
        });
        assert!(
            !dashboard.activity_log.is_empty(),
            "PTY exit is always logged"
        );
    }

    // ─── Overlay navigation helpers (Sprint 30UI.3) ─────────────────────

    #[test]
    fn test_nav_wrap_forward() {
        assert_eq!(nav_wrap(0, 5, 1), 1);
        assert_eq!(nav_wrap(4, 5, 1), 0, "forward wraps to first");
        assert_eq!(nav_wrap(4, 5, 3), 2);
    }

    #[test]
    fn test_nav_wrap_backward() {
        assert_eq!(nav_wrap(0, 5, -1), 4, "backward wraps to last");
        assert_eq!(nav_wrap(3, 5, -1), 2);
        assert_eq!(nav_wrap(2, 5, -3), 4);
    }

    #[test]
    fn test_nav_wrap_empty_and_single() {
        assert_eq!(nav_wrap(0, 0, 1), 0);
        assert_eq!(nav_wrap(0, 1, 1), 0);
        assert_eq!(nav_wrap(0, 1, -1), 0);
    }

    #[test]
    fn test_nav_page_no_wrap() {
        assert_eq!(nav_page(0, 20, 1, NAV_PAGE_SIZE), 6);
        assert_eq!(nav_page(18, 20, 1, NAV_PAGE_SIZE), 19, "clamps at last");
        assert_eq!(nav_page(10, 20, -1, NAV_PAGE_SIZE), 4);
        assert_eq!(nav_page(2, 20, -1, NAV_PAGE_SIZE), 0, "clamps at first");
        assert_eq!(nav_page(0, 0, 1, NAV_PAGE_SIZE), 0);
    }

    #[test]
    fn test_nav_clamp() {
        assert_eq!(nav_clamp(7, 3), 2);
        assert_eq!(nav_clamp(1, 3), 1);
        assert_eq!(nav_clamp(0, 0), 0);
    }

    fn model_info(id: &str, source: crate::providers::ModelSource) -> ModelInfo {
        ModelInfo {
            id: id.to_string(),
            is_default: false,
            display_name: None,
            tool_calling: None,
            context_tokens: None,
            source,
        }
    }

    #[test]
    fn test_model_picker_navigation_full_cycle() {
        let mut picker = ModelPicker::new();
        picker.set_models(vec![
            model_info("a", crate::providers::ModelSource::Discovered),
            model_info("b", crate::providers::ModelSource::Discovered),
            model_info("c", crate::providers::ModelSource::Discovered),
        ]);
        assert_eq!(picker.list_state.selected_index, 0);
        picker.next();
        assert_eq!(picker.list_state.selected_index, 1);
        picker.next();
        assert_eq!(picker.list_state.selected_index, 2);
        picker.next();
        assert_eq!(picker.list_state.selected_index, 0, "Down wraps to first");
        picker.prev();
        assert_eq!(picker.list_state.selected_index, 2, "Up wraps to last");
        picker.prev();
        assert_eq!(picker.list_state.selected_index, 1);
    }

    #[test]
    fn test_model_picker_home_end() {
        let mut picker = ModelPicker::new();
        picker.set_models(vec![
            model_info("a", crate::providers::ModelSource::Discovered),
            model_info("b", crate::providers::ModelSource::Discovered),
            model_info("c", crate::providers::ModelSource::Discovered),
        ]);
        picker.end();
        assert_eq!(picker.list_state.selected_index, 2);
        picker.home();
        assert_eq!(picker.list_state.selected_index, 0);
    }

    #[test]
    fn test_model_picker_page_navigation() {
        let mut picker = ModelPicker::new();
        let models: Vec<ModelInfo> = (0..20)
            .map(|i| {
                model_info(
                    &format!("m{}", i),
                    crate::providers::ModelSource::Discovered,
                )
            })
            .collect();
        picker.set_models(models);
        picker.page_next();
        assert_eq!(picker.list_state.selected_index, NAV_PAGE_SIZE);
        picker.end();
        picker.page_next();
        assert_eq!(
            picker.list_state.selected_index, 19,
            "PageDown clamps at last"
        );
        picker.page_prev();
        assert_eq!(picker.list_state.selected_index, 13);
        picker.home();
        picker.page_prev();
        assert_eq!(
            picker.list_state.selected_index, 0,
            "PageUp clamps at first"
        );
    }

    #[test]
    fn test_model_picker_filter_keeps_selection_valid() {
        let mut picker = ModelPicker::new();
        picker.set_models(vec![
            model_info("gpt-4o", crate::providers::ModelSource::Discovered),
            model_info("gpt-4o-mini", crate::providers::ModelSource::Discovered),
            model_info(
                "deepseek-v4-flash",
                crate::providers::ModelSource::ProviderDefault,
            ),
        ]);
        picker.next();
        picker.next();
        assert_eq!(picker.list_state.selected_index, 2);
        picker.push_filter('g');
        assert_eq!(picker.visible_models().len(), 2);
        assert_eq!(
            picker.list_state.selected_index, 0,
            "filter resets selection to the new window"
        );
        picker.next();
        assert_eq!(picker.list_state.selected_index, 1);
        assert_eq!(
            picker.selected().map(|m| m.id),
            Some("gpt-4o-mini".to_string())
        );
        // Even a stale out-of-range index never selects nothing.
        picker.list_state.selected_index = 9;
        assert_eq!(
            picker.selected().map(|m| m.id),
            Some("gpt-4o-mini".to_string())
        );
    }

    #[test]
    fn test_model_picker_empty_selection() {
        let mut picker = ModelPicker::new();
        picker.set_models(Vec::new());
        assert_eq!(picker.selected(), None);
        picker.next();
        assert_eq!(picker.list_state.selected_index, 0);
    }

    #[test]
    fn test_palette_navigation_keys() {
        let mut dashboard = Dashboard::new();
        let entries = vec![
            ("//model".to_string(), "m"),
            ("//provider".to_string(), "p"),
            ("//apikey".to_string(), "k"),
            ("//settings".to_string(), "s"),
        ];
        let len = entries.len();
        dashboard.palette_nav(1, len);
        assert_eq!(dashboard.palette_index, 1);
        dashboard.palette_nav(1, len);
        dashboard.palette_nav(1, len);
        dashboard.palette_nav(1, len);
        assert_eq!(dashboard.palette_index, 0, "Down wraps around");
        dashboard.palette_nav(-1, len);
        assert_eq!(dashboard.palette_index, 3, "Up wraps to last");
        dashboard.palette_home();
        assert_eq!(dashboard.palette_index, 0);
        dashboard.palette_end(len);
        assert_eq!(dashboard.palette_index, 3);
    }

    #[test]
    fn test_palette_page_navigation() {
        let mut dashboard = Dashboard::new();
        let len = 30;
        dashboard.palette_page(1, len);
        assert_eq!(dashboard.palette_index, NAV_PAGE_SIZE);
        dashboard.palette_page(-1, len);
        assert_eq!(dashboard.palette_index, 0);
        dashboard.palette_end(len);
        dashboard.palette_page(1, len);
        assert_eq!(dashboard.palette_index, 29, "page down clamps at end");
    }

    #[test]
    fn test_palette_filter_preserves_valid_selection() {
        let mut dashboard = Dashboard::new();
        dashboard.palette_index = 3;
        // 4 entries; filter shrinks to 4+ still valid.
        let entries = vec![
            ("//model".to_string(), "Show or change the current model"),
            (
                "//provider".to_string(),
                "Show or change the current provider",
            ),
            ("//apikey".to_string(), "Set a provider API key"),
            ("//settings".to_string(), "View and edit all settings"),
        ];
        dashboard.palette_filter("", &entries);
        assert_eq!(dashboard.palette_index, 3);
        // A filter that drops everything clamps to 0 rather than panicking.
        let empty: Vec<(String, &'static str)> = Vec::new();
        dashboard.palette_filter("zzz-no-match", &empty);
        assert_eq!(dashboard.palette_index, 0);
    }

    #[test]
    fn test_autocomplete_navigation_keys() {
        let mut dashboard = Dashboard::new();
        dashboard.autocomplete = vec![
            "//model".to_string(),
            "//provider".to_string(),
            "//apikey".to_string(),
            "//settings".to_string(),
        ];
        dashboard.autocomplete_nav(1);
        assert_eq!(dashboard.autocomplete_index, 1);
        dashboard.autocomplete_nav(-1);
        assert_eq!(dashboard.autocomplete_index, 0);
        dashboard.autocomplete_nav(-1);
        assert_eq!(dashboard.autocomplete_index, 3, "Up wraps to last");
        dashboard.autocomplete_nav(1);
        assert_eq!(dashboard.autocomplete_index, 0, "Down wraps to first");
        dashboard.autocomplete_home();
        assert_eq!(dashboard.autocomplete_index, 0);
        dashboard.autocomplete_end();
        assert_eq!(dashboard.autocomplete_index, 3);
    }

    #[test]
    fn test_autocomplete_page_navigation() {
        let mut dashboard = Dashboard::new();
        dashboard.autocomplete = (0..20).map(|i| format!("//cmd{}", i)).collect();
        dashboard.autocomplete_page(1);
        assert_eq!(dashboard.autocomplete_index, NAV_PAGE_SIZE);
        dashboard.autocomplete_end();
        dashboard.autocomplete_page(1);
        assert_eq!(dashboard.autocomplete_index, 19, "clamps at end");
        dashboard.autocomplete_page(-1);
        assert_eq!(dashboard.autocomplete_index, 13);
        dashboard.autocomplete_home();
        dashboard.autocomplete_page(-1);
        assert_eq!(dashboard.autocomplete_index, 0, "clamps at start");
    }

    #[test]
    fn test_autocomplete_apply_replaces_token() {
        let mut dashboard = Dashboard::new();
        dashboard.autocomplete = vec!["//verbose".to_string()];
        dashboard.autocomplete_index = 0;
        let applied = dashboard.autocomplete_apply("//verb").unwrap();
        assert_eq!(applied, "//verbose");
        assert!(dashboard.autocomplete.is_empty(), "selection consumed");
        assert_eq!(dashboard.autocomplete_index, 0);
    }

    #[test]
    fn test_autocomplete_apply_preserves_args() {
        let mut dashboard = Dashboard::new();
        dashboard.autocomplete = vec!["//model".to_string()];
        dashboard.autocomplete_index = 0;
        let applied = dashboard.autocomplete_apply("//m gpt-4o").unwrap();
        assert_eq!(applied, "//model gpt-4o");
    }
}
