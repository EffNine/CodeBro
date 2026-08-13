//! UI-level semantic action model.
//!
//! Runtime events are mapped deterministically into a bounded stream of
//! phase-grouped actions that the renderer draws inside the chat. The model
//! never invents state: every action, status, and metric below is derived from
//! a real [`AgentEvent`] (tool lifecycle, PTY exit codes, agent lifecycle,
//! machine-fact logs). Nothing is fabricated.
//!
//! ```text
//! Runtime Events  ──►  ActionStream (this module)  ──►  TUI renderer
//! ```

use std::collections::{BTreeSet, VecDeque};
use std::time::Instant;

use crate::agent::events::AgentEvent;
use crate::agent::status::AgentStatus;

use crate::tui::theme::{Phase, StatusGlyph};

/// Maximum number of phase groups retained in the chat history. Older groups
/// are dropped (their summary lines remain in the conversation where the
/// renderer collapsed them) so memory stays bounded for long sessions.
pub const MAX_GROUPS: usize = 48;
/// Maximum number of actions retained per group.
pub const MAX_ACTIONS_PER_GROUP: usize = 128;
/// Maximum tail of live PTY output kept on a running command action.
pub const MAX_LIVE_OUTPUT_CHARS: usize = 4_000;
/// Maximum distinct files tracked in the CONTEXT metric. A pathological
/// `list_files` result cannot grow memory without bound; once the cap is
/// reached new entries are ignored (the count stays real, never fabricated).
pub const MAX_FILES_TRACKED: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiActionKind {
    Thinking,
    ListingFiles,
    ReadingFile,
    Searching,
    Git,
    Planning,
    Editing,
    RunningCommand,
    Testing,
    Reviewing,
    Verification,
    Permission,
    Waiting,
    Result,
}

impl UiActionKind {
    /// The semantic emoji. Emoji is an enhancement; the label always remains.
    pub fn emoji(self) -> &'static str {
        match self {
            UiActionKind::Thinking => "🧠",
            UiActionKind::ListingFiles => "📂",
            UiActionKind::ReadingFile => "📖",
            UiActionKind::Searching => "🔎",
            UiActionKind::Git => "🌳",
            UiActionKind::Planning => "🗺",
            UiActionKind::Editing => "✏",
            UiActionKind::RunningCommand => "⚙",
            UiActionKind::Testing => "🧪",
            UiActionKind::Reviewing => "🔍",
            UiActionKind::Verification => "🧪",
            UiActionKind::Permission => "🛡",
            UiActionKind::Waiting => "⏳",
            UiActionKind::Result => "📊",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            UiActionKind::Thinking => "Thinking",
            UiActionKind::ListingFiles => "List Files",
            UiActionKind::ReadingFile => "Read File",
            UiActionKind::Searching => "Search",
            UiActionKind::Git => "Git",
            UiActionKind::Planning => "Planning",
            UiActionKind::Editing => "Editing",
            UiActionKind::RunningCommand => "Run Command",
            UiActionKind::Testing => "Testing",
            UiActionKind::Reviewing => "Reviewing",
            UiActionKind::Verification => "Verification",
            UiActionKind::Permission => "Permission",
            UiActionKind::Waiting => "Waiting",
            UiActionKind::Result => "Result",
        }
    }

    /// Map a real tool name onto the semantic vocabulary.
    pub fn from_tool(tool: &str) -> UiActionKind {
        let t = tool.to_lowercase();
        if t.contains("read") {
            UiActionKind::ReadingFile
        } else if t.contains("list") {
            UiActionKind::ListingFiles
        } else if t.contains("search") || t.contains("grep") {
            UiActionKind::Searching
        } else if t.contains("git") {
            UiActionKind::Git
        } else if t.contains("edit")
            || t.contains("write")
            || t.contains("create")
            || t.contains("patch")
            || t.contains("propose")
            || t.contains("apply")
        {
            UiActionKind::Editing
        } else if t.contains("test") || t.contains("playwright") {
            UiActionKind::Testing
        } else if t.contains("verify") {
            UiActionKind::Verification
        } else if t.contains("run")
            || t.contains("command")
            || t.contains("shell")
            || t.contains("exec")
        {
            UiActionKind::RunningCommand
        } else if t.contains("permission") || t.contains("approve") {
            UiActionKind::Permission
        } else {
            UiActionKind::Result
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiActionStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    Warning,
}

impl UiActionStatus {
    pub fn glyph(self) -> StatusGlyph {
        match self {
            UiActionStatus::Pending => StatusGlyph::Pending,
            UiActionStatus::Running => StatusGlyph::Running,
            UiActionStatus::Completed => StatusGlyph::Completed,
            UiActionStatus::Failed => StatusGlyph::Failed,
            UiActionStatus::Cancelled => StatusGlyph::Cancelled,
            UiActionStatus::Warning => StatusGlyph::Warning,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            UiActionStatus::Completed
                | UiActionStatus::Failed
                | UiActionStatus::Cancelled
                | UiActionStatus::Warning
        )
    }
}

/// A single tool/agent action shown as one compact line inside a phase group.
#[derive(Debug, Clone)]
pub struct UiAction {
    pub kind: UiActionKind,
    pub status: UiActionStatus,
    /// Tool name (e.g. `read_file`) or activity label.
    pub title: String,
    /// Compact detail (path, command, pattern). Always redacted/truncated.
    pub detail: String,
    /// Short result summary produced only from machine facts (exit codes,
    /// tool success flags). Never parsed from prose.
    pub result_summary: Option<String>,
    /// Authoritative exit code when a PTY-backed process finished.
    pub exit_code: Option<i32>,
    pub started_at: Option<Instant>,
    pub completed_at: Option<Instant>,
    /// Bounded tail of live PTY output attached to this action.
    live_output: String,
}

impl UiAction {
    fn new(kind: UiActionKind, title: String, detail: String) -> Self {
        UiAction {
            kind,
            status: UiActionStatus::Running,
            title,
            detail,
            result_summary: None,
            exit_code: None,
            started_at: Some(Instant::now()),
            completed_at: None,
            live_output: String::new(),
        }
    }

    fn append_output(&mut self, chunk: &str) {
        self.live_output.push_str(chunk);
        if self.live_output.chars().count() > MAX_LIVE_OUTPUT_CHARS {
            let drop = self.live_output.chars().count() - MAX_LIVE_OUTPUT_CHARS;
            let mut rest = self.live_output.clone();
            rest.drain(..drop);
            self.live_output = rest;
        }
    }

    /// Bounded tail of live PTY output attached to this action.
    pub fn live_output(&self) -> &str {
        &self.live_output
    }

    fn finish(&mut self, status: UiActionStatus, summary: Option<String>, exit_code: Option<i32>) {
        self.status = status;
        self.completed_at = Some(Instant::now());
        if exit_code.is_some() {
            self.exit_code = exit_code;
        }
        self.result_summary = summary;
        self.live_output.clear();
    }
}

/// Machine facts about the final verification of applied changes. Only ever
/// populated from authoritative sources (exit codes, tool success, the
/// runtime's machine-fact verification log).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerificationFacts {
    pub exit_code: Option<i32>,
    pub success: Option<bool>,
    pub denied: Option<bool>,
    pub timeout: Option<bool>,
}

