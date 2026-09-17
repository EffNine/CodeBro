# Proposal: remove the standalone `impact_analyze` MCP tool (keep the engine)

**Status: PROPOSED — decision required (approve / reject / defer). No code changed.**
**Date:** 2026-09-16. **Triggered by:** pre-registered Phase-2 symmetric rule
(zero `impact_analyze` calls in a task designed for it = removal-grade evidence).

## Evidence (all committed, re-verifiable)

- **0 direct calls in 19 agent sessions:** 5 ab-v2 condition-B trials (committed usage audit:
  "the impact-graph capability went unused"; "B did NOT use `impact_analyze` (the designated
  impact tool)") + 14 ON trial sessions across Phase-1 (6), Phase-1b (4), Phase-2 (4)
  (JSON `tool_use` parts where available; text-grep where default-format; repo-clean logs).
- **2 tasks designed for it, both bypassed:** ab-v2 T3 (hidden-impact; both arms used grep +
  a memo) and Phase-2 T8 (dispatch/decoy refactor; ON arms used orientation + post-task notes
  only, solved by reading; 4/4 green without it).
- **What agents use instead:** `engineering_memory` / `recall` (7–8 calls/round),
  `engineering_brief`, grep/read. The ON acquisition flow works without the standalone tool.
- **Counter-evidence HONESTLY noted:** the impact ENGINE feeds `engineering_brief`'s bounded
  traversal directly (`crate::impact::analyze`, not via the tool) plus doctor/debugging paths.
  Agents "choosing brief instead" may consume engine output silently — tool-call non-use does
  NOT prove engine uselessness. This proposal therefore targets the TOOL SURFACE ONLY.

## Scope of removal (mechanical, bounded)

- Remove the `impact_analyze` tool definition + handler wiring in `crates/mcp-server/src/mcp/mod.rs`.
- KEEP: `impact-engine` crate, `crate::impact::analyze` engine, brief/doctor/debugging/context
  consumers, health findings, risk signals — all untouched.
- Update: 25→24 tool contract (`integration.rs`, P8 contract tests), `docs/MCP_API_V1.md`,
  `crates/mcp-server/tests/final_gate_probe.rs`, `p6_engineering_e2e.rs`, AGENTS.md tool
  inventories, `docs/CODEBRO_PROJECT_HISTORY.md` MCP table (append-only note).
- Verify: full suite green + real-binary MCP smoke (24 tools) before merge.

## Options

| # | Option | Effect |
|---|---|---|
| A (recommended) | Remove standalone tool, keep engine + brief integration | Surface 25→24; every retained tool earns its place; engine value via brief stays available and becomes separately measurable |
| B | Remove engine entirely | REJECTED by author: blast radius (brief/doctor/debugging) unjustified by tool-call evidence alone |
| C | Keep tool + "resurface" (prompt nudges) | REJECTED by author: evidence-free; ab-v2 already showed nudges don't create use, and use was never shown valuable |
| D | Defer (keep, revisit after T9+) | Legitimate if the reader believes impact tasks haven't been fair yet — but T3+T8 were both designed fair and bypassed; state what new evidence would change the call |

## Decision (fill on resolution)

- [x] APPROVE (A) — schedule removal as normal change with the verification above
- [ ] REJECT — tool stays; record what evidence would reopen (proposer: a task where the
  affected set is unknowable by reading AND an ON session demonstrably uses the tool to win)
- [ ] DEFER — revisit after: _______________
- Decided by: operator (`okay` on Hermes-track execution) date: 2026-09-17 commit: (this removal)
