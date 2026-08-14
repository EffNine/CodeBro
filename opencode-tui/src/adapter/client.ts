// CodeBro SDK Client — replaces @opencode-ai/sdk/v2 createOpencodeClient
// Implements the OpencodeClient interface using stdio adapter
// License: MIT (CodeBro adaptation)

import { StdioAdapter } from "./index.js"

// ===== Type Definitions =====

export type Session = {
  id: string
  slug: string
  projectID: string
  directory: string
  title: string
  agent?: string
  model?: { id: string; providerID: string; variant?: string }
  time: { created: number; updated: number }
  parentID?: string
  status?: string
}

export type Message = UserMessage | AssistantMessage
export type UserMessage = { id: string; sessionID: string; role: "user"; content: string; parts: Part[]; time: { created: number } }
export type AssistantMessage = { id: string; sessionID: string; role: "assistant"; content: string; parts: Part[]; time: { created: number }; agent?: string; model?: { id: string; providerID: string } }
export type Part = TextPart | ToolPart | FilePart | ReasoningPart | StepStartPart | StepFinishPart | SnapshotPart | PatchPart | AgentPart | RetryPart | CompactionPart | SubtaskPart
export type TextPart = { type: "text"; id: string; messageID: string; sessionID: string; value: string }
export type ToolPart = { type: "tool"; id: string; messageID: string; sessionID: string; tool: string; input: unknown; state: { type: "pending"|"running"|"completed"|"error"; output?: string; error?: string } }
export type FilePart = { type: "file"; id: string; messageID: string; sessionID: string; path: string; diff?: string }
export type ReasoningPart = { type: "reasoning"; id: string; messageID: string; sessionID: string; value: string }
export type StepStartPart = { type: "step.start"; id: string; messageID: string; sessionID: string }
export type StepFinishPart = { type: "step.finish"; id: string; messageID: string; sessionID: string }
export type SnapshotPart = { type: "snapshot"; id: string; messageID: string; sessionID: string }
export type PatchPart = { type: "patch"; id: string; messageID: string; sessionID: string }
export type AgentPart = { type: "agent"; id: string; messageID: string; sessionID: string; agent: string }
export type RetryPart = { type: "retry"; id: string; messageID: string; sessionID: string }
export type CompactionPart = { type: "compaction"; id: string; messageID: string; sessionID: string }
export type SubtaskPart = { type: "subtask"; id: string; messageID: string; sessionID: string }
export type Provider = { id: string; name: string; models: Model[] }
export type Model = { id: string; name: string; providerID: string; cost?: { input: number; output: number } }
export type PermissionRequest = { id: string; sessionID: string; tool: string; reason: string }
export type QuestionRequest = { id: string; sessionID: string; question: string; options: string[] }
export type SessionStatus = "loading"|"idle"|"busy"|"retry"|"error"
export type Todo = { id: string; content: string; status: "pending"|"completed" }
export type LspStatus = { id: string; root: string; status: string }
export type McpStatus = { status: "connected"|"disconnected"|"error"; error?: string }
export type SnapshotFileDiff = { path: string; diff: string }
export type Config = { theme?: string; keybinds?: Record<string, unknown>; plugin?: string[]; leader_timeout?: number; mouse?: boolean; experimental?: { disable_paste_summary?: boolean } }
export type GlobalEvent = { id: string; type: string; properties: Record<string, unknown>; directory?: string; project?: string; workspace?: string; payload?: { type: string; properties?: Record<string, unknown> }; [key: string]: unknown }
export type EventSource = { subscribe: (h: (e: GlobalEvent) => void) => Promise<() => void> }

// Extended types
export type Agent = { id: string; name: string; description?: string; model?: string }
export type Command = { id: string; name: string; description?: string }
export type McpResource = { name: string; uri: string; description?: string }
export type FormatterStatus = { name: string; enabled: boolean }
export type ProviderListResponse = { all: Provider[]; default: Record<string, string>; connected: Provider[] }
export type ProviderAuthMethod = { type: string; metadata?: Record<string, string> }
export type VcsInfo = { branch?: string; default_branch?: string; status?: VcsFileStatus[] }
export type VcsFileStatus = { path: string; staged?: boolean; working_tree_status?: string }
export type VcsFileDiff = { path: string; diff: string }
export type ConsoleState = { consoleManagedProviders: string[]; switchableOrgCount: number }
export type Path = { home: string; state: string; config: string; worktree: string; directory: string }
export type Workspace = { id: string; name: string; directory: string; worktree?: string; strategy?: string }
export type LocationRef = { directory: string; workspaceID?: string }
export type FileSystemEntry = { path: string; type: "file"|"directory"; name: string }

