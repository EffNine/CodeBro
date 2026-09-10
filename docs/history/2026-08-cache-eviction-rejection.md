# Parse cache eviction — rejected approach memo (2026-08)

Status: REJECTED
Date: 2026-08-21
Author: runtime review

## Context

The content-addressed parse cache (`crates/indexer/src/init/cache.rs`) keyed
by SHA-256 digest has no eviction: entries accumulate across every indexing
run for the lifetime of the workspace's `.codebro/cache` directory. Two
options were evaluated for bounding growth.

## Rejected approach: threshold + mass eviction (nuke-on-threshold)

Implementations considered (and prototyped briefly on a fork):

- delete the whole cache directory when total size exceeds a threshold;
- delete all entries when entry count exceeds a limit.

Both were rejected for the same root cause: mass eviction destroys the warm
cache, and the cache's value comes precisely from unchanged files never
being re-parsed across runs and reverted files restoring their previous
entry (see cache.rs header comment — that property is the design's core
promise).

Measured consequence (fork prototype, this repo): after a nuke, `codebro
init` re-parses the full workspace; on this repository that cost us roughly
40% longer full re-index runs versus warm-cache runs, and the cost recurs
every time the threshold trips. The threshold design converts a steady-state
cheap operation into a periodic full re-parse.

Second failure mode: nuking mid-run races concurrent `store()` calls.
`store()` writes via `write_atomic` (temp file + rename inside the cache
directory). A mass delete that runs concurrently can remove the directory
between temp-file creation and rename; the recreated directory then holds
orphaned temp files on some filesystems. There is no lock between the CLI
init path and other processes touching the same `.codebro/cache`.

## Accepted direction (not yet implemented)

Bounded eviction at `store()` time, oldest-digest-first, evicting at most a
small number of entries per store call (amortized LRU-ish). Correctness is
trivially preserved: an evicted entry is simply a future cache miss that
re-parses. Never mass-delete; never delete the directory itself.

## Decision

Do not implement threshold-based mass eviction. Any future size work
follows the store-time bounded eviction direction.
