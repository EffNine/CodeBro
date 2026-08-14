// CodeBro-modified SDK context — uses stdio adapter
// Adapted from opencode/tui/src/context/sdk.tsx (MIT)
import { createOpencodeClient, type GlobalEvent } from "../adapter/client.js"
import { createSimpleContext } from "./helper"
import { batch, onCleanup, onMount } from "solid-js"

export type EventSource = {
  subscribe: (handler: (event: GlobalEvent) => void) => Promise<() => void>
}

export const { use: useSDK, provider: SDKProvider } = createSimpleContext({
  name: "SDK",
  init: (props: { url?: string; directory?: string; fetch?: typeof fetch; headers?: RequestInit["headers"]; events?: EventSource }) => {
    const abort = new AbortController()
    const sdk = createOpencodeClient({ directory: props.directory })

    const handlers = new Set<(event: GlobalEvent) => void>()
    const emitter = {
      emit(_type: "event", event: GlobalEvent) {
        for (const handler of handlers) handler(event)
      },
      on(_type: "event", handler: (event: GlobalEvent) => void) {
        handlers.add(handler)
        return () => { handlers.delete(handler) }
      },
    }

    let queue: GlobalEvent[] = []
    let timer: Timer | undefined
    let last = 0

    const flush = () => {
      if (queue.length === 0) return
      const events = queue
      queue = []
      timer = undefined
      last = Date.now()
      batch(() => { for (const event of events) emitter.emit("event", event) })
    }

    const handleEvent = (event: GlobalEvent) => {
      queue.push(event)
      const elapsed = Date.now() - last
      if (timer) return
      if (elapsed < 16) { timer = setTimeout(flush, 16); return }
      flush()
    }

    onMount(() => {
      const unsub = sdk.event.on("event", (event) => handleEvent(event))
      onCleanup(unsub)
    })

    onCleanup(() => { abort.abort(); handlers.clear() })

    return {
      get client() { return sdk },
      directory: props.directory,
      event: emitter,
      fetch: props.fetch ?? fetch,
      url: props.url ?? "stdio://codebro",
    }
  },
})
