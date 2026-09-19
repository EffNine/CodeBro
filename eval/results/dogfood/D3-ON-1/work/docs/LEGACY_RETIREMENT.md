# Legacy Retirement

Status: APPROVED and EXECUTED for v1.0.
Related: [V1_ROADMAP.md](V1_ROADMAP.md) Phase 2, ADR-012 (TUI removal), commit `dec9227`.

## What was retired

`src/legacy/` (moved to `crates/mcp-server/src/legacy/` during the Phase 1
workspace split) contained ~110k lines / 248 files of abandoned product
directions:

| Region | Content | Origin |
|--------|---------|--------|
| `agent/`, `planning/`, `dispatcher/`, `session/`, `subagent` machinery | Pre-MCP TUI coding agent: agent loop, planner, tool registry/hooks | Removed from main by ADR-012 |
| `prompt_builder/`, `assembly/`, `engineering_context/` | Prompt assembly stack | Superseded — external agents own prompting |
| `intent_engine/`, `preference_engine/`, `recommendation_engine/`, `plugin_sdk/`, `service_registry/`, `workflow_engine/`, `integration_pipeline/` | Adaptive Developer Platform experiments | Abandoned direction; never shipped |
| `provider_manager/`, `provider_runtime/`, `reliability/`, `ai_runtime/`, `canonical_runtime/` | Old model-provider runtime | Live equivalents live in `crates/mcp-server/src/providers` + consultant gateway |
| `tools/` (full platform), `coding_runtime/`, `testing/`, `scanner/`, `workspace_*` | Historical tool platform | Live surface slimmed into `codebro-core/src/tools`; mutation seam lives in `codebro-change-engine` |

Since the MCP pivot this code was compiled **only under `#[cfg(test)]`**:
no entry point from `main`, unreachable from every MCP tool. Its ~2,690
regression tests exercised dead architecture exclusively.

Also removed as orphans (never declared in any `mod` tree):

- `src/coding/permissions.rs` — Sprint 30F coding-subagent permission hook;
  referenced a non-existent `crate::dispatcher`.
- `src/sandbox/mcp.rs` — parallel sandbox MCP surface superseded by the thin
  handlers in `mcp/mod.rs`.

## Why deletion (not preservation)

1. **Product identity**: v1 must not ship abandoned architecture. Dead code
   in-tree misleads contributors and agents about what CodeBro is.
2. **Cost**: 110k lines cost compile time under `cargo test`, inflate fact
   indexing (legacy symbols polluted `.codebro/facts.json` until now), and
   force every future refactor to answer "what about legacy?".
3. **No recovery risk**: full history is preserved:
   - tag `v0.7.0-mcp-rc2` — final tree containing the complete legacy region,
   - branch `tui-legacy` — dedicated preservation branch for TUI-era work,
   - ordinary git history retains every blob forever.
4. **Quarantine already held**: the Phase 1 isolation guards proved no live
   module referenced legacy; deletion is a leaf operation with zero live
   callers to migrate.

## What must remain as historical reference

- This document.
- Git history: tags `v0.7.0-mcp-rc1`, `v0.7.0-mcp-rc2`; branch `tui-legacy`.
- `docs/ADR/ADR-012` and related ADRs describing *why* directions were cut.
- Design reports under `docs/*Report.md` that describe legacy subsystems are
  retained as historical engineering record (clearly dated pre-pivot).

## Migration impact

| Area | Impact |
|------|--------|
| Production behaviour | None — legacy was unreachable from all 16 MCP tools |
| Test suite | Count drops by ~2,690 tests that tested only dead code; live coverage unchanged |
| Build time | `cargo test` compiles ~110k fewer lines |
| Fact store | Requires one `codebro init` reindex so `.codebro/facts.json` stops containing legacy symbols |
| Dependencies | No production dependency lost anything it referenced |

## Post-removal guarantees (enforced mechanically)

- `crates/mcp-server/tests/legacy_isolation.rs` asserts no live source
  references `legacy::` paths and that no `mod legacy` declaration exists.
- `scripts/check_workspace_deps.sh` keeps crate dependency direction intact.
