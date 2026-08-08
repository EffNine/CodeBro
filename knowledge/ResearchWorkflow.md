# Research Workflow

**Phase**: P10.3A — Provider Research Platform
**Status**: APPROVED

Procedure for researching a provider, building its knowledge set, and moving it
through the certification lifecycle. Implements the Policy order:
**Read → Benchmark → Optimize → Certify**.

## 0. Workflow At-a-Glance

```
 COPY            READ            VERIFY            SHOW             STOP/DO
 _template  → (sources)   → (facts)         → (index)      wait-for-gate
   │              │
   │              ▼
   │     Provider Card (facts wp.sources)
   │     Capability Matrix
   │     Optimization Profile (statuses only)
   ▼
   certification.md=Draft
```

## 1. Onboard (COPY)

1. Copy `providers/_template/` → `providers/<name>/` (7 files).
2. Replace `{...}` placeholders. Do not rename/restructure files.
3. Set `certification.md` status to `Draft`. Create changelog record.

## 2. Read (Research)

1. Gather ALL official sources (docs/API ref/SDK/release/tech report/prompt guide).
2. Fill `provider_card.md`: identity, models, and ONLY sourced facts.
3. Fill `capability_matrix.md`: the 13 capability rows, each with status + source;
   use "Not documented" where no official statement exists.
4. Fill `optimization_profile.yaml` with `value/confidence/source/status`.
   Leave unproven claims as `Hypothesis` / `Benchmark Required`.
5. Fill `engineering_notes.md` with open questions, observations, caveats.
6. Stamp `Last verified`, bump artifact versions, log in `changelog.md`.

## 3. Verify (Research Complete)

1. Audit every fact has a source; any unsourced fact is removed or LOW confidence.
2. Confirm no benchmark ran ($Research Rule).
3. Set `certification.md` → **Research Complete**. Log gate.

Output: Research Complete (terminal for P10.3A).

## 4. Benchmark (Next phase, only after approval)

1. Researcher proposes `benchmark_spec.md` from research.
2. Gate: policy + source complete → approved spec → **Benchmark Ready**.
3. Benchmark Ownerfiles test matrix, executes on `benchmarks/` + Commit CI.
4. Record results in the Research Index. Optimizations that pass move to adopted.

## 5. Optimize (Next phase, only after Benchmark)

1. Update `optimization_profile.yaml`: flip `Benchmark Required` →
   `Documentation` for values a benchmark confirmed.
2. Adopt only confirmed values into provider plugin config.

## 6. Certify (Next, only after Optimize)

1. Set `certification.md` → **Certified**, dated, with reason.
2. Changelog row. Update Research Index.

## Gate Summary

| Gate | Entry condition | Exit action |
|------|-----------------|-------------|
| Draft | template copied | research artifacts populated |
| Research Complete | sourced card/matrix/profile | stop; submit to reviewer |
| Benchmark Ready | approved spec | run benchmarks |
| Benchmark | run passes criteria | record + bump status |
| Optimize | benchmark pass | adopt values / set status |
| Certified | optimize done | record + changelog |

Every gate transition requires Chief Architect review. The workflow never
skips: research must precede benchmark; benchmark must precede optimize;
optimize must precede certify.