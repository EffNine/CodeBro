// Shim for @opencode-ai/core/global
// License: MIT (CodeBro adaptation)

export const Global = {
  Service: {
    get home() { return process.env.HOME ?? "" },
    get state() { return process.env.XDG_STATE_HOME ?? `${process.env.HOME}/.state` },
    get data() { return process.env.XDG_DATA_HOME ?? `${process.env.HOME}/.local/share` },
  },
  Path: {
    home: () => process.env.HOME ?? "",
    state: () => process.env.XDG_STATE_HOME ?? `${process.env.HOME}/.state`,
    data: () => process.env.XDG_DATA_HOME ?? `${process.env.HOME}/.local/share`,
  },
}
