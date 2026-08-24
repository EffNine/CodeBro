# CodeBro v1.0.0 Release Notes

Release date: 2026-08-24
Branch: `cleanup/runtime-consolidation`
Baseline tag: `v0.7.0-mcp-rc2`

---

## 1. Release overview

CodeBro v1.0 is an **MCP-first engineering intelligence runtime** for AI
coding agents. Over stdio it exposes a frozen 17-tool contract that gives
external agents:

- verified repository facts (deterministic, provenance-carrying),
- persistent engineering memory (trust-aware, never promoted into facts),
- structural impact analysis ("what breaks if this changes?"),
- guarded mutation of project files,
- reproducible sandboxed execution evidence.

CodeBro is not an agent, not a chat/TUI product, and not a model provider.
Reasoning, planning and user interaction belong to the host agent; CodeBro
owns understanding, memory, safety and verification.

### Major architectural transformation

Since the `v0.7.0-mcp-rc2` baseline:

- The single binary crate became a **10-crate Cargo workspace**
  (`core`, `parsers`, `fact-store`, `identity-runtime`, `memory-runtime`,
  `sandbox-runtime`, `impact-engine`, `indexer`, `change-engine`,
  `mcp-server`) with dependency direction enforced by
  `scripts/check_workspace_deps.sh`.
- ~110k lines of retired pre-MCP architecture (TUI agent loop, Adaptive
  Developer Platform experiments) were **deleted**; evaluation and
  preservation mapping in `docs/LEGACY_RETIREMENT.md`, history on tags
  `v0.7.0-mcp-rc1/rc2` and branch `tui-legacy`.

### Positioning

*"The engineering intelligence runtime layer for AI coding agents."*

---

## 2. Completed phases (Phase 0 – Phase 10)

| Phase | Delivered | Commit |
|-------|-----------|--------|
| 0 | Baseline lock + V1 roadmap | `9843e3c` |
| 1 | Workspace modularization (10 crates, direction guards) | `bae0142` |
| 2 | Legacy retirement (~110k lines removed) | `4c19daf` |
| 3 | Repository intelligence graph (Language/Framework/EntryPoint fact kinds, multi-manifest discovery) | `2bf49b5` |
| 4 | Incremental indexing via content-addressed parse cache + `codebro facts diff` | `b51e09e` |
| 5 | Engineering memory V2 (lifecycle, expiry, provenance, conflict detection) | `7a02353` |
| 6 | Impact analysis V2 (routes/docs/configs edges, confidence + reason per edge) | `a4c3599` |
| 7 | Transactional multi-file change engine (`apply_changes`) | `45a921e` |
| 8 | Execution evidence envelope completed (environment capture) + pnpm/yarn/pytest detection | `677e440` |
| 9 | MCP API v1 frozen (`docs/MCP_API_V1.md`) | `948052d` |
| 10 | Production hardening (CI, release pipeline, audit, benchmarks) | `f8eca23` |

Important architectural decisions made during delivery:

- Two crates beyond the original 8-crate plan (`parsers`, `indexer`) were
  added to break the indexer↔impact↔parser dependency cycle cleanly.
- `RepoState`/`RepoIdentity` moved into `core` so facts, impact and the
  indexer share repository-state primitives without depending on the
  sandbox.
- Incremental indexing is **content-addressed**: unchanged files reuse
  cached tree-sitter parses; output stays byte-identical between cold and
  warm runs (regression-tested).
- Memory schema 1.1.0 loads stores written by 1.0.0 and vice versa;
  known 1.x schemas are accepted on load.
- Multi-file atomicity is **rollback-based**, honestly documented — POSIX
  has no true atomic multi-file commit.

---

## 3. Key capabilities

- **Repository intelligence** — language/package/framework/entry-point
  detection from manifests and source; frameworks require concrete
  dependency evidence; unknown dependencies invent nothing.
- **Fact graph** — canonical facts model (workspace, packages, modules,
  symbols incl. HTTP routes, tests, build targets, dependencies,
  relationships incl. calls/imports/documents/configures, references,
  diagnostics, architecture rules, languages, frameworks, entry points)
  frozen after build and validated (duplicate/broken-index/orphan rules).
