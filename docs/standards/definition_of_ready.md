# Definition of Ready

**Document:** `docs/standards/definition_of_ready.md`
**Version:** 1.0.0
**Part of:** CodeBro SOP v1.0

---

## 1. Purpose

The Definition of Ready (DoR) is a checklist that must be satisfied before any implementation work begins. It exists to prevent half-formed features from entering development and to ensure that every phase has clear, testable requirements.

**A feature may not begin until ALL items below are satisfied.**

---

## 2. Requirements

### 2.1 User-Facing Requirements

- [ ] **User story is written:** "As a <role>, I want <behavior>, so that <benefit>"
- [ ] **Acceptance criteria are defined:** Specific, testable conditions that prove the feature works
- [ ] **Edge cases are identified:** At least 3 edge cases are documented
- [ ] **Error paths are defined:** What happens when the feature fails?

### 2.2 Technical Requirements

- [ ] **Affected modules are identified:** Which `src/` modules will change?
- [ ] **New modules are justified:** If a new module is needed, the RFC describes why existing modules cannot handle it
- [ ] **Data flow is mapped:** How does data enter, transform, and exit the feature?
- [ ] **Configuration impact is assessed:** Does the feature require new config options?

### 2.3 Governance Requirements

- [ ] **RFC is approved (if required):** See [SOP](../SOP/codebro_sop_v1.md) Section 3 for when an RFC is required
- [ ] **ADR is approved (if required):** See [SOP](../SOP/codebro_sop_v1.md) Section 4 for when an ADR is required
- [ ] **Phase is defined:** The feature is scoped to a specific phase in the roadmap
- [ ] **Entry criteria are satisfied:** See [Development Protocol](../SOP/development_protocol.md) Section 3.4

### 2.4 Measurement Requirements

- [ ] **Baseline KPIs are recorded:** Current performance of the affected area is measured
- [ ] **Target KPIs are defined:** Quantitative targets for the feature are specified
- [ ] **Benchmark method is chosen:** How will KPIs be measured? (automated, manual, or both)
- [ ] **Success threshold is defined:** What KPI value constitutes "pass"?

### 2.5 Risk Requirements

- [ ] **Risks are identified:** At least the top 3 risks are documented
- [ ] **Mitigations are planned:** Each risk has at least one mitigation strategy
- [ ] **Fallback is defined:** If the feature fails, what is the rollback plan?

---

## 3. When an RFC Is Required

An RFC is required for any change that:

1. Introduces a new module or top-level directory under `src/`
2. Changes the signature of a public trait (`Provider`, `Tool`, `SubAgent`)
3. Adds a new dependency to `Cargo.toml`
4. Changes the event flow between major subsystems (`tui/` ↔ `agent/` ↔ `tools/`)
5. Affects more than 3 existing modules
6. Changes user-visible behavior (TUI layout, CLI commands, configuration)
7. Modifies the memory or session JSON schema

An RFC is **not** required for:

- Bug fixes that restore intended behavior
- Documentation updates
- Test additions or improvements
- Backward-compatible dependency bumps
- Single-module refactors with no behavior change

---

## 4. When an ADR Is Required

An ADR is required for any decision that:

1. Chooses between two or more technical approaches
2. Defines a new pattern or convention for the codebase
3. Sets a threshold, limit, or constant that future work will depend on
4. Modifies an existing architectural constraint from the Architecture Manifest
5. Changes how modules communicate (new event types, new channels)

---

## 5. Ready Checklist Template

Use this template when creating a phase or feature proposal:

```markdown
## Definition of Ready — <Feature/Phase Name>

### Requirements
- [ ] User story: <story>
- [ ] Acceptance criteria: <criteria>
- [ ] Edge cases: <list>
- [ ] Error paths: <list>

### Technical
- [ ] Affected modules: <list>
- [ ] New modules: <yes/no + justification>
- [ ] Data flow: <description>
- [ ] Config impact: <description>

### Governance
- [ ] RFC: <approved/rejected/not-required>
- [ ] ADR: <approved/rejected/not-required>
- [ ] Phase: P<N>
- [ ] Entry criteria: <satisfied/not-satisfied>

### Measurement
- [ ] Baseline KPIs: <values>
- [ ] Target KPIs: <values>
- [ ] Benchmark method: <method>
- [ ] Success threshold: <threshold>

### Risk
- [ ] Top risks: <list>
- [ ] Mitigations: <list>
- [ ] Fallback: <description>
```

---

## 6. References

- [SOP v1.0](../SOP/codebro_sop_v1.md)
- [Development Protocol](../SOP/development_protocol.md)
- [RFC Template](../RFC/template.md)
- [ADR Template](../ADR/template.md)
