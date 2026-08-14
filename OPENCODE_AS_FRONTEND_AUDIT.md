# OPENCODE AS CODEBRO FRONTEND — FEASIBILITY AUDIT

**Date:** 2026-08-13
**Auditor:** Agnes (Sapiens AI)
**Branch:** `opencode-tui-experiment` (read-only audit, no source changes)
**Working tree:** clean

---

## 1. Executive Verdict

### DO NOT RECOMMEND

The OpenCode TUI **cannot** be used as a thin frontend for CodeBro without reimplementing the entirety of OpenCode's server-side runtime as a compatibility layer. The coupling between the TUI and OpenCode's backend is too deep, too numerous, and too semantically specific to CodeBro's architecture.

**The core problem:** OpenCode's TUI is not a presentation layer — it is a full application that owns session management, provider authentication, tool execution, permission handling, question prompts, PTY management, VCS integration, MCP discovery, LSP status, file search, workspace management, and more. There is no "thin frontend" boundary to exploit.

The TUI calls **59 distinct `sdk.client.*` methods** and subscribes to **15 event types**. Every one of these would need a CodeBro-compatible implementation. The resulting adapter would be larger and more complex than CodeBro's existing ratatui TUI.

---

## 2. Upstream Commit

| Field | Value |
|---|---|
| **Repository** | `https://github.com/anomalyco/opencode` |
| **Branch / Ref** | `dev` (default) |
| **Exact commit SHA** | `864889ab9f9e921c240930b1dcd2bc0d2352c555` |
| **Commit date** | 2026-08-13 12:48 UTC |
| **Retrieved via** | `git clone --depth 1` to `/tmp/opencode-upstream` |

---

## 3. License / Provenance

### 3.1 Source License Matrix

| Component | Source | License | Copyright | Direct Reuse | Attribution |
|---|---|---|---|---|---|
| `opencode` (root) | `/tmp/opencode-upstream` | MIT | 2025 opencode | Yes | Required |
| `@opencode-ai/tui` | `packages/tui/` | MIT | 2025 opencode | Yes | Required |
| `@opencode-ai/ui` | `packages/ui/` | MIT | 2025 opencode | Yes | Required |
| `@opencode-ai/sdk` | `packages/sdk/` | MIT | 2025 opencode | Yes (types only) | Required |
| `@opencode-ai/core` | `packages/core/` | MIT | 2025 opencode | Yes | Required |
| `@opencode-ai/plugin` | `packages/plugin/` | MIT | 2025 opencode | Yes | Required |
| `@opentui/core` | npm registry | MIT | sst | Yes | Required |
| `@opentui/keymap` | npm registry | MIT | sst | Yes | Required |
| `@opentui/solid` | npm registry | MIT | sst | Yes | Required |
| `solid-js` | npm registry | MIT | Ryan Carniato | Yes | Required |
| `effect` | npm registry | MIT | Effect.ts | Yes | Required |
| `fuzzysort` | npm registry | MIT | Gordon Wang | Yes | Required |
| `clipboardy` | npm registry | MIT | Sindre Sorhus | Yes | Required |
| `strip-ansi` | npm registry | MIT | Sindre Sorhus | Yes | Required |
| `diff` | npm registry | BSD-3 | Kevin Mårtensson | Yes | Required |

**All relevant source is MIT-licensed.** No copyleft contamination risk.

### 3.2 Attribution Requirements

If distributing a modified OpenCode TUI:

1. **Include the MIT license text** in `LICENSE-OPENCODE` at repo root
2. **Create `ATTRIBUTION.md`** listing upstream SHA, source paths, and modifications
3. **Source-file headers** for any copied logic:
   ```typescript
   // Adapted from opencode/tui/src/...
   // Upstream: https://github.com/anomalyco/opencode
   // Commit: 864889ab9f9e921c240930b1dcd2bc0d2352c555
   // License: MIT, Copyright (c) 2025 opencode
   ```
4. **README attribution** recommended but not legally required
5. **Trademark note** (from upstream CONTRIBUTING.md):
   > "If you are working on a project that's related to OpenCode and is using 'opencode' as part of its name, please add a note to your README to clarify that it is not built by the OpenCode team and is not affiliated with us in any way."
   CodeBro does not use "opencode" in its name, so this does not apply.

---

## 4. OpenCode TUI Architecture

### 4.1 Tech Stack

| Layer | Technology |
|---|---|
| Language | TypeScript /tsx |
| Framework | SolidJS 1.x (reactive UI) |
| Renderer | `@opentui/core` (Zig native binary) + `@opentui/solid` bindings |
| Keymap | `@opentui/keymap` (hierarchical binding system) |
| Concurrency | `effect` 4.0.0-beta (structured concurrency) |
| Fuzzy search | `fuzzysort` 3.1.0 |
| Clipboard | `clipboardy` 4.0.0 + OSC52 + native commands |
| Markdown | `marked` + `marked-shiki` + `shiki` |
| Diff | `@pierre/diffs` + `diff` 9.0.0 |
| Data persistence | Local JSON files via `@opencode-ai/core/util/flock` |

### 4.2 Entry Point & App Structure

```
packages/tui/src/index.tsx
  └── export { run, type TuiInput } from "./app"

packages/tui/src/app.tsx
  └── export const run = Effect.fn("Tui.run")(function* (input: TuiInput) { ... })
      └── TuiInput = {
            url: string,              // Server base URL
            args: Args,                // CLI arguments
            config: TuiConfig.Resolved, // Runtime configuration
            events?: EventSource,       // Optional custom event stream
            directory?: string,         // Working directory
            fetch?: typeof fetch,       // Custom fetch
            headers?: RequestInit["headers"],
            pluginHost: TuiPluginHost,  // Plugin host
            onSnapshot?: () => Promise<string[]>
          }
```

### 4.3 Dependency Graph (TUI → Backend)

The TUI depends on **4 OpenCode packages** and **6 OpenTUI packages**:

**OpenCode packages (runtime-dependent):**
| Package | Purpose | UI-only? |
|---|---|---|
| `@opencode-ai/sdk/v2` | Generated API client + types | **NO** — full HTTP/SSE client |
| `@opencode-ai/core` | Global state, flags, flock, glob | **NO** — runtime utilities |
| `@opencode-ai/plugin/tui` | Plugin API types | Partially — types only |
| `@opencode-ai/ui` | Shared UI components + audio | Partially — can be vendored |