// V2 types
export type AgentV2Info = { id: string; name: string; description?: string }
export type CommandV2Info = { id: string; name: string; description?: string }
export type IntegrationInfo = { id: string; name: string; type: string }
export type ModelV2Info = { id: string; name?: string; providerID: string }
export type PermissionSavedInfo = { id: string; tool: string; policy: string }
export type PermissionV2Request = PermissionRequest & { strategy?: string }
export type ProviderV2Info = { id: string; name: string; models: ModelV2Info[] }
export type QuestionV2Request = QuestionRequest & { options?: string[] }
export type ReferenceInfo = { id: string; path: string }
export type SkillV2Info = { id: string; name: string; status: string }
export type SessionV2Info = Session & { agent?: string; model?: string }
export type SessionMessage = UserMessage | AssistantMessage
export type SessionMessageAssistant = AssistantMessage
export type SessionMessageAssistantText = { type: "text"; value: string; id: string }
export type SessionMessageAssistantReasoning = { type: "reasoning"; value: string; id: string }
export type SessionMessageAssistantTool = { type: "tool"; tool: string; input: unknown; id: string; state: { type: string; output?: string; error?: string } }
export type QuestionAnswer = { answer: string }
export type V2Event = GlobalEvent & { payload?: { type: string; properties?: Record<string, unknown> } }
export type ExperimentalWorkspaceAdapterListResponse = { data: { id: string; name: string; type: string }[] }

// ===== OpencodeClient Interface =====

export interface OpencodeClient {
  global: {
    event(o?: { signal?: AbortSignal }): Promise<{ stream: AsyncGenerator<GlobalEvent> }>
    health(): Promise<{ data: any }>
    dispose(): Promise<void>
    upgrade(t?: string): Promise<void>
    config: { get(): Promise<{ data: Config }>; update(c: Config): Promise<{ data: Config }> }
  }
  session: {
    list(o?: { start?: number; scope?: string; path?: string }): Promise<{ data: Session[] }>
    get(i: string, o?: { throwOnError?: boolean }): Promise<{ data: Session }>
    create(p?: any): Promise<{ data: Session }>
    delete(i: string): Promise<void>
    abort(i: string): Promise<void>
    status(i: string): Promise<{ data: SessionStatus }>
    messages(i: string, l?: number, o?: { throwOnError?: boolean }): Promise<{ data: Message[] }>
    diff(i: string): Promise<{ data: SnapshotFileDiff[] }>
    todo(i: string): Promise<{ data: Todo[] }>
    fork(i: string): Promise<{ data: Session }>
    revert(i: string, m: string): Promise<void>
    unrevert(i: string): Promise<void>
    summarize(i: string): Promise<void>
    shell(i: string, c: string): Promise<{ data: any }>
    command(i: string, t: string): Promise<void>
    children(i: string): Promise<{ data: Session[] }>
    permission(i: string): Promise<{ data: PermissionRequest[] }>
    question(i: string): Promise<{ data: QuestionRequest[] }>
    update(o: { sessionID: string; title?: string; agent?: string }): Promise<{ data: Session }>
  }
  provider: {
    list(): Promise<{ data: Provider[] }>
    auth(): Promise<{ data: Record<string, any> }>
    oauth: { authorize(p: string): Promise<void>; callback(p: string, c: string): Promise<void> }
  }
  permission: { reply(r: string, a: string): Promise<void> }
  question: { reply(r: string, a: string[]): Promise<void>; reject(r: string): Promise<void> }
  mcp: {
    status(): Promise<{ data: Record<string, McpStatus> }>
    connect(n: string): Promise<void>
    disconnect(n: string): Promise<void>
  }
  project: {
    current(o?: { workspace?: string }): Promise<{ data: { directory: string; name: string; id?: string; worktree?: string } }>
    directories(o?: { projectID?: string; workspace?: string }): Promise<{ data: { directory: string; name: string; worktree?: string; strategy?: string }[] }>
  }
  path: { get(p: string, o?: { workspace?: string }): Promise<{ data: Path }> }
  vcs: {
    info(o?: { workspace?: string }): Promise<{ data: { branch?: string; default_branch?: string } }>
    status(o?: { workspace?: string }): Promise<{ data: VcsFileStatus[] }>
    get(o?: { workspace?: string }): Promise<{ data: VcsInfo }>
  }
  lsp: { status(): Promise<{ data: LspStatus[] }> }
  pty: {
    create(c: string, w?: string): Promise<{ data: { id: string; pid: number } }>
    list(): Promise<{ data: any[] }>
    get(i: string): Promise<{ data: any }>
    remove(i: string): Promise<void>
  }
  command: { list(): Promise<{ data: any[] }> }
  file: {
    list(w?: string): Promise<{ data: any[] }>
    read(p: string): Promise<{ data: string }>
    status(p: string): Promise<{ data: any }>
  }
  find: {
    files(p: string, w?: string): Promise<{ data: string[] }>
    symbols(p: string, w?: string): Promise<{ data: any[] }>
    text(p: string, w?: string): Promise<{ data: any[] }>
  }
  auth: { set(p: string, a?: any): Promise<void>; remove(p: string): Promise<void> }
  config: { get(): Promise<{ data: Config }>; providers(): Promise<{ data: any[] }> }
  app: { agents(): Promise<{ data: any[] }> }
  instance: { dispose(): Promise<void> }
  formatter: { status(): Promise<{ data: any }> }
  tui: {
    promptAppend(t: string): Promise<void>
    commandExecute(c: string): Promise<void>
    sessionSelect(s: string): Promise<void>
    toastShow(t: any): Promise<void>
  }
  event: { on(t: string, h: (e: GlobalEvent) => void): () => void }

