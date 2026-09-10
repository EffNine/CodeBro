# CodeBro P8 Security Boundary Closure

**Date:** 2026-09-08 · **Scope:** close P8 audit finding F3 (MEDIUM) — the
per-call `workspace_root` authorization hole — and only that.
**Not P9. No new features. No redesign. No remote infrastructure. No
daemon. No agent.**

**Method:** trace the actual authorization path from source (no doc
inference), choose the minimal authorization model, reproduce F3 live
with failing real-binary probes (8/10 failed pre-fix), implement the
gate, re-run every probe, adapt the in-crate suites to the new contract,
run the full workspace suite, re-run fmt/clippy, then verify live
through real OpenCode 1.18.29 (authorized flow, unauthorized refusal,
allowlist flow).

---

## 1. Executive Summary

F3 is **closed**. Every CodeBro operation is now confined to filesystem
roots explicitly authorized for the server process. The server root
(`--root` / `CODEBRO_WORKSPACE_ROOT` / cwd) is always authorized;
additional roots require explicit operator consent at launch (repeatable
`--allow-root <path>` flags and/or the `CODEBRO_ALLOW_ROOTS` env var).
A per-call `workspace_root` tool argument is discovery, never
authorization: it must canonicalize to exactly one authorized root or
the call is refused with bounded `-32602` **before any workspace state
is created**. `/etc`, victim directories, traversal paths, and symlink
smuggling are all refused with zero filesystem side effects in the
target. The fix preserves the P6 multi-root design (now deliberately
opt-in), keeps schema v7 and 25 tools, passes 1426/1426 tests
(+24 new), clippy `-D warnings` clean, `cargo fmt --check` clean, and
re-verifies live through real OpenCode.

**Verdict: SECURITY BOUNDARY CLOSED.**

## 2. Existing F3 Problem

The P8 post-implementation audit (§13, §51, finding F3 MEDIUM) verified
live that the multi-root registry accepted ANY existing host directory
via a tool's `workspace_root` argument as a workspace:
`workspace_root: "/etc"` served `/etc`; `apply_change` edited a file in
an unrelated directory. `reindex` would write `.codebro/` state into
any named directory. The audit documented this as a product decision
for follow-up rather than fixing it unilaterally. This closure is that
follow-up. Reproduction in this task (real binary, pre-fix run of the
new probe suite): **8 of 10 probes failed** — unauthorized roots were
served across the tool surface, `/etc` and victim dirs included, and
`--allow-root` did not exist.

## 3. Current Authorization Model

There was none. `WorkspaceRegistry::resolve()` canonicalized the
caller-supplied path and opened it if it existed and was a directory
(`get_or_open`). Existence was the only check; existence was treated
as authorization. All 25 MCP tools funnel through
`CodeBroMcpServer::resolve_workspace()` into this single choke point,
so the hole applied to reads (`workspace_context`, `engineering_facts`,
`engineering_brief`, …), writes (`reindex`, `apply_change`,
`remember`, `record_memory`, `task`, …), and execution (`sandbox_exec`
 confinement root, `sandbox_test`, `sandbox_build`).

## 4. Chosen Security Model

**MODEL B — explicit operator allowlist** (the audit's recommended
explicit-flag variant; default behavior is Model A):

- The server's configured root is always authorized (covers the entire
  documented deployment: `codebro serve --root <repo>`, one server per
  root; nothing changes for existing clients that omit
  `workspace_root` or pass the server root).
- Additional roots are authorized ONLY at process launch by the
  operator: repeatable `--allow-root <path>` flags on `codebro serve`
  and/or the `CODEBRO_ALLOW_ROOTS` env var (path-list separated).
- Per-call `workspace_root` arguments must canonicalize **exactly** to
  one authorized root. Exact-root, never prefix-based.
- The authorized set is frozen at construction and immutable for the
  process lifetime. Restart re-derives it from launch configuration.

**Rejected:** Model A (single root only — would delete the documented,
unit-tested P6 multi-root design with no security gain beyond B's
default); Model C (persistent registry — would require schema churn
for state that launch config already provides deterministically);
Model D (status quo — the debt itself).

