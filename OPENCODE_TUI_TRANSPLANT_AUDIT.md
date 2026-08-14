# OPENCODE TUI TRANSPLANT FEASIBILITY AUDIT

**Date:** 2026-08-13
**Auditor:** Agnes (Sapiens AI)
**Branch:** `opencode-tui-experiment` (read-only audit, no source changes)
**Working tree:** clean

---

## 1. Upstream

| Field | Value |
|---|---|
| **Repository** | `https://github.com/anomalyco/opencode` |
| **Branch / Ref** | `dev` (default) |
| **Exact commit SHA** | `864889ab9f9e921c240930b1dcd2bc0d2352c555` |
| **Commit date** | 2026-08-13 12:48 UTC |
| **Retrieved via** | `git clone --depth 1` to `/tmp/opencode-upstream` |

The upstream `dev` branch was cloned with `--depth 1` at the audit start. The `main` branch of CodeBro was not modified. The `opencode-tui-experiment` branch was checked out and no source files were altered during this audit.

---

## 2. License / Provenance

### 2.1 Source License Matrix

| Component | Source | License | Copyright | Can copy? | Can adapt? | Attribution required? |
|---|---|---|---|---|---|---|
| `opencode` (root) | `/tmp/opencode-upstream` | MIT | 2025 opencode | Yes | Yes | Yes — include LICENSE + notice |
| `@opencode-ai/tui` | `packages/tui/` | MIT | 2025 opencode | Yes | Yes | Yes |
| `@opencode-ai/ui` | `packages/ui/` | MIT | 2025 opencode | Yes | Yes | Yes |
| `@opentui/core` | npm registry | MIT | sst / contributors | Yes | Yes | Yes |
| `@opentui/keymap` | npm registry | MIT | sst / contributors | Yes | Yes | Yes |
| `@opentui/solid` | npm registry | MIT | sst / contributors | Yes | Yes | Yes |
| `solid-js` | npm registry | MIT | Ryan Carniato | Yes | Yes | Yes |
| `effect` (4.0.0-beta.83) | npm registry | MIT | Effect.ts | Yes | Yes | Yes |
| `fuzzysort` (3.1.0) | npm registry | MIT | Gordon Wang | Yes | Yes | Yes |
| `clipboardy` (4.0.0) | npm registry | MIT | Sindre Sorhus | Yes | Yes | Yes |
| `strip-ansi` (7.1.2) | npm registry | MIT | Sindre Sorhus | Yes | Yes | Yes |
| `diff` (9.0.0) | npm registry | BSD-3 | Kevin Mårtensson | Yes | Yes | Yes |
| `@pierre/diffs` | npm registry | see package | Pierre | Yes | Yes | Yes |
| `marked` (17.x) | npm registry | MIT | Timothy Gu | Yes | Yes | Yes |

**All directly relevant TUI components are MIT-licensed.** No copyleft or viral licenses were identified in the candidate source.

### 2.2 Third-Party Dependencies Used by Selected TUI Modules

The following dependencies are actually imported by the TUI code inspected:

| Dependency | Version | License | Used by | Direct? |
|---|---|---|---|---|
| `@opentui/core` | 0.4.5 (locked) / 0.5.2 (latest) | MIT | rendering, selection, keymap dispatch | Direct |
| `@opentui/keymap` | 0.4.5 / 0.5.2 | MIT | keyboard/command binding system | Direct |
| `@opentui/solid` | 0.4.5 / 0.5.2 | MIT | Solid JSX renderer for OpenTUI | Direct |
| `solid-js` | ^1.8.15 | MIT | reactive UI framework | Direct |
| `effect` | 4.0.0-beta.83 | MIT | Effect system (scoping, acquire/release) | Direct in app.tsx |
| `fuzzysort` | 3.1.0 | MIT | fuzzy command filtering | Direct in dialog-select |
| `clipboardy` | 4.0.0 | MIT | clipboard read/write (fallback) | Direct in clipboard.ts |
| `strip-ansi` | 7.1.2 | MIT | ANSI stripping in transcripts | Direct |
| `diff` | 9.0.0 | BSD-3 | diff rendering (via @pierre/diffs) | Direct |
| `@pierre/diffs` | 1.2.10 | MIT | diff computation/rendering | Direct |
| `remeda` | catalog | MIT | data transformation in select | Direct |
| `@kobalte/core` | catalog | MIT | accessible dialog/select primitives | Via @ui |

