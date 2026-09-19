# Provider Research Architecture

**Phase**: P10.3A — Provider Research Platform
**Status**: APPROVED TO IMPLEMENT → IMPLEMENTED (Research-Only)

## 1. Mission

Create the official research and certification framework for AI providers.
This phase does NOT implement any provider. It builds the engineering
knowledge system used before any provider becomes CodeBro Certified.

**Knowledge is a first-class artifact.**

## 2. Ownership Contract

### The Research Platform OWNS

- Provider Cards
- Capability Matrix
- Optimization Profiles
- Certification Reports
- Benchmark Specifications
- Research Index
- Version Tracking
- Engineering Notes

### The Research Platform MUST NOT own

- Runtime logic
- Provider implementations
- API clients
- Benchmark execution
- Plugin code
- Production configuration

## 3. Directory Layout

```
knowledge/
  ProviderResearchArchitecture.md   (this file)
  ProviderResearchPolicy.md
  ResearchWorkflow.md
  DeepSeekProviderCard.md           (deliverable compile)
  CapabilityMatrix.md               (deliverable compile)
  OptimizationProfile.md            (deliverable compile)
  ImplementationReport.md
  providers/
    _template/
      provider_card.md
      capability_matrix.md
      optimization_profile.yaml
      certification.md
      benchmark_spec.md
      engineering_notes.md
      changelog.md
    deepseek/
      provider_card.md
      capability_matrix.md
      optimization_profile.yaml
      certification.md
      benchmark_spec.md
      engineering_notes.md
      changelog.md
```

`_template/` is the canonical contract. Onboarding a provider = copy
`_template/` → `providers/<name>/` and replace placeholders. New files may be
added per-provider (in `providers/<name>/`) but the seven core artifacts are
mandatory and must keep their names and structure.

## 4. Research Index

The Research Index is a registry of providers under research and their
certification status. Canonical form:

```
knowledge/index.md
```

(index file created in the next phase; status is today derivable from each
`providers/*/certification.md`).

Index contract:

| Provider | Provider Card | Capability Matrix | Optimization Profile | Certification | Benchmark Spec | Status |
|----------|---------------|-------------------|----------------------|---------------|----------------|--------|
| deepseek | v1.0.0 | ✓ | ✓ | Research Complete | Draft | research-complete |

Rules:
- Every research artifact is versioned and date-stamped.
- Any fact change bumps the artifact version and adds a `changelog.md` row.
- Certification status is the ONLY field that grants operational meaning.

## 5. Artifact Contracts

| Artifact | Format | Mandatory fields | Purpose |
|----------|--------|------------------|---------|
| Provider Card | Markdown | identity, models, sourced facts, sources | What the provider is, factually |
| Capability Matrix | Markdown | 13 capability rows + status + source | What the provider can do |
| Optimization Profile | YAML | value/confidence/source/status per entry | What MAY be exploited after proof |
| Certification | Markdown | status table + gates | Is it CodeBro Certified? |
| Benchmark Spec | Markdown | purpose, test matrix, env, cost ceiling | What a benchmark must prove |
| Engineering Notes | Markdown | observations/hypotheses/caveats | Informal working knowledge |
| Changelog | Markdown | dated versioned rows | Version tracking |

Optimization entry vocabulary (MUST be used verbatim):

```
value      : the optimization statement
confidence : HIGH | MEDIUM | LOW
source     : official reference
status     : Documentation | Hypothesis | Benchmark Required
```

Certification vocabulary:

```
Draft → Research Complete → Benchmark Ready → Certified → Deprecated
```

## 6. Research-Only Boundary

This architecture ships with ZERO runtime behavior:

- no runtime logic, no provider implementation, no API clients
- no benchmark execution, no plugin code, no production config
- no Rust/Go/JS source changes; the only artifacts are the documents above

Acceptance criterion: "Research reproducible" — every fact in every card
traces to an official source recorded in the artifact.

## 7. Non-Goals (explicitly excluded)

- Implementing DeepSeek (or any) provider plugin
- Executing any benchmark
- Publishing optimization values into runtime
- Operating the research platform at runtime (that is Workspace Runtime, next phase)