impl VerificationFacts {
    pub fn summary(&self) -> Option<String> {
        if let Some(code) = self.exit_code {
            if code == 0 {
                Some(format!("exit 0"))
            } else {
                Some(format!("exit {}", code))
            }
        } else if let Some(success) = self.success {
            Some(if success {
                "verified".to_string()
            } else if self.denied.unwrap_or(false) {
                "denied".to_string()
            } else if self.timeout.unwrap_or(false) {
                "timed out".to_string()
            } else {
                "failed".to_string()
            })
        } else {
            None
        }
    }
}

/// A bounded group of actions belonging to one phase of the task.
#[derive(Debug, Clone)]
pub struct UiActionGroup {
    pub phase: Phase,
    pub status: UiActionStatus,
    /// The task string when the agent announced it (real task text).
    pub task: Option<String>,
    /// The agent name that owns the group, when known.
    pub agent: Option<String>,
    pub started_at: Instant,
    /// Real duration from `AgentCompleted { duration_ms }`.
    pub duration_ms: Option<u64>,
    /// Failure/cancellation detail when the group did not complete.
    pub outcome: Option<String>,
    pub actions: VecDeque<UiAction>,
    /// Whether the group is drawn expanded (running groups are always
    /// expanded; completed groups collapse unless the user expands them).
    pub expanded: bool,
    /// Machine-fact verification outcome for the group, when available.
    pub verification: Option<VerificationFacts>,
}

impl UiActionGroup {
    fn new(phase: Phase, task: Option<String>, agent: Option<String>) -> Self {
        UiActionGroup {
            phase,
            status: UiActionStatus::Running,
            task,
            agent,
            started_at: Instant::now(),
            duration_ms: None,
            outcome: None,
            actions: VecDeque::new(),
            expanded: true,
            verification: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.status == UiActionStatus::Running
            || self
                .actions
                .iter()
                .any(|a| a.status == UiActionStatus::Running)
    }

    pub fn has_failure(&self) -> bool {
        self.status == UiActionStatus::Failed
            || self
                .actions
                .iter()
                .any(|a| a.status == UiActionStatus::Failed)
    }

    /// Real elapsed wall time (from events, not fabricated).
    pub fn elapsed_ms(&self) -> u64 {
        self.duration_ms
            .unwrap_or_else(|| self.started_at.elapsed().as_millis() as u64)
    }

    /// Distinct files mentioned by reading/listing/editing actions.
    pub fn files(&self) -> BTreeSet<&str> {
        self.actions
            .iter()
            .filter(|a| {
                matches!(
                    a.kind,
                    UiActionKind::ReadingFile | UiActionKind::ListingFiles | UiActionKind::Editing
                )
            })
            .filter_map(|a| {
                let d = a.detail.trim();
                if d.is_empty() {
                    None
                } else {
                    Some(d)
                }
            })
            .collect()
    }

    pub fn count_of(&self, kind: UiActionKind) -> usize {
        self.actions.iter().filter(|a| a.kind == kind).count()
    }

    /// Test outcome counts derived from real exit codes. ONLY actions whose
    /// kind is Testing or Verification (i.e. real test/verify tools such as
    /// `run_tests`, `verify`, `playwright_test`) count as tests. Plain
    /// `run_command` executions (cargo check, cargo build, cargo fmt, …) are
    /// commands, not tests: classifying them by parsing the command string
    /// would be a heuristic, so it is never done.
    pub fn test_counts(&self) -> (u32, u32) {
        let mut passed = 0;
        let mut total = 0;
        for a in &self.actions {
            if matches!(a.kind, UiActionKind::Testing | UiActionKind::Verification)
                && a.completed_at.is_some()
            {
                if let Some(code) = a.exit_code {
                    total += 1;
                    if code == 0 {
                        passed += 1;
                    }
                }
            }
        }
        (passed, total)
    }

    /// A compact one-line summary used when the group is collapsed. Built from
    /// real counts only.
    pub fn summary_line(&self) -> String {
        let phase = self.phase.label();
        let outcome = match self.status {
            UiActionStatus::Completed => format!("✓ {}", phase),
            UiActionStatus::Failed => format!("✗ {} failed", phase),
            UiActionStatus::Cancelled => format!("⏸ {} cancelled", phase),
            UiActionStatus::Warning => format!("⚠ {} warning", phase),
            _ => format!("● {} running", phase),
        };
        let mut parts: Vec<String> = Vec::new();
        let files = self.files().len();
        if files > 0 {
            parts.push(format!(
                "{} file{}",
                files,
                if files == 1 { "" } else { "s" }
            ));
        }
        let commands =
            self.count_of(UiActionKind::RunningCommand) + self.count_of(UiActionKind::Testing);
        if commands > 0 {
            parts.push(format!(
                "{} command{}",
                commands,
                if commands == 1 { "" } else { "s" }
            ));
        }
        let edits = self.count_of(UiActionKind::Editing);
        if edits > 0 {
            parts.push(format!(
                "{} edit{}",
                edits,
                if edits == 1 { "" } else { "s" }
            ));
        }
        let reads = self.count_of(UiActionKind::ReadingFile);
        if reads > 0 {
            parts.push(format!(
                "{} read{}",
                reads,
                if reads == 1 { "" } else { "s" }
            ));
        }
        let (passed, total) = self.test_counts();
        if total > 0 {
            parts.push(format!(
                "{} test{} passed",
                passed,
                if passed == 1 { "" } else { "s" }
            ));
        }
        if let Some(v) = &self.verification {
            if let Some(s) = v.summary() {
                parts.push(format!("verification {}", s));
            }
        }
        if parts.is_empty() {
            let n = self.actions.len();
            if n > 0 {
                return outcome + &format!(" · {} action{}", n, if n == 1 { "" } else { "s" });
            }
            return outcome;
        }
        outcome + " · " + &parts.join(" · ")
    }
}

/// The chat's bounded action history plus the derived intelligence-rail
/// counters. Owned by `TuiApp`; mutated only from real events.
#[derive(Debug, Clone)]
pub struct ActionStream {
    pub groups: VecDeque<UiActionGroup>,
    current_agent: Option<String>,
    current_phase: Phase,
    main_status: Option<AgentStatus>,
    /// Focused group index (from the back) for expand/collapse navigation.
    pub focused_from_back: Option<usize>,
    /// Real task graph snapshot for the rail's progress section.
    pub task_graph: Option<crate::agent::task_graph::TaskGraph>,
    // ---- Rail context counters (all event-derived) ----
    pub files_inspected: BTreeSet<String>,
    pub tools_used: BTreeSet<String>,
    pub tests_passed: u32,
    pub tests_total: u32,
    pub failures: u32,
    pub tool_calls: u32,
    pub last_activity: Option<Instant>,
}

impl Default for ActionStream {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionStream {
    pub fn new() -> Self {
        ActionStream {
            groups: VecDeque::new(),
            current_agent: None,
            current_phase: Phase::Main,
            main_status: None,
            focused_from_back: None,
            task_graph: None,
            files_inspected: BTreeSet::new(),
            tools_used: BTreeSet::new(),
            tests_passed: 0,
            tests_total: 0,
            failures: 0,
            tool_calls: 0,
            last_activity: None,
        }
    }