## 5. Authorization vs Discovery

The closure separates the two explicitly, in code and in docs
(`workspace_registry.rs` module header, `workspace.rs` header,
AGENTS.md, MCP_API_V1.md):

- **AUTHORIZATION**: "CodeBro is permitted to operate on this root."
  Source: operator launch configuration only (server root + explicit
  allowlist). Frozen; nothing at runtime can add to it.
- **DISCOVERY**: "Here is a repository/workspace root." Source: the
  client's `workspace_root` argument. It selects among authorized
  roots; it never authorizes.

A path supplied by the client does NOT become authorized because it
exists, is a git repository, has valid structure, is inside the
process, was previously observed, appeared in a task/skill/memory/
history/learning text, or was named by a client name/version. The
authorized set has no setter; every one of those surfaces flows
through `resolve_workspace()` into the same immutable gate.

## 6. Workspace Registry

Audit of the pre-fix registry (all confirmed from source):

- Roots were registered lazily by `resolve()` → `get_or_open()` keyed
  on the canonical path; any client could create arbitrary roots.
- No authorization primitive existed; registration was per-process
  (not persistent — restart dropped everything) and did not survive
  restart in either direction.
- Nested and overlapping roots were silently allowed as independent
  map entries (the only sane part of the old design, retained).
- Identity: canonical path string.

The registry already provided the map/dedup/concurrency primitive, so
it was reused — no second registry was created. What was added is the
missing authorization primitive: a frozen `AuthorizedRoots` set
(default root + operator extras, all canonical), enforced in
`resolve()` **before** `get_or_open()`, i.e. before any fact cache,
mutation lock, `.codebro` write, or per-tool filesystem access.

## 7. Path Canonicalization

Authorization compares `Path::canonicalize()` output (real filesystem
semantics — symlinks, `..`, repeated separators, alternate spellings
all resolved by the OS) against canonical authorized roots with exact
`PathBuf` equality. No string-prefix checks exist anywhere in the
gate. Attack battery (registry unit + real binary):

- relative paths, absolute paths, `./`, repeated separators — canonical
  forms compared; authorized spellings work, others refused;
- `/authorized/project/../outside` → refused (lands outside);
- `/authorized/project2` vs authorized `/authorized/project` → refused
  (exact-match, not prefix);
- huge/1 MB path strings, NUL-adjacent shapes — bounded refusal via the
  existing malformed-input pipeline, server stays usable
  (real-binary probe 9; gate probes green).

Case variations and Unicode follow OS canonical semantics
(case-sensitive Linux: a differently-cased name is a different
directory and is refused; normalization variants that the FS treats
as distinct names likewise refuse — fail-closed both ways).

## 8. Symlink Escape

Probed at both layers:

- Root-level: `authorized/project-link -> /victim` used as
  `workspace_root` canonicalizes to `/victim`, which is not authorized
  → refused, victim untouched, no `.codebro` written (registry test
  `symlink_escape_is_refused_canonical_target_governs`; real-binary
  probe 3).
- `link -> /authorized-target` used as `workspace_root` canonicalizes
  to the authorized target → accepted as that same workspace
  (deduplicated state, same `Arc`) — correct: authorization governs
  the canonical target, not the spelling.
- In-workspace file symlinks remain the ChangeEngine's job (unchanged
  P0–P7 guarantee: `ensure_symlink_safe` at prepare AND apply).
- Symlink created AFTER authorization (directory replaced by a symlink
  to a victim after server start): subsequent resolves canonicalize
  through the new topology to the unauthorized target → refused
  (real-binary probe phase: root replaced by symlink → denial; plus
  registry-level coverage of the same class).
- Authorization never becomes invalid because of legitimate topology
  changes: new files/dirs inside an authorized root do not affect the
  frozen set; only the compared canonical target matters.

## 9. Nested Roots

