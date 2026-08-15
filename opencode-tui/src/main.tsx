// CodeBro TUI entry point — launches the OpenCode-derived frontend
// License: MIT (CodeBro adaptation)

import { StdioAdapter } from "./adapter/index"
import { TuiConfig } from "./config"
import { createCliRenderer } from "@opentui/core"
import { render } from "@opentui/solid"
import { registerOpencodeSpinner } from "./component/register-spinner"
import { createDefaultOpenTuiKeymap } from "@opentui/keymap/opentui"
import { registerOpencodeKeymap } from "./keymap"
import { run, type TuiInput } from "./app"
import { Effect, Layer } from "effect"
import { Global } from "./shims/core-global"

async function main() {
  const adapter = new StdioAdapter()
  adapter.start()
  registerOpencodeSpinner()

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
    
    if (result._tag === "Failure") {
      const err = (result as any).cause?.failures?.[0]?.error
      console.error("[codebro-tui] App error:", err?.message)
    }
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error)
    console.error("[codebro-tui] fatal:", msg)
  }
  
  adapter.stop()
}

main().catch((err) => {
  const msg = err instanceof Error ? err.message : String(err)
  console.error("[codebro-tui] uncaught:", msg)
  process.exit(1)
})