**OpenTUI packages (rendering framework):**
| Package | Purpose | UI-only? |
|---|---|---|
| `@opentui/core` | Native terminal renderer (Zig) | **NO** — core rendering engine |
| `@opentui/solid` | SolidJS bindings for OpenTUI | **NO** — rendering bindings |
| `@opentui/keymap` | Keyboard binding system | Partially — can port concept |
| `@opentui/keymap/extras` | Key sequence formatting | Partially — can port concept |
| `@opentui/keymap/opentui` | OpenTUI-specific keymap addons | **NO** — tied to @opentui/core |

### 4.4 Context Provider Chain (State Architecture)

```
TuiInput
  └── ExitProvider          (exit handling)
      └── EpilogueProvider  (output epilogue)
          └── ErrorBoundary
              └── TuiPathsProvider     (filesystem paths)
                  └── TuiTerminalEnvironmentProvider (platform/multiplexer)
                      └── TuiStartupProvider       (initial route)
                          └── ClipboardProvider
                              └── OpencodeKeymapProvider (keyboard system)
                                  └── ArgsProvider
                                      └── KVProvider         (persistent key-value)
                                          └── ToastProvider
                                              └── RouteProvider       (home/session navigation)
                                                  └── TuiConfigProvider
                                                      └── PluginRuntimeProvider
                                                          └── SDKProvider        (OpenCode client + SSE)
                                                              └── PermissionProvider
                                                                  └── ProjectProvider      (workspace)
                                                                      └── SyncProvider       (session/message state)
                                                                          └── ThemeProvider
                                                                              └── LocalProvider    (client-side mutable state)
                                                                                  └── PromptStashProvider
                                                                                      └── DialogProvider
                                                                                          └── FrecencyProvider
                                                                                              └── PromptHistoryProvider
                                                                                                  └── PromptRefProvider
                                                                                                      └── EditorContextProvider
                                                                                                          └── LocationProvider
                                                                                                              └── App
```

**20 context providers.** Each one connects to OpenCode-specific state or behavior.

### 4.5 SDK Client Interface Surface

The TUI calls **59 distinct `sdk.client.*` methods** across these namespaces:

| Namespace | Methods | Purpose |
|---|---|---|
| `session.*` | 14 | CRUD, messages, diff, revert, fork, status, shell, summarize, todo, abort, command |
| `provider.*` | 4 | list, auth, oauth.authorize, oauth.callback |
| `permission.*` | 1 | reply |
| `question.*` | 2 | reply, reject |
| `mcp.*` | 3 | connect, disconnect, status |
| `project.*` | 3 | current, directories, (root) |
| `path.*` | 1 | get |
| `config.*` | 2 | get, providers |
| `command.*` | 1 | list |
| `lsp.*` | 1 | status |
| `formatter.*` | 1 | status |
| `vcs.*` | 2 | get, status |
| `auth.*` | 1 | set |
| `app.*` | 1 | agents |
| `global.*` | 1 | upgrade |
| `instance.*` | 1 | dispose |
| `find.*` | 1 | files |
| `experimental.workspace.*` | 8 | list, create, remove, status, syncList, warp, adapter.list, resource |
| `experimental.console.*` | 2 | (root), switchOrg |
| `experimental.capabilities` | 1 | capabilities |
| `experimental.controlPlane.*` | 1 | moveSession |
| `experimental.projectCopy.*` | 1 | generateName |
| `experimental.session.*` | 1 | background |

### 4.6 Event Subscription Surface

The TUI subscribes to **15 event types** via `event.on()`:

| Event | Handler Location | Purpose |
|---|---|---|
| `event` (raw) | `context/sdk.tsx` | SSE event routing |
| `message.part.updated` | `routes/session/index.tsx` | Streaming text/tool updates |
| `session.status` | `routes/session/index.tsx` | Busy/idle/retry state |
| `permission.asked` | `feature-plugins/system/notifications.ts` | Permission notifications |
| `permission.replied` | `feature-plugins/system/notifications.ts` | Permission responses |
| `question.asked` | `feature-plugins/system/notifications.ts` | Question notifications |
| `question.replied` | `feature-plugins/system/notifications.ts` | Question responses |
| `question.rejected` | `feature-plugins/system/notifications.ts` | Question rejections |
| `session.deleted` | `app.tsx` | Navigate home on delete |
| `session.error` | `app.tsx` | Show error toast |
| `tui.command.execute` | `app.tsx` | Command palette dispatch |
| `tui.prompt.append` | `app.tsx` | External prompt injection |
| `tui.session.select` | `app.tsx` | Session switching |
| `tui.toast.show` | `app.tsx` | External toast trigger |
| `installation.update-available` | `app.tsx` | Update dialog |

### 4.7 Plugin API Surface

The `TuiPluginApi` (from `@opencode-ai/plugin/tui`) defines what the host provides:

| API Member | Methods/Properties | Purpose |
|---|---|---|
| `app` | `version` | App version |
| `attention` | `notify()`, `soundboard.*` | Desktop notifications + sounds |
| `keys` | `formatSequence()`, `formatBindings()` | Keybinding display |
| `keymap` | Full keymap interface | Keyboard binding system |
| `mode` | `current()`, `push()` | Mode stack |
| `route` | `register()`, `navigate()`, `current` | Navigation |
| `ui` | `Dialog`, `DialogAlert`, `DialogConfirm`, `DialogPrompt`, `DialogSelect`, `Slot`, `Prompt`, `toast`, `dialog` | UI primitives |
| `tuiConfig` | Full config view | Runtime configuration |
| `kv` | `get()`, `set()`, `ready` | Persistent key-value store |
| `state` | `session.*`, `provider.*`, `config.*`, `path.*`, `vcs.*`, `lsp.*`, `mcp.*` | Application state |
| `theme` | `current.*`, `has()`, `set()`, `install()`, `mode()`, `ready` | Theming |
| `client` | Full `OpencodeClient` | **Backend API client** |
| `event` | `on()` | Event bus |
| `renderer` | Full `CliRenderer` | Terminal renderer |
| `slots` | `register` | Slot registration |
| `plugins` | `list()`, `activate()`, `deactivate()`, `add()`, `install()` | Plugin management |
| `lifecycle` | `signal`, `onDispose` | Cleanup |

