import { run, type TuiInput } from "./app"
import { StdioAdapter } from "./adapter/index"
import { TuiConfig } from "./config"
import { Effect, Layer } from "effect"
import { Global } from "./shims/core-global"

async function main() {
  const adapter = new StdioAdapter()
  adapter.start()

  const config = TuiConfig.resolve({}, { terminalSuspend: false })

  const input: TuiInput = {
    url: "stdio://codebro",
    args: {},
    config,
    directory: process.cwd(),
    fetch: globalThis.fetch,
    pluginHost: { async start() {}, async dispose() {} },
  }

  try {
    const program = run(input).pipe(
      Effect.provide(Global.node.implementation as Layer.Any),
    )
    const result = await Effect.runPromiseExit(program)
    console.error("[codebro-tui] exit:", result._tag)
    if (result._tag === "Failure" && (result as any).cause?.failures?.[0]?.error) {
      const err = (result as any).cause.failures[0].error
      console.error("[codebro-tui] error:", err?.message)
      if (err?.stack) console.error(err.stack)
    }
  } catch (error: any) {
    console.error("[codebro-tui] uncaught:", error?.message)
    if (error?.stack) console.error(error.stack)
  }
  adapter.stop()
}

main().catch((err: any) => {
  console.error("[codebro-tui] fatal:", err?.message)
  process.exit(1)
})
