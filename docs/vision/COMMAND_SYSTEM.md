# Command System

**Version:** 1.1.0
**Status:** Active
**Date:** 2026-08-08

---

## Overview

CodeBro's command system is the primary interface between the user and the runtime. Commands are typed at the input field and are prefixed to distinguish their namespace from natural language task input.

The command system is designed for speed, discoverability, and context-awareness. Every command should be reachable within two keystrokes.

---

## Namespace Philosophy

Commands are organized into three namespaces, each with a distinct prefix and purpose. The namespace reflects what the command operates on:

| Namespace | Prefix | Operates On |
|-----------|--------|-------------|
| Engineering | `/` | The **project** — code, tasks, workflows |
| Runtime | `//` | **CodeBro** — configuration, display, session state |
| Shell | `!` | The **system** — direct shell execution |

This distinction is fundamental: `/` commands help you work on your code. `//` commands help you work with CodeBro. `!` commands bypass CodeBro entirely.

---

## `/` — Engineering Commands

Engineering commands operate on the **project**. They control tasks, code analysis, and engineering workflows. These are the commands the user types most frequently.

| Command | Purpose |
|---------|---------|
| `/help` | Show available commands and shortcuts |
| `/review` | Review pending or recent code changes |
| `/build` | Build the project |
| `/test` | Run tests |
| `/fix` | Fix the last error or failing test |
| `/search` | Search the codebase (symbols, text, patterns) |
| `/refactor` | Refactor a selected piece of code |
| `/explain` | Explain a selected piece of code or error |
| `/benchmark` | Run benchmarks and compare results |
| `/doctor` | Diagnose project health (dependencies, lints, tests) |
| `/playwright` | Run Playwright tests or generate Playwright tests |
| `/status` | Show current task state, model, and provider |

### Design Principle

Engineering commands are verbs that describe what the user wants to **do to the project**. They are action-oriented. `/review`, `/build`, `/test`, `/fix` — these are things you do to code.

---

## `//` — Runtime Commands

Runtime commands operate on **CodeBro itself**. They control the TUI, configuration, session state, and extensions. All settings that configure how CodeBro behaves belong here.

| Command | Purpose |
|---------|---------|
| `//model` | Show or change the current model |
| `//provider` | Show or change the current provider |
| `//apikey` | Update the API key |
| `//settings` | View and edit all settings |
| `//preferences` | View and edit preferences |
| `//profile` | Switch profiles |
| `//session` | Manage sessions (list, resume, delete) |
| `//resume` | Resume the last session |
| `//theme <name>` | Change the color theme |
| `//verbose` | Toggle detailed output mode |
| `//compact` | Toggle compact display mode |
| `//memory` | View or clear project memory |
| `//mcp` | Manage MCP servers |
| `//skills` | Manage installed skills |
| `//plugins` | Manage plugins |
| `//update` | Check for and apply updates |
| `//version` | Show CodeBro version and build info |
| `//export` | Export current configuration or session |
| `//import <file>` | Import configuration or session from file |
| `//clear` | Clear the terminal screen |
| `//tasks` | Show the current task graph |
| `//agents` | Show subagent status |
| `//metrics` | Show task metrics (tokens, cost, duration) |
| `//approve` | Approve pending file changes |
| `//reject` | Reject pending file changes |
| `//save` | Save the current session |

### Design Principle

Runtime commands are nouns or configuration descriptors. They describe **how CodeBro operates**, not what it does to the project. `//model`, `//provider`, `//theme` — these configure the tool, not the code.

---

## `!` — Shell Commands

Shell commands are executed directly in the user's shell. They bypass the runtime and run as if typed at the terminal prompt. Use `!` for one-off commands that do not require runtime involvement.

| Command | Purpose |
|---------|---------|
| `!ls` | List directory contents |
| `!git status` | Check git status |
| `!cargo test` | Run tests |
| `!grep -r "pattern" .` | Search files |
| `!docker ps` | List running containers |
| `!exit` | Exit CodeBro and return to the shell |

Shell commands are logged to the activity stream but are not part of the runtime's task trace. They are the user's commands, not the runtime's.

---

## Autocomplete

Autocomplete is the primary discovery mechanism for commands. It is context-aware and filters in real time as the user types.

### Behavior
- Typing `/` opens the command palette with all engineering commands.
- Typing `//` opens the command palette with all runtime commands.
- Typing `!` opens the command palette with common shell commands.
- Typing a partial command (e.g., `/tes`) filters to matching commands (e.g., `/test`).
- Typing a command followed by a space shows available arguments and flags.

### Context Awareness
The command palette filters based on the current state:
- `//approve` only appears when there are pending file changes.
- `//reject` only appears when there are pending file changes.
- `//tasks` only appears when a task graph exists.
- `//agents` only appears when subagents are active.

