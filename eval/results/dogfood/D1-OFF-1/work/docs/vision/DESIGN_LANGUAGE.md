# Design Language

**Version:** 1.1.0
**Status:** Active
**Date:** 2026-08-08

---

## Purpose

This document defines the visual and interaction language of CodeBro's TUI. It is the reference for every rendering decision — from whitespace to color to animation. When a design question arises, the answer is found here.

This is not a style guide. It is a set of constraints that keep the TUI calm, functional, and engineering-focused.

---

## Whitespace Philosophy

Whitespace is the primary layout tool. It defines structure without adding visual noise.

### Principles
- **Whitespace earns its place.** Every empty line must separate distinct logical units. Decorative spacing is forbidden.
- **More whitespace is better than less.** The terminal has limited space, but cramming content creates cognitive fatigue. When in doubt, add a blank line.
- **Whitespace is consistent.** The same type of separation always uses the same amount of whitespace.

### Rules
| Separation | Whitespace |
|------------|-----------|
| Between task input and runtime response | 1 empty line |
| Between consecutive task inputs | 2 empty lines |
| Between logical sections within a response | 1 empty line |
| Between the task output and the input field | 1 empty line |
| Panel header and content | 1 empty line |
| Between panel sections | 1 empty line |

### What This Rejects
- Separator lines (`──`, `───`) between elements
- Padding lines that serve no logical purpose
- Inconsistent spacing (1 line here, 2 lines there, with no reason)
- Dense packing that makes scanning difficult

---

## Typography

The TUI uses a single typeface: the terminal's default font. No custom fonts. No font switching.

### Font Size
- Default: terminal default (typically 10–14pt)
- No programmatic font size changes
- Respects the user's terminal font size setting

### Weight and Style
- **Bold** — User messages, panel headers, active status indicators
- **Normal** — Runtime responses, tool output, task descriptions
- **Dim** — Collapsed content, pending steps, metadata
- **Italic** — Rarely used. Only for emphasis within runtime responses when the LLM output contains it.

### Monospace
All code, commands, diffs, and tool output are rendered in monospace. This is non-negotiable — code in proportional font is illegible.

---

## Spacing

Spacing is measured in terminal characters, not pixels.

### Horizontal Spacing
- Indentation: 2 spaces per level (not tabs, not 4 spaces)
- Panel content: indented 2 spaces from the panel edge
- Nested tool calls: indented 4 spaces from the parent task
- Subagent status: name left-aligned, status right-aligned, description in between

### Vertical Spacing
- See Whitespace Philosophy above.

---

## Icon Philosophy

Icons are information density tools. They convey status in a single character where text would require more.

### Principles
- **Icons replace text only when the meaning is unambiguous.** `✓` means success everywhere. `⟳` means running everywhere.
- **Icons never replace text in labels or headers.** The command palette shows `/test`, not `⚡test`.
- **Icons are ASCII-compatible.** No Unicode symbols that require special terminal font support.
- **Every icon has a text equivalent.** In accessibility mode or when icons fail to render, the text equivalent is shown.

### Icon Set
| Concept | Icon | Text Equivalent |
|---------|------|----------------|
| Success | `✓` | done |
| Running | `⟳` | running |
| Waiting | `○` | waiting |
| Error | `✗` | error |
| Warning | `!` | warning |
| Search | `⌕` | search |
| File | `▤` | file |
| Shell | `⚡` | shell |
| Memory | `◈` | memory |
| Skill | `⚙` | skill |

---

## Loading States

Loading states indicate that the runtime is working. They are minimal and non-intrusive.

### Principles
- **Loading states are append-only.** They appear in the task output and do not replace existing content.
- **Loading states use the spinner character.** `⠋ ⠙ ⠹ ⠸ ⠼ ⠴` — the standard Unicode braille pattern, rotated.
- **Loading states are paired with a text description.** `⟳ Searching symbols...` not just a spinner.
- **Loading states disappear when the work completes.** They are replaced by the result.

### What This Rejects
- Full-screen loading overlays
- Animated progress bars for unknown-duration operations
- Spinners without text description
- Loading states that replace the task output

---

## Progress States

Progress states indicate known-duration operations. They are more informative than loading states.

### Principles
- **Progress bars are used only for known-duration operations.** Building, testing, indexing — operations with a measurable completion point.
- **Progress bars are text-based.** `██████░░░░` — 10 blocks, filled proportionally.
- **Progress is shown inline with the task output.** Not in a separate panel.
- **Progress updates are smooth.** No jumpy or flashing progress indicators.

### What This Rejects
- Progress bars for unknown-duration operations (e.g., waiting for LLM response)
- Percentage numbers without context (e.g., "47%" — 47% of what?)
- Animated progress that draws attention away from the task output

---

## Error States

Error states are always visible. They are never collapsed, hidden, or deferred.

### Principles
- **Errors use red accent color.** Consistent across all error types.
- **Errors show the error message and the location.** File, line, column — wherever applicable.
- **Errors are append-only.** They appear in the task output where the error occurred, not in a separate toast or modal.
- **Errors are actionable.** Every error message includes what went wrong and what the user can do about it.

### Error Hierarchy
| Severity | Color | Visibility |
|----------|-------|------------|
| Fatal (crash, unhandled exception) | Red, bold | Always visible, halts task |
| Error (tool failure, permission denied) | Red | Always visible |
| Warning (non-fatal issue, deprecation) | Yellow | Always visible |
| Info (notification, update) | Cyan | Visible in normal mode, hidden in compact mode |