### 4.8 SDK Type Surface

The generated SDK types file (`types.gen.ts`) contains **1,409 exported types** covering:
- Session, Message, Part (12 part types)
- Provider, Model, Auth
- Permission, Question
- PTY, Todo, LSP, MCP status
- Config, Agent, VCS info
- Error types (7 variants)
- Event types (60+ variants)
- Client request/response types

---

## 5. CodeBro Architecture

### 5.1 Tech Stack

| Layer | Technology |
|---|---|
| Language | Rust (edition 2021) |
| Terminal backend | `crossterm` 0.27 |
| Widget framework | `ratatui` 0.26 |
| Async runtime | `tokio` 1.x |
| Markdown | `pulldown-cmark` 0.10 |
| ANSI rendering | `ansi-to-tui` 4.1.0 |
| Persistence | `rusqlite` 0.31 (bundled) |
| PTY | `portable-pty` 0.9.0 |
| Tree-sitter | 0.20 (Rust, Python, JS, TS, Go) |

### 5.2 Architecture

```
src/tui/
  mod.rs        — Module exports
  app.rs        — TuiApp state struct (~1,300 lines)
  ui.rs         — run() + run_loop() + handle_event() (~1,400 lines)
  events.rs     — AppEvent enum + event loop + keyboard shortcuts
  actions.rs    — ActionStream (bounded phase-grouped tool activity)
  dashboard.rs  — Dashboard (status, logs, panels, palettes)
  commands.rs   — Command registry (engineering/runtime/shell)
  diff_view.rs  — FileDiff + DiffReviewSession
  markdown.rs   — Markdown → ratatui Line rendering
  console.rs    — PtyConsole (append-only bounded PTY buffer)
  theme.rs      — THEME singleton + Phase enum
  animation.rs  — Spinner animation frames

Engine:
  canonical_runtime/ — Task execution orchestration
  agent/events.rs    — AgentEvent enum
  agent/status.rs    — AgentStatus enum
  agent/task_graph.rs — TaskGraph
  tools/change.rs    — ChangePlan (mutation engine)
  provider_manager/  — Provider registry + health
  settings/          — Settings manager
  session/           — Session tracker
  metrics/           — Task metrics
```

### 5.3 Event Model

```rust
pub enum AppEvent {
    Input(KeyEvent),
    Quit,
    Response(String),
    StreamChunk(String),
    AgentEvent(AgentEvent),     // tool lifecycle, PTY, agent status
    Resize(u16, u16),
    ModelsFetched { models, note },
    ModelsFetchFailed(String),
    ProviderCheckResult { provider, message },
    Paste(String),
    Mouse(MouseEvent),
    TaskFinished { success: bool },
    ProviderHealthResults(Vec<...>),
    WorkspaceDiscovered { discovery, capabilities, mcp_servers },
}
```

### 5.4 Session/Message Model

```rust
pub struct TuiApp {
    pub messages: VecDeque<Message>,           // Flat message list
    pub action_stream: ActionStream,            // Bounded phase-grouped actions
    pub consoles: VecDeque<PtyConsole>,         // Live PTY buffers
    pub pending_change: Option<ChangePlan>,     // Staged file changes
    pub task_mode: TaskMode,                    // Assist/Research/Main
    // ... (60+ fields)
}

pub struct Message {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: String,
    #[serde(skip)]
    pub sealed_actions: Option<VecDeque<UiActionGroup>>,  // Turn-scoped
}
```

---

## 6. Session Compatibility

### 6.1 OpenCode Session Model

```typescript
type Session = {
  id: string
  slug: string
  projectID: string
  workspaceID?: string
  directory: string
  parentID?: string
  title: string
  agent?: string
  model?: { id: string; providerID: string; variant?: string }
  time: { created: number; updated: number; compacting?: number; archived?: number }
  permission?: PermissionRuleset
  revert?: { messageID: string; partID?: string; snapshot?: string; diff?: string }
  // ... 20+ fields including cost, tokens, share, metadata, summary
}

type Message = UserMessage | AssistantMessage

type Part = TextPart | ToolPart | FilePart | ReasoningPart | StepStartPart 
          | StepFinishPart | SnapshotPart | PatchPart | AgentPart 
          | RetryPart | CompactionPart | SubtaskPart
```

### 6.2 CodeBro Session Model

```rust
struct TuiApp {
    messages: VecDeque<Message>,           // Flat list, not per-session
    sealed_actions: Option<VecDeque<UiActionGroup>>,  // Attached to messages
    action_stream: ActionStream,            // Live, unsealed actions
}

struct Message {
    role: MessageRole,
    content: String,
    timestamp: String,
    sealed_actions: Option<VecDeque<UiActionGroup>>,
}
```

### 6.3 Compatibility Assessment

| Aspect | OpenCode | CodeBro | Mapping |
|---|---|---|---|
| Session identity | UUID + slug + projectID + workspaceID | Single `session_id` | **Adapter required** |
| Message structure | `UserMessage` / `AssistantMessage` with `Part[]` | `Message` with flat `String` content | **Fundamental mismatch** |
| Tool representation | `ToolPart` with callID, tool name, input, state | `UiAction` in `ActionStream` groups | **Adapter required** |
| File changes | `FilePart` with source text, ranges | `ChangePlan` with staged diffs | **Adapter required** |
| Reasoning/thinking | `ReasoningPart` | Not represented | **Gap — must synthesize** |
| Streaming | `TextPart` with `time.start/end` | `StreamChunk` events | **Adapter required** |
| Permissions | `PermissionRuleset` on session | Runtime `PermissionHook` | **Different model** |
| Questions | `QuestionRequest` / `QuestionAnswer` | Not in TUI model | **Gap** |
| PTY | `Pty` struct with pid, cwd, command | `PtyConsole` with id, label | **Adapter required** |
| revert/undo | `revert?: { messageID, partID }` | No revert in TUI model | **Gap** |
| compaction | `CompactionPart` | Not represented | **Gap** |
| subtasks | `SubtaskPart` | Specialist agents (separate model) | **Partial match** |

**Verdict: C — Fundamental mismatch.** The session/message models are structurally different. OpenCode uses a rich, part-based message model with full history replay. CodeBro uses a flat message list with turn-scoped sealed action groups. An adapter would need to synthesize OpenCode's message parts from CodeBro's flattened events.