    pub fn phase_for(agent: &str) -> Option<Phase> {
        match agent {
            "research" => Some(Phase::Research),
            "testing" => Some(Phase::Testing),
            "planning" => Some(Phase::Planning),
            "coding" => Some(Phase::Coding),
            "review" => Some(Phase::Review),
            _ => None,
        }
    }

    fn phase_from_main_status(status: &AgentStatus) -> Phase {
        match status {
            AgentStatus::Thinking => Phase::Main,
            AgentStatus::Searching => Phase::Research,
            AgentStatus::Planning => Phase::Planning,
            AgentStatus::Executing => Phase::Coding,
            AgentStatus::Testing => Phase::Verification,
            AgentStatus::Reviewing => Phase::Review,
            _ => Phase::Main,
        }
    }

    pub fn current_phase(&self) -> Phase {
        self.current_phase
    }

    pub fn has_running(&self) -> bool {
        self.groups.iter().any(|g| g.is_running())
    }

    /// Mark every phase-grouping ended by the user's Ctrl+C. Preserves partial
    /// results: completed actions keep their facts.
    pub fn cancel_active(&mut self) {
        for group in self.groups.iter_mut() {
            if group.duration_ms.is_none() {
                group.duration_ms = Some(group.started_at.elapsed().as_millis() as u64);
            }
            if !group.status.is_terminal() {
                group.status = UiActionStatus::Cancelled;
                group.outcome = Some("Cancelled by user".to_string());
            }
            for action in group.actions.iter_mut() {
                if action.status == UiActionStatus::Running {
                    action.status = UiActionStatus::Cancelled;
                    action.completed_at = Some(Instant::now());
                    action.live_output.clear();
                }
            }
            // Cancelled groups collapse to their summary; partial results stay
            // inspectable via expand.
            group.expanded = false;
        }
        self.current_agent = None;
        self.touch();
    }

    /// Finalize the in-flight group(s) when the task's final response arrives.
    /// `success` comes from the runtime task result.
    ///
    /// Invariant: a completed group must never carry Running actions — the
    /// parent response has definitively ended, so outstanding actions
    /// transition out of Running too. No action result is fabricated; only
    /// the lifecycle state moves to the parent's outcome.
    pub fn finalize_response(&mut self, success: bool) {
        for group in self.groups.iter_mut() {
            if group.duration_ms.is_none() {
                group.duration_ms = Some(group.started_at.elapsed().as_millis() as u64);
            }
            if !group.status.is_terminal() {
                group.status = if success {
                    UiActionStatus::Completed
                } else {
                    UiActionStatus::Failed
                };
                if !success && group.outcome.is_none() {
                    group.outcome = Some("Task failed".to_string());
                }
            }
            for action in group.actions.iter_mut() {
                if action.status == UiActionStatus::Running {
                    action.completed_at = Some(Instant::now());
                    action.status = if success {
                        UiActionStatus::Completed
                    } else {
                        UiActionStatus::Failed
                    };
                    action.live_output.clear();
                }
            }
            // Completed groups auto-collapse; failures/warnings stay expanded
            // until the user addresses them (design-spec progressive disclosure).
            group.expanded = group.has_failure()
                || matches!(group.status, UiActionStatus::Warning);
        }
        self.current_agent = None;
        self.touch();
    }

    /// Take ownership of all groups (used to seal a turn onto a user message).
    /// Counters and focus reset; the stream is ready for the next turn.
    pub fn take_groups(&mut self) -> VecDeque<UiActionGroup> {
        let groups = std::mem::take(&mut self.groups);
        self.focused_from_back = None;
        self.current_agent = None;
        self.current_phase = Phase::Main;
        groups
    }

    pub fn touch(&mut self) {
        self.last_activity = Some(Instant::now());
    }

    // ─── Group management ──────────────────────────────────────────────

    fn active_group(&mut self) -> Option<&mut UiActionGroup> {
        self.groups
            .iter_mut()
            .rev()
            .find(|g| !g.status.is_terminal())
    }

    /// Returns the index (from the back) of the group for `phase`, creating
    /// one when no active group exists yet.
    fn group_for(&mut self, phase: Phase, agent: Option<String>) -> usize {
        // Prefer the active (non-terminal) group of the same phase.
        if let Some(idx) = self
            .groups
            .iter()
            .rposition(|g| !g.status.is_terminal() && g.phase == phase)
        {
            let from_back = self.groups.len() - 1 - idx;
            if let Some(g) = self.groups.back_mut() {
                // Keep the tail group's task text in sync when present.
                if let Some(agent) = &agent {
                    if g.agent.is_none() && g.phase == phase {
                        g.agent = Some(agent.clone());
                    }
                }
            }
            let _ = from_back;
            return idx;
        }
        self.groups
            .push_back(UiActionGroup::new(phase, None, agent));
        while self.groups.len() > MAX_GROUPS {
            self.groups.pop_front();
        }
        self.groups.len() - 1
    }

    fn push_action(&mut self, group_idx: usize, action: UiAction) {
        if let Some(group) = self.groups.get_mut(group_idx) {
            group.actions.push_back(action);
            while group.actions.len() > MAX_ACTIONS_PER_GROUP {
                group.actions.pop_front();
            }
        }
        self.touch();
    }

    /// Open an action in the phase group that is currently receiving events.
    /// Main-loop tool events are routed to the semantically-matching phase.
    pub fn begin_tool_action(&mut self, tool: &str, args: &str) {
        let kind = UiActionKind::from_tool(tool);
        self.tools_used.insert(tool.to_string());
        self.tool_calls += 1;

        let detail = summarize_args(args);
        if !detail.is_empty() && matches!(kind, UiActionKind::ReadingFile) {
            self.files_inspected.insert(detail.clone());
        }
        if !detail.is_empty() && matches!(kind, UiActionKind::Editing) {
            self.files_inspected.insert(detail.clone());
        }

        // Main agent's acting-phase tools land in the phase their semantics
        // belong to; specialist events stay in the specialist's phase group.
        let phase = match (self.current_agent.as_deref(), self.current_phase) {
            (Some("main"), current) => match kind {
                UiActionKind::Editing | UiActionKind::Permission => Phase::Coding,
                UiActionKind::Testing => Phase::Testing,
                UiActionKind::Verification => Phase::Verification,
                UiActionKind::ReadingFile
                | UiActionKind::ListingFiles
                | UiActionKind::Searching
                | UiActionKind::Git => Phase::Research,
                _ => current,
            },
            _ => self.current_phase,
        };

        let idx = self.group_for(phase, self.current_agent.clone());
        let action = UiAction::new(kind, tool.to_string(), detail);
        self.push_action(idx, action);
    }

