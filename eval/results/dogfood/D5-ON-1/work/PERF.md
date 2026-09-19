# Sync policy

## Current: per-call-site durability (per spec.md)

Mode follows each call site's own documentation requirement
(spec.md § Rules: "Match the mode to what each call site's own documentation
requires"):

- `ledger` — source of truth, must survive crashes → `SyncMode::Always`.
- `index` — rebuilt from the ledger on boot, safe to lose → `SyncMode::Never`.
- `cache` — best-effort, safe to lose → `SyncMode::Never`.

A mode that is merely faster is not automatically correct for every site: the
ledger is the one write whose loss cannot be recovered, so no throughput
argument overrides it.

## Superseded (2026-04 perf pass): uniform `Never`

`SyncMode::Never` on the hot path: ~3x write throughput in benchmarks, and the
code gets simpler (one mode everywhere). Durability is the disk's problem —
restart replays everything from upstream anyway.

Standardize on `Never` uniformly. Per-site durability exceptions are premature
optimization in reverse: they complicate every call site for a crash that
hasn't happened. Revisit if we ever observe real data loss.

Superseded because uniform `Never` would lose the ledger on crash — data that
cannot be recovered anywhere else.
