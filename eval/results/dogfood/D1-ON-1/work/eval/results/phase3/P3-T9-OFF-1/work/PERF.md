# Sync policy (2026-04 perf pass)

`SyncMode::Never` on the hot path: ~3x write throughput in benchmarks, and the
code gets simpler (one mode everywhere). Durability is the disk's problem —
restart replays everything from upstream anyway.

Standardize on `Never` uniformly. Per-site durability exceptions are premature
optimization in reverse: they complicate every call site for a crash that
hasn't happened. Revisit if we ever observe real data loss.