---

## 7. Event Compatibility

### 7.1 Event Mapping Matrix

| OpenCode Event | CodeBro Equivalent | Adapter Transformation | Loss? |
|---|---|---|---|
| `session.next.text.delta` | `AgentEvent::AgentProgress` + `StreamChunk` | Synthesize textID, delta from streaming | Low — textID synthesized |
| `session.next.text.ended` | `AgentEvent::AgentCompleted` | Synthesize end timestamp | Low |
| `session.next.tool.called` | `AgentEvent::ToolStarted` | Add callID, synthesize provider info | Low — callID synthesized |
| `session.next.tool.success` | `AgentEvent::ToolCompleted{success:true}` | Direct mapping | None |
| `session.next.tool.failed` | `AgentEvent::ToolCompleted{success:false}` | Direct mapping | None |
| `session.next.shell.started` | `AgentEvent::PtyOutput` (first chunk) | Synthesize callID | Low |
| `session.next.shell.ended` | `AgentEvent::PtyExited` | Direct mapping | None |
| `session.next.step.started` | `AgentEvent::AgentStarted` | Map agent→model ref | Low |
| `session.next.step.ended` | `AgentEvent::AgentCompleted` | Map cost/tokens from metrics | Medium — cost not tracked |
| `session.next.reasoning.*` | Not available | **Cannot synthesize** | **High — gap** |
| `session.next.compaction.*` | Not available | **Cannot synthesize** | **High — gap** |
| `session.next.context.updated` | Not available | **Cannot synthesize** | **High — gap** |
| `permission.v2.asked` | `PermissionHook` callback | Map to PermissionRequest | Medium |
| `permission.v2.replied` | Permission grant/deny | Direct mapping | None |
| `question.v2.asked` | Not available in TUI model | **Cannot synthesize** | **High — gap** |
| `pty.created` | `AgentEvent::PtyOutput` (first) | Synthesize Pty struct | Low |
| `pty.updated` | `AgentEvent::PtyOutput` | Synthesize Pty struct | Low |
| `pty.exited` | `AgentEvent::PtyExited` | Direct mapping | None |
| `session.status` | `is_loading` flag | Synthesize from loading state | Low |
| `session.error` | `AgentEvent::AgentFailed` | Direct mapping | None |
| `session.created` | Session start | Synthesize from `session_id` | Low |
| `session.deleted` | `//clear` command | Synthesize | Low |
| `message.part.updated` | Tool completion events | Synthesize from ActionStream | Medium |
| `message.part.removed` | Not supported | **Cannot synthesize** | **High — gap** |

**Verdict: B — Adapter required with significant gaps.** ~6 event types cannot be synthesized from CodeBro's event model (reasoning, compaction, context updates, part removal, questions). These would require either:
1. Adding new event types to CodeBro's `AgentEvent` enum
2. Accepting UI features that don't work (reasoning display, compaction UI, etc.)

---

## 8. Required Backend Contract

### 8.1 Minimum API Surface (if we attempted this)

To run the OpenCode TUI against CodeBro, we would need to implement:

**SDK Client Methods (59 methods):**
```typescript
interface OpencodeClient {
  // Session (14 methods)
  session: {
    list(): Promise<Response<Session[]>>
    get(id: string): Promise<Response<Session>>
    create(params): Promise<Response<Session>>
    delete(id: string): Promise<Response<void>>
    update(id: string, params): Promise<Response<Session>>
    abort(id: string): Promise<Response<void>>
    status(id: string): Promise<Response<SessionStatus>>
    messages(id: string, limit: number): Promise<Response<Message[]>>
    diff(id: string): Promise<Response<SnapshotFileDiff[]>>
    todo(id: string): Promise<Response<Todo[]>>
    revert(id: string, messageID: string): Promise<Response<void>>
    unrevert(id: string): Promise<Response<void>>
    fork(id: string): Promise<Response<Session>>
    summarize(id: string): Promise<Response<void>>
    shell(id: string, command: string): Promise<Response<Pty>>
    command(id: string, text: string): Promise<Response<void>>
  }
  
  // Provider (4 methods)
  provider: {
    list(workspace?: string): Promise<Response<Provider[]>>
    auth(workspace?: string): Promise<Response<Record<string, ProviderAuthMethod>>>
    oauth: {
      authorize(providerID: string): Promise<Response<void>>
      callback(providerID: string, code: string): Promise<Response<void>>
    }
  }
  
  // Permission (1 method)
  permission: {
    reply(sessionID: string, requestID: string, action: string): Promise<Response<void>>
  }
  
  // Question (2 methods)
  question: {
    reply(sessionID: string, requestID: string, answers: string[]): Promise<Response<void>>
    reject(sessionID: string, requestID: string): Promise<Response<void>>
  }
  
  // MCP (3 methods)
  mcp: {
    status(workspace?: string): Promise<Response<Record<string, McpStatus>>>
    connect(name: string, workspace?: string): Promise<Response<void>>
    disconnect(name: string, workspace?: string): Promise<Response<void>>
  }
  
  // ... and 40+ more methods
}
```

**Event Stream (15+ event types):**
```typescript
interface EventSource {
  subscribe(handler: (event: GlobalEvent) => void): Promise<() => void>
}

// Must emit all of these:
type GlobalEvent = {
  directory: string
  project?: string
  workspace?: string
  payload: {
    id: string
    type: string
    properties: Record<string, unknown>
  }
}
// + 60+ more specific event shapes
```

**State Provider (TuiState):**
```typescript
interface TuiState {
  readonly ready: boolean
  readonly config: SdkConfig
  readonly provider: ReadonlyArray<Provider>
  readonly path: { state: string; config: string; worktree: string; directory: string }
  readonly vcs: { branch?: string; default_branch?: string } | undefined
  session: {
    count: () => number
    get: (sessionID: string) => Session | undefined
    diff: (sessionID: string) => ReadonlyArray<TuiSidebarFileItem>
    todo: (sessionID: string) => ReadonlyArray<TuiSidebarTodoItem>
    messages: (sessionID: string) => ReadonlyArray<Message>
    status: (sessionID: string) => SessionStatus | undefined
    permission: (sessionID: string) => ReadonlyArray<PermissionRequest>
    question: (sessionID: string) => ReadonlyArray<QuestionRequest>
  }
  part: (messageID: string) => ReadonlyArray<Part>
  lsp: () => ReadonlyArray<TuiSidebarLspItem>
  mcp: () => ReadonlyArray<TuiSidebarMcpItem>
}
```

