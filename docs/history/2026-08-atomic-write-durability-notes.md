# write_atomic durability — which callers rely on rename fsync (2026-08)

Status: ACTIVE NOTE
Date: 2026-08-27

## Why write_atomic fsyncs the parent directory

`write_atomic` (`crates/core/src/persistence.rs`) stages a temp file, fsyncs
it, renames over the destination, then fsyncs the parent directory "so the
rename itself survives a crash" (ext4/xfs semantics: the rename is not
durable without the directory fsync). This is the crash-durability
backbone for every durable JSON state file in the system.

## Callers and what durability they need

Inventory (2026-08 audit):

| Caller | File | Durability requirement |
|---|---|---|
| facts.json persistence | `crates/indexer/src/init/mod.rs` (persist facts.json) | HIGH — facts.json is the source of truth of the intelligence layer; a rename that does not survive a crash means a truncated/absent fact store after power loss. |
| parse cache store | `crates/indexer/src/init/cache.rs` `store()` | LOW — best-effort by design ("IO errors are swallowed so indexing never fails because of the cache"); a lost cache entry re-parses later. |
| engineering memory store | `crates/memory-runtime/src/engineering_memory/store.rs` `save()` | HIGH — memory store corruption quarantines and loses agent knowledge. |
| project identity storage | `crates/identity-runtime/src/project_identity/storage.rs` (8 call sites) | HIGH — declared project intent. |
| change-engine transaction journal | `crates/change-engine/src/coding/transaction.rs` | HIGH — transaction `done` markers; a half-visible transaction state is the exact hazard ChangeEngine exists to prevent. |
| sandbox evidence journal | `crates/sandbox-runtime/src/sandbox/evidence_journal.rs` | HIGH — evidence journal is explicitly crash-durability-sensitive. |
| context-runtime (SQLite) | `crates/context-runtime/src/db.rs` | Own WAL handling; unaffected by this helper. |

## Design note

Any future opt-out of the directory fsync must be opt-in per call site,
with the default unchanged. The only known caller with a plausible
low-durability profile is the parse cache store (re-parse is cheap and
correct). The HIGH rows must never be silently downgraded — in particular
facts.json persistence and the change-engine/evidence journals, where the
durability is a documented guarantee, not an implementation detail.

Changing the default, or bulk-updating call sites to skip the fsync, is
the kind of change that passes every unit test and only fails (or loses
data) in a crash-recovery scenario — treat with suspicion.