    /// Complete the most recent running action matching `tool`.
    pub fn complete_tool_action(
        &mut self,
        tool: &str,
        result: &str,
        success: bool,
    ) -> Option<usize> {
        let kind = UiActionKind::from_tool(tool);
        let summary = summarize_result(result, kind);
        let mut done = false;
        for group in self.groups.iter_mut().rev() {
            for action in group.actions.iter_mut().rev() {
                if action.title == tool && action.kind == kind {
                    // An already-finalized action (e.g. finalized by PtyExited
                    // with the authoritative exit code) only receives the
                    // missing result summary — it is never duplicated.
                    if action.status == UiActionStatus::Running {
                        // PTY-backed commands are finalized authoritatively by
                        // PtyExited (exit code); a non-PTY tool's own success
                        // flag is authoritative for it. Failures are counted
                        // once.
                        let failed =
                            if matches!(kind, UiActionKind::Verification | UiActionKind::Testing) {
                                if action.exit_code.is_some() {
                                    action.exit_code != Some(0)
                                } else {
                                    !success
                                }
                            } else {
                                !success
                            };
                        let status = if failed {
                            UiActionStatus::Failed
                        } else {
                            UiActionStatus::Completed
                        };
                        action.finish(status, summary.clone(), None);
                        if failed && action.exit_code.is_none() {
                            self.failures += 1;
                        }
                    } else if action.result_summary.is_none() {
                        action.result_summary = summary.clone();
                    }
                    done = true;
                    break;
                }
            }
            if done {
                break;
            }
        }
        // A successful list_files returns real path entries (one per line);
        // reflect them in the CONTEXT file count without guessing.
        if success && matches!(kind, UiActionKind::ListingFiles) {
            self.record_listed_files(result);
        }
        if !done {
            // A completion without a matching start (rare) still surfaces as a
            // completed action so facts are never lost.
            let phase = self.current_phase;
            let idx = self.group_for(phase, self.current_agent.clone());
            let mut action = UiAction::new(kind, tool.to_string(), summarize_args(result));
            action.finish(
                if success {
                    UiActionStatus::Completed
                } else {
                    UiActionStatus::Failed
                },
                summary,
                None,
            );
            if !success {
                self.failures += 1;
            }
            self.push_action(idx, action);
        }
        self.touch();
        None
    }

    /// Attach live PTY output to the running command action (bounded tail).
    pub fn pty_output(&mut self, content: &str) {
        if content.is_empty() {
            return;
        }
        if let Some(group) = self
            .groups
            .iter_mut()
            .rev()
            .find(|g| !g.status.is_terminal())
        {
            if let Some(action) = group
                .actions
                .iter_mut()
                .rev()
                .find(|a| a.status == UiActionStatus::Running)
            {
                action.append_output(content);
            }
        }
        self.touch();
    }

    /// Reflect a successful `list_files` result in the CONTEXT file count.
    /// The ListFiles tool returns real path entries (one per line); header-ish
    /// and empty lines are skipped. When the result cannot be parsed safely
    /// nothing is added — the count is never fabricated.
    fn record_listed_files(&mut self, result: &str) {
        if result.trim().is_empty() {
            return;
        }
        for line in result.lines() {
            if self.files_inspected.len() >= MAX_FILES_TRACKED {
                break;
            }
            let path = line.trim();
            if path.is_empty()
                || path.starts_with('[')
                || path.contains("===")
                || path.starts_with("Error")
            {
                continue;
            }
            self.files_inspected.insert(truncate(path, 120));
        }
    }

    /// Finalize the running command action from the authoritative PTY exit.
    pub fn pty_exited(&mut self, exit_code: i32, status: &str) {
        let action_status = match status {
            "cancelled" => UiActionStatus::Cancelled,
            "timed out" => UiActionStatus::Warning,
            "error" => UiActionStatus::Failed,
            _ if exit_code == 0 => UiActionStatus::Completed,
            _ => UiActionStatus::Failed,
        };
        // The summary line is human-readable; the raw machine exit code stays
        // on the action. `exit -1` must never be presented as a real process
        // exit: a timeout is "timed out", not "exit -1".
        let summary = Some(match status {
            "timed out" => "timed out".to_string(),
            "cancelled" => "cancelled".to_string(),
            "error" => "error".to_string(),
            _ if exit_code == 0 => "exit 0".to_string(),
            _ => format!("exit {}", exit_code),
        });

        let mut matched = false;
        if let Some(group) = self
            .groups
            .iter_mut()
            .rev()
            .find(|g| !g.status.is_terminal())
        {
            if let Some(action) = group
                .actions
                .iter_mut()
                .rev()
                .find(|a| a.status == UiActionStatus::Running)
            {
                action.finish(action_status, summary.clone(), Some(exit_code));
                matched = true;
            }
        }
        if !matched {
            // A PTY exit without a tracked start: surface the machine fact.
            let phase = self.current_phase;
            let idx = self.group_for(phase, self.current_agent.clone());
            let mut action = UiAction::new(
                UiActionKind::RunningCommand,
                "run_command".to_string(),
                String::new(),
            );
            action.finish(action_status, summary, Some(exit_code));
            self.push_action(idx, action);
        }

        self.tests_total += 1;
        if exit_code == 0 {
            self.tests_passed += 1;
        } else if !matches!(status, "cancelled" | "timed out") {
            self.failures += 1;
        }
        self.touch();
    }

    // ─── Expand / collapse navigation ──────────────────────────────────

    /// Cycle the focused group (from the back). Returns true when the focus
    /// moved so the caller can redraw. First focus lands on the newest group
    /// (`from_back = 0`).
    pub fn cycle_focus(&mut self, forward: bool) -> bool {
        if self.groups.is_empty() {
            self.focused_from_back = None;
            return false;
        }
        let n = self.groups.len();
        let next = match self.focused_from_back {
            None => 0, // first Tab: newest group
            Some(cur) => {
                let cur = cur.min(n - 1);
                if forward {
                    if cur == n - 1 {
                        0
                    } else {
                        cur + 1
                    }
                } else if cur == 0 {
                    n - 1
                } else {
                    cur - 1
                }
            }
        };
        self.focused_from_back = Some(next);
        true
    }

    /// Toggle the focused group between expanded and collapsed. Returns true
    /// when the group was found.
    pub fn toggle_focused(&mut self) -> bool {
        let Some(from_back) = self.focused_from_back else {
            return false;
        };
        let n = self.groups.len();
        if from_back >= n {
            self.focused_from_back = None;
            return false;
        }
        let idx = n - 1 - from_back;
        if let Some(group) = self.groups.get_mut(idx) {
            group.expanded = !group.expanded;
            return true;
        }
        false
    }

    /// Collapse completed groups (used automatically when height is tight).
    pub fn collapse_completed(&mut self) {
        for group in self.groups.iter_mut() {
            if group.status.is_terminal() && !group.has_failure() {
                group.expanded = false;
            }
        }
    }

    /// The most important current activity for the rail: the newest running
    /// action, else the newest action of the active group.
    pub fn current_activity(&self) -> Option<(&UiActionGroup, &UiAction)> {
        for group in self.groups.iter().rev() {
            if let Some(action) = group
                .actions
                .iter()
                .rev()
                .find(|a| a.status == UiActionStatus::Running)
            {
                return Some((group, action));
            }
        }
        for group in self.groups.iter().rev() {
            if !group.status.is_terminal() {
                if let Some(action) = group.actions.back() {
                    return Some((group, action));
                }
            }
        }
        None
    }

    /// Whether any new activity arrived while the user scrolled up.
    pub fn has_live_activity(&self) -> bool {
        self.has_running()
    }

    /// Reset the action history (used by `//clear`).
    pub fn clear(&mut self) {
        *self = ActionStream::new();
    }