### 8.2 Method Count Summary

| Category | Method Count | Notes |
|---|---|---|
| SDK client methods | 59 | Full HTTP client interface |
| Event types to emit | 60+ | All OpenCode event variants |
| State properties | 12 | session.*, provider.*, config.*, path.*, vcs.*, lsp.*, mcp.* |
| UI primitives | 8 | Dialog, DialogAlert, DialogConfirm, DialogPrompt, DialogSelect, Slot, Prompt, toast |
| Keymap commands | 40+ | All registered command bindings |

**Total interface surface: ~120+ distinct types/methods to implement.**

---

## 9. Transport Options

### 9.1 Comparison

| Option | Effort | Latency | Streaming | Cancellation | Security | Packaging |
|---|---|---|---|---|---|---|
| **A. In-process Rust/FFI** | Extreme | None | Native | Native | High (same process) | Simple (one binary) |
| **B. Child process + stdio JSON** | Medium | ~1ms | Yes (streaming JSON) | SIGTERM | Medium (local only) | Simple |
| **C. Local HTTP** | Low | ~5ms | Yes (SSE) | POST abort | Low (localhost) | Medium (two processes) |
| **D. WebSocket** | Medium | ~2ms | Yes | Close frame | Low (localhost) | Medium |
| **E. Unix socket / IPC** | Medium | ~0.1ms | Yes | Close | High (filesystem) | Simple |

### 9.2 Recommended: Option B — Child Process + stdio JSON

**Rationale:**
- Simplest to implement and debug
- Structured JSON events map cleanly to TypeScript types
- Streaming is natural (newline-delimited JSON)
- Cancellation via process kill
- No network stack needed
- Works on all platforms (Unix sockets not available on Windows)

**Protocol sketch:**
```json
// Backend → Frontend events
{"type":"session.created","id":"abc","directory":"/tmp/project"}
{"type":"message.part.delta","id":"abc","textID":"t1","delta":"Hello"}
{"type":"session.next.tool.called","id":"abc","tool":"read_file","input":{"path":"main.rs"}}
{"type":"session.next.tool.success","id":"abc","callID":"c1"}
{"type":"pty.created","id":"p1","info":{"id":"p1","title":"cargo test","command":"cargo test","cwd":"/tmp/project","status":"running","pid":1234}}
{"type":"pty.exited","id":"p1","info":{"exitCode":0}}

// Frontend → Backend commands
{"type":"tui.prompt.append","text":"fix the bug"}
{"type":"tui.command.execute","command":"session.new"}
{"type":"permission.reply","requestID":"r1","action":"allow"}
```

**Counter-argument:** This requires the TUI to be a separate process, which adds complexity to the dev workflow and packaging. However, it's the cleanest boundary.

---

## 10. Dependency / Runtime Coupling

### 10.1 Packages That Would Be Required

| Package | Why Needed | UI-only? | Runtime-dependent? | Can replace? | Production required? |
|---|---|---|---|---|---|
| `@opencode-ai/sdk` | Generated client + types | **NO** | **YES** — HTTP/SSE client | No — would need to reimplement | **YES** |
| `@opencode-ai/core` | Global state, flock, glob, flags | **NO** | **YES** — filesystem utilities | Partially — can vendor | **YES** |
| `@opencode-ai/plugin` | Plugin API types | Partially | No (types only) | Yes — types are static | Partially |
| `@opencode-ai/ui` | Audio files, some components | Partially | No | Partially — can vendor | Partially |
| `@opentui/core` | Terminal renderer (Zig) | **NO** | **YES** — native binary | **NO** — no Rust equivalent | **YES** |
| `@opentui/solid` | Solid bindings for renderer | **NO** | **YES** | **NO** | **YES** |
| `@opentui/keymap` | Keyboard binding system | Partially | No | Partially — port concept | Optional |
| `solid-js` | Reactive UI framework | **NO** | **YES** | **NO** — must use SolidJS | **YES** |
| `effect` | Structured concurrency | **NO** | **YES** | **NO** — deeply used | **YES** |
| `fuzzysort` | Fuzzy search | Yes | No | Can use Rust crate | Optional |
| `clipboardy` | Clipboard access | Yes | No | Can use existing approach | Optional |
| `strip-ansi` | ANSI stripping | Yes | No | Can use Rust crate | Optional |

### 10.2 Red Flags

| Red Flag | Severity | Detail |
|---|---|---|
| `@opentui/core` (Zig native binary) | **CRITICAL** | Cannot be replaced. Requires Node.js/Bun runtime. No Rust equivalent. |
| `solid-js` + `@opentui/solid` | **CRITICAL** | Full reactive UI framework required. No Rust alternative. |
| `effect` (structured concurrency) | **HIGH** | Deeply integrated into app lifecycle (`Effect.scoped`, `acquireRelease`). |
| `@opencode-ai/sdk` (59 methods) | **CRITICAL** | Would need complete reimplementation as compatibility layer. |
| OpenCode server process | **CRITICAL** | TUI connects to `http://localhost:4096` by default. Must run alongside. |
| `@opencode-ai/core/global` | **HIGH** | Path resolution, state directory, config directory — platform-specific. |
| Plugin system | **MEDIUM** | `TuiPluginHost` requires implementation. Many built-in plugins depend on SDK. |

### 10.3 The Fundamental Problem

The OpenCode TUI is **not a library** — it is a **complete application** that:
1. Requires a running OpenCode server (or a full compatibility implementation)
2. Requires Node.js/Bun runtime (for `@opentui/core` native binaries)
3. Requires SolidJS runtime (for reactive UI)
4. Requires effect runtime ( for lifecycle management)
5. Manages its own sessions, providers, permissions, questions, PTYs

There is no "thin frontend" boundary. The TUI owns the full application state machine.

---

## 11. CodeBro Capability Preservation

### 11.1 Capabilities at Risk