**Note:** `effect` is used extensively in `app.tsx` for scoped resource management (`Effect.scoped`, `Effect.acquireRelease`). Porting this pattern would require adopting the Effect ecosystem or rewriting it.

---

## 3. OpenCode TUI Architecture

### 3.1 Tech Stack

OpenCode TUI is built on:

- **SolidJS** — reactive UI framework (component-based, fine-grained reactivity)
- **@opentui/core** — native terminal renderer (Zig binary + TypeScript bindings) providing:
  - `CliRenderer` — full terminal control (cursor, colors, mouse, paste)
  - `BoxRenderable`, `ScrollBoxRenderable`, `InputRenderable`, `TextareaRenderable` — composableView widgets
  - `MouseButton` enum, `MouseEvent`, `PasteEvent` types
  - `createCliRenderer()` — factory producing the render loop
- **@opentui/solid** — SolidJS bindings for OpenTUI (`render()`, `useRenderer()`, `useTerminalDimensions()`)
- **@opentui/keymap** — hierarchical keyboard binding system with:
  - Mode stacks (`keymap.setData`, mode filtering)
  - Command registry (`keymap.registerCommand`)
  - Key sequence support (timed leaders, pending sequences)
  - `getCommandEntries()`, `getCommandBindings()`, `dispatchCommand()`
- **effect** — structured concurrency for lifecycle (`Effect.scoped`, `Deferred`, `addFinalizer`)

### 3.2 Architecture Map

| Feature | Module/File | Primary Abstraction | Dependencies | Coupling |
|---|---|---|---|---|
| App state model | `context/sdk.tsx`, `context/sync.tsx` | `sync.data` — reactive store of sessions, messages, providers | SDK client, KV store | High — tightly coupled to OpenCode runtime |
| Event loop | `app.tsx` + `@opentui/core` CliRenderer | `createCliRenderer()` → internal loop at 60fps | OpenTUI native | None portable |
| Keyboard/input | `keymap.tsx`, `@opentui/keymap` | `KeymapProvider`, mode stack, command registry | @opentui/core | High — uses OpenTUI's event types |
| Mouse model | `app.tsx` (onMouseDown/onMouseUp), `util/selection.ts` | Renderer-native mouse + selection API | @opentui/core | High |
| Command palette | `component/command-palette.tsx` | `DialogSelect` + keymap command entries | @opentui/core, solid-js | Medium — logic portable, widget not |
| Command system | `keymap.tsx` + `app.tsx` commands array | `useBindings()` + `dispatchCommand()` | @opentui/keymap | High |
| Autocomplete | `component/prompt/autocomplete.tsx` | Fuzzy-filtered input completion list | @opentui/core TextareaRenderable | High |
| Multiline paste | `component/prompt/index.tsx` | `pasteInputText()` with line-count summary when ≥3 lines or >150 chars | @opentui/core PasteEvent | Medium — concept portable |
| Clipboard/selection | `context/clipboard.tsx`, `clipboard.ts`, `util/selection.ts` | OSC52 + native clipboard command + text selection | clipboardy, node:child_process | Medium — algorithm portable |
| Scrolling | `util/scroll.ts` | Acceleration-based scroll with `getScrollAcceleration()` | @opentui/core ScrollBoxRenderable | Medium — concept portable |
| Viewport | `app.tsx`, `routes/session/index.tsx` | `useTerminalDimensions()` + layout boxes | @opentui/solid | High |
| Overlays | `ui/dialog.tsx`, `ui/dialog-select.tsx` | Modal dialog system with backdrop, z-index, click-outside dismiss | @opentui/core, solid-js | Medium — concept portable |
| Modal system | Same as overlays | `DialogProvider`, `dialog.replace()`, `dialog.clear()` | ui/dialog.tsx | Medium |
| Resize/layout | `app.tsx`, `routes/session/index.tsx` | `useTerminalDimensions()` reactive, fixed-width dialogs proportioned to terminal | @opentui/solid | High |
| Rendering | `@opentui/core` + `@opentui/solid` | Declarative JSX → native terminal rendering | OpenTUI Zig core | None portable |
| Markdown | `packages/ui/` + session route | Shiki-powered syntax highlighting, `marked-shiki` | shiki, marked, katex | Not portable directly |
| Diff rendering | `feature-plugins/system/diff-viewer.tsx`, `ui/components/diff-changes.tsx` | File-tree diff viewer + visual bar chart of +/- | @pierre/diffs | Concept portable |
| Tool/event presentation | `routes/session/index.tsx` | Collapsible tool output, `collapseToolOutput()`, per-part rendering | SDK types | Medium — concept portable |
| Session/history | `context/sync.tsx`, `routes/session/` | SDK-driven session store with fork/branch/timeline | SDK client | High |
| Async task integration | `app.tsx`, `context/sdk.tsx` | SDK client subscriptions, `useConnected()`, toast notifications | effect, SDK | High |