    /// Deterministic event → action mapping. Must stay pure: it never touches
    /// files, providers, or wall-clock state beyond real event payloads.
    pub fn handle_event(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::AgentStarted { agent, task } => {
                self.current_agent = Some(agent.clone());
                let phase = Self::phase_for(agent).unwrap_or(Phase::Main);
                self.current_phase = phase;
                self.main_status = Some(AgentStatus::Thinking);
                let idx = self.group_for(phase, Some(agent.clone()));
                if let Some(group) = self.groups.get_mut(idx) {
                    group.task = Some(task.clone());
                }
                self.touch();
            }
            AgentEvent::AgentStatusChanged { agent, status } => {
                self.current_agent = Some(agent.clone());
                let phase =
                    Self::phase_for(agent).unwrap_or_else(|| Self::phase_from_main_status(status));
                self.current_phase = phase;
                self.main_status = Some(status.clone());
                if agent == "main" && *status == AgentStatus::Thinking {
                    let idx = self.group_for(Phase::Main, Some("main".to_string()));
                    let action = UiAction::new(
                        UiActionKind::Thinking,
                        "thinking".to_string(),
                        String::new(),
                    );
                    self.push_action(idx, action);
                }
                self.touch();
            }
            AgentEvent::AgentProgress { action, .. } => {
                self.touch();
                if !action.is_empty() {
                    self.current_phase = self.current_phase;
                }
            }
            AgentEvent::ToolStarted { tool, args } => {
                self.begin_tool_action(tool, args);
            }
            AgentEvent::ToolCompleted {
                tool,
                result,
                success,
            } => {
                self.complete_tool_action(tool, result, *success);
            }
            AgentEvent::PtyOutput { content, .. } => {
                self.pty_output(content);
            }
            AgentEvent::PtyExited {
                exit_code, status, ..
            } => {
                self.pty_exited(*exit_code, status);
            }
            AgentEvent::AgentCompleted { agent, duration_ms } => {
                if let Some(phase) = Self::phase_for(agent) {
                    self.finish_agent_group(
                        phase,
                        UiActionStatus::Completed,
                        None,
                        Some(*duration_ms),
                    );
                } else if agent == "main" {
                    // Main-agent terminal events always target the Main phase
                    // group — never the last active specialist phase.
                    self.finish_agent_group(
                        Phase::Main,
                        UiActionStatus::Completed,
                        None,
                        Some(*duration_ms),
                    );
                }
                self.current_agent = None;
                self.touch();
            }
            AgentEvent::AgentFailed { agent, error } => {
                self.failures += 1;
                if let Some(phase) = Self::phase_for(agent) {
                    self.finish_agent_group(phase, UiActionStatus::Failed, Some(error), None);
                } else if agent == "main" {
                    self.finish_agent_group(Phase::Main, UiActionStatus::Failed, Some(error), None);
                }
                self.touch();
            }
            AgentEvent::AgentCancelled { agent } => {
                if let Some(phase) = Self::phase_for(agent) {
                    self.finish_agent_group(phase, UiActionStatus::Cancelled, None, None);
                } else if agent == "main" {
                    self.finish_agent_group(Phase::Main, UiActionStatus::Cancelled, None, None);
                }
                self.touch();
            }
            AgentEvent::TaskGraphUpdated { graph } => {
                self.task_graph = Some(graph.clone());
                self.touch();
            }
            AgentEvent::Log { level, message } => {
                if level == "coding" && message.contains("verification completed") {
                    let facts = parse_verification_facts(message);
                    if let Some(facts) = facts {
                        self.attach_verification(facts);
                    }
                }
                self.touch();
            }
            _ => {
                self.touch();
            }
        }
    }

    fn finish_agent_group(
        &mut self,
        phase: Phase,
        status: UiActionStatus,
        outcome: Option<&str>,
        duration_ms: Option<u64>,
    ) {
        if let Some(idx) = self
            .groups
            .iter()
            .rposition(|g| g.phase == phase && !g.status.is_terminal())
        {
            let group = self.groups.get_mut(idx).expect("group exists");
            group.status = status;
            if let Some(ms) = duration_ms {
                group.duration_ms = Some(ms);
            }
            group.outcome = outcome.map(|o| o.to_string());
            // Completed groups collapse to their summary line; a group that
            // carries failures stays expanded so the details remain visible.
            group.expanded = group.has_failure();
            // Running actions inside a completed group are finalized honestly:
            // a completed agent means its outstanding actions ended.
            for action in group.actions.iter_mut() {
                if action.status == UiActionStatus::Running {
                    action.completed_at = Some(Instant::now());
                    action.status = match status {
                        UiActionStatus::Failed => UiActionStatus::Failed,
                        UiActionStatus::Cancelled => UiActionStatus::Cancelled,
                        _ => UiActionStatus::Completed,
                    };
                }
            }
        }
    }

