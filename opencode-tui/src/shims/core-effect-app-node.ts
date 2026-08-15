// Shim for @opencode-ai/core/effect/app-node
// License: MIT (CodeBro adaptation)

import { Brand, Context, Layer } from "effect"

type AnyNode = Node<unknown, unknown, any>
type NodeList<Item extends AnyNode = AnyNode> =
  | readonly []
  | readonly [Item, ...Item[]]

declare const $OutputType: unique symbol
declare const $ErrorType: unique symbol

export type Tag<Name extends string = string> = Name &
  Brand.Brand<"LayerNode.Tag">

const makeTag = Brand.nominal<Tag>()

export interface Node<
  A = unknown,
  E = never,
  T extends Tag | undefined = undefined,
> {
  readonly kind: "layer" | "unbound" | "group"
  readonly name: string
  readonly service?: Context.Service.Any
  readonly implementation?: Layer.Any
  readonly dependencies: readonly AnyNode[]
  readonly tag?: T
  readonly [$OutputType]?: () => A
  readonly [$ErrorType]?: () => E
}

export type TagConfig = Readonly<Record<string, readonly string[]>>
type TagNames<Config extends TagConfig> = keyof Config & string
type NodeInTags<Names extends string> = Node<
  unknown,
  unknown,
  Tag<Names> | undefined
>
type DistributiveOmit<A, K extends PropertyKey> = A extends unknown
  ? Omit<A, K>
  : never

export interface Tags<Config extends TagConfig> {
  readonly values: { readonly [Name in TagNames<Config>]: Tag<Name> }
  readonly make: <Name extends TagNames<Config>>(
    name: Name,
  ) => <const Implementation extends Layer.Any, const Items extends NodeList>(
    input: DistributiveOmit<
      MakeInput<Implementation, Items, Tag<Name>>,
      "tag"
    > &
      CheckTags<Items, Name | Extract<Config[Name][number], string>>,
    ) => Node<
      Layer.Success<Implementation>,
      Layer.Error<Implementation> | Error<Items[number]>,
      Tag<Name>
    >
  )
}

type MakeInput<
  Implementation extends Layer.Any,
  Items extends NodeList,
  T extends Tag | undefined = undefined,
> = NodeIdentity & {
  readonly layer: Implementation
  readonly deps: Items
  readonly tag?: T
}

type NodeIdentity =
  | { readonly service: Context.Service.Any; readonly name?: never }
  | { readonly name: string; readonly service?: never }

type CheckTags<Items extends NodeList, Names extends string> = [
  Exclude<Items[number], NodeInTags<Names>>,
] extends [never]
  ? unknown
  : { readonly "Invalid tag dependencies": Exclude<Items[number], NodeInTags<Names>> }

export function tags<const Config extends { readonly [Name in keyof Config]: readonly (keyof Config & string)[] }>(
  config: Config,
): Tags<Config> {
  const names = Object.keys(config) as TagNames<Config>[]
  const values = Object.fromEntries(
    names.map((name) => [name, makeTag(name)]),
  ) as Tags<Config>["values"]
  return {
    values,
    make: ((name: TagNames<Config>) => (
      input: DistributiveOmit<MakeInput<Layer.Any, NodeList, Tag>, "tag"> &
        CheckTags<NodeList, Name | Extract<Config[Name][number], string>>
    ) =>
      make({ ...input, tag: values[name] }) as Node<
        Layer.Success<Layer.Any>,
        Layer.Error<Layer.Any> | Error<AnyNode>,
        Tag<Name>
      >
    ) as Tags<Config>["make"],
  }
}

export const makeGlobalNode = ((input: {
  service: Context.Service.Any
  layer: Layer.Any
  deps: readonly unknown[]
}) => ({
  kind: "layer",
  name: "global",
  service: input.service,
  implementation: input.layer,
  dependencies: input.deps as never,
})) as <A, E>(
  input: { service: Context.Service.Any; layer: Layer.Any; deps: readonly unknown[] },
) => Node<A, E, Tag<"global">>

export * as Node from "./app-node"
