# UX Principles

**Version:** 1.1.0
**Status:** Active
**Date:** 2026-08-08

---

## 1. Calm Interface

**The terminal should feel like a workshop, not a dashboard.**

### Rationale
Developers work in terminals for hours at a time. A loud, cluttered, animated interface creates cognitive fatigue. A calm interface recedes into the background and lets the work take center stage.

### Guidelines
- Use muted colors. Accent colors only for status indicators (success, warning, error).
- Avoid constant animations. Spinners are acceptable for waiting states; progress bars are acceptable for known-duration operations.
- Do not fill the screen with information the user did not ask for.
- The default view is the task output. Panels are overlays, not permanent fixtures.

### What This Rejects
- Chatbot-style avatars and decorative elements
- Auto-playing animations or sound effects
- "Active now" indicators that create urgency
- Dashboard-style metric tickers

---

## 2. Minimalism

**Every element on screen must earn its place.**

### Rationale
Terminal space is finite. Each line of text displaces content the user came to see. Minimalism is not an aesthetic choice — it is a spatial constraint.

### Guidelines
- Default panels show only the task output.
- Tool calls, memory updates, and skill activations are shown inline, not in separate panels, unless the user explicitly requests them.
- Status information lives in the title bar, not in the main content area.
- Empty panels collapse to zero height.

### What This Rejects
- Welcome screens that occupy the full viewport
- Sidebar navigation with persistent icons
- Redundant status displays (title bar AND panel)
- Decorative separators between every element

---

## 3. No Unnecessary Borders

**Boundaries are shown through spacing, not lines.**

### Rationale
Terminal borders (`┌──┐│  │└──┘`) consume characters that could display content. They also create visual noise that competes with the actual information. Modern TUI design uses whitespace and color to define boundaries.

### Guidelines
- Use empty lines to separate sections, not `──` separators.
- Use color and indentation to distinguish panels, not box-drawing characters.
- When borders are necessary (e.g., for a modal dialog), use them sparingly and remove them as soon as the modal is dismissed.
- The task output area has no border — it is the default state.

### What This Rejects
- Box-drawing characters around every panel
- Header bars with decorative borders
- Frame elements around the input field
- Border-only visual hierarchy

---

## 4. No Dashboard Feeling

**This is a tool, not a command center.**

### Rationale
Dashboards are designed for monitoring — they show many things at once so operators can spot anomalies. CodeBro is designed for doing — the user has a specific task and the runtime helps them complete it. A dashboard aesthetic creates the wrong mental model.

### Guidelines
- The primary view is the task output. Everything else is secondary.
- Runtime status, memory changes, and skill activations are event-driven — they appear when something happens, not as permanent displays.
- Do not show metrics (token count, cost, uptime) unless the user explicitly opens the metrics panel.
- The layout adapts to the task, not to a fixed grid of information.

### What This Rejects
- Persistent runtime status bars
- Always-visible token counters or cost displays
- Grid layouts with equal-weight panels
- "System health" indicators in the default view

---

## 5. Keyboard-First (Default: Arrow Keys + Tab)

**Every interaction must be possible without a mouse. The default navigation uses arrow keys and Tab, not Vim bindings.**

### Rationale
Developers live in keyboards. Reaching for a mouse breaks flow. The TUI must be fully navigable and operable with keyboard input. The default scheme uses standard arrow keys and Tab — keys every terminal user already knows. Vim bindings are an optional input scheme for users who prefer them.

### Guidelines
- Default navigation uses `Up` / `Down` / `Left` / `Right` arrow keys.
- `Tab` cycles forward through completions; `Shift+Tab` cycles backward.
- `Enter` submits input or confirms a selection.
- `Esc` dismisses panels, cancels operations, and closes modals.
- All actions have keyboard shortcuts (`Ctrl+P` for command palette, `Ctrl+C` for cancel).
- The command palette is the primary discovery mechanism — not a mouse-hover menu.
- Vim bindings (hjkl, `Ctrl+N`/`Ctrl+P` for history) are available as an opt-in scheme.

### What This Rejects
- Vim bindings as the default navigation scheme
- Mouse-only interactions (clicking buttons that have no keyboard equivalent)
- Hover-to-reveal menus
- Drag-and-drop operations
- Tooltips that require mouse hover

---

## 6. Selectability

**Everything visible should be selectable.**

### Rationale
Engineers constantly copy: logs, errors, diffs, stack traces, commands. If text cannot be selected, the TUI becomes an obstacle to the developer's workflow. No custom widget should interfere with text selection. Selection is a terminal-level operation, not a TUI feature.

### Guidelines
- All output text is selectable with the mouse or touchpad.
- Bracketed paste mode is always enabled so pasted text is treated as a single input.
- Selecting text with the mouse and pressing `Enter` pastes it into the input field.
- The command palette supports mouse clicks for selection.
- No custom widget overlays text in a way that prevents selection.
- Scrollback content remains selectable after the task completes.

### What This Rejects
- Custom text widgets that capture mouse selection
- Overlays that make output non-selectable
- Input fields that reject pasted text
- Output that is rendered in a way that prevents copying

---

## 7. Mouse Philosophy

**The mouse is for selection, copying, and scrolling. Keyboard is for navigation and input.**

### Rationale
Keyboard-first does not mean mouse-hostile. Selecting text, scrolling long outputs, and navigating large task histories are often faster with a mouse. The TUI should support both without conflict. The mouse is a secondary input device — it enables selection and navigation but never replaces keyboard input.

### Guidelines
- Mouse wheel scrolls scrollback and long outputs.
- `Shift+PageUp` / `Shift+PageDown` scroll by page (terminal convention).
- Clicking on a tool call in the activity log shows its details (if the panel is open).
- The command palette supports mouse clicks for selection.
- Clicking does not trigger actions that require keyboard confirmation (e.g., approve/reject).

