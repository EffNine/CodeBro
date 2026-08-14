// Shim for @opencode-ai/core/util/which
// License: MIT (CodeBro adaptation)

export async function which(name: string): Promise<string | null> {
  const { execSync } = await import("child_process")
  try {
    const result = execSync(`which ${name}`, { encoding: "utf-8", stdio: "pipe" })
    return result.trim() || null
  } catch {
    return null
  }
}
