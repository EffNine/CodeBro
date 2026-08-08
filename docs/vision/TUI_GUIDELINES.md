# TUI Guidelines

**Version:** 1.1.0
**Status:** Active
**Date:** 2026-08-08

---

## Layout Philosophy

The TUI layout is defined by a single rule: **the task output is the canvas; everything else is an annotation.**

### Default State
The default layout occupies the full terminal with the task output taking up the majority of space. No panels are visible by default. The input field is at the bottom.

### Layout Zones
```
┌─────────────────────────────────────────────────────┐
│ Title Bar          [workspace] [model] [status]      │
├─────────────────────────────────────────────────────┤
│                                                     │
│  Task Output Area (scrollable PTY, takes remaining  │
│  space — live, color-preserving, selectable)         │
│                                                     │
│                                                     │
├─────────────────────────────────────────────────────┤
│ Input Field: >                                      │
└─────────────────────────────────────────────────────┘
```

### Panel Overlay
Panels (agents, activity log, memory, skills, metrics) are overlays that replace the task output area temporarily. They are dismissed by pressing `Esc` or the panel's close command. The task output is never permanently displaced.

---

## Whitespace Usage

Whitespace is the primary layout tool. Borders and separators are secondary.

### Rules
- One empty line between logical sections (task input, runtime response, tool output).
- Two empty lines between turns (consecutive task inputs or consecutive runtime responses).
- No empty lines within a single response.
- The input field is separated from the task output by exactly one empty line.
- Panel headers are followed by one empty line before content.

### What This Rejects
- Separator lines (`──`) between every element
- Padding lines that serve no purpose
- Dense packing that makes scanning difficult

---

## Information Hierarchy

Information is ordered by importance, not by type. The user's eye should fall on the most relevant information first.

### Priority Order
1. **Task input** — always at the top of each turn
2. **Runtime response** — the primary content the user is reading
3. **Tool calls** — collapsed by default, visible on expansion
4. **System notifications** — memory updates, skill changes, warnings
5. **Activity log** — only visible when the activity panel is open

### Visual Weight
- Task input: bold prefix, standard text
- Runtime response: standard text, markdown rendered
- Tool calls: muted color, monospace
- Notifications: accent color, smaller font
- Errors: red accent, always visible

---

## Icon System

Icons are used sparingly and only where they convey information that text cannot.

### Rules
- Icons are ASCII-compatible (no Unicode box-drawing in content, only in layout structures when necessary).
- Each icon has a text equivalent that is shown on hover or in accessibility mode.
- Icons are consistent across the entire TUI — the same concept always uses the same icon.
- Icons do not replace text labels in the command palette or panel headers.

### Icon Map
| Concept | Icon | Usage |
|---------|------|-------|
| Success | `✓` | Completed tasks, approved changes |
| Running | `⟳` | Active tool calls, in-progress agents |
| Waiting | `○` | Queued tasks, pending approvals |
| Error | `✗` | Failed operations, rejected changes |
| Warning | `!` | Non-fatal issues, deprecations |
| Search | `⌕` | Semantic search, symbol lookup |
| File | `▤` | File operations |
| Shell | `⚡` | Command execution |
| Memory | `◈` | Memory operations |
| Skill | `⚙` | Skill operations |

---

## Task Rendering

Tasks are rendered as structured entries in the task output, not as separate UI elements.

### Structure
```
User: Add caching to the auth service

Runtime: Planning...
  ├─ Read src/auth/service.rs
  ├─ Read src/auth/cache.rs
  └─ Propose patch (3 hunk, 12 lines)

[Diff Preview — press Enter to view]
```

### Rules
- Task plans are shown inline, not in a separate panel, by default.
- Each step in a plan is a collapsible entry.
- Completed steps are shown with `✓`. In-progress steps with `⟳`. Pending steps are dimmed.
- The diff preview is always available via a keyboard shortcut, never auto-expanded.

---

## Subagent Rendering

Subagents are shown only when active. They disappear when idle.

### Structure
```
Agents
  Research   ✓  Found auth module in middleware.rs
  Planning   ✓  Plan generated
  Coding     ⟳  Applying patch to src/auth/middleware.rs
  Testing    ○  Waiting
  Review     ○  Waiting
```

### Rules
- Subagent names are left-aligned. Status indicators are right-aligned.
- Status descriptions are shown only for active agents.
- Completed agents collapse to their final status line.
- The agent panel is toggled with `Ctrl+A` and is hidden by default.

---

## Live Terminal Viewer (PTY)

Every task owns a PTY. The PTY is rendered live, read-only, color-preserving, scrollable, and selectable. The engineer can inspect any running task without interrupting it.