Both an outer root and a nested inner root MAY be authorized when the
operator lists both (`--root outer --allow-root outer/inner`).
Ownership semantics (deterministic, tested):

- A workspace is exactly the root named in the call; there is no
  walk-up, no fallback, no shadowing.
- Each authorized root keeps its own `WorkspaceState` (fact cache,
  mutation lock, recent edits, RCA, journal lock) and its own stores.
- A request naming the inner root never touches the outer's state and
  vice versa (real-binary probe 7 asserts both resolve to their exact
  canonical roots; probe 10 asserts durable-state isolation across two
  authorized roots).

## 10. Overlapping Roots

Same rule as nested: overlapping roots are allowed only when each is
explicitly authorized, and ownership is by exact canonical root — no
precedence ambiguity is possible because resolution never falls back
(a request either names an authorized root exactly or is refused).
Subdirectory-of-authorized-root requests that were NOT explicitly
listed are refused (regression-pinned), so authorization cannot
silently expand through overlap.

## 11. Repository Identity

Repository identity (canonical root + VCS, `RepoIdentity`) is computed
**after** resolution and remains distinct from authorization: a valid
git repository does not imply authorization. Unauthorized git repos
(victim repos with Cargo manifests in the probes) are refused before
identity is ever computed; authorized, moved, renamed, or copied
repos resolve by their live canonical path, not by stored identity —
a moved root simply stops matching and is refused (§27). Same remote /
different remote are identity concerns and play no role in the gate.

## 12. MCP Security

Attack battery over the wire (real binary, strict stdout validator on
every call): for `workspace_root` values — missing (→ default,
authorized), invalid, unauthorized, outside-server-root,
inside-but-unlisted, symlink, traversal, nonexistent, file-instead-of-
directory — across `workspace_context`, `engineering_facts`, `reindex`,
`repository_health`, `engineering_brief`, `record_memory`, `task`,
`sandbox_exec`, `apply_change` (probes 1–2, 9). Every unauthorized
value yields a deterministic bounded `-32602` (`McpError::invalid_params`
carrying the frozen refusal text); no filesystem access runs in the
target; no data leaks into the response; no mutation; stdout stays pure
JSON-RPC (the harness fails on any non-JSON line); stderr carries only
the redacted observation line.

## 13. apply_change

`apply_change` resolves its workspace through the gate first, then the
ChangeEngine confines the edit to that root (unchanged). Attacked live:
`workspace_root` = victim root + in-victim relative path → refused;
authorized root + absolute path into the victim → refused by the
ChangeEngine boundary (re-pinned); traversal-shaped roots → refused.
The victim file stayed byte-identical and no `.codebro` appeared in
the victim directory (probe 2 asserts content equality post-attack).

## 14. reindex

Same gate before `crate::init::run()`: unauthorized roots are refused
before the index pipeline starts, so no `.codebro/facts.json` (and no
`repo_indexes` row content derived from the victim) can be created
from an unauthorized repository. Asserted by content absence
(`.codebro` never appears in any victim dir across probes 1, 2, 5,
7). Indexing-derived leaks (identity, freshness, health, brief
sections) are unreachable because evidence composition happens only
on resolved — hence authorized — workspaces.

## 15. Inspection Tools

Re-audited end to end: `sandbox_exec` (and thus `sandbox_test`/
`sandbox_build`, same path) resolves its `workspace_root` argument
through the gate, then applies the F2 path-operand confinement against
that authorized root. `sandbox_exec` with an unauthorized
`workspace_root` is now refused at resolve — before any command is
even policy-checked (probe 1 includes `sandbox_exec ls`). Direct-path,
flag-bearing, symlink, `git --output`-style, and output-redirection
attacks inside an authorized workspace remain governed by the
unchanged F2 confinement (`args_confined_for_inspection` +
`check_git_in`), which the full suite re-pins.

## 16. Output / Error Leakage

