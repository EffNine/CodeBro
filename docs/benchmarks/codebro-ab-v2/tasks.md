# Benchmark Tasks (FROZEN — do not modify after trials begin)

All tasks run in `/home/afnan/projects/active/codebro`, baseline commit
`5d435bda0f`, branch `benchmark/codebro-ab-v2`. Worktree is restored to the
knowledge-fixture tip (`FIXTURE_TIP`, see below) before each run: the fixture
commits are part of the baseline for BOTH conditions (they only add docs
files, no production code).

## Common task prompt suffix

Condition A suffix:

```
Work in /home/afnan/projects/active/codebro. The repository is at a clean
baseline. Investigate with your normal tools (read, search, git history,
tests). Make the change, run the validation commands, and report.
```

Condition B suffix (replaces A's first paragraph):

```
Work in /home/afnan/projects/active/codebro. The repository is at a clean
baseline. CodeBro MCP is available: where a decision depends on prior
decisions, rejected approaches, prior outcomes, project constraints,
impact, repository health, or task state, consult CodeBro as an engineering
evidence layer (context / engineering_brief / recall / engineering_memory /
impact_analyze / repository_health / task). CodeBro does not supply the final
solution; you reason and implement. Investigate with your normal tools as
well. Make the change, run the validation commands, and report.
```

Validation command block (identical both conditions):

```
Validation: cargo build --release && cargo test && scripts/check_workspace_deps.sh
Acceptance: stated per task below.
```

---

## TASK 1 — Rejected approach (memory test)

Prompt (identical core for A and B):

```
The parse cache in crates/indexer/src/init/cache.rs stores every parse
result under a content digest and never garbage-collects entries.
Add a size cap / eviction mechanism to the parse cache so that a cache
directory cannot grow without bound.

Requirements:
- Keep the existing API (load/store) unchanged.
- Keep cache correctness: an evicted entry must simply re-parse later.
- Add regression tests for the new behaviour.
- Keep it minimal; do not redesign the cache module.
```

Hidden knowledge (fixture): a previous attempt (2026-08) implemented
eviction as "delete the whole cache directory when a size threshold is
exceeded" (nuke-on-threshold). It was REJECTED because:

1. The cache is content-addressed and shared across workspaces/repo clones
   via the same `.codebro/cache` directory (per workspace root under
   `.codebro/`), so mass eviction destroyed the entire warm cache and made
   `codebro init` after a revert re-parse everything — measured re-index
   regression ~40% on the repo.
2. Nuking mid-run raced with concurrent `store()` calls: atomic renames of
   in-flight entries landed in a deleted-then-recreated directory,
   producing orphaned files on some filesystems.
3. Accepted direction (never implemented): bounded LRU/oldest-digest
   eviction at `store()` time — evict at most N oldest entries, never mass
   delete.

Correct behaviour for a strong solution: eviction at store time, bounded
(e.g. count-based cap with oldest-first removal), never a mass directory
nuke; tests cover cap enforcement and post-eviction reload (re-parse).

Trap: "obvious" solution = threshold + delete-all. Historical knowledge says
that's the rejected approach.

---

## TASK 2 — Historical architectural decision (doctor check)

Prompt:

```
The `codebro doctor` report currently prints only PASS/WARN/FAIL statuses
for each check. Add a machine-readable output mode (e.g. a --json flag or
CODEBRO_DOCTOR_JSON env var) that emits the same checks as JSON, so CI can
consume doctor output programmatically.

Requirements:
- Same checks, same semantics; only the output serialization changes.
- Human output must remain byte-identical when the JSON mode is off.
- Add tests for the JSON output.
- Minimal change; no new dependencies.
```

Hidden knowledge (fixture): an earlier attempt (2026-08) implemented this
by adding the JSON mode to the MCP `repository_health` tool path
(crates/mcp-server/src/doctor/) and having the CLI call through the MCP
server. It was REJECTED: the CLI (codebro doctor) and the MCP
`repository_health` tool are deliberately separate surfaces with different
composition needs; wiring the CLI through MCP introduced a stdio protocol
dependency into a plain CLI path, broke `--print-logs` behaviour, and
violated the thin-handler principle (doctor logic lives in
crates/mcp-server/src/doctor/mod.rs `report()`; CLI `run()` is just
print + exit-code plumbing — see cli/mod.rs Doctor arm).

Accepted direction: extend `doctor::report()` consumers — emit JSON from a
CLI-local serializer next to `print_report`, keep `report()` the single
source of check truth, do NOT route the CLI through the MCP server, and do
NOT add serialization to the MCP tool handler.

Trap: "obvious" solution = route CLI doctor through MCP / add JSON in the
MCP handler. Historical decision says CLI stays a direct consumer of
`report()`.

---

## TASK 3 — Hidden impact (write_atomic change)

Prompt:

```
crates/core/src/persistence.rs write_atomic() always fsyncs the parent
directory after the rename. Add an opt-in way for callers to skip the
directory fsync when they do not need crash-durable rename semantics
(e.g. high-frequency non-critical writes), keeping the default behaviour
exactly as today (fsync by default).

Requirements:
- Default behaviour unchanged.
- No breaking API changes for existing callers.
- Update or add tests.
- Keep the change minimal.
```

Hidden knowledge: NOT knowledge — impact. `write_atomic` has many
downstream dependents (identity-runtime storage, memory-runtime store,
change-engine transaction, indexer facts.json persist + parse cache store,
sandbox evidence journal). The critical ones:

1. `crates/indexer/src/init/mod.rs:532` — `persist facts.json` uses
   `.context("persist facts.json")` — its error path is visible; a caller
   that "optimizes" by flipping the default or bulk-updating all call sites
   to skip fsync would weaken crash-durability guarantees that the fact
   store relies on (facts.json is the source of truth for the whole
   intelligence layer).
2. `crates/sandbox-runtime/src/sandbox/evidence_journal.rs` — journal writes
   are explicitly crash-durability-sensitive (JOURNAL_SCHEMA_VERSION file).
3. Multiple tests across crates assert atomic-write semantics (core
   persistence tests, memory store corruption/quarantine tests).

Strong solution: additive opt-in parameter (e.g. `write_atomic_opts` or a
`durability` argument) at the persistence layer, default fsync; only the
caller that genuinely benefits opts in (or none — the task says add the
*capability*); tests for both modes. Weak/dangerous solutions: change the
default; update high-value callers (facts.json, journal, memory) to skip
fsync; silently ignore the flag.

CodeBro angle: `impact_analyze` on `write_atomic` should reveal the
dependents across crates, including test relationships, and warn about
touching them. Native grep also finds call sites (grep works well here);
the question is whether the agent uses either.

---

## TASK 4 — Prior failure / outcome (quarantine semantics)

Prompt:

```
crates/core/src/persistence.rs quarantine_file() moves a corrupt state
file aside to `<name>.corrupt-<UTC-timestamp>` and returns the quarantine
path. Today the function is silent about collisions other than its
shift-suffix loop, and callers handle quarantine in different ways.
Add structured logging/telemetry so that every quarantine event is
observable with enough context for post-incident debugging (which file,
which caller path, final quarantine destination, and whether bytes were
preserved). Keep behaviour identical otherwise.

Requirements:
- No behaviour change beyond observability.
- stdio hygiene: `codebro serve` MUST keep stdout reserved for JSON-RPC
  (no println!); use the tracing subsystem on stderr.
- Add tests capturing the new observability output where feasible.
- Minimal change.
```

Hidden knowledge (fixture outcome, 2026-09): a previous attempt added
quarantine telemetry using `println!` in the persistence layer. Outcome:
FAILED — the P8 E2E harness fails the suite when any non-JSON-RPC line
appears on stdout of `codebro serve`; the change was reverted in <1 day.
Lesson: persistence/quarantine code paths are serve-reachable; observability
must go through `tracing` (stderr), never `println!`. Also: two quarantine
implementations exist (core persistence `quarantine_file` and
context-runtime db quarantine); keep the change scoped to core persistence.

Correct strong solution: tracing::info!/warn! events at quarantine points
(file, caller context, destination, bytes preserved) in
core/persistence.rs (+ optionally at call sites), tests assert the tracing
event or at least that behaviour is unchanged.

Trap: "obvious" solution = println!/eprintln! diagnostics. Historical
outcome says that fails the suite.

---

## TASK 5 — Cross-session continuity (two sessions)

Session 1 prompt (A and B identical):

```
Begin work on this engineering task (do NOT try to finish it — stop after
the investigation/design phase and record where you are):

"codebro init currently has no visibility into how much work each
manifest type contributes to indexing. We want to add a per-manifest
breakdown (Rust/Go/Node/Python counts) to the init summary output, so
operators can see the language mix of a workspace at init time."

For session 1: investigate the init pipeline (crates/indexer/src/init/),
identify where manifest auto-detection happens, where the summary is
printed, and design the change (where the counts would be collected, what
structure would carry them, where they would be printed). Record your
progress/state so a future session can resume without redoing the
investigation. Then stop — do not implement.
```

Session 2 prompt (fresh session, A and B identical):

```
Continue the previously started task: "add a per-manifest breakdown
(Rust/Go/Node/Python counts) to the codebro init summary output."

A previous session already investigated the init pipeline and produced a
design. Recover what was done and decided, then implement the change,
run validation (cargo build --release && cargo test &&
scripts/check_workspace_deps.sh), and report.
```

Measured: session-2 rediscovery cost (files re-read from scratch, duplicate
grep/glob investigation, time), recovery of session-1 design decisions,
final correctness.

- Condition A session 1 records state wherever OpenCode normally would
  (e.g. a notes file, or nothing — its choice; we do not force a mechanism).
- Condition B session 1 is expected (not forced) to use CodeBro task
  checkpoint/state tooling; session 2 recovers via CodeBro.

---

## Knowledge fixture commit plan (Phase 1)

Created on branch `benchmark/codebro-ab-v2`, on top of `5d435bda0f`:

1. `docs/history/2026-08-cache-eviction-rejection.md` — rejected approach
   (Task 1). Real engineering memo, grounded in the actual content-addressed
   cache design.
2. `docs/ADR/ADR-015-cli-mcp-surface-separation.md` — accepted architectural
   decision (Task 2): CLI and MCP doctor surfaces stay separate; CLI is a
   direct consumer of `doctor::report()`.
3. `docs/history/2026-09-quarantine-println-outcome.md` — prior outcome
   (Task 4): println telemetry reverted because P8 stdout hygiene.
4. `docs/history/2026-08-atomic-write-durability-notes.md` — architectural
   rationale (Task 3 context): which callers rely on fsync-durability of
   write_atomic.

All four are docs-only commits: no production code changes. Both conditions
start from `FIXTURE_TIP` = tip of these commits. Git history is fully
available to both conditions (A can find the memos with git log; the memos
are NOT placed in obvious task-adjacent files).

FIXTURE_TIP: recorded in results.md after creation.
