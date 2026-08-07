# Regression Registry

Track all regressions detected during development and after release.

## Naming Convention

```
docs/reports/regressions/REG-XXX-short-description.md
```

## Registry

| ID | Phase | Severity | Category | Status | Fixed In |
|----|-------|----------|----------|--------|----------|
| — | — | — | — | — | — |

## Categories

| Category | Definition |
|----------|------------|
| **Functional** | Feature no longer produces correct output |
| **Performance** | KPI degrades beyond acceptable threshold |
| **Visual** | TUI rendering is broken or degraded |
| **Data** | Persisted data is corrupted or lost |
| **Compatibility** | Existing config or data no longer works |
| **Security** | Previously-secure behavior becomes insecure |

## Severity Definitions

| Severity | Response Time | Fix Deadline |
|----------|--------------|--------------|
| **P0** | Immediate | Current cycle |
| **P1** | Within 24h | Current cycle |
| **P2** | Within 1 week | Next release |
| **P3** | Best effort | Backlog |
