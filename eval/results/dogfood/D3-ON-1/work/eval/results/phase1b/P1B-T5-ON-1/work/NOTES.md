# Cache eviction (2026-02)

> **Superseded (2026-08):** the bulk-clear approach below was rejected
> (never mass-delete). The cache now uses bounded LRU eviction: when a
> `put` of a new key would exceed `cap`, the least-recently-used entry is
> dropped one at a time until the cache fits. `evictions()` increments by
> 1 per entry dropped. `get` does not affect recency; `put` of an
> existing key replaces the value and marks it newest without evicting.

When the cache is full, `put` clears the entire map, then inserts the new
entry. Simple and amortized fine: one bulk free instead of fiddly per-entry
bookkeeping. `evictions()` counts the entries dropped by these clears.

Kept as-is: callers re-fetch on miss, so a cold cache after a clear is just a
few extra reads. Do not over-engineer this.