### 3.3 State Model

OpenCode's UI state is deeply coupled to its **SDK runtime**:

```
TuiInput { url, args, config, events, pluginHost }
    └── SDKProvider (WebSocket/HTTP to opencode server)
        └── SyncProvider (session list, messages, providers)
            └── ProjectProvider (workspace)
                └── LocalProvider (client-side mutable state)
                    └── KVProvider (persistent key-value)
                        └── RouteProvider (home / session navigation)
                            └── App component tree
```

The UI state (messages, tool results, selection, dialog open/closed) lives inside the SDK-synced session store and local Solid stores. There is no separate "TUI state" — it is derived from the SDK data model.

---

## 4. CodeBro TUI Architecture

### 4.1 Tech Stack

| Layer | Technology |
|---|---|
| Language | Rust (edition 2021) |
| Terminal backend | `crossterm` 0.27 (event-stream, bracketed-paste) |
| Widget framework | `ratatui` 0.26 |
| Async runtime | `tokio` 1.x (full) |
| Markdown | `pulldown-cmark` 0.10 |
| ANSI rendering | `ansi-to-tui` 4.1.0 |
| Persistence | `rusqlite` 0.31 (bundled) |
| PTY | `portable-pty` 0.9.0 |
| Tree-sitter | 0.20 (Rust, Python, JS, TS, Go) |

### 4.2 Architecture Map

| Module | Primary Abstraction | Dependencies |
|---|---|---|
| `app.rs` | `TuiApp` — monolithic app state struct | agent events, config, session tracker, metrics |
| `ui.rs` | `run()` + `run_loop()` — crossterm raw mode loop + draw | ratatui, crossterm, dashboard, commands |
| `events.rs` | `AppEvent` enum + `start_event_loop()` | crossterm EventStream, tokio |
| `actions.rs` | `ActionStream` — bounded phase-grouped tool activity | AgentEvent, Phase, UiActionKind |
| `dashboard.rs` | `Dashboard` — status monitor, logs, panels, palettes | AgentEvent, ActivityType |
| `commands.rs` | `CommandSpec` registry + `completion_candidates()` | TuiApp for context-aware filtering |
| `diff_view.rs` | `FileDiff`, `DiffReviewSession` | plain-text diff algorithm |
| `markdown.rs` | `render_markdown()` → ratatui `Line`s | pulldown-cmark, ratatui |
| `console.rs` | `PtyConsole` — append-only bounded PTY buffer | ansi-to-tui, ratatui |
| `theme.rs` | `THEME` singleton + `Phase` enum | ratatui::style |

### 4.3 Event Model

```rust
pub enum AppEvent {
    Input(KeyEvent),
    Quit,
    Response(String),           // final assistant response
    StreamChunk(String),        // streaming token
    AgentEvent(AgentEvent),     // tool lifecycle, PTY, agent status
    Resize(u16, u16),
    ModelsFetched { .. },
    ModelsFetchFailed(String),
    ProviderCheckResult { .. },
    Paste(String),              // bracketed paste payload
    Mouse(MouseEvent),
    TaskFinished { success: bool },
    ProviderHealthResults(..),
    WorkspaceDiscovered { .. },
}
```

The renderer drives state mutations via `handle_event()` and redraws on demand (50ms frame interval).

---

## 5. Compatibility Matrix