### What This Rejects
- Silent error swallowing
- Errors hidden behind "click to expand"
- Error messages without actionable guidance
- Custom error widgets that prevent copying the error text

---

## Success States

Success states are brief and unobtrusive. They confirm completion without celebrating.

### Principles
- **Success uses green accent color.** Consistent across all success types.
- **Success messages are brief.** "Done." "2 passed." "Patch applied." — not paragraphs.
- **Success does not trigger animation.** No confetti, no flashing, no sound.
- **Success disappears after 3 seconds** unless the user expands the task details.

### What This Rejects
- Extended success animations
- Success messages that compete with the task output for attention
- Celebratory language ("Awesome! Your code is perfect!")

---

## Expansion Behavior

Expansion is the primary mechanism for revealing detail. It is triggered by the user, never by the system.

### Principles
- **Everything is collapsed by default.** Tool calls, plan steps, subagent details, activity log entries.
- **Expansion indicators are clear.** `▸` for collapsed, `▾` for expanded.
- **Expansion state is per-session.** If the user expands a tool call, it stays expanded for the duration of the task. It resets on the next task.
- **Expansion never deletes content.** Collapsing a tool call hides it; it does not remove it. Expansion restores the full content.

### Default Collapse States
| Element | Default |
|---------|---------|
| Tool calls | Collapsed |
| Plan steps | Collapsed |
| Subagent details | Collapsed |
| Activity log entries | Collapsed |
| User messages | Expanded |
| Runtime responses | Expanded |
| Error messages | Expanded |
| PTY output | Expanded |

### What This Rejects
- Auto-expansion of any content
- Collapsing content the user explicitly asked to see
- Hidden content with no expand path

---

## Collapse Behavior

Collapse is the counterpart to expansion. It reduces visual density without losing information.

### Principles
- **Collapse is reversible.** The user can always expand collapsed content.
- **Collapse is explicit.** The user triggers collapse (or accepts the default collapsed state). The system never collapses content the user has expanded.
- **Collapse preserves structure.** Collapsing a tool call shows a summary line (`▸ read_file src/main.rs`) — not nothing.

### What This Rejects
- Collapsing content the user has explicitly expanded
- Collapsing error messages
- Collapsing PTY output
- Collapsing without a summary line

---

## Live Terminal Behavior

The PTY is the defining UX feature of CodeBro. It behaves like a real terminal.

### Principles
- **The PTY is live.** Output appears as the process produces it. No batching. No buffering.
- **The PTY is read-only during task execution.** The user can scroll, select, and read — but input goes through the main input field.
- **ANSI colors are preserved.** No color stripping. Rust compiler colors, test framework colors, and shell prompt colors are all rendered as-is.
- **The PTY is scrollable.** Full scrollback is available at all times.
- **The PTY is selectable.** Any line can be selected and copied, even after the task completes.
- **The PTY is never cleared between tasks.** Each task appends to the scrollback. The user can scroll back to see any previous task's output.

### What This Rejects
- Clearing the terminal between tasks
- Stripping ANSI colors
- Making output non-selectable
- Buffering output and displaying it all at once
- Replacing live output with a static summary

---

## Verbosity Levels

The TUI has three verbosity levels. They control what is visible in the default view.

### Levels
| Level | Name | Visible |
|-------|------|---------|
| `1` | Compact | Task input, runtime response, input field. No panels. No tool details. |
| `2` | Normal | Compact + inline collapsed tool calls + title bar. Default. |
| `3` | Detailed | Normal + activity log + agent panel + expanded tool calls. |

### Rules
- Errors and warnings are always visible at all verbosity levels.
- PTY output is always fully visible at all verbosity levels.
- The verbosity level is shown in the title bar.
- Switching verbosity levels is instantaneous.
- Verbosity level persists across tasks but resets on session restart.

### What This Rejects
- Hiding errors at any verbosity level
- Hiding PTY output at any verbosity level
- More than three levels
- Slow or laggy verbosity transitions

---

## Animation Philosophy

Animations are forbidden by default. They are allowed only when they convey information that static rendering cannot.

### Allowed Animations
| Animation | When | Duration |
|-----------|------|----------|
| Spinner (`⠋ ⠙ ⠹ ⠸ ⠼ ⠴`) | Task is in progress, duration unknown | 100ms per frame |
| Progress bar fill | Known-duration operation (build, test, index) | Instant update |
| Panel fade (optional) | Panel opens/closes | 100ms |

### Forbidden Animations
- Any animation that draws attention away from the task output
- Animated transitions between panels
- Pulsing, blinking, or flashing elements
- Sound effects
- Particle effects or decorative animations

### Principle
**If the animation is not conveying information, it is noise.** The terminal is a text medium. Animation should be used sparingly and only when it improves understanding.

---

## Summary

| Aspect | Rule |
|--------|------|
| Whitespace | Primary layout tool; empty lines separate logical units |
| Typography | Terminal default font; monospace for all code |
| Spacing | 2-space indent; consistent vertical separation |
| Icons | ASCII-compatible; replace text only when unambiguous |
| Loading | Append-only spinner with text description |
| Progress | Text-based bar for known-duration operations only |
| Error | Red, always visible, actionable, copyable |
| Success | Green, brief, unobtrusive, self-dismissing |
| Expansion | User-triggered; collapsed by default; state per-session |
| Collapse | Reversible; preserves structure with summary line |
| Live Terminal | PTY is live, read-only, color-preserving, scrollable, selectable |
| Verbosity | Three levels: compact / normal / detailed |
| Animation | Forbidden by default; spinner and progress bar are the only exceptions |