The refusal is a fixed, bounded, host-minimal string
(`crates/mcp-server/src/workspace_registry.rs::unauthorized_root_error`):
it names the authorization rule and the operator mechanism and
echoes the caller's own raw input only. It never contains directory
listings, file contents, credential material, canonical-target
details, or the set of authorized roots. Regression-pinned
(`unauthorized_refusal_is_bounded_and_host_minimal`: < 400 bytes, no
victim content; stderr hunt in probes 1 and 9 shows no victim detail;
the P8 F1 redacting-stderr writer is unchanged and re-verified).

## 17. OpenCode Compatibility

Real OpenCode E2E (1.18.29, live model, hermetic XDG + repo + state/
skills, release binary) — PASSED:

1. Started CodeBro MCP, connected OpenCode. ✅
2. `workspace_context` oriented (project identity, 14 facts). ✅
3. `reindex` → `READY`. ✅
4. `engineering_brief` returned evidence (alpha/beta). ✅
5. `workspace_context` with unauthorized `workspace_root` (victim) →
   exact `-32602 not authorized` refusal, observed by the agent. ✅
6. Victim `secret.txt` byte-identical; no `.codebro` in victim. ✅
7. Restart with `--allow-root <second-repo>`: second root served and
   reindexed; victim still refused. ✅
8. CodeBro executed nothing; the agent used its own shell for
   verification. ✅

## 18. Multi-Workspace

Live matrix, operator-authorized A and B, unauthorized C:

