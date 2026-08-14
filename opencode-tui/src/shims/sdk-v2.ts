// Shim for @opencode-ai/sdk/v2
// Comprehensive type shim for CodeBro TUI
// License: MIT (CodeBro adaptation)

// All types defined inline to avoid circular resolution issues

export type Session = {
  id: string; slug: string; projectID: string; directory: string; title: string
  agent?: string; model?: { id: string; providerID: string; variant?: string }
  time: { created: number; updated: number }
  parentID?: string; status?: string
  workspaceID?: string; share?: string
}

export type Message = UserMessage | AssistantMessage
export type UserMessage = { id: string; sessionID: string; role: "user"; content: string; parts: Part[]; time: { created: number } }
export type AssistantMessage = { id: string; sessionID: string; role: "assistant"; content: string; parts: Part[]; time: { created: number; completed?: number }; agent?: string; model?: { id: string; providerID: string }; providerID?: string; modelID?: string; error?: string; finish?: string }
export type Part = TextPart | ToolPart | FilePart | ReasoningPart | StepStartPart | StepFinishPart | SnapshotPart | PatchPart | AgentPart | RetryPart | CompactionPart | SubtaskPart
export type TextPart = { type: "text"; id: string; messageID: string; sessionID: string; value: string; text?: string; synthetic?: boolean; ignored?: boolean }
export type ToolPart = { type: "tool"; id: string; messageID: string; sessionID: string; tool: string; input: unknown; state: { type: "pending"|"running"|"completed"|"error"; output?: string; error?: string; status?: string; metadata?: Record<string,unknown>; title?: string; input?: unknown }; callID?: string }
export type FilePart = { type: "file"; id: string; messageID: string; sessionID: string; path: string; diff?: string; mime?: string; filename?: string }
export type ReasoningPart = { type: "reasoning"; id: string; messageID: string; sessionID: string; value: string; text?: string; time?: { created: number; completed?: number } }
export type StepStartPart = { type: "step.start"; id: string; messageID: string; sessionID: string }
export type StepFinishPart = { type: "step.finish"; id: string; messageID: string; sessionID: string }
export type SnapshotPart = { type: "snapshot"; id: string; messageID: string; sessionID: string }
export type PatchPart = { type: "patch"; id: string; messageID: string; sessionID: string }
export type AgentPart = { type: "agent"; id: string; messageID: string; sessionID: string; agent: string }
export type RetryPart = { type: "retry"; id: string; messageID: string; sessionID: string }
export type CompactionPart = { type: "compaction"; id: string; messageID: string; sessionID: string }
export type SubtaskPart = { type: "subtask"; id: string; messageID: string; sessionID: string }
export type Provider = { id: string; name: string; models: Model[] }
export type Model = { id: string; name: string; providerID: string; cost?: { input: number; output: number }; limit?: number }
export type PermissionRequest = { id: string; sessionID: string; tool: string; reason: string; metadata?: Record<string,unknown>; patterns?: string[]; always?: boolean; permission?: string }
export type QuestionRequest = { id: string; sessionID: string; question: string; options: string[]; questions?: string[] }
export type SessionStatus = "loading"|"idle"|"busy"|"retry"|"error" | { type: string; message?: string; attempt?: number }
export type Todo = { id: string; content: string; status: "pending"|"completed" }
export type LspStatus = { id: string; root: string; status: string }
export type McpStatus = { status: "connected"|"disconnected"|"error"; error?: string }
export type SnapshotFileDiff = { path: string; diff: string; session?: string; file?: string; status?: string }
export type Config = { theme?: string; keybinds?: Record<string, unknown>; plugin?: string[]; leader_timeout?: number; mouse?: boolean; experimental?: { disable_paste_summary?: boolean }; diff_style?: string; scroll_speed?: number }
export type GlobalEvent = { id: string; type: string; properties: Record<string, unknown>; directory?: string; project?: string; workspace?: string; payload?: { type: string; properties?: Record<string, unknown> }; [key: string]: unknown }
export type EventSource = { subscribe: (h: (e: GlobalEvent) => void) => Promise<() => void> }

export type Agent = { id: string; name: string; description?: string; model?: string; native?: boolean }
export type Command = { id: string; name: string; description?: string }
export type McpResource = { name: string; uri: string; description?: string }
export type FormatterStatus = { name: string; enabled: boolean }
export type ProviderListResponse = { all: Provider[]; default: Record<string, string>; connected: Provider[] }
export type ProviderAuthMethod = { type: string; metadata?: Record<string, string> }
export type ProviderAuthAuthorization = { url?: string; code?: string; deviceCode?: string; verificationUri?: string }
export type VcsInfo = { branch?: string; default_branch?: string; status?: VcsFileStatus[] }
export type VcsFileStatus = { path: string; staged?: boolean; working_tree_status?: string }
export type VcsFileDiff = { path: string; diff: string }
export type ConsoleState = { consoleManagedProviders: string[]; switchableOrgCount: number; activeOrgName?: string }
export type Path = { home: string; state: string; config: string; worktree: string; directory: string }
export type Workspace = { id: string; name: string; directory: string; worktree?: string; strategy?: string; type?: string }
export type LocationRef = { directory: string; workspaceID?: string }
export type FileSystemEntry = { path: string; type: "file"|"directory"; name: string }
export type ProjectDirectories = { data: { directory: string; name: string; worktree?: string; strategy?: string }[] }
export type ExperimentalConsoleListOrgsResponse = { data: { orgID: string; name: string }[]; orgs?: { orgID: string; name: string }[] }
export type ExperimentalWorkspaceAdapterListResponse = { data: { id: string; name: string; type: string }[] }

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
export type ExperimentalWorkspaceCreateOptions = { name?: string; directory?: string }
