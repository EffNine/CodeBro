// Shim for @opencode-ai/core/global
// License: MIT (CodeBro adaptation)
// Provides a proper Effect Context.Service and LayerNode-compatible node

import { Context, Layer, Effect } from "effect"

export class Service extends Context.Service<Service, GlobalInterface>()(
  "@opencode/Global",
) {}

export interface GlobalInterface {
  readonly home: string
  readonly state: string
  readonly data: string
}

type Node = {
  readonly kind: "layer"
  readonly name: string
  readonly service?: Context.Service.Any
  readonly implementation?: Layer.Any
  readonly dependencies: readonly unknown[]
}

function makeGlobalNode(input: {
  service: typeof Service
  layer: Layer.Any
  deps: readonly unknown[]
}): Node {
  return {
    kind: "layer",
    name: "@codebro/Global",
    service: input.service,
    implementation: input.layer,
    dependencies: input.deps,
  }
}

const paths = {
  get home() {
    return process.env.HOME ?? ""
  },
  get state() {
    return process.env.XDG_STATE_HOME ?? `${process.env.HOME ?? ""}/.state`
  },
  get data() {
    return process.env.XDG_DATA_HOME ?? `${process.env.HOME ?? ""}/.local/share`
  },
}

export const Path = paths

const layer = Layer.sync(Service, () => ({
  home: Path.home,
  state: Path.state,
  data: Path.data,
}))

export const node = makeGlobalNode({ service: Service, layer, deps: [] })

// Backwards-compatible exports for existing consumers
export const Global = {
  Service,
  Path,
  node,
}