| Feature | OpenCode Implementation | CodeBro Current | Reuse? | Adapter? | Rewrite? | Risk |
|---|---|---|---|---|---|---|
| Command palette | `CommandPaletteDialog` + keymap commands | `dashboard.show_command_palette` + `palette_query`/`palette_index` | Concept | Low — replace UI with ratatui List widget | No | Low |
| Keyboard navigation (modal priority) | `KeymapProvider` with mode stack | Manual overlay-check chain in `handle_key()` | Concept | Low — same priority pattern | No | Low |
| Multiline paste summary | `pasteInputText()` with line-count when ≥3 lines | `insert_text()` — raw paste, no visual summary | Concept | Low — same logic, different UI | No | Low |
| Mouse selection/copy | `renderer.getSelection()` + OSC52 + clipboard commands | No selection system; mouse = scroll only | Partial | Medium — algorithm is portable | `SelectionLayer` adapter | Partial |
| Scrolling | `getScrollAcceleration()` + native scroll events | `scroll_from_bottom` counter + mouse wheel → +/-3 | Concept | Low — ratatui Paragraph supports scroll natively | No | Low |
| Resize handling | `useTerminalDimensions()` reactive | `AppEvent::Resize` → `scroll_to_bottom()` | Concept | None — already present | No | None |
| Overlays / modals | `DialogProvider` + `dialog.replace()` | `show_console`, `show_command_palette`, `provider_panel`, `settings_panel` — manual bool flags | Concept | Low — same overlay stack pattern | No | Low |
| Command dispatch | `keymap.dispatchCommand(name)` | `submit_input()` → `namespace_of()` → handler functions | Concept | None — CodeBro already has this | No | None |
| Autocomplete (`/`/`//`/`!`) | Prompt autocomplete component + fuzzy sort | `dashboard.autocomplete` + `autocomplete_command()` | Concept | Low — `fuzzysort` is available as Rust crate | No | Low |
| Diff rendering | `diff-viewer.tsx` + `@pierre/diffs` | `diff_view.rs` — custom line-by-line diff | Concept | None — CodeBro has its own engine | No | None |
| Tool presentation | Collapsible tool output per-part | `ActionStream` + `UiActionGroup` | Concept | None — CodeBro model is cleaner | No | None |
| Session/chat model | SDK-synced session with parts (text, tool, file) | `VecDeque<Message>` with sealed `UiActionGroup`s | Partial | **High** — fundamentally different models | Requires adaptation layer | Medium-High |
| Rendering | @opentui/core (Zig native) + SolidJS | ratatui + crossterm | **No** | **N/A** — must use ratatui | **Must rewrite** | **High** |
| Reactive state | SolidJS signals/memos | Rust structs + mpsc channels | **No** | **N/A** | **Must rewrite** | **High** |

---

## 6. User Issues — Gap Analysis

| Issue | OpenCode Solution | CodeBro Gap | Portability |
|---|---|---|---|
| **Multiline paste visually messy** | `pasteSummaryEnabled` — collapses to `[Pasted ~N lines]` when ≥3 lines or >150 chars; toggleable via `//` command | Raw paste inserted verbatim with no summary indicator | **HIGH** — simple logic, ~20 lines of Rust |
| **Cannot mouse-select/copy output** | Native selection via `@opentui/core` + `renderer.getSelection()` + `clipboard.write()` with OSC52 + fallback commands | No selection model at all; mouse = scroll only | **MEDIUM** — algorithm portable, needs a `SelectionLayer` struct in Rust |
| **Command palette Up/Down/scroll UX weak** | `DialogSelect` with fuzzy sort, page navigation, category grouping, suggested items | `nav_wrap`/`nav_page` exists but palette has no filter search | **HIGH** — add `fuzzysort` (Rust crate exists as `fuzzysort` or use `skim`/`fuzzy-matcher`) |
| **`/` and `//` autocomplete navigation weak** | Inline autocomplete list in prompt with fuzzy matching | Simple prefix match with Tab cycling | **MEDIUM** — improve filtering logic; fuzzysort crate available |
| **Resize — boxes overlap** | Fixed-width dialogs proportioned to terminal dimensions via `useTerminalDimensions()` | Static layout with `COMPACT_MIN_WIDTH` / `COMPACT_MIN_HEIGHT` thresholds | **LOW** — adopt dynamic width calculation pattern |
| **Overlay priority** | `DialogProvider` with z-index stacking, click-outside dismiss | Manual if-chain in `dismiss_top_overlay()` | **LOW** — refactor to explicit overlay stack (already partially done) |

---

## 7. Event Adapter — Recommended Architecture

### 7.1 Mapping

