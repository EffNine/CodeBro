// CodeBro Adapter — stdio JSON bridge
// Replaces OpenCode's HTTP/SSE client with local stdio protocol
// License: MIT (CodeBro adaptation of OpenCode)

export interface CodeBroEvent {
  id: string
  type: string
  properties: Record<string, unknown>
  [key: string]: unknown
}

export interface CodeBroAdapter {
  request<T>(cmd: string, payload?: unknown): Promise<T>
  onEvent(handler: (event: CodeBroEvent) => void): () => void
  start(): void
  stop(): void
}

export class StdioAdapter implements CodeBroAdapter {
  private reqId = 0
  private pending = new Map<number, { r: (v: any) => void; e: (e: any) => void }>()
  private handlers = new Set<(e: CodeBroEvent) => void>()
  private eof = false

  request<T>(cmd: string, payload?: unknown): Promise<T> {
    const id = ++this.reqId
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { r: resolve, e: reject })
      process.stdout.write(JSON.stringify({ id, cmd, payload: payload ?? null }) + "\n")
    })
  }

  onEvent(h: (e: CodeBroEvent) => void): () => void {
    this.handlers.add(h)
    return () => { this.handlers.delete(h) }
  }

  feed(line: string) {
    if (!line.trim()) return
    let m: any
    try { m = JSON.parse(line) } catch { return }
    if (m.event) {
      for (const h of this.handlers) { try { h(m.event) } catch {} }
    } else if (m.error) {
      const p = this.pending.get(m.id)
      if (p) { this.pending.delete(m.id); p.e(new Error(String(m.error))) }
    } else if (m.id != null && "result" in m) {
      const p = this.pending.get(m.id)
      if (p) { this.pending.delete(m.id); p.r(m.result) }
    }
  }

  start() {
    process.stdin.setEncoding("utf-8")
    let buf = ""
    process.stdin.on("data", (c: string) => {
      buf += c
      let n: number
      while ((n = buf.indexOf("\n")) !== -1) { this.feed(buf.slice(0, n)); buf = buf.slice(n + 1) }
    })
    process.stdin.on("end", () => { this.eof = true })
  }

  stop() { process.stdin.removeAllListeners() }
}