  // Extended namespaces for full TUI compatibility
  v2: {
    session: {
      get(o: { sessionID: string; throwOnError?: boolean }): Promise<{ data: SessionV2Info }>
      messages(o: { sessionID: string; throwOnError?: boolean }): Promise<{ data: SessionMessage[] }>
      permission: { list(o: { sessionID: string; throwOnError?: boolean }): Promise<{ data: PermissionV2Request[] }> }
      question: { list(o: { sessionID: string; throwOnError?: boolean }): Promise<{ data: QuestionV2Request[] }> }
    }
    permission: { saved: { list(o: { projectID?: string; throwOnError?: boolean }): Promise<{ data: PermissionSavedInfo[] }> } }
    location: { get(o: { location: LocationRef; throwOnError?: boolean }): Promise<{ data: LocationRef }> }
    agent: { list(o: { location?: LocationRef; throwOnError?: boolean }): Promise<{ data: AgentV2Info[] }> }
    command: { list(o: { location?: LocationRef; throwOnError?: boolean }): Promise<{ data: CommandV2Info[] }> }
    integration: { list(o: { location?: LocationRef; throwOnError?: boolean }): Promise<{ data: IntegrationInfo[] }> }
    model: { list(o: { location?: LocationRef; throwOnError?: boolean }): Promise<{ data: ModelV2Info[] }> }
    provider: { list(o: { location?: LocationRef; throwOnError?: boolean }): Promise<{ data: ProviderV2Info[] }> }
    reference: { list(o: { location?: LocationRef; throwOnError?: boolean }): Promise<{ data: ReferenceInfo[] }> }
    skill: { list(o: { location?: LocationRef; throwOnError?: boolean }): Promise<{ data: SkillV2Info[] }> }
    fs: { find(o: { pattern: string; cwd?: string; throwOnError?: boolean }): Promise<{ data: FileSystemEntry[] }> }
    projectCopy: {
      create(o: { name?: string; throwOnError?: boolean }): Promise<{ data: { id: string; name: string } }>
      refresh(o: { projectID: string; throwOnError?: boolean }): Promise<{ data: unknown }>
    }
  }
  experimental: {
    capabilities: { experimentalBackgroundSubagents: boolean }
    console: { switchOrg(orgID: string): Promise<{ data: unknown }> }
    resource: { get(id: string): Promise<{ data: unknown }> }
    session: { background(options: { sessionID: string; task: string }): Promise<{ data: { id: string } }> }
    workspace: {
      create(options: { name?: string; directory?: string }): Promise<{ data: Workspace }>
      remove(workspaceID: string): Promise<{ data: unknown }>
      list(): Promise<{ data: Workspace[] }>
      status(): Promise<{ data: { workspaceID: string; status: string }[] }>
      syncList(): Promise<{ data: Workspace[] }>
      warp(workspaceID: string): Promise<{ data: unknown }>
      adapter: { list(): Promise<ExperimentalWorkspaceAdapterListResponse> }
    }
    projectCopy: { generateName(): Promise<{ data: string }> }
    controlPlane: { moveSession(options: { sessionID: string; targetWorkspaceID: string }): Promise<{ data: unknown }> }
  }
  prompt: {
    submit(sessionID: string, text: string): Promise<{ data: Session }>
  }
  share: { create(sessionID: string): Promise<{ data: { url: string } }> }
  unshare: { delete(sessionID: string): Promise<{ data: unknown }> }
}