### Structure
```
> cargo test --lib
   Compiling codebro v0.7.0
    Finished test [unoptimized + debuginfo]
     Running unittests src/lib.rs (target/debug/deps/codebro-abc123)

test auth::tests::test_token_expiry ... ok
test auth::tests::test_invalid_token ... ok

test result: ok. 2 passed; 0 failed
```

### PTY Contract
- **Every task owns a PTY.** The runtime allocates a pseudo-terminal for each task. Output is not captured through pipes — it flows through a real terminal.
- **The PTY is rendered live.** Output appears as the process produces it. LLM responses stream token-by-token. Shell commands stream line-by-line.
- **The terminal is read-only.** The user can scroll, select, and read — but cannot type into the PTY while the task is running. All input goes through the main input field.
- **ANSI colors are preserved.** Compiler output, test frameworks, and shell prompts retain their full color. No color stripping.
- **Output remains scrollable.** Full scrollback is available via mouse wheel, `Shift+PageUp`/`Shift+PageDown`, or the scrollback buffer.
- **Output remains selectable.** Any line in the PTY output can be selected and copied with the mouse, even after the task completes.
- **History remains available after task completion.** The PTY output is never cleared. It persists in the scrollback for the duration of the session.
- **Inspect without interrupting.** The engineer can scroll through and read any running task's output without stopping it.

### What This Rejects
- Capturing output through pipes instead of a PTY
- Stripping ANSI colors from output
- Making output non-selectable
- Clearing the terminal when a new task starts
- Blocking scrollback access during task execution
- Replacing live output with a static summary

---

## Copy & Paste Behavior

Copy and paste must work reliably in a terminal environment.

### Rules
- Bracketed paste mode is always enabled. Pasted text is treated as a single input, not individual keystrokes.
- Selecting text with the mouse and pressing `Enter` pastes it into the input field.
- The runtime's output is selectable — users can copy tool output, diffs, errors, and PTY output.
- Paste of multi-line text into the input field uses `Shift+Enter` for newlines (standard terminal convention).
- Copy from the TUI does not require a special key combination — standard terminal copy (`Cmd+C` / `Ctrl+Shift+C`) works.

### What This Rejects
- Custom copy/paste key bindings that conflict with terminal conventions
- Input fields that do not accept pasted text
- Output that is not selectable
- PTY output that cannot be copied after task completion

---

## Expand/Collapse Behavior

Expand/collapse is the primary mechanism for managing information density.

### Rules
- Every collapsible element has a clear expand/collapse indicator (`▸` / `▾`).
- Collapse state is remembered per-session — if the user collapses a tool call, it stays collapsed until the next task.
- Collapse does not delete information — the full content is always available on expansion.
- The default state is collapsed for: tool calls, long outputs, activity log entries, subagent details.
- The default state is expanded for: task input, runtime response, error messages, PTY output.

### What This Rejects
- Auto-expanding elements that the user did not trigger
- Collapsing content that the user explicitly asked to see
- Hidden content that can only be accessed through a non-obvious path

---

## Adaptive Layout

The layout adapts to the terminal size and the current task state.

### Rules
- The task output area resizes to fill available space. Panels expand and contract with the terminal.
- When the terminal is narrow (< 80 columns), panels collapse to single-column mode.
- When the terminal is wide (>= 120 columns), the task output area expands and side panels may show alongside.
- The title bar always fits in a single line, regardless of terminal width.
- Font size changes are respected — the layout recomputes on every frame.

### What This Rejects
- Fixed-width layouts that break on small terminals
- Text that overflows the terminal width
- Panels that do not resize with the terminal

---

## Verbosity Modes

The TUI supports three verbosity levels, selectable via `Ctrl+V` or the command palette (`//verbose` / `//compact`).

### Levels
| Level | Key | What Changes |
|-------|-----|--------------|
| **Compact** | `1` | Only task output + input. No panels, no status, no tool details. |
| **Normal** | `2` | Task output + inline tool calls (collapsed) + title bar. Default. |
| **Detailed** | `3` | Task output + expanded tool calls + activity log + agent panel. |

### Rules
- Verbosity mode persists across tasks but resets on session restart.
- The mode is shown in the title bar so the user always knows the current level.
- Switching verbosity modes is instantaneous — no reload required.
- Errors and warnings are always visible regardless of verbosity level.
- PTY output is always fully visible regardless of verbosity level.

### What This Rejects
- Verbosity modes that hide errors or warnings
- Verbosity modes that hide PTY output
- Persistent verbosity settings that survive across projects (project context matters)
- More than three levels (the spectrum is compact / normal / detailed)

---

## Summary

The TUI guideline can be summarized in one sentence:

> **The task output is the interface. Everything else is a tool the user elects to use.**

Every layout decision, icon choice, and interaction pattern flows from this principle. When in doubt, ask: does this make the task output clearer, or does it add noise?
