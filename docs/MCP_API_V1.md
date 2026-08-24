# CodeBro MCP API v1

Status: **FROZEN for v1.0** (Phase 9 of [V1_ROADMAP.md](V1_ROADMAP.md)).
This document defines the stable, versioned MCP contract. After v1.0,
additions are additive; removals, renames, and semantic changes require a
major version bump plus a migration path.

- Transport: stdio (`rmcp`), JSON-RPC 2.0, protocol version `2024-11-05`.
- Server name: `codebro`; binary: `codebro serve --root <workspace>`.
- Tool arguments are passed as JSON strings and deserialized at runtime;
  every schema below is the authoritative shape.

## Versioning scheme

| Change | Compatibility | Version action |
|--------|---------------|----------------|
| New optional argument | Additive | minor |
| New tool | Additive | minor |
| New field in a response object | Additive | minor |
| Removing/renaming an argument or field | Breaking | major |
| Narrowing accepted value domains (e.g. enum removal) | Breaking | major |
| Changing a semantic guarantee (e.g. trust model) | Breaking | major |

`.codebro` state files follow the same discipline; writers may only add
fields carrying serde defaults (see `facts.json` `file_digests`, memory
schema `1.1.0`).

## Error contract

All tools return JSON-RPC errors via MCP:

| Situation | Error class |
|-----------|-------------|
| Invalid arguments (unknown kind, empty key, path traversal, ambiguous/stale edit) | `invalid_params` (-32602) |
| Runtime/store failures (persist errors, backend unavailable) | `internal_error` |
| Unknown tool / method | rmcp standard codes |

Deterministic zero-result responses (e.g. fact search misses) are **not**
errors — they return structured payloads with recovery hints.

## Frozen tool inventory (17)

### Read tools

| Tool | Arguments | Result highlights |
|------|-----------|-------------------|
| `workspace_context` | – | project identity, workspace root, per-kind fact counts (incl. `languages`, `frameworks`, `entry_points`) |
| `engineering_facts` | `query` (required unless `kind`/`path`), `kind` ∈ {workspace, module, package, symbol, test, build_target, dependency, relationship, reference, diagnostic, architecture_rule, language, framework, entry_point}, `path`, `limit ≤ 50` | deterministic ranked records: score desc → kind → name → path; provenance summary; freshness |
| `engineering_memory` | `task_keywords[]`, `active_file_tags[]` | bounded entries (≤20, token budget 500, min confidence 0.3) ranked importance → confidence; expired/superseded entries excluded; explicit truncation markers |
| `memory_stats` | – | entry count, tag distribution, average confidence, oldest/newest |
| `impact_analyze` | `target`, `target_type` {symbol,file,module,package}, optional `depth ≤ 5`, `direction`, `relationship_types[]`, `max_nodes` (default 1000) | status (incl. ambiguity matches), direct/transitive relationships each carrying `confidence` ∈ [0,1] (verified 0.95 / heuristic 0.55 / unknown 0.35, ×0.85 per hop), `reason`, evidence records, affected tests/modules/packages, completeness, traversal metadata, freshness |
| `sandbox_status` | – | backend {local,opensandbox}, availability, capability descriptor |
| `repository_health` | – | per-check results over workspace/.codebro/identity/facts/memory/git |

### Write tools

| Tool | Arguments | Guarantees |
|------|-----------|------------|
| `record_memory` | `key` ≤256ch, `value` ≤64KB (secret-redacted), `tags[]` ≤32×64ch, `confidence`, `importance`, `source?`, `expires_in_secs?`, `session?` | upsert by key replaces full logical entry preserving id/created_at; key conflicts supersede prior entry with lineage; near-duplicates flagged; writes never touch the fact store |
| `delete_memory` | `key`, `confirm=true` (else no-op) | deleting missing keys errors |
| `update_identity` | description/constraints/decisions/roadmap/sprint/conventions/patterns/architecture_summary | list fields append unique entries; duplicates reported skipped; requires existing identity |
| `apply_change` | `path`, `old` (empty = create), `new` | single-file guarded mutation: boundary, traversal denial, symlink escape prevention, stale-content protection, ambiguity rejection |
| `apply_changes` | `changes[]{path,old,new}` | multi-file transaction: validate all against current content → conflict pass re-checks staleness across the set → sequential apply with rollback on first failure; all-or-nothing |
| `sandbox_exec` | `command` (read-only build/test/lint policy), `working_directory?`, `timeout?`, `metadata?` | fail-closed execution; full evidence envelope below |
| `sandbox_test` | `command?` (auto: cargo/go/npm/pnpm/yarn/pytest), `expected_exit_code?`, `expected_success?`, `affected_fact_ids?` | auto-detected runner + pass/fail verification with violations |
| `sandbox_build` | same contract as sandbox_test (auto: cargo check/go build/npm-pnpm-yarn run build) | build/check verification |
| `consult` | `question`, `provider` {auto,conductor}, `mode` {architecture,debugging,code_review,planning,research,second_opinion}, file contexts, context flags | opinion injection of engineering context; never mutates state |

### Execution evidence envelope (v1)

Every `sandbox_*` execution returns:

```json
{
  "execution": {
    "command", "requested_command", "resolved_command",
    "working_directory",
    "exit_code", "success", "duration_ms",
    "timestamp", "execution_id",
    "stdout", "stderr",
    "timeout", "cancelled", "denied", "denied_reason",
    "backend", "mode",
    "repo_identity": {"project_id","root","repository_type"},
    "repo_state": {"commit_sha","working_tree_dirty","working_tree_hash"},
    "sandbox_capabilities": {...},
    "reproducibility": "deterministic|likely_deterministic|non_deterministic|unknown",
    "environment": {"os","arch","family"}
  },
  "verification": {"verified", "summary", "violations", ["impacted_fact_ids"]}
}
```

## Stability guarantees enforced mechanically

1. Dependency direction is validated by `scripts/check_workspace_deps.sh`.
2. The legacy quarantine guards live in `crates/mcp-server/tests/`.
3. Trust-model separation is pinned by `tests/trust_separation.rs`
   (memory writes cannot touch `facts.json`).
4. Determinism: identical input trees produce byte-identical `facts.json`
   (regression-tested), including warm parse-cache runs.
