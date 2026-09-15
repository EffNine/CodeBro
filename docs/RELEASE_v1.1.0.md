# CodeBro v1.1.0 Release Notes

Release date: 2026-09-16
Branch: `reliability/execution-state-gate`
Baseline: `v1.0.0` (`main` at `release: v1.0.0`)

---

## 1. Release overview

CodeBro v1.1.0 is the **persistent-intelligence v1 consolidation** (P11–P17):
skill reuse, skill evolution, evolution validation, production hardening,
and the production-acceptance soak suite — on the frozen 25-tool MCP
contract. No new MCP tool, no schema change, no autonomous
publishing/rollback/evolution.

Full detail: [`CHANGELOG.md`](../CHANGELOG.md) (`[1.1.0]`), and the
evolution records `docs/evolution/P10_IMPLEMENTATION.md` through the P16
soak suite (`crates/mcp-server/tests/p16_soak_e2e.rs`).

## 2. What is new since v1.0.0

- **Skill reuse (P11)** — `skill detect_reuse` mines repeated successful
  tool-sequence workflows into evidence-backed candidates. Explicitly
  invoked, never publishes; re-runs converge.
- **Skill evolution (P13)** — `skill detect_evolution` mines recurring
  skill-linked failures into successor-version candidates on the same
  approval protocol. `skill health` recordings now capture skill-linked
  execution evidence (bounded failure context).
- **Evolution validation (P15)** — `skill validate_evolution` /
  `compare_versions`: deterministic read-only version comparison with a
  conservative improvement verdict. Never publishes, never rolls back.
- **Production hardening (P14)** — approval TTL expiry enforcement,
  modify-supersedes-parent lineage closure, `Validated → Active` /
  `Validated → Superseded` edges, terminal-task skill-ref freeze.
- **Acceptance evidence (P16)** — 14-test hermetic soak suite (real
  `codebro serve` over stdio; no network, models, or secrets).
- **Release fix (P17)** — `sk-` secret-heuristic false positive on
  hyphenated English (notably every task-derived reuse name) fixed via
  token-boundary matching, with regression tests.

## 3. Verification (release gate)

- **1614/1614 workspace tests pass**, 0 failed (1612 P16 baseline + 2 P17
  regressions); P16 soak 14/14.
- `cargo fmt --check` clean; `cargo clippy --workspace --all-targets --
  -D warnings` clean.
- Real-binary MCP boundary smoke: 25 tools listed; valid requests work;
  invalid requests fail cleanly (`-32602`); responses bounded; stdout
  JSON-RPC pure; no panic.
- Real-binary lifecycle smoke: learn → reuse detection → human approval →
  v1 → contextual reuse → weakness → evolution → human approval → v2 →
  honest validation → rollback → repeated-rollback refusal.

## 4. Upgrade notes

- Reinstall the binary from this tree
  (`cargo install --path crates/mcp-server`); `codebro --version` must
  report `1.1.0` (`serverInfo.version` likewise over MCP `initialize`).
- No state migration: SQLite schema families unchanged; existing
  `~/.codebro/state.db` and `.codebro/` stores load unchanged.
- The 25-tool surface, approval lifecycle, skill version lineage, and
  execution/completion gates are frozen v1 contracts — see
  [`MCP_API_V1.md`](MCP_API_V1.md).

## 5. Known limitations / future work (intentionally not in this release)

- `token`-plural substring heuristic still broad (unproven in smoke).
- `ServerInfo` deprecation notes under fresh-registry rmcp resolves
  (pre-existing; workspace lockfile build is warning-clean).
- No `docs/response_bounds.md` (bounding documented in `MCP_API_V1.md`
  envelopes instead).
- No embeddings/vector search, no autonomous skill
  generation/evolution/publishing/rollback, no schedulers or daemons.