    fn attach_verification(&mut self, facts: VerificationFacts) {
        if let Some(idx) = self
            .groups
            .iter()
            .rposition(|g| g.phase == Phase::Verification && !g.status.is_terminal())
        {
            if let Some(group) = self.groups.get_mut(idx) {
                group.verification = Some(facts.clone());
            }
        } else if let Some(idx) = self
            .groups
            .iter()
            .rposition(|g| g.phase == Phase::Coding && !g.status.is_terminal())
        {
            if let Some(group) = self.groups.get_mut(idx) {
                group.verification = Some(facts.clone());
            }
        }
    }
}

/// Parse the runtime's machine-fact verification log line:
/// `Coding verification completed: exit=0 success=true denied=false timeout=false`
fn parse_verification_facts(message: &str) -> Option<VerificationFacts> {
    let mut facts = VerificationFacts::default();
    for part in message.split_whitespace() {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        match key {
            "exit" => facts.exit_code = value.parse::<i32>().ok(),
            "success" => facts.success = value.parse::<bool>().ok(),
            "denied" => facts.denied = value.parse::<bool>().ok(),
            "timeout" => facts.timeout = value.parse::<bool>().ok(),
            _ => {}
        }
    }
    if facts.exit_code.is_none() && facts.success.is_none() {
        return None;
    }
    Some(facts)
}

/// Extract a compact, redacted detail from a tool argument blob.
fn summarize_args(args: &str) -> String {
    let args = crate::tools::shell::redact_secrets_public(args);
    let args = args.trim();
    if let Some(path) = extract_json_string(&args, "path") {
        return truncate(&path, 72);
    }
    if let Some(command) = extract_json_string(&args, "command") {
        return truncate(&command, 72);
    }
    if let Some(pattern) = extract_json_string(&args, "pattern") {
        return truncate(&pattern, 48);
    }
    if args.len() <= 72 && !args.contains(' ') {
        return args.to_string();
    }
    if args.len() <= 72 && !args.contains('{') {
        return args.to_string();
    }
    truncate(args, 60)
}

fn extract_json_string(input: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let start = input.find(&needle)?;
    let rest = &input[start + needle.len()..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    let after = after.strip_prefix('"')?;
    let mut out = String::new();
    for ch in after.chars() {
        if ch == '"' {
            break;
        }
        if ch == '\\' {
            continue;
        }
        out.push(ch);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// A short result summary derived from machine facts only.
fn summarize_result(result: &str, kind: UiActionKind) -> Option<String> {
    if result.is_empty() {
        return None;
    }
    let first = result.lines().next().unwrap_or(result);
    let binding = crate::tools::shell::redact_secrets_public(first);
    let first = binding.trim();
    if matches!(
        kind,
        UiActionKind::Verification | UiActionKind::RunningCommand
    ) {
        // Exit facts are attached via PtyExited; do not infer from prose.
        return None;
    }
    if first.starts_with("Error:") || first.starts_with("error:") {
        return Some(truncate(first, 60));
    }
    if first.starts_with("Change applied") {
        // The diff summary carries real +/- counts in the preview body.
        let (add, del) = diff_counts(result);
        return Some(if add == 0 && del == 0 {
            "change applied".to_string()
        } else {
            format!("+{} -{}", add, del)
        });
    }
    if matches!(kind, UiActionKind::Result) {
        return Some(truncate(first, 60));
    }
    None
}

/// Count +/− lines in a diff preview (real line counts from the change).
fn diff_counts(preview: &str) -> (u32, u32) {
    let mut add = 0;
    let mut del = 0;
    for line in preview.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if let Some(rest) = line.strip_prefix('+') {
            if !rest.is_empty() {
                add += 1;
            }
        } else if let Some(rest) = line.strip_prefix('-') {
            if !rest.is_empty() {
                del += 1;
            }
        }
    }
    (add, del)
}

fn truncate(s: &str, width: usize) -> String {
    let count = s.chars().count();
    if count <= width {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::events::AgentEvent;
    use crate::agent::status::AgentStatus;

    fn tool_started(tool: &str, args: &str) -> AgentEvent {
        AgentEvent::ToolStarted {
            tool: tool.to_string(),
            args: args.to_string(),
        }
    }

    fn tool_completed(tool: &str, result: &str, success: bool) -> AgentEvent {
        AgentEvent::ToolCompleted {
            tool: tool.to_string(),
            result: result.to_string(),
            success,
        }
    }

    #[test]
    fn test_kind_mapping() {
        assert_eq!(
            UiActionKind::from_tool("read_file"),
            UiActionKind::ReadingFile
        );
        assert_eq!(
            UiActionKind::from_tool("list_files"),
            UiActionKind::ListingFiles
        );
        assert_eq!(UiActionKind::from_tool("git_status"), UiActionKind::Git);
        assert_eq!(UiActionKind::from_tool("edit_file"), UiActionKind::Editing);
        assert_eq!(
            UiActionKind::from_tool("run_command"),
            UiActionKind::RunningCommand
        );
        assert_eq!(UiActionKind::from_tool("run_tests"), UiActionKind::Testing);
        assert_eq!(
            UiActionKind::from_tool("verify"),
            UiActionKind::Verification
        );
        assert_eq!(
            UiActionKind::from_tool("propose_change"),
            UiActionKind::Editing
        );
        assert_eq!(
            UiActionKind::from_tool("unknown_tool"),
            UiActionKind::Result
        );
    }

    #[test]
    fn test_phase_from_agent() {
        assert_eq!(ActionStream::phase_for("research"), Some(Phase::Research));
        assert_eq!(ActionStream::phase_for("coding"), Some(Phase::Coding));
        assert_eq!(ActionStream::phase_for("review"), Some(Phase::Review));
        assert_eq!(ActionStream::phase_for("main"), None);
    }

    #[test]
    fn test_tool_lifecycle_creates_and_completes_action() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("read_file", "{\"path\": \"src/main.rs\"}"));
        assert_eq!(stream.tool_calls, 1);
        assert_eq!(stream.files_inspected.len(), 1);
        let group = stream.groups.back().unwrap();
        assert_eq!(group.actions.len(), 1);
        assert_eq!(group.actions[0].status, UiActionStatus::Running);
        assert_eq!(group.actions[0].detail, "src/main.rs");

        stream.handle_event(&tool_completed("read_file", "fn main() {}", true));
        let group = stream.groups.back().unwrap();
        assert_eq!(group.actions[0].status, UiActionStatus::Completed);
        assert!(group.actions[0].completed_at.is_some());
    }

    #[test]
    fn test_failed_tool_counts_as_failure() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("run_command", "cargo test"));
        stream.handle_event(&tool_completed("run_command", "Error: exit 101", false));
        assert_eq!(stream.failures, 1);
        let group = stream.groups.back().unwrap();
        assert_eq!(group.actions[0].status, UiActionStatus::Failed);
    }

