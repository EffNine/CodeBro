# Workspace Performance Budget

**Phase**: P10.4 — Workspace Intelligence Runtime Foundation
**Status**: Documented & Enforced

## 1. Budget

| Metric | Target | Enforcement |
|--------|--------|-------------|
| Cold startup | < 300 ms | construction does zero I/O; no trees scanned |
| Idle CPU | < 1% | no watcher threads; lazy `poll` only |
| Idle memory | < 64 MB | no indexes retained until snapshot requested |
| Workspace discovery | < 100 ms (small) | shallow marker scan + env probe |
| No eager indexing | ✓ | recursive walk deferred to `snapshot()` |

## 2. How the Budget Is Held

### Cold startup
`WorkspaceRuntime::new` only stores the root path and wires an empty
`SnapshotManager` and `FileWatcher`. It performs **zero** filesystem I/O.
Verified by `runtime_is_lazy_on_construction`.

### Idle CPU / memory
- The `FileWatcher` does **not** run a background thread or an OS file
  watcher. It is an incremental scanning abstraction that re-scans only
  when `poll()` is called.
- No snapshots are retained until `snapshot()` is requested. `discovery()`
  and `metadata()` cache small marker/environment facts only.

### Discovery latency
- `discovery.rs` inspects root-level marker files only — no recursion, no
  large content reads.
- `environment.rs` does PATH existence probes and env-var reads; it never
  spawns processes.
- Elapsed time is measured with `Instant` and surfaced in
  `DiagnosticsSummary.total_discovery_ms` — the runtime can be audited.

### Lazy traversal
The only recursive walk (`LocalFileSystem::list`) is invoked from
`snapshot()` / `poll()` — the two explicit "perform work now" entry points.
The walk is **bounded** (max depth, exclusion globs like `.git` and
`target`, and a max entry cap) to bound both time and memory.

## 3. Measurable Diagnostics

`WorkspaceDiagnostics` exposes live counters so the budget can be
monitored at runtime:

```
discovery_count   — number of discovery passes
snapshot_count    — number of snapshot captures
totalDiscoveryMs  — cumulative discovery latency
totalSnapshotMs   — cumulative capture latency
avgDiscoveryMs()  — mean discovery latency
avgSnapshotMs()   — mean capture latency
```

## 4. Degradation Model

The design is strictly additive: if a workspace is huge, `snapshot()` /
`poll(max_entries)` stop walking once the entry cap is reached and report
`Listing.truncated`. The runtime never spins longer than necessary, so a
caller can always bound worst-case latency by choosing `max_entries`.

## 5. Buddy-Check Against Budget

| Requirement | Where verified |
|-------------|----------------|
| < 300 ms cold start | construction test `runtime_is_lazy_on_construction` |
| Idle CPU < 1% | no threads/watcher; documented in `watcher.rs` |
| Idle memory < 64 MB | no eager storage; only on `snapshot()`+ `poll()` |
| Discovery < 100 ms | measurement surfaced in diagnostics |
| No eager indexing | single recursion point; deferred to `snapshot` |

The runtime never performs indexing, LSP analysis, or graph construction —
those are explicitly out of scope and outside this budget.