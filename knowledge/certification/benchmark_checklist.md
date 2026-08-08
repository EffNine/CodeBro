# Benchmark Checklist

**Framework**: Benchmark & Certification · Gate checklist used by Chief Architect before a benchmark is allowed to run, and before Certification.

## A. Pre-Benchmark (Research Complete → Benchmark Ready)

- [ ] Provider Card sources verified (Research gate)
- [ ] Capability Matrix source-complete
- [ ] Optimization Profile annotated; no unproven meets "adopted"
- [ ] Benchmark Spec approved; every section filled (Purpose/Inputs/Expected/Success/Failure/Metrics/Repeatability/Replay)
- [ ] Dataset `id@version` pinned, not provider-specific
- [ ] Seed set
- [ ] Budget (tokens/cost/repeats/concurrency) declared
- [ ] Replay id assigned
- [ ] Controls: no credentials, no provider run deviation

## B. Run Gate (Benchmark Ready → Certified)

- [ ] Spec → run config serialized
- [ ] run valid (seed honored, budget respected, controls)
- [ ] All mandatory thresholds met
- [ ] No failure criteria tripped
- [ ] Result conforms to Result Schema (`report_template.md`)
- [ ] Replay golden recorded (hashed)
- [ ] Certification report written; changelog row added

## C. Certification

- [ ] OpenAI/Anthropic compliance (framework untouched — generic) confirmed
- [ ] Verdict PASS → `Certified`; else FAIL with reason
- [ ] (optional) `Certified Optimized` only with benchmark-confirmed values
- [ ] (optional) Reference Model designation

## D. Replay Checklist (any later time)

- [ ] Offline only — zero tokens, zero HTTP, zero keys
- [ ] Compares verdict + metric drift ≤ tolerance
- [ ] No new failure-criteria triggers
- [ ] Report updated with replay result

## Failure Rules

If anything in A is left FILLED but FALSE at B, the run is auto-invalid. Replay
with a drifted sign may freeze or flush a previously certified level.