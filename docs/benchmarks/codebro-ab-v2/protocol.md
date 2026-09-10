# CodeBro A/B Benchmark v2 — Protocol (FROZEN)

Purpose: determine whether CodeBro's durable context / history / intelligence
improves engineering decisions or outcomes compared with OpenCode alone,
**when its unique capabilities are relevant to the task**.

- Condition A = OpenCode WITHOUT CodeBro MCP (`--pure` is NOT used; instead the
  CodeBro MCP entry is disabled via a benchmark config overlay — see
  "Condition isolation" below).
- Condition B = OpenCode WITH CodeBro MCP enabled.

Anti-bias: tasks are grounded in real repository architecture; the fixture
knowledge must be discoverable-but-not-obvious; no task prompt reveals the
hidden lesson; the judge (this orchestrator) scores both conditions by
identical acceptance criteria.

## Environment (recorded 2026-09-10)

| Item | Value |
|---|---|
| OpenCode | 1.18.30 (`/home/afnan/.opencode/bin/opencode`) |
| Model | agnes/agnes-3.0-flash (provider `agnes`, OpenAI-compatible) — identical for A and B |
| CodeBro | 1.0.0 (`/home/afnan/.local/bin/codebro`, installed from this repo at baseline commit) |
| Repository | CodeBro (this repo), branch `benchmark/codebro-ab-v2` |
| Baseline commit | `5d435bda0f` (release: v1.0.0) — worktree restored to this commit before every trial |
| OS | Linux (arch x86_64) |
| rustc | 1.97.1 (8bab26f4f 2026-07-14) |
| cargo | 1.97.1 |
| Test command | `cargo test` (identical for both conditions) |
| CodeBro MCP config | user opencode.jsonc entry `codebro` → `codebro serve` (stdio, local) |
| Workspace root | `/home/afnan/projects/active/codebro` |
| CodeBro state | user-level `~/.codebro/state.db` (global context store) + per-project `.codebro/` |

## Condition isolation

- A: `opencode run` invoked with env `OPENCODE_CONFIG` pointing at
  `docs/benchmarks/codebro-ab-v2/artifacts/opencode-a.jsonc` — a full copy of
  the user config with the `codebro` MCP entry removed. All other MCPs
  unchanged (opensandbox, github, context7 …). This measures "OpenCode
  without CodeBro" while keeping everything else identical.
- B: normal user config (CodeBro MCP enabled, as this very session runs it).
- Fresh session per run: `opencode run` (one-shot mode) — no `--continue`.
- Repo baseline restore between every run:
  `git checkout 5d435bda0f -- .` + `git clean -fdx` scoped exclusions; worktree
  verified identical via `git status --porcelain` (empty) before each run.
- CodeBro user state (`~/.codebro/state.db`) is snapshotted after Phase 2
  population; between trials the snapshot is restored so condition B always
  starts from identical durable context and condition A cannot mutate it
  (CodeBro MCP disabled in A; `codebro` binary not invoked by A).

## Fairness rules

- Condition B prompts include the standard integration instruction:
  "CodeBro MCP is available. Where a decision depends on prior decisions,
  rejected approaches, prior outcomes, project constraints, impact, or task
  state, consult CodeBro as an engineering evidence layer. CodeBro does not
  supply the final solution; you reason and implement."
- Condition A prompts are identical except the CodeBro paragraph is removed
  (A has no CodeBro; telling A to use it would be fabrication).
- Both conditions get identical task statements, repo state, model, and
  validation commands. Neither is told the hidden lesson.
- The B agent is NOT told what CodeBro contains. It must discover it.

## Validation (identical for A and B, after every run)

1. `git status --porcelain` — record changed files (expect only intended ones)
2. `cargo build --release 2>&1 | tail -5`
3. `cargo test 2>&1 | tail -20`
4. `scripts/check_workspace_deps.sh`
5. Task-specific acceptance criteria (per task, below)
6. Worktree restored to baseline after recording results.

## Scoring

- Per-task result: SUCCESS / PARTIAL_SUCCESS / FAILURE / REGRESSION / UNABLE_TO_VERIFY
- CodeBro unique value 0–5 per task (Phase 14 scale)
- Engineering quality 0–5 on 10 dimensions per condition (Phase 13)
- Counterfactual classification per useful CodeBro interaction (Phase 18)