### What This Rejects
- Mouse interactions that conflict with keyboard navigation
- Hidden mouse support that is inconsistent with keyboard behavior
- Click targets that are too small for terminal rendering
- Mouse clicks that execute destructive actions without keyboard confirmation

---

## 8. Progressive Disclosure

**Information is revealed in layers, not dumped all at once.**

### Rationale
The user's attention is the scarce resource. Showing everything at once overwhelms. Showing nothing until it is needed delays understanding. Progressive disclosure finds the balance.

### Guidelines
- The task output is always visible. Runtime panels are hidden by default.
- Tool call details are collapsed by default; expand on hover or click.
- Error details are hidden until the user requests them (`//details` or similar).
- The command palette filters by context — only relevant commands appear.

### What This Rejects
- All panels open by default on startup
- Expanding all tool calls automatically
- Showing full stack traces for every error
- Presenting every available command in a static list

---

## 9. Live Task Console

**Every task owns a PTY. The terminal is read-only, color-preserving, scrollable, and selectable. The engineer can inspect any running task without interrupting it.**

### Rationale
The black-box problem: the user submits a task and sees nothing for 10 seconds. This creates anxiety and distrust. A live task console shows what the runtime is doing as it does it — searching symbols, reading files, running commands — in a real terminal emulator. The PTY output is append-only, never replaced. The engineer can scroll through the full output at any time, copy any line, and inspect the state of a running process without stopping it.

### PTY Contract
- **Every task owns a PTY.** The runtime does not capture output through pipes or buffers — it allocates a pseudo-terminal.
- **The PTY is rendered live.** Output appears as the process produces it, token by token for LLM responses, line by line for shell commands.
- **The terminal is read-only.** The user can scroll and select but cannot type into the PTY while the task is running. Input goes through the main input field.
- **ANSI colors are preserved.** Rust compiler output, test frameworks, and shell prompts retain their color. No color stripping.
- **Output remains scrollable.** Full scrollback is available via mouse wheel, `Shift+PageUp`/`Shift+PageDown`, or the scrollback buffer.
- **Output remains selectable.** Any line in the PTY output can be selected and copied, even after the task completes.
- **History remains available after task completion.** The PTY output is never cleared. It persists in the scrollback for the duration of the session.
- **The engineer can inspect any running task without interrupting it.** Scrolling, selecting, and reading output is always possible, even while the task is executing.

### What This Rejects
- Full-screen loading indicators that hide the task output
- Replacing the task output with a status message
- Hiding tool calls or shell output until they complete
- Batch-displaying all activity after the task finishes
- Stripping ANSI colors from output
- Making output non-selectable after task completion
- Clearing the PTY when a new task starts

---

## 10. Context-Aware Navigation

**The available actions depend on what the runtime is currently doing.**

### Rationale
A static command list is noisy. Showing only the commands relevant to the current state reduces cognitive load and prevents accidental execution of irrelevant commands.

### Guidelines
- `//approve` appears only when there is a pending file change.
- `//cancel` appears only when the runtime is actively working.
- `//sessions` shows recent sessions, not all sessions, by default.
- The command palette filters based on the current runtime state.

### What This Rejects
- Static command lists that show all possible commands at all times
- Context-blind autocomplete (showing `//sessions` when the runtime is mid-task)
- Hidden commands that have no discoverability path

---

## 11. Respect Terminal Muscle Memory

**CodeBro extends terminal workflows. It does not invent new interaction patterns.**

### Rationale
Developers have years of terminal muscle memory — `Ctrl+C` to interrupt, `Ctrl+L` to clear, `Ctrl+R` to search history, `Up/Down` to scroll, `Tab` to complete, `Escape` to cancel. CodeBro must respect these. Overriding them creates friction and frustration. The default navigation scheme (arrow keys + Tab) matches what developers already do in their shell.

### Guidelines
- `Ctrl+C` cancels the current task (consistent with terminal convention).
- `Ctrl+L` clears the screen (consistent with terminal convention).
- `Up` / `Down` navigate task and command history (consistent with readline convention).
- `Ctrl+R` searches command history (consistent with shell convention).
- `Ctrl+Z` suspends the process (consistent with POSIX convention).
- `Tab` completes commands, files, and model names (consistent with shell convention).
- `Shift+Tab` cycles completion in reverse (standard terminal convention).
- `Enter` submits input (consistent with all terminal input).
- `Esc` cancels and returns to previous state (consistent with vim, less, and many CLIs).

### What This Rejects
- Overriding `Ctrl+C` for a custom action
- Replacing `Up/Down` with something else for history navigation
- Capturing keys that have established terminal meanings
- Unexpected behavior that breaks muscle memory
- Requiring Vim bindings as the default navigation scheme

---

## Summary

| Principle | One-Line Statement |
|-----------|-------------------|
| Calm Interface | The terminal is a workshop, not a dashboard |
| Minimalism | Every element must earn its place |
| No Unnecessary Borders | Spacing defines boundaries, not lines |
| No Dashboard Feeling | This is a tool, not a command center |
| Keyboard-First (Arrow + Tab) | Default navigation is arrow keys and Tab; Vim is optional |
| Selectability | Everything visible is selectable and copyable |
| Mouse Philosophy | Mouse is for selection, copying, and scrolling only |
| Progressive Disclosure | Information appears only when relevant |
| Live Task Console | Every task owns a PTY; output is live, readable, and inspectable |
| Context-Aware Navigation | Available actions depend on the current task state |
| Respect Terminal Muscle Memory | CodeBro extends terminal habits; it does not replace them |
