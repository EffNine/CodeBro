# RFC Template

**Document:** `docs/RFC/template.md`
**Version:** 1.0.0
**Part of:** CodeBro SOP v1.0

---

## RFC Metadata

| Field | Value |
|-------|-------|
| **RFC Number** | RFC-XXX (assigned on acceptance) |
| **Title** | <Short descriptive title> |
| **Author** | <Name> |
| **Status** | Draft / In Review / Accepted / Rejected / Superseded |
| **Created** | YYYY-MM-DD |
| **Updated** | YYYY-MM-DD |
| **Supersedes** | RFC-XXX (if applicable) |
| **Related ADR** | ADR-XXX (if applicable) |

---

## 1. Summary

<One paragraph summarizing the proposed change and its rationale. This should be understandable by someone who has not read the rest of the document.>

---

## 2. Motivation

<Why is this change needed? What problem does it solve? What is the cost of not making this change?>

### 2.1 Problem Statement

<Clear statement of the problem>

### 2.2 Goals

<What this change will achieve>

- [ ] <Goal 1>
- [ ] <Goal 2>

### 2.3 Non-Goals

<What this change will NOT achieve>

- [ ] <Non-goal 1>
- [ ] <Non-goal 2>

---

## 3. Proposed Change

<Detailed description of the proposed change. Include:>

### 3.1 User-Facing Behavior

<How will users interact with this change? What will they see/different?>

### 3.2 Technical Approach

<What is the technical solution? Include:>
- Architecture diagrams (ASCII or referenced)
- Data flow descriptions
- Module interactions
- Configuration changes

### 3.3 Changes to Existing Systems

<Which existing modules, traits, or types are affected?>

| Module | Change Type | Description |
|--------|------------|-------------|
| `<module>` | Added/Modified/Removed | <what changes> |

### 3.4 New Dependencies

<Are any new crates or system tools needed?>

| Dependency | Purpose | Version | Justification |
|------------|---------|---------|---------------|
| `<crate>` | <why needed> | `<version>` | <why existing deps don't suffice> |

---

## 4. Alternatives Considered

<What other approaches were considered and why were they rejected?>

| Alternative | Pros | Cons | Reason Rejected |
|-------------|------|------|-----------------|
| <Alternative 1> | ... | ... | ... |
| <Alternative 2> | ... | ... | ... |

---

## 5. Implementation Plan

<How will this be implemented? Include phases, milestones, and estimates.>

### 5.1 Phases

| Phase | Description | Estimated Effort | Dependencies |
|-------|-------------|-----------------|--------------|
| P1 | <phase 1> | <X days> | None |
| P2 | <phase 2> | <X days> | P1 |
| P3 | <phase 3> | <X days> | P2 |

### 5.2 Milestones

| Milestone | Description | Acceptance Criteria |
|-----------|-------------|---------------------|
| M1 | <milestone 1> | <criteria> |
| M2 | <milestone 2> | <criteria> |

### 5.3 Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| <risk 1> | <L/M/H> | <L/M/H> | <mitigation> |
| <risk 2> | <L/M/H> | <L/M/H> | <mitigation> |

---

## 6. Validation Plan

<How will success be measured?>

### 6.1 Unit Tests

<What unit tests are needed?>

### 6.2 Integration Tests

<What integration tests are needed?>

### 6.3 Manual Tests

<What manual validation is needed?>

### 6.4 Benchmark Requirements

<What KPIs must be measured?>

| KPI | Baseline | Target | Method |
|-----|----------|--------|--------|
| <kpi> | <value> | <target> | <method> |

---

## 7. Impact Analysis

<What is the impact of this change?>

### 7.1 Affected Modules

| Module | Impact | Risk |
|--------|--------|------|
| <module> | <description> | <L/M/H> |

### 7.2 Configuration Impact

<Does this change require config file updates?>

### 7.3 Data Format Impact

<Does this change stored data formats (sessions, memory, config)?>

### 7.4 Migration Path

<How do existing users/data migrate?>

---

## 8. Open Questions

<List any unresolved questions>

- [ ] <Question 1>
- [ ] <Question 2>

---

## 9. Decision

| Option | Votes For | Votes Against | Notes |
|--------|-----------|---------------|-------|
| Accept | <n> | <n> | |
| Reject | <n> | <n> | |
| Revise & Resubmit | <n> | <n> | |

**Decision:** <Accepted / Rejected / Revise & Resubmit>
**Date:** YYYY-MM-DD
**Reviewed by:** <names>

---

## 10. References

- [SOP v1.0](../SOP/codebro_sop_v1.md)
- [Development Protocol](../SOP/development_protocol.md)
- [Validation Protocol](../SOP/validation_protocol.md)
- [Benchmark Protocol](../SOP/benchmark_protocol.md)