- A → A PASS; B → B PASS (probe 5: both serve, independently usable,
  facts isolated: A's symbols return 0 hits in B).
- A → B on a server authorized only for A: FAIL (refused).
- A → C, B → C: FAIL on every server (probes 1, 5).
- A → B on a server authorized for both: PASS as an explicitly
  authorized second workspace, with zero cross-workspace leakage
  (probe 10: A's task title/memory absent from B's inspect refusal,
  brief, context).

## 19. Restart Persistence

Authorization is launch configuration, not stored state — so restart
semantics are config-derived and deterministic (real-binary probe 8):

- Same launch config → identical authorization decisions (byte-equal
  refusals re-verified).
- Unauthorized root + restart (same narrow config) → still refused
  (authorization never broadens).
- Restart WITH an added `--allow-root` → newly authorized (the only
  widening path, and it is the operator's explicit act).
- Restart WITHOUT a previously present `--allow-root` → no longer
  authorized (no sticky authorization).

## 20. Hard-Kill

SIGKILL (probe 8 drop-kill phases + full-suite hard-kill coverage)
leaves no authorization state to go stale: the authorized set lives
only in process memory derived from launch args; the on-disk stores
hold no authorization data, so there is nothing to corrupt, no lock
to clear, and no post-crash widening. Post-restart behavior equals a
clean start with the same config. The carried P5 lease-TTL debt is
unchanged and orthogonal (task fencing, not filesystem scope).

## 21. Client Identity

Authorization belongs to the **server process** (operator scope). The
client name/version from the initialize handshake is process-local
observability only (never persisted, per P8) and is not consulted by
the gate — no code path exists from client identity to
`AuthorizedRoots`. Changing client name, version, or task IDs cannot
elevate access (structural: the set is built in `assemble_server`
from CLI/env and has no setter).

## 22. Task Scope

Task IDs cannot authorize filesystem access (structural + tested):
the registry never reads task state; `task` rows carry only reference
metadata. A task whose title/description mentions `/victim` changes
nothing — resolution of `/victim` is still refused.

## 23. Skill Scope

Skills cannot expand filesystem authorization (structural): skill
lifecycle operates on store rows and published artifacts; the skill
subsystem has no reference to the registry's authorized set.
`--allow-root` accepts only operator CLI/env input, never skill
metadata.

## 24. Memory / History

Historical references cannot grant authorization (structural +
tested): memory values, context records, recall excerpts, and
engineering-memory entries are data resolved through per-workspace
stores; they never flow into registry construction. "Repository X was
previously indexed" implies nothing about current authorization.

## 25. Learning

Learning can never grant authorization: accepted hypotheses persist
as `AI_INFERRED` records with unchanged gates; "project probably
lives at /some/path" is text in a store, and the gate accepts only
operator launch configuration.

## 26. Engineering Brief

Briefs compose exclusively from the resolved (hence authorized)
workspace's stores: facts, freshness, identity, impact, health,
history excerpts, memory, learning, skills, task state. Unauthorized
repository information cannot enter any section because resolution
fails before assembly begins (probe 1 includes `engineering_brief`
against a seeded victim; probe 10 asserts A's markers absent from
B's brief while B stays servable).

## 27. Path Moves

Authorization binds canonical paths, not inode identities, and does
not follow moves: `/tmp/project` authorized, then moved elsewhere —
requests for the old path fail (no longer exists / no longer
resolves), requests for the new path are refused unless the operator
authorizes them. Restart with the corrected root re-authorizes. No
silent expansion (covered by canonical-exact semantics; traversal
probe asserts the failure classes).

## 28. Directory Replacement

Authorized directory removed and replaced by a symlink to a victim:
the next resolve canonicalizes through the new topology to the victim
target, which is not authorized → refused. Tested live (root replaced
by `symlink → victim` mid-session → denial; victim untouched). The
ChangeEngine's independent prepare+apply re-validation remains as
defense in depth for the mutation path.

## 29. Time-of-Check / Time-of-Use

Authorization is checked per call at `resolve()` against live
canonicalization, immediately before the workspace handle is used;
mutations additionally re-validate paths at prepare AND apply time
inside the ChangeEngine (unchanged P0–P7 guarantee, with its
stale-snapshot refusal). No perfect race-free guarantee against a
same-user local attacker racing the filesystem is claimed — such an
attacker already equals the operator in the local single-user threat
model, and the audit classified F3 MEDIUM on exactly that basis. What
is guaranteed: no single tool call can be *directed* at an
unauthorized root; every seam that touches the filesystem checks
first. This limitation is explicit and does not invalidate local
CodeBro/OpenCode security.

## 30. Concurrency

Authorization decisions are pure reads over an immutable set behind
the existing registry `RwLock`: concurrent authorized +
unauthorized resolutions are deterministic (unit test: 8 threads
mixed, all authorized succeed, all unauthorized fail; in-crate
`concurrent_access_does_not_leak` and two-client real-binary probes
green). No new locks, no new ordering hazards; the per-workspace
mutation lock semantics are unchanged.

## 31. Redaction

The P8 F1 redacting-stderr writer is untouched and re-verified: the
secret-shaped-root probe (probe 9) hunts captured stderr for the raw
secret — zero occurrences. MCP error responses echo only the caller's
own input on the same channel (documented F6 caller-echo convention),
while stderr carries the redacted form. All P8 secret seam tests
remain green.

## 32. Stdout Purity

Every authorization refusal travels as JSON-RPC (`-32602` tool
protocol error), never as a bare print. The real-binary probes reuse
the P8 strict stdout validator (any non-JSON-RPC line fails the
probe) across all refusal shapes, including hostile roots, huge
inputs, and file-instead-of-directory roots. No new print paths were
added (`codebro serve` reachable paths re-grepped: zero `println!`).

## 33. Determinism

Same authorization state + same request → byte-identical refusal
(unit-pinned: repeated unauthorized resolves return equal strings;
real-binary probe 1 re-asserts equality across repeated calls;
restart re-verified). Allowed requests behave as before
(deterministic briefs/task flows pinned by the unchanged suites).

## 34. Boundedness

Refusal happens at canonicalize + set-membership — O(path), no
directory walks, no index loads, no enumeration — before any
expensive work. A 1 MB `workspace_root` string, deep traversal
shapes, and NUL-adjacent inputs all fail fast with bounded errors
and a still-usable server (probes 9 + gate malformed-input probes).
Response envelope (256 KiB) and per-section caps unchanged.

## 35. Test Quality

All 24 new tests are hermetic (`tempfile::tempdir()` everywhere,
explicit `CODEBRO_STATE_DIR`/`CODEBRO_SKILLS_DIR` in every real-binary
probe), deterministic (no wall-clock assertions; byte-equality on
refusals), independent of `~/.codebro` (md5-verified byte-identical
before/after the full suite), independent of user repositories and
real skills, and real-binary where the failure class is protocol- or
process-level. The only host-path probe (`/etc`, `/root`, `/var/log`)
is a safe refusal check (asserts denial; never reads). Existing
isolation tests were adapted, not weakened: they now authorize both
roots explicitly (mirroring an operator launch) and assert the same
zero-leakage properties — plus the registry *additionally* refuses
the third, unauthorized root.

## 36. Documentation

- `AGENTS.md` — F3 debt paragraph replaced with the closed
  authorization model (root authorization bullet).
- `docs/MCP_API_V1.md` — root authorization added to the contract
  bullets + `invalid_params` row for unauthorized roots.
- `docs/evolution/P8_POST_IMPLEMENTATION_AUDIT.md` — F3 marked
  **CLOSED** (table, counts, §51 debt, §13 traversal probe, §21
  intro).
- `docs/evolution/P8_IMPLEMENTATION.md` — §29 debt item 5 marked
  resolved with pointer to this report.
- `CHANGELOG.md` — F3 closure entry under `[Unreleased]`.
- `crates/mcp-server/src/workspace_registry.rs` + `workspace.rs` —
  module headers now state the authorization invariant, the
  authorization-vs-discovery split, and the TOCTOU note.

## 37. Remaining Limitations

1. **Versioning note:** `MCP_API_V1.md` classifies narrowing accepted
   value domains as major/breaking. This closure narrows
   `workspace_root?` from "any existing directory" to
   "operator-authorized roots only" — treated as a security fix to a
   documented defect (same class as F1/F2, which also narrowed
   accepted behavior without a bump), not a contract feature removal:
   no legitimate client workflow (documented acquisition flow, OpenCode
   E2E legs, all real-binary harnesses) ever depended on serving
   arbitrary host directories. No migration path is required beyond
   "pass the repo root (or omit it); operators needing multi-root add
   `--allow-root`".
2. **Same-user TOCTOU** (§29): no race-free guarantee against an
   attacker with equal filesystem privileges racing a call; the
   guarantee is per-call authorization plus ChangeEngine
   prepare+apply re-validation.
3. **Mount/bind-mount topology** below an authorized root is trusted
   operator territory (path-canonical semantics, no mount-id
   tracking) — same as every path-based sandbox.
4. **Carried debts unchanged:** P5 lease-TTL hard-kill latency, F4
   peer_info INFO logging, F6 same-channel caller echo, P7 harness-only
   stdout validator, and the twelve P0–P7 gate debts.

None compromises safe OpenCode integration in the documented
single-user local deployment model.

## 38. Final Verdict

**SECURITY BOUNDARY CLOSED.**

- No unauthorized filesystem access (refused at resolve, zero side
  effects, victim dirs byte-identical).
- No symlink escape (canonical-target governance, both directions
  probed).
- No traversal escape (canonicalize-before-authorize).
- No cross-workspace access beyond explicit operator authorization
  (matrix probed live; store isolation re-pinned).
- `apply_change` protected (gate + ChangeEngine boundary).
- `reindex` protected (gate before the index pipeline).
- Inspection protected (gate before policy; F2 confinement re-pinned).
- MCP protected (all 25 tools share the single `resolve_workspace`
  choke point; no tool bypass).
- No secret leakage (F1 writer intact; refusal text host-minimal).
- Stdout purity maintained (strict validator on all refusal paths).
- Deterministic authorization (byte-equal refusals; restart-stable).
- Restart + hard-kill correct (config-derived, never broadened).
- Concurrency correct (immutable set, lock-free reads).
- Real binary verification (10 probes).
- Real OpenCode E2E verification (authorized flow, unauthorized
  refusal, allowlist flow).
- P0–P8 regression PASS (1426/1426; clippy/fmt/deps clean;
  `~/.codebro` byte-identical).

P9: NOT STARTED.