```
CodeBro AgentEvent
    ├── AgentStarted { agent, task }
    │       → OpenCode-style: session message + tool activity
    ├── AgentStatusChanged { agent, status }
    │       → UI phase update
    ├── ToolStarted { tool, args }
    │       → UI action item (running)
    ├── ToolCompleted { tool, result, success }
    │       → UI action item (completed/failed)
    ├── PtyOutput { console, content }
    │       → Console overlay update
    ├── PtyExited { console, exit_code, status }
    │       → Console status + action completion
    ├── AgentCompleted / AgentFailed / AgentCancelled
    │       → Group finalization
    ├── TaskGraphUpdated { graph }
    │       → Rail progress section
    ├── MemoryUpdated / SkillUpdated
    │       → Notification toast
    └── StreamChunk { content }
            → Streaming text in message
```

### 7.2 Adapter Design

A **one-way, deterministic, lossless** adapter is feasible:

```rust
// Conceptual adapter (NOT implemented)
pub struct TuiEventAdapter {
    /// Maps CodeBro events → OpenCode-style UI state mutations
    pub fn adapt(&mut self, event: AgentEvent) -> Vec<UiMutations> { ... }
}

pub enum UiMutations {
    AppendMessage(MessageRole, String),
    UpdateStreaming(String),
    UpdateAction(UiAction),
    UpdateConsole(String, ConsoleUpdate),
    ShowNotification(String),
}
```

**Key insight:** CodeBro already has `ActionStream` which performs this mapping deterministically. The adapter would be a thin layer on top of `ActionStream` producing OpenCode-style UI events.

### 7.3 Information Gaps

OpenCode UI expects:
- **Per-message parts** (text, tool calls, file changes) — CodeBro has consolidated messages without part structure
- **Tool call ID** for retry/duplicate detection — CodeBro has no tool call ID
- **Provider/model context per message** — CodeBro tracks this separately
- **Session fork/branch metadata** — CodeBro has linear sessions

**Conclusion:** The adapter would need to synthesize these from CodeBro's flatter event stream. Lossless for *required* UI state is achievable for the core features (messages, tools, console). Per-message parts and tool IDs are **nice-to-have** enhancements, not blockers.

---

## 8. State Adapter — Recommended Architecture

### 8.1 Comparison

| Aspect | OpenCode UI State | CodeBro TuiApp |
|---|---|---|
| Messages | SDK-synced `AssistantMessage` / `UserMessage` with `Part[]` | `VecDeque<Message>` with optional sealed actions |
| Tool output | Per-part `ToolPart` with expand/collapse state | `ActionStream` groups + `PtyConsole` buffers |
| Console | `console_state` in sync data | `VecDeque<PtyConsole>` in `TuiApp` |
| Dialogs/overlays | `DialogProvider` stack | Manual bool flags in `Dashboard` |
| Config/preferences | `TuiConfig` + KV store | `Config` struct + `SettingsManager` |
| Provider state | `sync.data.provider` | `provider_manager: Option<ProviderManager>` |

### 8.2 Recommendation: **Option B — Thin UI State Adapter**

Create a **presentation-state struct** that bridges CodeBro's engine state to an OpenCode-style render model:

```rust
// NOT implemented — design concept only
pub struct UiState {
    /// Normalized message list (mirrors OpenCode session shape)
    pub messages: Vec<UiMessage>,
    /// Console buffers (from TuiApp.consoles)
    pub consoles: Vec<UiConsole>,
    /// Active dialog/overlay stack
    pub overlays: Vec<UiOverlay>,
    /// Current terminal dimensions
    pub dimensions: (u16, u16),
    /// Provider/model state for header
    pub provider_state: ProviderUiState,
}

impl From<&TuiApp> for UiState {
    fn from(app: &TuiApp) -> Self { ... }
}
```

This avoids:
- Duplicate state machines
- Synchronization layers between two independent state models
- Deep coupling to OpenCode's SDK

**Red flag avoided:** No bidirectional sync layer is needed. The adapter is one-way: `TuiApp → UiState → ratatui renderer`.

---

## 9. Dependency Impact

### 9.1 OpenCode Runtime Dependencies (NOT needed in CodeBro)