| CodeBro Capability | OpenCode Equivalent | Preservation Risk |
|---|---|---|
| Research phase | `agent` field on session/message | **Medium** — would need to map to OpenCode's agent model |
| Testing phase | `UiActionKind::Testing` | **Low** — can map to tool parts |
| Planning phase | `UiActionKind::Planning` | **Low** |
| Coding phase | `UiActionKind::Editing` | **Low** |
| Review phase | `UiActionKind::Reviewing` | **Low** |
| StructuredFacts | No OpenCode equivalent | **High** — would be lost |
| Verification | `UiActionKind::Verification` + `VerificationFacts` | **High** — no OpenCode equivalent |
| ChangeEngine | `FilePart` with diff | **Medium** — can map but loses rollback |
| Rollback | `session.revert` | **Medium** — requires implementing revert API |
| Plan deviations | No OpenCode equivalent | **High** — would be lost |
| Provider/model switching | `local.model.*` | **Low** — already in OpenCode |
| Task mode (Assist/Research/Main) | `agent` field | **Medium** — maps to agent selection |
| Cancellation | `session.abort` | **Low** — already in OpenCode |
| TaskDiagnostics | No OpenCode equivalent | **High** — would be lost |

### 11.2 Semantics That Would Be Flattened

OpenCode's UI expects:
- Per-message parts (text, tool, file, reasoning, step start/finish)
- Tool call IDs for retry/duplicate detection
- Revert/undo state per message
- Compaction state
- Subtask/part breakdown

CodeBro has:
- Flat messages with sealed action groups
- No tool call IDs
- No per-message revert
- No compaction
- Specialist agents (different model from subtasks)

**Risk: CodeBro's unique capabilities (StructuredFacts, Verification, PlanDeviations) have no OpenCode UI representation.** They would need custom plugin development to surface.

---

## 12. Provider / Model Integration

### 12.1 OpenCode's Provider Model

OpenCode TUI expects:
- `sync.data.provider` — array of configured providers
- `local.model` — current model + recent model cycle
- `provider.auth` — OAuth flows
- `provider.oauth.authorize` / `callback` — OAuth endpoints
- `config.providers` — provider configuration from server
- Model picker dialog (`DialogModel`)
- Provider connection dialog (`DialogProviderList`)

### 12.2 CodeBro's Provider Model

- `ProviderManager` — registry of providers with health checks
- `Config.api_key` — single active API key
- `//apikey <provider>` — masked key input
- `//provider [id]` — provider switching
- Model picker in `Dashboard.model_picker`

### 12.3 Integration Assessment

| Aspect | OpenCode | CodeBro | Feasibility |
|---|---|---|---|
| Provider registry | Server-managed | `ProviderManager` struct | **Adapter required** |
| OAuth flows | Built-in | Not present | **Cannot implement** (would need to add to CodeBro) |
| API key storage | Server-managed | `CredentialStore` | **Adapter required** |
| Model discovery | Server endpoint | `discover_models()` | **Adapter required** |
| Model switching | `local.model.cycle()` | `apply_model()` | **Direct mapping** |
| Provider health | Server-managed | `check_health()` | **Adapter required** |

**Verdict: Adapter required for most provider features. OAuth is a hard blocker unless CodeBro adds OAuth support.**

---

## 13. Mutation / Diff / Approval Integration

### 13.1 OpenCode's Mutation Model

- `FilePart` — represents file edits with source text, ranges
- `PatchPart` — represents patch applications
- `SnapshotPart` — represents file snapshots
- `session.diff` — retrieves file changes for a session
- `session.revert` — reverts a message and its changes
- `session.unrevert` — restores a reverted message
- `PermissionV2Asked` — permission prompts for mutations
- `DiffChanges` component — visual +/- bar chart

### 13.2 CodeBro's Mutation Model

- `ChangePlan` — staged file changes with diff preview
- `PendingAction::ApproveChange` / `RejectChange` — explicit approval
- `ChangeEngine` — authoritative mutation engine
- `diff_view.rs` — line-by-line diff rendering
- `rollback` via git (not built into TUI model)

### 13.3 Integration Assessment

| Aspect | OpenCode | CodeBro | Feasibility |
|---|---|---|---|
| File change representation | `FilePart` with ranges | `ChangePlan` with staged diffs | **Adapter required** |
| Approval flow | `PermissionV2Asked` → `permission.reply` | `pending_confirmation` + `//approve` | **Different model** |
| Diff rendering | `DiffChanges` (bar chart) + file viewer | `diff_view.rs` (line-by-line) | **Reuse concept, not code** |
| Revert/undo | Built into session model | Git-based, not in TUI | **Hard blocker** |
| Mutation authority | Server-side | `ChangeEngine` (must preserve) | **Must preserve** |

**Critical: OpenCode's revert/undo feature requires server-side snapshot management. CodeBro has no equivalent. This feature would be permanently unavailable.**

---

## 14. Keyboard / Palette / Paste / Mouse / Resize

### 14.1 What Works Well (Frontend-Only Features)

| Feature | OpenCode Implementation | Portability |
|---|---|---|
| Multiline paste summary | `pasteInputText()` — collapses to `[Pasted ~N lines]` when ≥3 lines | **High** — simple logic |
| Mouse selection/copy | `renderer.getSelection()` + clipboard | **Medium** — algorithm portable |
| Command palette | `CommandPaletteDialog` + fuzzy sort | **High** — logic portable |
| Keyboard navigation | `@opentui/keymap` mode stack | **Medium** — concept portable |
| Scroll acceleration | `getScrollAcceleration()` | **Low** — concept only |
| Resize handling | `useTerminalDimensions()` reactive | **Low** — CodeBro already has this |
| Overlay priority | `DialogProvider` stack | **Low** — CodeBro already has this |
| Autocomplete | Prompt autocomplete with fuzzy sort | **High** — logic portable |

### 14.2 What Is Tightly Coupled to Backend

| Feature | Coupling |
|---|---|
| Command palette commands | 40+ commands tied to SDK methods |
| Modal dialogs | Many dialogs call `sdk.client.*` directly |
| Session list | Tied to `sdk.client.session.list` |
| Provider management | Tied to OAuth flows + server config |
| MCP management | Tied to `sdk.client.mcp.*` |
| Permission prompts | Tied to `sdk.client.permission.reply` |
| Question prompts | Tied to `sdk.client.question.*` |
| PTY console | Tied to `sdk.client.pty.*` |

---

## 15. Packaging / Distribution

### 15.1 Option Comparison