    #[test]
    fn test_pty_exit_is_authoritative() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("run_command", "cargo test"));
        stream.handle_event(&AgentEvent::PtyOutput {
            console: "c1".to_string(),
            content: "running 3 tests".to_string(),
        });
        stream.handle_event(&AgentEvent::PtyExited {
            console: "c1".to_string(),
            exit_code: 0,
            status: "completed".to_string(),
        });
        let group = stream.groups.back().unwrap();
        let action = &group.actions[0];
        assert_eq!(action.status, UiActionStatus::Completed);
        assert_eq!(action.exit_code, Some(0));
        assert_eq!(action.result_summary.as_deref(), Some("exit 0"));
        assert_eq!(stream.tests_passed, 1);
        assert_eq!(stream.tests_total, 1);
    }

    #[test]
    fn test_pty_exit_nonzero_is_failure() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("run_command", "cargo test"));
        stream.handle_event(&AgentEvent::PtyExited {
            console: "c1".to_string(),
            exit_code: 101,
            status: "failed".to_string(),
        });
        let group = stream.groups.back().unwrap();
        assert_eq!(group.actions[0].status, UiActionStatus::Failed);
        assert_eq!(stream.failures, 1);
    }

    #[test]
    fn test_pty_timeout_is_warning() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("run_command", "cargo test"));
        stream.handle_event(&AgentEvent::PtyExited {
            console: "c1".to_string(),
            exit_code: -1,
            status: "timed out".to_string(),
        });
        let group = stream.groups.back().unwrap();
        assert_eq!(group.actions[0].status, UiActionStatus::Warning);
    }

    #[test]
    fn test_agent_lifecycle_groups_phases() {
        let mut stream = ActionStream::new();
        stream.handle_event(&AgentEvent::AgentStarted {
            agent: "research".to_string(),
            task: "Find parser".to_string(),
        });
        stream.handle_event(&AgentEvent::AgentStatusChanged {
            agent: "research".to_string(),
            status: AgentStatus::Searching,
        });
        stream.handle_event(&tool_started("read_file", "src/parser.rs"));
        stream.handle_event(&AgentEvent::AgentCompleted {
            agent: "research".to_string(),
            duration_ms: 4100,
        });
        assert_eq!(stream.groups.len(), 1);
        let group = &stream.groups[0];
        assert_eq!(group.phase, Phase::Research);
        assert_eq!(group.status, UiActionStatus::Completed);
        assert_eq!(group.duration_ms, Some(4100));
        assert_eq!(group.task.as_deref(), Some("Find parser"));
    }

    #[test]
    fn test_agent_failure_marks_group_failed() {
        let mut stream = ActionStream::new();
        stream.handle_event(&AgentEvent::AgentStarted {
            agent: "coding".to_string(),
            task: "Fix parser".to_string(),
        });
        stream.handle_event(&AgentEvent::AgentFailed {
            agent: "coding".to_string(),
            error: "verification unavailable".to_string(),
        });
        assert_eq!(stream.groups.len(), 1);
        assert_eq!(stream.groups[0].status, UiActionStatus::Failed);
        assert_eq!(
            stream.groups[0].outcome.as_deref(),
            Some("verification unavailable")
        );
    }

    #[test]
    fn test_agent_cancelled_marks_group_cancelled() {
        let mut stream = ActionStream::new();
        stream.handle_event(&AgentEvent::AgentStarted {
            agent: "testing".to_string(),
            task: "Run tests".to_string(),
        });
        stream.handle_event(&AgentEvent::AgentCancelled {
            agent: "testing".to_string(),
        });
        assert_eq!(stream.groups[0].status, UiActionStatus::Cancelled);
    }

    #[test]
    fn test_user_cancel_preserves_partial_results() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("read_file", "a.rs"));
        stream.handle_event(&tool_completed("read_file", "ok", true));
        stream.handle_event(&tool_started("read_file", "b.rs"));
        stream.cancel_active();
        let group = stream.groups.back().unwrap();
        assert_eq!(group.status, UiActionStatus::Cancelled);
        let completed = group
            .actions
            .iter()
            .filter(|a| a.status == UiActionStatus::Completed)
            .count();
        let cancelled = group
            .actions
            .iter()
            .filter(|a| a.status == UiActionStatus::Cancelled)
            .count();
        assert_eq!(completed, 1, "completed facts must be preserved");
        assert_eq!(cancelled, 1);
    }

    #[test]
    fn test_verification_facts_parsed_from_machine_log() {
        let mut stream = ActionStream::new();
        stream.handle_event(&AgentEvent::AgentStatusChanged {
            agent: "main".to_string(),
            status: AgentStatus::Testing,
        });
        stream.handle_event(&tool_started("run_command", "cargo test"));
        stream.handle_event(&AgentEvent::Log {
            level: "coding".to_string(),
            message:
                "Coding verification completed: exit=0 success=true denied=false timeout=false"
                    .to_string(),
        });
        let group = stream.groups.back().unwrap();
        let facts = group.verification.as_ref().unwrap();
        assert_eq!(facts.exit_code, Some(0));
        assert_eq!(facts.success, Some(true));
        assert_eq!(group.summary_line().contains("exit 0"), true);
    }

    #[test]
    fn test_finalize_response_completes_groups() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("run_command", "cargo test"));
        stream.finalize_response(true);
        let group = stream.groups.back().unwrap();
        assert_eq!(group.status, UiActionStatus::Completed);
    }

    #[test]
    fn test_groups_are_bounded() {
        let mut stream = ActionStream::new();
        for i in 0..100 {
            stream.handle_event(&AgentEvent::AgentStarted {
                agent: "research".to_string(),
                task: format!("t{}", i),
            });
            stream.handle_event(&AgentEvent::AgentCompleted {
                agent: "research".to_string(),
                duration_ms: 1,
            });
        }
        assert!(stream.groups.len() <= MAX_GROUPS);
    }

    #[test]
    fn test_actions_per_group_are_bounded() {
        let mut stream = ActionStream::new();
        for i in 0..300 {
            stream.handle_event(&tool_started(
                "read_file",
                &format!("{{\"path\": \"src/f{}.rs\"}}", i),
            ));
            stream.handle_event(&tool_completed("read_file", "ok", true));
        }
        let group = stream.groups.back().unwrap();
        assert!(group.actions.len() <= MAX_ACTIONS_PER_GROUP);
    }

    #[test]
    fn test_no_secret_args_reach_actions() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started(
            "run_command",
            "curl -H \"Authorization: Bearer sk-abc123def456ghi789\" http://x",
        ));
        let group = stream.groups.back().unwrap();
        let detail = group.actions[0].detail.clone();
        assert!(!detail.contains("sk-abc123def456ghi789"));
        assert!(!detail.contains("abc123def456ghi789"));
    }

    #[test]
    fn test_diff_counts_from_proposal_result() {
        let preview = "--- a.rs\n+++ b.rs\n- old\n+ new\n+ added\n";
        assert_eq!(diff_counts(preview), (2, 1));
    }

    #[test]
    fn test_focus_cycle_and_toggle() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("read_file", "a.rs"));
        stream.handle_event(&tool_completed("read_file", "ok", true));
        stream.handle_event(&tool_started("read_file", "b.rs"));
        stream.handle_event(&tool_completed("read_file", "ok", true));
        assert!(stream.cycle_focus(true));
        assert!(stream.toggle_focused());
        let group = stream.groups.back().unwrap();
        assert_eq!(group.expanded, false);
        assert!(stream.toggle_focused());
        let group = stream.groups.back().unwrap();
        assert_eq!(group.expanded, true);
    }

    #[test]
    fn test_summary_line_real_counts() {
        let mut stream = ActionStream::new();
        stream.handle_event(&AgentEvent::AgentStarted {
            agent: "research".to_string(),
            task: "Inspect".to_string(),
        });
        stream.handle_event(&tool_started("read_file", "src/parser.rs"));
        stream.handle_event(&tool_completed("read_file", "ok", true));
        stream.handle_event(&tool_started("list_files", "src"));
        stream.handle_event(&tool_completed("list_files", "files", true));
        stream.handle_event(&AgentEvent::AgentCompleted {
            agent: "research".to_string(),
            duration_ms: 100,
        });
        let line = stream.groups.back().unwrap().summary_line();
        assert!(line.contains("✓ Research"));
        assert!(line.contains("file"));
    }

    #[test]
    fn test_current_activity_returns_running_action() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("run_command", "cargo test"));
        let (group, action) = stream.current_activity().unwrap();
        assert_eq!(group.phase, Phase::Main);
        assert_eq!(action.title, "run_command");
    }

    #[test]
    fn test_extract_json_string() {
        assert_eq!(
            extract_json_string("{\"path\": \"src/main.rs\"}", "path"),
            Some("src/main.rs".to_string())
        );
        assert_eq!(extract_json_string("{\"x\": 1}", "path"), None);
    }

    // ─── F1: no stale Running actions after finalization ──────────────

    #[test]
    fn test_finalize_response_clears_running_actions() {
        let mut stream = ActionStream::new();
        stream.handle_event(&AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: "t".to_string(),
        });
        stream.handle_event(&AgentEvent::AgentStatusChanged {
            agent: "main".to_string(),
            status: AgentStatus::Thinking,
        });
        // A tool that never completes before the response arrives.
        stream.handle_event(&tool_started("read_file", "a.rs"));
        stream.finalize_response(true);
        let group = stream.groups.back().unwrap();
        assert_eq!(group.status, UiActionStatus::Completed);
        assert_eq!(
            group
                .actions
                .iter()
                .filter(|a| a.status == UiActionStatus::Running)
                .count(),
            0,
            "completed group must not carry Running actions"
        );
        assert!(!group.is_running());
    }

    #[test]
    fn test_finalize_response_failure_clears_running_actions() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("run_command", "cargo test"));
        stream.finalize_response(false);
        let group = stream.groups.back().unwrap();
        assert_eq!(group.status, UiActionStatus::Failed);
        assert!(
            group
                .actions
                .iter()
                .all(|a| a.status != UiActionStatus::Running),
            "failed group must not carry Running actions"
        );
    }

    // ─── F2: main terminal events always target Phase::Main ───────────

    fn main_with_current_phase_elsewhere() -> ActionStream {
        let mut stream = ActionStream::new();
        stream.handle_event(&AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: "t".to_string(),
        });
        stream.handle_event(&AgentEvent::AgentStatusChanged {
            agent: "main".to_string(),
            status: AgentStatus::Searching,
        });
        // Specialist activity changes the active phase away from Main.
        stream.handle_event(&AgentEvent::AgentStarted {
            agent: "research".to_string(),
            task: "r".to_string(),
        });
        stream.handle_event(&AgentEvent::AgentStatusChanged {
            agent: "research".to_string(),
            status: AgentStatus::Searching,
        });
        stream
    }

    #[test]
    fn test_main_completion_targets_main_phase() {
        let mut stream = main_with_current_phase_elsewhere();
        stream.handle_event(&AgentEvent::AgentCompleted {
            agent: "main".to_string(),
            duration_ms: 1500,
        });
        let main_group = stream
            .groups
            .iter()
            .find(|g| g.phase == Phase::Main)
            .expect("main group exists");
        assert_eq!(main_group.status, UiActionStatus::Completed);
        let research_group = stream
            .groups
            .iter()
            .find(|g| g.phase == Phase::Research)
            .expect("research group exists");
        assert_eq!(
            research_group.status,
            UiActionStatus::Running,
            "specialist phase must not be touched by main completion"
        );
    }

    #[test]
    fn test_main_failure_targets_main_phase() {
        let mut stream = main_with_current_phase_elsewhere();
        stream.handle_event(&AgentEvent::AgentFailed {
            agent: "main".to_string(),
            error: "boom".to_string(),
        });
        let main_group = stream
            .groups
            .iter()
            .find(|g| g.phase == Phase::Main)
            .expect("main group exists");
        assert_eq!(main_group.status, UiActionStatus::Failed);
        assert_eq!(main_group.outcome.as_deref(), Some("boom"));
        let research_group = stream
            .groups
            .iter()
            .find(|g| g.phase == Phase::Research)
            .expect("research group exists");
        assert_eq!(
            research_group.status,
            UiActionStatus::Running,
            "specialist phase must not be touched by main failure"
        );
    }

    #[test]
    fn test_main_cancellation_targets_main_phase() {
        let mut stream = main_with_current_phase_elsewhere();
        stream.handle_event(&AgentEvent::AgentCancelled {
            agent: "main".to_string(),
        });
        let main_group = stream
            .groups
            .iter()
            .find(|g| g.phase == Phase::Main)
            .expect("main group exists");
        assert_eq!(main_group.status, UiActionStatus::Cancelled);
        let research_group = stream
            .groups
            .iter()
            .find(|g| g.phase == Phase::Research)
            .expect("research group exists");
        assert_eq!(
            research_group.status,
            UiActionStatus::Running,
            "specialist phase must not be touched by main cancellation"
        );
    }

    // ─── F6: human-readable timeout summary ───────────────────────────

    #[test]
    fn test_timeout_summary_is_human_readable() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("run_command", "cargo test"));
        stream.handle_event(&AgentEvent::PtyExited {
            console: "c1".to_string(),
            exit_code: -1,
            status: "timed out".to_string(),
        });
        let group = stream.groups.back().unwrap();
        let action = &group.actions[0];
        assert_eq!(action.status, UiActionStatus::Warning);
        assert_eq!(action.result_summary.as_deref(), Some("timed out"));
        // The raw machine exit code remains authoritative and intact.
        assert_eq!(action.exit_code, Some(-1));
    }

    #[test]
    fn test_cancelled_summary_is_human_readable() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("run_command", "cargo test"));
        stream.handle_event(&AgentEvent::PtyExited {
            console: "c1".to_string(),
            exit_code: -1,
            status: "cancelled".to_string(),
        });
        let group = stream.groups.back().unwrap();
        assert_eq!(group.actions[0].status, UiActionStatus::Cancelled);
        assert_eq!(
            group.actions[0].result_summary.as_deref(),
            Some("cancelled")
        );
    }

    // ─── F7: only real test/verify tools count as tests ───────────────

    #[test]
    fn test_cargo_check_is_not_counted_as_test() {
        let mut stream = ActionStream::new();
        stream.handle_event(&AgentEvent::AgentStarted {
            agent: "testing".to_string(),
            task: "baseline".to_string(),
        });
        stream.handle_event(&AgentEvent::AgentStatusChanged {
            agent: "testing".to_string(),
            status: AgentStatus::Testing,
        });
        stream.handle_event(&tool_started("run_command", "cargo check"));
        stream.handle_event(&AgentEvent::PtyExited {
            console: "c1".to_string(),
            exit_code: 0,
            status: "completed".to_string(),
        });
        let group = stream.groups.back().unwrap();
        let (passed, total) = group.test_counts();
        assert_eq!(
            (passed, total),
            (0, 0),
            "cargo check (run_command) must not count as a test"
        );
        let summary = group.summary_line();
        assert!(
            !summary.contains("test passed"),
            "summary must not claim a test passed: {}",
            summary
        );
        assert!(
            summary.contains("1 command"),
            "command count kept: {}",
            summary
        );
    }

    #[test]
    fn test_real_test_tool_increments_test_count() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("run_tests", "cargo test"));
        stream.handle_event(&AgentEvent::PtyExited {
            console: "c1".to_string(),
            exit_code: 0,
            status: "completed".to_string(),
        });
        let group = stream.groups.back().unwrap();
        let (passed, total) = group.test_counts();
        assert_eq!((passed, total), (1, 1));
        assert!(group.summary_line().contains("1 test passed"));
    }

    // ─── F8: list_files results feed the CONTEXT file count ───────────

    #[test]
    fn test_list_files_result_updates_file_count() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("list_files", "src"));
        stream.handle_event(&tool_completed(
            "list_files",
            "src/main.rs\nsrc/lib.rs\nsrc/tui/mod.rs\n\n",
            true,
        ));
        assert_eq!(stream.files_inspected.len(), 3);
        assert!(stream.files_inspected.contains("src/main.rs"));
        assert!(stream.files_inspected.contains("src/tui/mod.rs"));
    }

    #[test]
    fn test_list_files_malformed_result_does_not_fabricate() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("list_files", "src"));
        // Failed list: no paths to count.
        stream.handle_event(&tool_completed(
            "list_files",
            "Error: permission denied",
            false,
        ));
        assert!(stream.files_inspected.is_empty());
        // Empty successful result: nothing fabricated.
        stream.handle_event(&tool_started("list_files", "src"));
        stream.handle_event(&tool_completed("list_files", "   \n", true));
        assert!(stream.files_inspected.is_empty());
        // Header-ish lines are not counted as files.
        stream.handle_event(&tool_started("list_files", "src"));
        stream.handle_event(&tool_completed(
            "list_files",
            "=== Repository Files ===\n[dir] target\nreal.rs\n",
            true,
        ));
        assert_eq!(stream.files_inspected.len(), 1);
        assert!(stream.files_inspected.contains("real.rs"));
    }

    #[test]
    fn test_list_files_file_count_is_bounded() {
        let mut stream = ActionStream::new();
        stream.handle_event(&tool_started("list_files", "src"));
        let many: String = (0..(MAX_FILES_TRACKED + 100))
            .map(|i| format!("src/f{}.rs\n", i))
            .collect();
        stream.handle_event(&tool_completed("list_files", &many, true));
        assert!(stream.files_inspected.len() <= MAX_FILES_TRACKED);
    }
}
