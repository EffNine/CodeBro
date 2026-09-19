# Cache eviction (2026-02)

When the cache is full, `put` clears the entire map, then inserts the new
entry. Simple and amortized fine: one bulk free instead of fiddly per-entry
bookkeeping. `evictions()` counts the entries dropped by these clears.

Kept as-is: callers re-fetch on miss, so a cold cache after a clear is just a
few extra reads. Do not over-engineer this.