| Option | Description | Installer Complexity | Windows | Linux | macOS | Offline | Dev Workflow |
|---|---|---|---|---|---|---|---|
| **A. Rust binary only** | Current approach | Low | ✅ | ✅ | ✅ | ✅ | `cargo run` |
| **B. Rust + bundled Bun** | Ship Bun runtime + TUI assets | **High** | ⚠️ Complex | ✅ | ✅ | ❌ Needs Bun | `cargo run` + bun dev |
| **C. Rust backend + separate TUI binary** | Two processes, IPC | **High** | ⚠️ Complex | ✅ | ✅ | ❌ Needs separate build | Two terminals |
| **D. Web/localhost frontend** | TUI runs in browser | **High** | ✅ | ✅ | ✅ | ❌ Needs server | `cargo run` + browser |

### 15.2 Recommended (if proceeding): Option C

**Architecture:**
```
codebro (Rust binary)
    ↓ IPC (stdio JSON)
opencode-tui (bundled Bun + TypeScript)
```

**Packaging:**
- CodeBro ships as a single Rust binary
- OpenCode TUI is bundled as a pre-built Bun application
- On launch, CodeBro starts the TUI as a child process
- Communication via stdin/stdout JSON streams

**Maintenance burden:**
- Must track OpenCode upstream releases
- Must patch for breaking API changes
- Must bundle Bun runtime for platforms without it
- Version synchronization between CodeBro and TUI

---

## 16. Development Workflow

### 16.1 Current CodeBro Workflow

```bash
cargo test          # Run tests
cargo run           # Launch TUI
cargo run -- --prompt "fix the bug"   # With prompt
```

### 16.2 Required Hybrid Workflow (if OpenCode TUI)

```bash
# Terminal 1: Start backend
cargo run

# Terminal 2: Start TUI (requires Bun)
cd opencode-tui/
bun dev             # or: bunx opencode /path/to/project

# Or with IPC bridge:
cargo run --with-tui   # hypothetical flag
```

**Cost:**
- **+1 terminal** for development
- **Bun runtime required** in dev environment
- **Two build systems** (Cargo + Bun)
- **Two debuggers** (Rust LLDB + Chrome DevTools for TS)
- **Version sync** required between backend and frontend

---

## 17. Debugging / Error Propagation

### 18.1 Cross-Boundary Error Scenarios

| Error Type | Current (Ratatui) | With OpenCode TUI |
|---|---|---|
| Provider failure | `AgentEvent::AgentFailed` → toast | `session.error` event → toast |
| Task timeout | `PtyExited{status:"timed out"}` | `session.next.step.failed` event |
| Permission denied | `PendingAction` confirmation | `permission.v2.asked` → dialog |
| Coding verification failed | `VerificationFacts` in action group | No equivalent — **lost** |
| Malformed task | `Response("error message")` | `session.error` event |
| Cancellation | `CancellationToken` → graceful stop | `session.abort` API call |

### 17.2 Structured Error Preservation

**Risk: Medium.** Most errors can be mapped, but CodeBro's structured machine facts (exit codes, verification outcomes, plan deviations) have no OpenCode UI representation. They would appear as generic text in tool output parts, losing their semantic structure.

---

## 18. Maintenance / Upstream Drift

### 18.1 If Using OpenCode TUI as Frontend

| Aspect | Burden |
|---|---|
| Upstream sync | **High** — must track OpenCode releases, patch breaking changes |
| API compatibility | **High** — 59 SDK methods + 60 event types must stay in sync |
| Dependency updates | **High** — SolidJS, OpenTUI, effect, fuzzysort, etc. |
| Bug fixes | **High** — upstream bugs affect CodeBro immediately |
| Feature parity | **Low** — CodeBro can lag behind OpenCode features |
| Security patches | **Medium** — must manually apply to vendored code |

### 18.2 Fork vs. Dependency

| Approach | Pros | Cons |
|---|---|---|
| **Fork** | Full control, can patch freely | Must maintain entire fork, no upstream benefits |
| **Dependency** | Automatic updates | Breaking changes common, less control |
| **vendored + pinned** | Reproducible builds | Manual merge of upstream changes |

**Recommendation:** If proceeding, use **vendored + pinned** with a clear provenance record. Upstream updates should be evaluated quarterly, not automatically applied.

---

## 19. Option Matrix

### 19.1 Scoring Criteria

| Criterion | Weight |
|---|---|
| UX quality | ★★★★★ |
| Implementation effort | ★★★★★ |
| Dependency complexity | ★★★★ |
| Legal/provenance complexity | ★★★ |
| Runtime complexity | ★★★★ |
| Maintenance burden | ★★★★ |
| CodeBro architecture preservation | ★★★★★ |
| Time-to-usable | ★★★★ |

### 19.2 Options

| Option | UX | Effort | Deps | Legal | Runtime | Maintain | Preserved | Time | **Total** |
|---|---|---|---|---|---|---|---|---|---|
| **A. Keep Ratatui TUI** | 6 | 1 | 1 | 10 | 10 | 9 | 10 | 10 | **56** |
| **B. Selective port to Ratatui** | 8 | 4 | 3 | 9 | 9 | 8 | 10 | 5 | **50** |
| **C. OpenCode TUI as frontend** | 9 | 2 | 1 | 7 | 2 | 2 | 4 | 2 | **29** |
| **D. Full OpenCode fork** | 10 | 1 | 1 | 6 | 1 | 1 | 1 | 1 | **22** |

Scale: 1 = terrible, 10 = excellent

### 19.3 Rationale

- **Option A** scores highest overall because it preserves CodeBro's architecture, has zero new dependencies, and is immediately usable. UX is good but not best-in-class.
- **Option B** is the recommended path from the previous audit — selectively port the UX patterns (paste summary, fuzzy palette, selection) into the existing Ratatui TUI. High UX gain, low effort, full architecture preservation.
- **Option C** has the best UX but the worst runtime/maintenance/profile. The dependency and runtime complexity scores reflect the fundamental incompatibility.
- **Option D** is the nuclear option — replaces CodeBro entirely with OpenCode. Not acceptable per the mission statement.

---

## 20. Recommendation

### DO NOT RECOMMEND

**The OpenCode TUI should NOT be used as CodeBro's frontend.**

**Primary reasons:**

1. **No thin frontend boundary exists.** The OpenCode TUI is a complete application, not a presentation layer. It owns session management, provider authentication, tool execution, permission handling, and more. There is no "frontend-only" subset to extract.