### What This Rejects
- Showing all commands at all times regardless of context
- Hiding commands that are relevant to the current state
- Static autocomplete lists that do not update as the user types

---

## Tab Completion

`Tab` completes the current input. It works for commands, arguments, file paths, and model names.

### Behavior
- `Tab` on a partial command cycles through matching commands.
- `Tab` on a command with arguments completes the argument value.
- `Tab` on a file path completes from the project's file index.
- `Tab` on a model name completes from the provider's available models.
- `Shift+Tab` cycles in the reverse direction.

### What This Rejects
- Tab completing unrelated values (e.g., completing a command argument with a file path)
- Tab triggering actions other than completion (e.g., submitting the input)
- Silent failures when no completion is available

---

## Arrow Navigation

Arrow keys navigate history and scroll content. They do not submit input.

### Behavior
- `Up` / `Down` navigate command and task history.
- `Up` / `Down` in the task output scroll the view.
- `Left` / `Right` navigate within the input field.
- `Ctrl+Left` / `Ctrl+Right` jump by word in the input field.
- `Home` / `End` jump to the beginning / end of the input field.

### What This Rejects
- Arrow keys submitting input
- Arrow keys triggering runtime actions (these have dedicated shortcuts)
- Conflicts with terminal scrollback (`Shift+Up` / `Shift+Down`)

---

## Context-Aware Suggestions

The TUI suggests actions based on the current state, not based on a static list.

### When Suggestions Appear
- A pending file change triggers the suggestion: `Press Enter to review diff`
- A tool call fails triggers the suggestion: `Press Enter to see error details`
- A new task starts triggers the suggestion: `Type /help for available commands`
- An MCP server is discovered triggers the suggestion: `Type //mcp to review discovered servers`

### How Suggestions Are Presented
- Suggestions appear as a single line below the input field.
- Suggestions are dismissed automatically when the user starts typing.
- Suggestions are shown for at most 5 seconds.
- Only one suggestion is shown at a time.

### What This Rejects
- Persistent suggestion banners that block the input field
- Multiple simultaneous suggestions
- Suggestions that do not match the current state
- Suggestions that cannot be dismissed

---

## History

Command history is persisted across sessions and is searchable.

### Behavior
- Command history is stored in `~/.codebro/history.json`.
- `Up` / `Down` navigate history (same as shell history).
- `Ctrl+R` opens incremental history search.
- History is scoped per-workspace — commands from one project do not appear in another.
- The history file is rotated at 10,000 entries (oldest entries are dropped).

### What This Rejects
- Global history that mixes commands from all projects
- History that is lost on restart
- History that exposes sensitive command output (only the command itself is stored, not the output)

---

## Command Preview

Before executing a potentially destructive command, CodeBro shows a preview of what will happen.

### Behavior
- `//approve` shows a preview: `This will apply 3 file changes. Proceed? (Y/n)`
- `//reject` shows a preview: `This will discard 3 file changes. Proceed? (Y/n)`
- `!rm -rf` shows a preview: `This will delete files permanently. Proceed? (Y/n)`
- Shell commands that match dangerous patterns (`rm -rf`, `git push --force`, `chmod -R 777`) always show a preview.

### What This Rejects
- Silent execution of destructive commands
- Previews that do not show the actual change
- Previews that are buried in output and easily missed

---

## Dynamic Command Registration

Commands are registered dynamically by the core, Skills, and MCP servers.

### Core Commands
Core commands are registered at startup in the command registry. They are always available.

### Skill Commands
Skills can register commands that are available only when the skill is active or relevant. A skill that manages database migrations might register `/migrate` when a migration skill is loaded.

### MCP Commands
MCP servers can register commands that expose their tools as slash commands. An MCP server that manages Kubernetes resources might register `!kubectl deploy`, `!kubectl logs`, `!kubectl scale`.

### Registration Rules
- Dynamic commands are merged with core commands in the autocomplete list.
- Dynamic commands are prefixed with the source name to avoid conflicts (e.g., `/k8s deploy`).
- Dynamic commands are unloaded when the source (skill or MCP server) is deactivated.
- Dynamic commands follow the same namespace rules as core commands.

### What This Rejects
- Command name collisions between core and dynamic commands
- Dynamic commands that persist after their source is removed
- Commands registered without a namespace prefix

---

## Summary

| Namespace | Prefix | Operates On | Examples |
|-----------|--------|-------------|----------|
| Engineering | `/` | The **project** | `/review`, `/build`, `/test`, `/fix`, `/search`, `/refactor` |
| Runtime | `//` | **CodeBro** | `//model`, `//provider`, `//theme`, `//verbose`, `//mcp`, `//approve`, `//tasks` |
| Shell | `!` | The **system** | `!git status`, `!cargo test`, `!docker ps` |

The command system exists to make the runtime fast, discoverable, and safe. Every design decision above serves one of those three goals.