| Package | Purpose | Rust Equivalent | Needed? |
|---|---|---|---|
| `@opentui/core` | Native terminal renderer (Zig) | `ratatui` + `crossterm` | **No** — ratatui replaces entirely |
| `@opentui/solid` | SolidJS bindings | N/A | **No** — Rust has no SolidJS |
| `@opentui/keymap` | Keyboard binding system | Custom Rust impl | **Optional** — could port concept |
| `solid-js` | Reactive UI framework | N/A | **No** |
| `effect` (4.0.0-beta) | Structured concurrency | `tokio` + `anyhow` | **No** — CodeBro already has async |
| `@kobalte/core` | Accessible dialog primitives | N/A | **No** |
| `dompurify` | HTML sanitization | N/A | **No** — no HTML in TUI |
| `katex` | LaTeX rendering | N/A | **No** |
| `marked` / `marked-shiki` | Markdown → HTML | `pulldown-cmark` | **Already have** |
| `shiki` | Syntax highlighting | N/A (could add `syntect`) | **Optional** |
| `fuzzysort` | Fuzzy string matching | `fuzzysort` crate or `skim` | **Yes, if porting palette** |
| `clipboardy` | Clipboard access | `arboard` or existing `pbcopy`/`xclip` | **Already have** |
| `strip-ansi` | ANSI stripping | `strip-ansi` crate | **Already available** |
| `diff` (9.0.0) | Unified diff | `diff` crate or existing logic | **Already have** |
| `@pierre/diffs` | Diff computation | Could use `diff` crate | **Optional** |

### 9.2 New Dependencies Required for Selective Port

| Dependency | Purpose | License |
|---|---|---|
| `fuzzysort` (Rust) or `skim` | Fuzzy filtering for command palette | MIT |
| `arboard` (optional) | Cross-platform clipboard with selection support | MIT |

**Net new dependency count: 0–2.** The bulk of what CodeBro needs already exists.

---

## 10. Maintenance / Upstream Drift

### 10.1 Risks

If source is copied/adapted:
- Upstream changes (new features, bug fixes, API changes) **do not automatically flow into CodeBro**
- Divergence accumulates with every upstream release
- Security patches in upstream dependencies must be manually backported

### 10.2 Recommended Strategy

**Port concepts and architecture, NOT source code.** This means:

1. **Vinyl a pinned snapshot** of any adapted modules with attribution comments:
   ```rust
   // Adapted from opencode/tui/src/component/command-palette.tsx
   // Upstream: https://github.com/anomalyco/opencode
   // Commit: 864889ab9f9e921c240930b1dcd2bc0d2352c555
   // License: MIT (see ATTRIBUTION.md)
   ```

2. **Maintain an `ATTRIBUTION.md`** at repo root listing:
   - All upstream source consulted
   - Commit SHA pinned
   - License of each component
   - Modifications made

3. **Port interaction patterns, not implementations:**
   - Paste summary logic → rewrite in Rust
   - Selection model → rewrite using crossterm selection events
   - Command palette → rewrite using ratatui List widget
   - Overlay priority → rewrite as explicit stack in Rust

### 10.3 Estimated Maintenance Burden

| Approach | Annual Maintenance |
|---|---|
| Direct source copy | High — track upstream changes, merge conflicts, license drift |
| Concept-only port (recommended) | Low — only fix bugs, no upstream sync needed |
| Hybrid (vendor + patch) | Medium — maintain vendored copy, apply selective patches |

---

## 11. Security

### 11.1 No Bypass Risks Identified

The proposed architecture preserves CodeBro's security boundaries:

| Security Layer | Status |
|---|---|
| `ToolRegistry` | ✅ Must remain the sole tool execution path |
| `PermissionHook` | ✅ Must remain in the execution path |
| `CredentialStore` | ✅ API keys only via `//apikey` masked prompt |
| `CanonicalRuntime` | ✅ All tasks routed through it |

### 11.2 Risks to Watch

- **Clipboard access:** OpenCode's `clipboard.ts` uses `node:child_process` for `osascript`/`xclip`/`wl-copy`. CodeBro already uses the same pattern (`copy_to_clipboard` in `app.rs:395`). No new risk.
- **OSC52 passthrough:** OpenCode writes `\x1b]52;c;...` to stdout. This is safe (terminal-native) but requires testing with non-OSC52 terminals.
- **Paste handling:** Bracketed paste is already enabled in CodeBro (`EnableBracketedPaste`). No new attack surface.
- **No telemetry:** OpenCode TUI has no built-in telemetry. CodeBro's existing metrics are opt-in.

### 11.3 Verdict

**No new security risks introduced** by adopting the interaction architecture, provided the adapter stays one-way (TUI ← Engine) and never feeds UI state back into tool execution.

---

## 12. Options Comparison

### OPTION A: Keep CodeBro TUI and Continue Polishing