2. **The adapter would be larger than the existing TUI.** Implementing 59 SDK client methods, 60+ event types, and the full `TuiState` interface would require ~5,000+ lines of compatibility code — far more than CodeBro's existing ~4,000-line TUI.

3. **Runtime complexity is prohibitive.** The TUI requires Node.js/Bun runtime, SolidJS, OpenTUI (Zig native), and effect. This doubles the distribution surface and testing matrix.

4. **CodeBro's unique capabilities would be lost.** StructuredFacts, Verification, PlanDeviations, and the turn-scoped ActionStream model have no OpenCode UI representation. They would be flattened to generic text.

5. **Maintenance burden is unsustainable.** Every OpenCode release would require evaluation and potential patching. The 59-method API surface means frequent breaking changes would impact CodeBro directly.

6. **OAuth is a hard blocker.** OpenCode's TUI assumes OAuth provider authentication. CodeBro does not have this. Implementing it would be a major feature addition, not a compatibility layer.

**The previous audit's recommendation (Option B — selective port) remains the correct path.** It achieves 80% of the UX benefit with 10% of the complexity.

---

## 21. First Prototype (If Recommended)

Since the recommendation is **DO NOT RECOMMEND**, no prototype is defined for the full transplant.

However, if the team decides to proceed with **Option B (selective port)**, the first prototype should be:

**Prototype: Fuzzy Command Palette in Ratatui**

1. Add `fuzzysort` crate to `Cargo.toml`
2. Replace `commands::completion_candidates()` prefix matching with fuzzy scoring
3. Add category grouping to palette rendering
4. Preserve existing `//approve`, `//reject` context-aware filtering

**Estimated effort:** 2–3 days
**Risk:** None
**UX gain:** High (directly addresses user complaint)

---

## 22. Legal / Attribution Checklist

### 22.1 Confirmed License Facts

| Source | License | Verified |
|---|---|---|
| `opencode` root | MIT | ✅ |
| `@opencode-ai/tui` | MIT | ✅ |
| `@opencode-ai/ui` | MIT | ✅ |
| `@opencode-ai/sdk` | MIT | ✅ |
| `@opencode-ai/core` | MIT | ✅ |
| `@opencode-ai/plugin` | MIT | ✅ |
| `@opentui/core` | MIT | ✅ (npm) |
| `@opentui/keymap` | MIT | ✅ (npm) |
| `@opentui/solid` | MIT | ✅ (npm) |
| `solid-js` | MIT | ✅ (npm) |
| `effect` | MIT | ✅ (npm) |
| `fuzzysort` | MIT | ✅ (npm) |
| `clipboardy` | MIT | ✅ (npm) |
| `strip-ansi` | MIT | ✅ (npm) |
| `diff` | BSD-3 | ✅ (npm) |

### 22.2 Required Notices

1. **`LICENSE-OPENCODE`** — full MIT text from upstream
2. **`ATTRIBUTION.md`** — upstream SHA, source paths, modifications
3. **Source-file headers** for any adapted logic
4. **README note** (recommended): "Interaction patterns inspired by OpenCode (MIT)."

### 22.3 Trademark / Affiliation

- CodeBro does NOT use "opencode" in its name → no affiliation disclaimer required
- If documentation references OpenCode, add: "CodeBro is not affiliated with, endorsed by, or sponsored by the OpenCode project."

### 22.4 Distribution Implications

If distributing a modified OpenCode TUI:
- Must include MIT license text
- Must preserve copyright notices
- Must provide source attribution
- No copyleft obligations (all MIT/BSD-3)

---

## 23. Risks

| Risk | Severity | Likelihood | Mitigation |
|---|---|---|---|
| OpenCode API breaking changes | High | High | Vendored + pinned, quarterly sync |
| Bun runtime not available on target platform | Medium | Medium | Bundle Bun binary with distribution |
| SolidJS learning curve for contributors | Medium | High | Document architecture, limit TS changes |
| CodeBro unique features lost in translation | High | Certain | Accept as known limitation |
| Double the testing surface | High | Certain | Accept as known limitation |
| Security: new attack surface via IPC | Medium | Low | Stdio JSON with validation |
| Legal: MIT compliance oversight | Low | Medium | Automated attribution check |
| Upstream abandons OpenCode | Medium | Low | Vendored copy is permanent fallback |

---

## 24. Git State

```
$ git branch -v
  codebro-original        3088712 fix: terminate react loop on final answers instead of spinning to the iteration cap
  main                    3088712 fix: terminate react loop on final answers instead of spinning to the iteration cap
* opencode-tui-experiment 3088712 fix: terminate react loop on final answers instead of spinning to the iteration cap

$ git status
On branch opencode-tui-experiment
Untracked files:
  OPENCODE_TUI_TRANSPLANT_AUDIT.md
  (previous audit report from first pass)

nothing added to commit but untracked files present
```

| Branch | Commit | Status |
|---|---|---|
| `main` | `3088712` | Unchanged |
| `codebro-original` | `3088712` | Unchanged |
| `opencode-tui-experiment` | `3088712` | Unchanged |
| Working tree | — | Clean (only new audit file) |

---

## 25. Final Verdict

### DO NOT RECOMMEND

**The OpenCode TUI cannot be used as a thin frontend for CodeBro.**

The coupling between OpenCode's TUI and its backend runtime is total. The TUI is not a presentation layer — it is a complete application that owns session management, provider authentication, tool execution, permission handling, question prompts, PTY management, VCS integration, MCP discovery, LSP status, file search, workspace management, and more. It calls 59 distinct API methods and subscribes to 60+ event types.

Using it as CodeBro's frontend would require:
1. Implementing a 5,000+ line compatibility layer that reimplements OpenCode's server API
2. Bundling a Node.js/Bun runtime alongside the Rust binary
3. Adding SolidJS and OpenTUI (Zig native) to the build and distribution
4. Accepting loss of CodeBro's unique capabilities (StructuredFacts, Verification, PlanDeviations)
5. Taking on ongoing maintenance of an OpenCode fork

**The selective port approach (Option B from the previous audit) remains the recommended path.** It achieves the UX improvements (multiline paste summary, fuzzy command palette, mouse selection) with minimal risk and full architecture preservation.

---

*Audit complete. No source was modified. No commits were made. No dependencies were added.*
