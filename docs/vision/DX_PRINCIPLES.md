# Developer Experience Principles

## Overview

These principles govern every interaction, panel, command, and workflow in CodeBro's Developer Experience Platform (P5). They are the lens through which every design decision is made.

---

## Principle 1: Zero Configuration

### Statement
The tool must work immediately upon installation with no manual file editing.

### Implications
- First-run onboarding requires only an API key
- All defaults are sensible and safe
- Configuration files are implementation details, not user concerns
- Environment variables override everything but are optional

### Implementation Checklist
- [ ] `codebro` runs without `~/.codebro/config.toml`
- [ ] Model auto-detection works on first launch
- [ ] Provider health is checked automatically
- [ ] Workspace is detected without user intervention

---

## Principle 2: Progressive Discovery

### Statement
Simple tasks should be trivially easy; complex tasks should be discoverable, not searchable.

### Implications
- Common actions are one keystroke away
- Advanced features are visible but not intrusive
- Help is contextual and just-in-time
- The command palette (`Ctrl+P`) is the primary navigation mechanism

### Implementation Checklist
- [ ] All settings accessible via `/settings` command
- [ ] All providers accessible via `/providers` command
- [ ] Workspace info visible in title bar
- [ ] Shortcuts shown in status bar

---

## Principle 3: Human Approval

### Statement
No destructive or irreversible action proceeds without explicit user consent.

### Implications
- API key changes require confirmation
- Provider switches are confirmed
- Workspace integration enabling requires approval
- Configuration changes are reviewed before persisting

### Implementation Checklist
- [ ] Every `/settings` change shows a preview
- [ ] Provider removal prompts for confirmation
- [ ] Workspace integrations ask before enabling
- [ ] Configuration resets require double confirmation

---

## Principle 4: Everything Accessible from the TUI

### Statement
The terminal UI is the primary interface. No feature should require leaving the TUI.

### Implications
- All configuration is edit-in-place
- Provider management is TUI-native
- Discovery results are presented in-panel
- No `vim`/`nano` config editing required

### Implementation Checklist
- [ ] `/settings` opens an interactive panel
- [ ] `/providers` manages API keys in-terminal
- [ ] `/discover` shows workspace findings
- [ ] `/onboard` guides first-time setup

---

## Principle 5: Developer First

### Statement
Every millisecond of latency is a millisecond taken from the developer's flow.

### Implications
- Startup must be fast (< 200ms with config)
- Settings changes must be immediate (< 100ms)
- Panel toggles must be instant (< 50ms)
- No synchronous blocking on I/O in the event loop

### Implementation Checklist
- [ ] Config load is async where possible
- [ ] Provider health checks are non-blocking
- [ ] Workspace discovery runs in background
- [ ] All TUI state changes are O(1)

---

## Principle 6: Observable AI Actions

### Statement
Users must always know what the AI is doing, why, and have the ability to intervene.

### Implications
- Every tool call is logged with arguments (secrets redacted)
- Every provider request shows model and token estimate
- Every workspace detection is visible
- Every approval request is explicit

### Implementation Checklist
- [ ] Activity log shows all tool executions
- [ ] Provider health status is visible
- [ ] Workspace integrations show detection results
- [ ] All approval flows have clear accept/reject

---

## Principle 7: No Hidden Automation

### Statement
Every automated action is logged, visible, and reversible.

### Implications
- Background discovery does not silently enable features
- Auto-detected models are confirmed before use
- Configuration changes are logged to session
- No background processes run without user knowledge

### Implementation Checklist
- [ ] Discovery results are presented, not auto-applied
- [ ] Model selection is confirmed by user
- [ ] All config writes are logged
- [ ] No background tasks without visibility

---

## Anti-Patterns (What Not to Do)

| Anti-Pattern | Why It Violates Principles |
|-------------|---------------------------|
| Silent config generation | Violates Principle 7 (No Hidden Automation) |
| Auto-applying discovered integrations | Violates Principle 3 (Human Approval) |
| Requiring manual file edits for settings | Violates Principle 4 (TUI Accessible) |
| Blocking the event loop for I/O | Violates Principle 5 (Developer First) |
| Hiding advanced features behind nested menus | Violates Principle 2 (Progressive Discovery) |
| Auto-starting providers without health check | Violates Principle 6 (Observable Actions) |

---

## Principle Hierarchy

When principles conflict, the following priority order applies:

1. **Human Approval** (safety-critical)
2. **No Hidden Automation** (trust-critical)
3. **Developer First** (performance-critical)
4. **Everything Accessible from TUI** (usability-critical)
5. **Observable AI Actions** (transparency-critical)
6. **Progressive Discovery** (discoverability-critical)
7. **Zero Configuration** (convenience-critical)
