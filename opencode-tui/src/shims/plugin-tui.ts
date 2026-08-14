// Shim for @opencode-ai/plugin/tui
// License: MIT (CodeBro adaptation)

export type TuiAttentionSoundName = "bip-bop-01" | "nope-03" | "staplebops-06" | "bip-bop-03" | "yup-01" | "done" | "subagent_done" | "error" | "question" | "permission"

export type TuiAttentionWhen = "always" | "background" | "foreground"

export type TuiAttentionNotifySkipReason = "background" | "foreground" | "disabled"

export type TuiAttentionNotifyResult = { skipped?: TuiAttentionNotifySkipReason }

export type TuiAttentionNotifyInput = {
  message: string
  sound?: TuiAttentionSoundName
  notification?: boolean | { when: string }
  soundPack?: TuiAttentionSoundName
}

export type TuiAttention = {
  notify: (input: TuiAttentionNotifyInput) => Promise<TuiAttentionNotifyResult>
  dispose: () => void
}

export type TuiKV = {
  get<T>(key: string, fallback?: T): T
  set(key: string, value: unknown): void
  delete(key: string): void
  keys(): string[]
}

export type TuiPluginStatus = "installing" | "active" | "inactive" | "error" | { active: boolean; source?: string; spec?: string; enabled?: boolean; id?: string }

export type TuiRouteCurrent =
  | { type: "home"; name?: string }
  | { type: "session"; sessionID: string; name?: string }
  | { type: "plugin"; id: string; data?: Record<string, unknown>; name?: string }

export type TuiRouteDefinition = {
  id: string
  name?: string
  render: (props: { params?: Record<string, unknown> }) => unknown
}

export type TuiSlotProps = {
  children?: unknown
  [key: string]: unknown
}

export type TuiSlotContext = {
  name: string
  mode: "single_winner" | "multiple"
}

export type TuiSlotMap = Map<string, Set<(props: TuiSlotProps) => unknown>>

export type TuiCommand = {
  name: string
  title?: string
  description?: string
  category?: string
  run: () => void | Promise<void>
  hidden?: boolean
  enabled?: boolean | (() => boolean)
  slashName?: string
  slashAliases?: string[]
  suggested?: boolean
  value?: string
  slash?: string
  onSelect?: () => void
  keybind?: string
}

export type TuiDialogSelectOption<T = unknown> = {
  key: T
  value: T
  title: string
  description?: string
  category?: string
  disabled?: boolean
  footer?: string
  onSelect: () => void
}

export type TuiPlugin = {
  name: string
  install: (api: TuiPluginApi) => void | Promise<void>
  uninstall?: (api: TuiPluginApi) => void | Promise<void>
}

export type TuiPluginModule = {
  default: TuiPlugin
  id?: string
}

export type TuiPluginInstallOptions = {
  source: string
  spec?: unknown
}

export type TuiPluginInstallResult = {
  ok: boolean
  message: string
}

export type TuiPluginApi = {
  app: { version: string; channel: string }
  attention: TuiAttention
  command: {
    registerLayer(name: string, commands: TuiCommand[]): () => void
    unregisterLayer(name: string): void
    register?: (name: string, commands: TuiCommand[]) => () => void
  }
  keys: {
    formatKeySequence(keys: string[]): string
    formatKeyBindings(bindings: { keys: string[]; action: string }[]): string
    formatSequence?: (keys: string[]) => string
  }
  keymap: unknown
  mode: {
    push(mode: string): void
    pop(): void
    current: string
  }
  route: {
    navigate(to: unknown, data?: unknown): void
    register(def: TuiRouteDefinition | TuiRouteDefinition[]): void
    get current(): TuiRouteCurrent
  }
  ui: {
    Dialog: unknown
    DialogAlert: unknown
    DialogConfirm: unknown
    DialogPrompt: unknown
    DialogSelect: unknown
    Slot: unknown
    Prompt: unknown
    toast: {
      show(options: { title?: string; message: string; variant?: string; duration?: number }): void
      (options: { title?: string; message: string; variant?: string; duration?: number }): void
      error: (message: string) => void
      success: (message: string) => void
    }
    dialog: {
      replace(render: (dialog: unknown, onClose?: () => void) => void): void
      clear(): void
      setSize(size: string): void
      size: string
      depth: number
      open: boolean
    }
  }
  state: unknown
  client: unknown
  event: unknown
  kv: TuiKV
  slots: {
    register(name: string, mode: "single_winner" | "multiple"): () => void
  }
  plugins: {
    install(name: string, module: TuiPluginModule): boolean | Promise<{ ok: boolean; message: string }>
    add(plugin: TuiPlugin): boolean | Promise<boolean>
    deactivate(name: string): boolean | Promise<boolean>
    list?: () => unknown[]
    activate?: (name: string) => boolean
  }
  theme: {
    current: string
    selected: string
    setCurrent(name: string): void
    setSelected(name: string): void
  }
  tuiConfig: unknown
  renderer: unknown
  lifecycle?: {
    onMount?: (fn: () => void) => void
    onDestroy?: (fn: () => void) => void
  }
}
