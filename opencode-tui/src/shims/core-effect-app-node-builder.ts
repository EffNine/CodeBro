// Shim for @opencode-ai/core/effect/app-node-builder
// License: MIT (CodeBro adaptation)

import type { Node } from "./app-node"
import { Context, Layer, Effect } from "effect"

export function build<A, E>(
  root: Node<A, E, any>,
  _replacements?: readonly [Node<unknown, unknown, any>, Node<unknown, unknown, any>][],
): Layer.Layer<A, E, never> {
  // Flatten the node tree into a single layer.
  // For a simple global node, we just return its implementation directly.
  if (root.implementation) {
    return root.implementation as Layer.Layer<A, E, never>
  }
  // Fallback: create a layer from the service if no implementation
  if (root.service) {
    return Layer.sync(root.service as Context.Service.Any, () =>
      (root.service as any).of({}),
    ) as Layer.Layer<A, E, never>
  }
  // Last resort: return a passthrough layer
  return Layer.makeFinal(
    {} as A,
    () => Effect.succeed({} as A),
  ) as Layer.Layer<A, E, never>
}