- **Engineering memory** — persistent, bounded (20 entries / 500-token
  budget / 0.3 min confidence by default), secret-redacted, expiring,
  conflict-aware. Structurally separated from the fact store; a memory
  write cannot touch `facts.json` (pinned by tests).
- **Impact analysis** — directed traversal over the relationship graph
  with per-edge confidence (provenance quality × depth decay) and reason;
  ambiguity is reported, never guessed.
- **Transactional changes** — validate-all → preview → conflict pass →
  sequential apply with automatic rollback; single-file seam unchanged.
- **MCP tools** — 17 tools, contract frozen in `docs/MCP_API_V1.md`.
- **Evidence system** — every sandbox execution returns command, exit
  code, duration, redacted stdout/stderr, timestamp, execution id,
  repo identity/state (git revision + deterministic dirty-tree hash),
  backend capabilities, reproducibility classification, and environment
  capture.

## 4. Performance baseline

Measured by `scripts/bench.sh` on synthetic repos
(crates × modules × functions), Linux, rustc 1.97.1, release build.
Full table: `docs/benchmark/results.md`.

| repo | shape | cold init | warm init (parse cache) | facts.json |
|------|-------|-----------|--------------------------|------------|
| small | 2×4×20 | 14 ms | 10 ms | 136 KiB |
| medium | 8×16×40 | 101 ms | 44 ms | 4056 KiB |
| large | 24×48×80 (~92k fns) | 2214 ms | 1169 ms | 72 MiB |

The development repository itself indexes in well under a second warm.

## 5. MCP API v1 stability statement

The 17-tool contract documented in `docs/MCP_API_V1.md` is **frozen** for
the v1.x line. Future changes are additive only: new optional arguments,
new tools, new response fields. Removals, renames, narrowed value domains
and semantic-guarantee changes require a major version bump and a
documented migration path. `.codebro` state files follow the same rule
(new fields must carry serde defaults so older readers keep loading).

## 6. Known limitations

Stated plainly, without optimism bias:

- **Deferred language grammars.** Rust, Go, Python, JavaScript/TypeScript
  are indexed today. C/C++, Java, Kotlin, Ruby and PHP remain future work;
  the parser platform is designed to accept new grammars additively.
- **Rollback-based transactional atomicity.** A crash mid-transaction can
  leave a partial state on disk; the engine guarantees all-or-nothing
  against *failures it observes*, not power loss.
- **Indexing scale.** Incrementality covers parsing (the dominant cost);
  relationship construction and serialization still run over the whole
  model each init. Very large repositories should expect cold-index times
  to grow roughly linearly with source volume.
- **Local sandbox backend has no OS-level isolation** (`isolation: none`
  in its capability descriptor); policy gating is command-name based.
  Configure `OPEN_SANDBOX_URL` for isolated remote execution — when
  configured but unavailable, execution fails closed rather than falling
  back silently.
- **Memory bounds are fixed defaults** (entries/token budget/confidence);
  there is no operator configuration surface yet.
- **Single-writer assumption.** Concurrent `codebro init` runs against one
  workspace are serialized only by filesystem atomicity, not by locking.
  Similarly, MCP mutating calls (`record_memory`, `delete_memory`,
  `update_identity`, `apply_*`) are safe when issued sequentially — the
  normal agent pattern of awaiting each response — but a client that
  pipelines multiple mutations without waiting may observe last-writer-
  wins on the affected state file. Remediation path: a per-workspace
  mutation lock in the server.
- **One accepted security advisory.** `cargo audit` is clean at release
  except RUSTSEC-2025-0009 (`ring` 0.17.9: AES functions may panic when
  overflow checking is enabled — not reachable in release builds where
  overflow checks are off). Upgrading `ring` requires a newer `cc` than
  the tree-sitter 0.20 grammar pins allow; recorded with rationale in
  `.cargo/audit.toml`, remediation lands with the future grammar-stack
  upgrade. The h2 advisory was resolved before tagging (`h2 0.4.19`).
