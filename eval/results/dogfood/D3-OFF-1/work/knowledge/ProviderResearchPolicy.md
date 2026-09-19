# Provider Research Policy

**Phase**: P10.3A — Provider Research Platform
**Status**: APPROVED

## 1. Purpose

Govern how providers are researched, benchmarked, optimized, and certified so
that CodeBro only ever interacts with providers whose capabilities have been
demonstrated, costed, and verified against primary sources.

## 2. Research Sourcing

Research is performed ONLY from:

- Official Documentation
- Official API Reference
- Official SDK
- Official Release Notes
- Official Technical Reports
- Official Prompt Guide

Sources must be cited in the artifact's Sources table. Secondary sources are
NOT acceptable for certification facts.

## 3. The Policy: Read → Benchmark → Optimize → Certify

The sequence is mandatory and non-skippable:

1. **Read before Benchmark.** Research the provider. Produce a sourced
   Provider Card + Capability Matrix. No benchmark may start before Research
   Complete.
2. **Benchmark before Optimize.** A benchmark spec must be approved and run
   before an optimization is treated as proven. Optimizations without
   benchmarks are `Hypothesis` or `Benchmark Required`, never adopted.
3. **Optimize before Certify.** Only after optimization values pass their
   benchmark may the provider be marked `Certified`.

## 4. Optimization Discipline

Every optimization entry carries:

| Field | Allowed values |
|-------|----------------|
| value | statement |
| confidence | HIGH · MEDIUM · LOW |
| source | official reference |
| status | Documentation · Hypothesis · Benchmark Required |

- `Documentation` → directly stated by an official source (usually HIGH).
- `Hypothesis` → inferred/unverified (usually LOW/MEDIUM).
- `Benchmark Required` → proven only by an actual benchmark; not adopted until
  the benchmark runs.
- A `Benchmark Required` optimization MUST NOT enter runtime config before
  its benchmark passes.

## 5. Certification States

```
Draft → Research Complete → Benchmark Ready → Certified → Deprecated
```

Transition rules:

| Transition | Requires |
|------------|----------|
| Draft → Research Complete | sourced card + capability matrix + annotated optimization profile |
| Research Complete → Benchmark Ready | approved benchmark spec + Chief Architect approval |
| Benchmark Ready → Certified | benchmark executed + pass all pass criteria + optimization adopted |
| Certified → Deprecated | single source of truth: `certification.md` state change + changelog row |

## 6. Policy Boundaries

- Research → fill `providers/*/{provider_card,capability_matrix,optimization_profile}.{md,yaml}`.
- Certification → set a state in `providers/*/certification.md` and log it.
- The Research Platform does NOT run user workloads; it only describes.

## 7. Enforcement

- CI/docs lint checks that a certification change also updates the changelog.
- No runtime change may reference an un-certified optimization.
- Every fact reference to a non-official source is a policy violation.

## 8. Roles

- **Researcher** — populates knowledge for a provider.
- **Chief Architect** — approves Research Complete and every gate transition.
- **Benchmark Owner** — the only role that may execute benchmarks (future).

## 9. Exceptions & Amendments

Any exception to this policy must be an approved ADR. Do NOT amend this policy
ad hoc.