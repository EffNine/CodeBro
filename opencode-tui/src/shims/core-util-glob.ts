// Shim for @opencode-ai/core/util/glob
// License: MIT (CodeBro adaptation)

export const Glob = {
  async scan(_pattern: string, _options?: { cwd?: string }): Promise<string[]> {
    return []
  },
}