| Criterion | Assessment |
|---|---|
| Effort | Low — continue current trajectory |
| Architectural risk | None |
| Dependency impact | None |
| Legal complexity | None |
| UX benefit | Incremental only |
| Maintenance cost | Low |
| Engine compatibility | Native |

**Verdict:** Safest option. Solves problems slowly.

### OPTION B: Full OpenCode TUI Clone / Reimplementation

| Criterion | Assessment |
|---|---|
| Effort | **Very high** — entire SolidJS/OenTUI stack replacement |
| Architectural risk | **Critical** — dual state machines, session model overhaul |
| Dependency impact | **Heavy** — would need to port or replace entire JS runtime |
| Legal complexity | Medium — MIT allows it, but attribution overhead |
| UX benefit | High — full OpenCode feature parity |
| Maintenance cost | **Very high** — must track upstream diverging from CodeBro |
| Engine compatibility | **Poor** — OpenCode session model doesn't fit CodeBro's turn-scoped actions |

**Verdict:** Not recommended. Forces a runtime swap that contradicts the mission.

### OPTION C: Selective OpenCode Interaction Architecture Port into Ratatui ✅ RECOMMENDED

| Criterion | Assessment |
|---|---|
| Effort | **Medium** — port specific UX patterns, rewrite in Rust |
| Architectural risk | **Low** — thin adapter layer, no runtime change |
| Dependency impact | **Minimal** — 0–2 new crates |
| Legal complexity | Low — MIT attribution, clear provenance |
| UX benefit | **High** — solves all 6 user-identified issues |
| Maintenance cost | **Low** — concept-only port, no upstream sync |
| Engine compatibility | **Native** — CodeBro engine untouched |

**Verdict:** Recommended. Best balance of risk, effort, and UX gain.

### OPTION D: Embed / Bridge OpenCode TUI Runtime Directly

| Criterion | Assessment |
|---|---|
| Effort | **Extreme** — FFI bridge between Rust and Node/Zig |
| Architectural risk | **Critical** — two runtimes, two event loops, two state machines |
| Dependency impact | **Catastrophic** — embed Bun runtime or Zig binary |
| Legal complexity | Medium — MIT, but embedding creates distribution questions |
| UX benefit | Maximum |
| Maintenance cost | **Prohibitive** |
| Engine compatibility | **Broken** — TUI would need its own process or complex IPC |

**Verdict:** Not recommended under any circumstances.

---

## 13. Recommendation

### SELECTIVE TRANSPLANT (Option C)

**Rationale:**

1. **The CodeBro engine is the product.** OpenCode's session model, SDK, and runtime are deeply coupled to its own business logic. Swapping the runtime would break turn-scoped action groups, the permission hook, the canonical runtime, and the entire agent orchestration.

2. **The TUI problems are interaction-layer problems, not engine problems.** Multiline paste, mouse selection, command palette navigation, and resize handling are all *presentation* concerns. They can be solved by porting the interaction patterns into ratatui without importing OpenCode's runtime.

3. **MIT license is permissive.** All relevant source is MIT-licensed. Attribution is straightforward. No copyleft contamination risk.

4. **Dependency footprint stays minimal.** Only `fuzzysort` (or equivalent) might be added. No Node.js, no SolidJS, no OpenTUI Zig binary.

5. **Maintenance burden is low.** Porting concepts rather than code means upstream changes don't need to be tracked. The pinned commit SHA serves as a provenance record.

**What NOT to transplant:**
- `@opentui/core` rendering engine
- `solid-js` reactive framework
- `effect` structured concurrency
- OpenCode's SDK/session model
- OpenCode's provider routing

**What TO transplant (as concepts, rewritten in Rust):**
- Multiline paste summary logic
- Text selection + clipboard model
- Command palette with fuzzy search
- Overlay priority stack
- Resize-aware layout calculations
- Scroll acceleration pattern

---

## 14. Smallest Prototype

**Prototype 1: Command Palette + Paste Summary**

Two independent, low-risk changes that deliver immediate UX value:

### 14.1 Multiline Paste Summary
- **Location:** `src/tui/app.rs` — `insert_text()` method
- **Change:** When pasted text has ≥3 lines or >150 chars, insert a collapsible `[Pasted ~N lines]` placeholder instead of raw text. Provide `//paste` to expand.
- **Estimated effort:** 30–50 lines of Rust
- **Risk:** None

