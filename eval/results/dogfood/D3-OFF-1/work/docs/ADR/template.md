# ADR Template

**Document:** `docs/ADR/template.md`
**Version:** 1.0.0
**Part of:** CodeBro SOP v1.0

---

## ADR Metadata

| Field | Value |
|-------|-------|
| **ADR Number** | ADR-XXX (assigned on acceptance) |
| **Title** | <Short descriptive title of the decision> |
| **Author** | <Name> |
| **Status** | Proposed / Accepted / Deprecated / Superseded |
| **Created** | YYYY-MM-DD |
| **Updated** | YYYY-MM-DD |
| **Supersedes** | ADR-XXX (if applicable) |
| **Related RFC** | RFC-XXX (if applicable) |

---

## 1. Context

<What is the issue or opportunity that this decision addresses? Provide sufficient background for someone to understand the decision without reading external documents.>

### 1.1 Background

<Relevant history, previous attempts, existing constraints>

### 1.2 Constraints

<What constraints must this decision respect?>
- <constraint 1>
- <constraint 2>

### 1.3 Stakeholders

<Who is affected by this decision?>
- <stakeholder 1>: <impact>
- <stakeholder 2>: <impact>

---

## 2. Decision

<What is the decision that has been made?>

### 2.1 Decision Statement

<One or two sentences stating the decision clearly>

### 2.2 Rationale

<Why was this decision made? What factors were considered?>

### 2.3 Principles Applied

<Which architectural principles guided this decision?>
- <principle 1>
- <principle 2>

---

## 3. Consequences

<What are the consequences of this decision?>

### 3.1 Positive Consequences

- <positive 1>
- <positive 2>

### 3.2 Negative Consequences

- <negative 1>
- <negative 2>

### 3.3 Trade-offs

| Aspect | Trade-off | Mitigation |
|--------|-----------|------------|
| <aspect> | <trade-off> | <mitigation> |

### 3.4 Impact on Architecture

<How does this decision affect the overall architecture?>

| Module | Impact |
|--------|--------|
| <module> | <impact description> |

### 3.5 Impact on Future Work

<What future work does this enable or constrain?>

---

## 4. Alternatives Considered

<What other decisions were considered?>

| Alternative | Description | Pros | Cons | Why Rejected |
|-------------|-------------|------|------|--------------|
| A | <description> | ... | ... | ... |
| B | <description> | ... | ... | ... |
| C | <description> | ... | ... | ... |

---

## 5. Implementation Notes

<Practical guidance for implementing this decision>

### 5.1 Code Patterns

<What code patterns should be followed?>

```rust
// Example of the recommended pattern
pub struct Example {
    // ...
}
```

### 5.2 Anti-Patterns

<What patterns should be avoided?>

```rust
// Example of what NOT to do
pub struct BadExample {
    // ...
}
```

### 5.3 Migration Steps

<If this decision changes existing behavior, how should migration be done?>

1. <step 1>
2. <step 2>
3. <step 3>

---

## 6. References

- [SOP v1.0](../SOP/codebro_sop_v1.md)
- [Development Protocol](../SOP/development_protocol.md)
- [RFC-XXX](../../RFC/rfc-xxx.md)

---

## 7. History

| Date | Change | Author |
|------|--------|--------|
| YYYY-MM-DD | Created | <name> |
| YYYY-MM-DD | <change> | <name> |
