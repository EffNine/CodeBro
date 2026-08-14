// Shim for @opencode-ai/core/util/flock
// License: MIT (CodeBro adaptation)

export const Flock = {
  withLock<T>(_path: string, fn: () => Promise<T>): Promise<T> {
    return fn()
  },
  withLockSync<T>(_path: string, fn: () => T): T {
    return fn()
  },
}