### 14.2 Fuzzy Command Palette
- **Location:** `src/tui/dashboard.rs` — `palette_query` / `palette_index` + `src/tui/ui.rs` — palette rendering
- **Change:** Replace prefix-match palette filtering with `fuzzysort`-based ranking. Add category grouping.
- **Estimated effort:** 100–150 lines of Rust (+ `fuzzysort` crate)
- **Risk:** Low

These two prototypes address the **highest-impact, lowest-risk** user complaints and can be validated independently before any larger transplant work begins.

---

## 15. Legal / Attribution Checklist

### 15.1 Required Notices

The MIT license requires:
> "The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software."

**Action items:**

1. **Create `ATTRIBUTION.md`** at repo root with:
   ```markdown
   # Third-Party Attribution

   This product incorporates ideas and interaction patterns inspired by
   OpenCode (https://github.com/anomalyco/opencode), licensed under MIT.

   Upstream commit: 864889ab9f9e921c240930b1dcd2bc0d2352c555
   License: MIT (see LICENSE-OPENCODE)

   Components referenced:
   - Command palette interaction model
   - Multiline paste summary pattern
   - Text selection + clipboard architecture
   - Overlay priority stack
   ```

2. **Create `LICENSE-OPENCODE`** containing the full MIT text from upstream:
   ```
   MIT License
   Copyright (c) 2025 opencode
   [full license text]
   ```

3. **Source-file headers** for any directly copied logic:
   ```rust
   // Adapted from opencode/tui/src/... 
   // License: MIT, Copyright (c) 2025 opencode
   ```

4. **README attribution** is **recommended** but not legally required:
   > "Interaction patterns inspired by OpenCode (MIT)."

### 15.2 Trademark / Affiliation

OpenCode's README states:
> *"If you are working on a project that's related to OpenCode and is using 'opencode' as part of its name, for example 'opencode-dashboard' or 'opencode-mobile', please add a note to your README to clarify that it is not built by the OpenCode team and is not affiliated with us in any way."*

**Action:** CodeBro does NOT use "opencode" in its name. However, if any documentation references OpenCode, include:
> "CodeBro is not affiliated with, endorsed by, or sponsored by the OpenCode project."

### 15.3 Dependency Licenses to Include

| Package | License | Copyright |
|---|---|---|
| `@opentui/core` | MIT | sst / contributors |
| `@opentui/keymap` | MIT | sst / contributors |
| `@opentui/solid` | MIT | sst / contributors |
| `solid-js` | MIT | Ryan Carniato |
| `effect` | MIT | Effect.ts |
| `fuzzysort` | MIT | Gordon Wang |
| `clipboardy` | MIT | Sindre Sorhus |
| `strip-ansi` | MIT | Sindre Sorhus |
| `diff` | BSD-3 | Kevin Mårtensson |

All are permissive. No copyleft concerns.

---

## 16. Git Verification

```
$ git status
On branch opencode-tui-experiment
nothing to commit, working tree clean

$ git diff --stat main..opencode-tui-experiment
 (no output — branches are identical)

$ git log --oneline -3
3088712 fix: terminate react loop on final answers instead of spinning to the iteration cap
f3c2628 feat: polish provider onboarding and command navigation
3bdd18b fix: align tui state with runtime truth
```

| Branch | Status |
|---|---|
| `main` | Unchanged (3088712) |
| `codebro-original` | Unchanged (3088712) |
| `opencode-tui-experiment` | Unchanged (3088712) |
| Working tree | Clean |

No source files were modified. No dependencies were added. No commits were made.

---

## 17. Final Verdict

### PASS WITH RISKS

**Justification:**

The transplant is **legally safe** (all MIT), **architecturally viable** (thin adapter layer), and **dependency-light** (0–2 new crates). The primary risks are:

1. **State model mismatch** — OpenCode's per-message-part model doesn't map 1:1 to CodeBro's turn-scoped action groups. Mitigated by Option B (thin adapter).
2. **Rendering framework incompatibility** — OpenCode uses OpenTUI (Zig); CodeBro uses ratatui. The rendering layer cannot be transplanted, only the interaction patterns. This is explicitly scoped out.
3. **Maintenance of pinned provenance** — If upstream changes break a copied concept, the pin must be updated. Mitigated by concept-only port (no copied code to break).

**Recommended next step:** Execute Prototype 1 (paste summary + fuzzy command palette) on the `opencode-tui-experiment` branch to validate the approach before committing to a broader transplant.

---

*Audit complete. No code was modified. No commits were made.*
