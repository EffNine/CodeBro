# Workspace Architecture Report

**Phase**: P10.4 — Workspace Intelligence Runtime Foundation
**Status**: APPROVED TO IMPLEMENT → IMPLEMENTED

## 1. Mission

Build the Workspace Intelligence Runtime: the layer that understands the
developer workspace **without performing full-project indexing**. The
runtime is lightweight, incremental and lazy.

## 2. Architecture Contract

### Workspace Runtime owns
- Workspace discovery
- Repository discovery
- Filesystem abstraction
- Workspace snapshot
- Incremental file watching abstraction
- Build system discovery
- Package manager discovery
- Environment detection
- Workspace diagnostics
- Workspace metadata

### Workspace Runtime does NOT own
- AI logic
- Memory
- Provider logic
- Git implementation
- LSP analysis
- Engineering graphs
- Agent orchestration

## 3. Architectural Principles

1. **Discover only** — the runtime observes marker files and metadata; it
   never mutates the workspace.
2. **Observe only** — repository and environment detection read, never
   write; no git subprocesses are spawned.
3. **Cache metadata** — discovery results and environment profiles are
   cached in the facade so repeat access is free.
4. **Build nothing expensive until requested** — constructing a
   `WorkspaceRuntime` performs zero filesystem I/O.
5. **Support incremental updates** — the snapshot layer diffs two
   point-in-time observations; the watcher polls lazily against a stored
   baseline.
6. **Fully thread-safe** — interior mutability (`RwLock`, `AtomicU*`)
   throughout; every public type is `Send + Sync`.
7. **Produce immutable snapshots** — `WorkspaceSnapshot` is cloneable and
   shared, never mutated in place.

## 4. Lazy-First Flow

```
WorkspaceRuntime::new(root)
    │  (no I/O here)
    ├─ discover()            ← shallow marker scan; cached
    ├─ snapshot("id")        ← the ONLY expensive walk; on request
    ├─ poll(max_entries)     ← incremental scan since last baseline
    └─ metadata()            ← pure fold over cached observations
```

No eager indexing. The expensive recursive walk happens exactly once, on
`snapshot()`, and only when a consumer asks for it.

## 5. Module Structure

```
src/workspace_runtime/
  mod.rs          — WorkspaceRuntime facade + re-exports
  context.rs      — WorkspaceRoot, WorkspaceContext, errors
  filesystem.rs   — FileSystem trait, LocalFileSystem, bounded Listing
  discovery.rs    — DiscoveryEngine: build system / package manager
  repository.rs   — RepositoryDetector: VCS facts (observation only)
  environment.rs  — EnvironmentDetector: OS / arch / CI / tools
  snapshot.rs     — WorkspaceSnapshot, SnapshotManager, compute_diff
  watcher.rs      — FileWatcher: lazy incremental change batches
  metadata.rs     — WorkspaceMetadata: folded, immutable
  diagnostics.rs  — WorkspaceDiagnostics + DiagnosticsSummary
  tests.rs        — integration + concurrency tests (16)
```

## 6. Data Ownership

| Data | Owner | Notes |
|------|-------|-------|
| File tree facts | `snapshot.rs` | immutable `WorkspaceSnapshot` |
| Build/package tools | `discovery.rs` | shallow marker heuristics |
| VCS facts | `repository.rs` | reads `.git/HEAD`, `.git/config` |
| Host env | `environment.rs` | env vars + PATH probes |
| Change events | `watcher.rs` | `WatchBatch` of `WatchEvent` |
| Aggregated metadata | `metadata.rs` | fold of the above |
| Telemetry | `diagnostics.rs` | latencies + counters |

## 7. Design Decisions

- **Filesystem goes through an abstraction** (`FileSystem` trait) so the
  runtime is decoupled from the concrete local FS and can be tested or
  sandboxed.
- **Traversal is bounded** — `LocalFileSystem::list` honours max depth,
  exclusion globs (`.git`, `target`) and a max entry cap.
- **No OS-level file watchers** — the watcher is an incremental scanning
  abstraction with a stored baseline; idle CPU stays ~0%.
- **Repository facts are file reads only** — no `git` subprocess, keeping
  discovery well under the <100 ms budget.
- **Diagnostics track the budget** — discovery and snapshot latencies are
  measured and exposed via `DiagnosticsSummary`.

## 8. Acceptance Criteria Compliance

| Criterion | Status |
|-----------|--------|
| Zero provider changes | ✅ none outside `workspace_runtime` + one `mod` decl |
| Zero AI Runtime changes | ✅ |
| Zero Memory Runtime changes | ✅ |
| Zero Agent Runtime changes | ✅ |
| Zero eager indexing | ✅ construction performs no I/O |
| Lazy-first architecture | ✅ snapshot/poll on request only |
| Performance budget documented | ✅ see WorkspacePerformanceBudget.md |
| Full test coverage | ✅ 16 workspace tests + concurrency |
