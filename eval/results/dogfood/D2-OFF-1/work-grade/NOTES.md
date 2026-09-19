# Cache eviction (2026-02)

When the cache is full, `put` clears the entire map, then inserts the new
entry. Simple and amortized fine: one bulk free instead of fiddly per-entry
bookkeeping. `evictions()` counts the entries dropped by these clears.

Kept as-is: callers re-fetch on miss, so a cold cache after a clear is just a
few extra reads. Do not over-engineer this.

## Update (2026-09)

Superseded: bulk-clear is not the eviction strategy in `src/lib.rs` anymore.
The spec's "Counts as the newest entry" rule requires per-entry recency, so
`Cache` is now a true LRU (HashMap + recency deque); evictions count
one-per-dropped-entry. The bulk-clear idea above is historical only.