// ===== Client Implementation =====

export function createOpencodeClient(cfg?: { baseUrl?: string; directory?: string; fetch?: typeof fetch; headers?: Record<string, string> }): OpencodeClient {
  const ad = new StdioAdapter()
  ad.start()
  const req = <T>(cmd: string, payload?: unknown) => ad.request<T>(cmd, payload)
  const cwd = cfg?.directory ?? process.cwd()

  return {
    get global() {
      return {
        async event(_o?: { signal?: AbortSignal }) { return { stream: (async function* () { yield* [] })() } },
        async health() { return { data: { status: "ok" } } },
        async dispose() { ad.stop() },
        async upgrade(_t?: string) {},
        config: { async get() { return { data: {} as Config } }, async update(c: Config) { return { data: c } } },
      }
    },
    get session() {
      return {
        async list(_o?: any) { return req<any[]>("session.list").then(d => ({ data: d ?? [] })) },
        async get(i: string, _o?: any) { return req<any>("session.get", { id: i }).then(d => ({ data: d })) },
        async create(p?: any) { return req<any>("session.create", p).then(d => ({ data: d })) },
        async delete(i: string) { return req("session.delete", { id: i }) },
        async abort(i: string) { return req("session.abort", { id: i }) },
        async status(i: string) { return req<any>("session.status", { id: i }).then(d => ({ data: d })) },
        async messages(i: string, _l?: number, _o?: any) { return req<any[]>("session.messages", { id: i }).then(d => ({ data: d ?? [] })) },
        async diff(i: string) { return req<any[]>("session.diff", { id: i }).then(d => ({ data: d ?? [] })) },
        async todo(i: string) { return req<any[]>("session.todo", { id: i }).then(d => ({ data: d ?? [] })) },
        async fork(i: string) { return req<any>("session.fork", { id: i }).then(d => ({ data: d })) },
        async revert(i: string, m: string) { return req("session.revert", { id: i, messageId: m }) },
        async unrevert(i: string) { return req("session.unrevert", { id: i }) },
        async summarize(i: string) { return req("session.summarize", { id: i }) },
        async shell(i: string, c: string) { return req("session.shell", { id: i, command: c }) },
        async command(i: string, t: string) { return req("session.command", { id: i, text: t }) },
        async prompt(_p: any) { return req<any>("session.command", { sessionID: _p.sessionID, text: _p.parts?.find((x: any) => x.type === "text")?.text ?? "" }).then(d => ({ data: d })) },
        async children(i: string) { return req<any[]>("session.children", { id: i }).then(d => ({ data: d ?? [] })) },
        async permission(i: string) { return req<any[]>("session.permission", { id: i }).then(d => ({ data: d ?? [] })) },
        async question(i: string) { return req<any[]>("session.question", { id: i }).then(d => ({ data: d ?? [] })) },
        async update(o: { sessionID: string; title?: string; agent?: string }) {
          return req<any>("session.update", { sessionID: o.sessionID, title: o.title, agent: o.agent }).then(d => ({ data: d }))
        },
      }
    },
    get provider() {
      return {
        async list() { return req<any[]>("provider.list").then(d => ({ data: d ?? [] })) },
        async auth() { return req<any>("provider.auth").then(d => ({ data: d ?? {} })) },
        oauth: { async authorize(_p: string) {}, async callback(_p: string, _c: string) {} },
      }
    },
    get permission() {
      return { async reply(r: string, a: string) { return req("permission.reply", { requestId: r, action: a }) } }
    },
    get question() {
      return {
        async reply(r: string, a: string[]) { return req("question.reply", { requestId: r, answers: a }) },
        async reject(r: string) { return req("question.reject", { requestId: r }) },
      }
    },
    get mcp() {
      return {
        async status() { return req<any>("mcp.status").then(d => ({ data: d ?? {} })) },
        async connect(n: string) { return req("mcp.connect", { name: n }) },
        async disconnect(n: string) { return req("mcp.disconnect", { name: n }) },
      }
    },
    get project() {
      return {
        async current(_o?: any) { return req<any>("project.current").then(d => ({ data: d ?? { directory: cwd, name: "CodeBro" } })) },
        async directories(_o?: any) { return req<any[]>("project.directories").then(d => ({ data: d ?? [] })) },
      }
    },
    get path() {
      return { async get(p: string, _o?: any) { return req<any>("path.get", { path: p }).then(d => ({ data: d ?? { home: "", state: "", config: "", worktree: "", directory: p } })) } }
    },
    get vcs() {
      return {
        async info(_o?: any) { return req<any>("vcs.info").then(d => ({ data: d ?? {} })) },
        async status(_o?: any) { return req<any[]>("vcs.status").then(d => ({ data: d ?? [] })) },
        async get(_o?: any) { return req<any>("vcs.get").then(d => ({ data: d ?? {} })) },
      }
    },
    get lsp() {
      return { async status() { return req<any[]>("lsp.status").then(d => ({ data: d ?? [] })) } }
    },
    get pty() {
      return {
        async create(c: string, w?: string) { return req<any>("pty.create", { command: c, cwd: w }).then(d => ({ data: d! })) },
        async list() { return req<any[]>("pty.list").then(d => ({ data: d ?? [] })) },
        async get(i: string) { return req<any>("pty.get", { id: i }) },
        async remove(i: string) { return req("pty.remove", { id: i }) },
      }
    },
    get command() {
      return { async list() { return req<any[]>("command.list").then(d => ({ data: d ?? [] })) } }
    },
    get file() {
      return {
        async list(w?: string) { return req<any[]>("file.list", { cwd: w }).then(d => ({ data: d ?? [] })) },
        async read(p: string) { return req<any>("file.read", { path: p }).then(d => ({ data: d ?? "" })) },
        async status(p: string) { return req<any>("file.status", { path: p }) },
      }
    },
    get find() {
      return {
        async files(p: string, w?: string) { return req<any[]>("find.files", { pattern: p, cwd: w }).then(d => ({ data: d ?? [] })) },
        async symbols(p: string, w?: string) { return req<any[]>("find.symbols", { pattern: p, cwd: w }).then(d => ({ data: d ?? [] })) },
        async text(p: string, w?: string) { return req<any[]>("find.text", { pattern: p, cwd: w }).then(d => ({ data: d ?? [] })) },
      }
    },
    get auth() {
      return {
        async set(p: string, a?: any) { return req("auth.set", { provider: p, auth: a }) },
        async remove(p: string) { return req("auth.remove", { provider: p }) },
      }
    },
    get config() {
      return {
        async get() { return req<any>("config.get").then(d => ({ data: d ?? ({} as Config) })) },
        async providers() { return req<any[]>("config.providers").then(d => ({ data: d ?? [] })) },
      }
    },
    get app() {
      return { async agents() { return req<any[]>("app.agents").then(d => ({ data: d ?? [] })) } }
    },
    get instance() {
      return { async dispose() { return req("instance.dispose") } }
    },
    get formatter() {
      return { async status() { return req<any>("formatter.status").then(d => ({ data: d })) } }
    },
    get tui() {
      return {
        async promptAppend(t: string) { return req("tui.prompt.append", { text: t }) },
        async commandExecute(c: string) { return req("tui.command.execute", { command: c }) },
        async sessionSelect(s: string) { return req("tui.session.select", { sessionId: s }) },
        async toastShow(t: any) { return req("tui.toast.show", { toast: t }) },
      }
    },
    get event() {
      return {
        on(t: string, h: (e: GlobalEvent) => void) {
          return ad.onEvent((e) => { if (t === "event" || e.type === t) h(e as GlobalEvent) })
        },
      }
    },
    // Extended namespaces
    get v2() {
      return {
        session: {
          async get(o: { sessionID: string; throwOnError?: boolean }) { return req<any>("session.get", { id: o.sessionID }).then(d => ({ data: d })) },
          async messages(o: { sessionID: string; throwOnError?: boolean }) { return req<any[]>("session.messages", { id: o.sessionID }).then(d => ({ data: d ?? [] })) },
          permission: {
            async list(o: { sessionID: string; throwOnError?: boolean }) { return req<any[]>("session.permission", { id: o.sessionID }).then(d => ({ data: d ?? [] })) },
          },
          question: {
            async list(o: { sessionID: string; throwOnError?: boolean }) { return req<any[]>("session.question", { id: o.sessionID }).then(d => ({ data: d ?? [] })) },
          },
        },
        permission: {
          saved: {
            async list(_o?: any) { return req<any[]>("permission.saved.list").then(d => ({ data: d ?? [] })) },
          },
        },
        location: {
          async get(o: { location: LocationRef; throwOnError?: boolean }) { return req<any>("path.get", { path: o.location.directory }).then(d => ({ data: d ?? o.location })) },
        },
        agent: {
          async list(_o?: any) { return req<any[]>("app.agents").then(d => ({ data: d ?? [] })) },
        },
        command: {
          async list(_o?: any) { return req<any[]>("command.list").then(d => ({ data: d ?? [] })) },
        },
        integration: {
          async list(_o?: any) { return req<any[]>("find.integration").then(d => ({ data: d ?? [] })) },
        },
        model: {
          async list(_o?: any) { const r = await req<any[]>("provider.list"); const items = r ?? []; return { data: items.flatMap((p: any) => p.models.map((m: any) => ({ id: m.id, name: m.name, providerID: m.providerID }))) } },
        },
        provider: {
          async list(_o?: any) { return req<any[]>("provider.list").then(d => ({ data: (d ?? []).map(p => ({ id: p.id, name: p.name, models: p.models.map(m => ({ id: m.id, name: m.name, providerID: m.providerID })) })) })) },
        },
        reference: {
          async list(_o?: any) { return req<any[]>("find.reference").then(d => ({ data: d ?? [] })) },
        },
        skill: {
          async list(_o?: any) { return req<any[]>("app.skills").then(d => ({ data: (d ?? []).map((s: any) => ({ id: s.id, name: s.name, status: s.status ?? "draft" })) })) },
        },
        fs: {
          async find(o: { pattern: string; cwd?: string; throwOnError?: boolean }) { return req<any[]>("find.files", { pattern: o.pattern, cwd: o.cwd ?? cwd }).then(d => ({ data: (d ?? []).map((p: any) => ({ path: p, type: "file" as const, name: p.split("/").pop() ?? p })) })) },
        },
        projectCopy: {
          async create(_o?: any) { return { data: { id: "copy-" + Date.now(), name: "Copy" } } },
          async refresh(_o?: any) { return { data: null } },
        },
      }
    },
    get experimental() {
      return {
        capabilities: { experimentalBackgroundSubagents: false },
        console: {
          async switchOrg(_orgID: string) { return { data: null } },
        },
        resource: {
          async get(_id: string) { return { data: null } },
        },
        session: {
          async background(_options: { sessionID: string; task: string }) { return { data: { id: "bg-" + Date.now() } } },
        },
        workspace: {
          async create(_options: any) { return { data: { id: "ws-" + Date.now(), name: "Default", directory: cwd } } },
          async remove(_workspaceID: string) { return { data: null } },
          async list() { return { data: [] as any[] } },
          async status() { return { data: [] } },
          async syncList() { return { data: [] as any[] } },
          async warp(_workspaceID: string) { return { data: null } },
          adapter: {
            async list() { return { data: [] } },
          },
        },
        projectCopy: {
          async generateName() { return { data: "copy" } },
        },
        controlPlane: {
          async moveSession(_options: any) { return { data: null } },
        },
      }
    },
    get prompt() {
      return {
        async submit(sessionID: string, text: string) {
          return req<any>("session.command", { sessionID, text }).then(d => ({ data: d }))
        },
      }
    },
    get share() {
      return {
        async create(_sessionID: string) { return { data: { url: "" } } },
      }
    },
    get unshare() {
      return {
        async delete(_sessionID: string) { return { data: null } },
      }
    },
  } as OpencodeClient
}
