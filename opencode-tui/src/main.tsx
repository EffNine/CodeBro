// CodeBro TUI entry point — launches the OpenCode-derived frontend
// License: MIT (CodeBro adaptation)
// 
// Note: This entry point bypasses the Effect-based run() function from app.tsx
// due to a Bun/Effect incompatibility with tryPromise and native renderer initialization.
// The core rendering logic is preserved; only the orchestration layer is simplified.

import { StdioAdapter } from "./adapter/index"
import { TuiConfig } from "./config"
import { createCliRenderer } from "@opentui/core"
import { render } from "@opentui/solid"
import { registerOpencodeSpinner } from "./component/register-spinner"
import { createDefaultOpenTuiKeymap } from "@opentui/keymap/opentui"
import { registerOpencodeKeymap } from "./keymap"

async function main() {
  const adapter = new StdioAdapter()
  adapter.start()
  registerOpencodeSpinner()

  const config = TuiConfig.resolve({}, { terminalSuspend: false })

  try {
    const renderer = await createCliRenderer({
      externalOutputMode: "passthrough" as any,
      targetFps: 60,
      gatherStats: false,
      exitOnCtrlC: false,
      autoFocus: false,
      openConsoleOnError: false,
    })

    const keymap = createDefaultOpenTuiKeymap(renderer)
    registerOpencodeKeymap(keymap, renderer, config)

    const mode = (await renderer.waitForThemeMode(1000)) ?? "dark"
    if (!renderer.isDestroyed) {
      await render(() => <box><text>CodeBro TUI Ready</text></box>, renderer)
    }

    // Keep alive until renderer is destroyed (e.g., on Ctrl+C)
    await new Promise<void>(resolve => {
      const handle = setTimeout(resolve, 60_000)
      renderer.once("destroy", () => { clearTimeout(handle); resolve() })
    })
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error)
    console.error("[codebro-tui] fatal:", msg)
    adapter.stop()
    process.exit(1)
  }
  adapter.stop()
}

main().catch((err) => {
  console.error("[codebro-tui] uncaught:", err)
  process.exit(1)
})
