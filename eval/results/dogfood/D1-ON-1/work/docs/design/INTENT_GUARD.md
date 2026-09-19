# Intent Guard (WS4)

Deterministic wrong-project / wrong-context safety layer for context assembly.

## Problem

CodeBro's durable context is user-level and global across workspaces. Global
records apply everywhere by design, but a global record can still be *about*
one specific project ("in hermes-agent always run the soak suite"). When a
session works in a different repository, injecting that record contaminates
the context. The same class of failure applies to tasks that name another
project while the session is bound to this one.

The guard answers one bounded question at packet-assembly time:

> Does this context plausibly belong to the project the session is actually
> working in?

It is a context-quality layer, not a judge: it never rewrites knowledge,
never promotes authority, never writes anything, and never calls an LLM.

## Inputs (all already available, all deterministic)

| Signal | Source |
|---|---|
| Current workspace basename | canonical workspace root |
| Declared project identity (name, repository URL) | `.codebro/project_identity.json` (only when actually loaded; the default sentinel name `unknown` is not a declaration) |
| Other known workspaces | `ContextStore::distinct_project_workspace_roots` — roots that carry at least one project-scoped record (bounded, sorted) |
| Task text | the caller's `task` argument |
| Record scope / kind / content / id | the resolved fingerprint records |

Matching is lexical on normalized identifiers (alphanumerics, lowercased,
separators removed), so `hermes-agent`, `hermes agent`, and `HermesAgent`
compare equal. No embeddings, no network, no clock.

## Rules

1. **Exclude clearly foreign global records.** A global record that names
   exactly one other known workspace and does not name the current project is
   excluded from the packet.
2. **Flag ambiguous records, never drop them.** A record that names the
   current project *and* another known workspace (or several others) is kept
   and annotated `ambiguous_project_reference` — it may be deliberate
   cross-project knowledge.
3. **Intents are never excluded.** A confirmed user goal is never silently
   hidden by a heuristic; a foreign-referencing intent is only flagged.
4. **Scope is authoritative.** Project- and task-scoped records are never
   content-scanned: their scope already binds them to this workspace (and
   task), which is the stronger guarantee.
5. **Identity consistency.** A loaded identity whose name matches neither the
   workspace basename nor the repository segment produces an
   `identity_mismatch` warning (a wrong-repo signal).
6. **Task consistency.** A task that names another known workspace and not
   the current one produces a `task_mentions_foreign_workspace` warning.

## Output

A bounded, serializable `IntentGuardReport` attached additively to the
`context` packet (`guard`) and the `engineering_brief` (`guard`):

```json
{
  "verdict": "aligned | review | unverified",
  "identity_checked": true,
  "identity_matched": true,
  "intent_coverage": "task | project | global | none",
  "known_workspaces": 1,
  "excluded_records": ["ctx::..."],
  "flagged_records": ["ctx::..."],
  "signals": [
    {"code": "foreign_records_excluded", "severity": "warn", "detail": "..."}
  ]
}
```

Verdict semantics:

| Verdict | Meaning |
|---|---|
| `aligned` | Identity matched and/or an actionable intent anchors the viewpoint; no contradiction |
| `review` | At least one `warn` signal: identity mismatch, foreign task reference, excluded/flagged records |
| `unverified` | No identity and no intent to align against — the guard cannot vouch either way |

A `review` verdict also appends one bounded line to the packet's `notes`.
The OpenCode session plugin renders the top two `warn` details as an
`Intent guard: review — …` line and per-record `guard:` annotations.

Bounds: 8 signals, 16 ids per list, 160 characters per detail, 64 other
workspaces considered. Signal details contain record **ids** and workspace
**basenames** only — never record content.

## Invariants

- **Read-only.** The guard runs inside `context_packet::context_record_excerpts_guarded`
  and `build_context_packet`; it never touches a store row. Verified by
  `guard_never_writes_and_never_promotes`.
- **No authority promotion.** Records keep their authority; the guard only
  filters and annotates.
- **One canonical path.** The `context` MCP tool, the `codebro context` CLI,
  and the `engineering_brief` all resolve records through the same guarded
  function, so both surfaces agree on what was excluded.
- **Backward compatible.** `guard` is an optional additive field; packets and
  briefs without it (library callers, pre-WS4 data) parse unchanged.
- **Fail-safe.** A failing known-workspace query degrades to
  identity/basename-only evaluation; it never fails the packet.

## Tests

- `crates/mcp-server/src/intent_guard.rs` — 16 unit tests: matching/mismatched
  identity, exclusion, ambiguity, global/project/task intent coverage,
  no-intent behavior, determinism (order independence), bounds, no content
  leakage, alias handling, scope-authority, sentinel handling.
- `crates/mcp-server/src/context_packet.rs` — 8 integration tests: exclusion
  and reporting through the real packet, read-only guarantee, cross-project
  annotation, aligned/unverified verdicts, legacy JSON parsing, structural
  digest coverage, byte-stable output.
- `crates/context-runtime/src/store.rs` — 1 query test (sorted, unique,
  bounded, project-scope only).
- `crates/mcp-server/src/engineering_brief.rs` — 1 propagation/omission test.
- `integrations/opencode/test/codebro-context.test.mjs` — 1 plugin probe.
