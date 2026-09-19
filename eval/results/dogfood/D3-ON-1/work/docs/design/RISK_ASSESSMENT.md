# Risk Assessment — Adaptive Developer Platform

**Document:** `docs/design/RISK_ASSESSMENT.md`
**Version:** 1.0.0
**Phase:** P6 — Design Summit
**Status:** Proposed
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Risk Overview

This document assesses risks associated with the Adaptive Developer Platform design. Risks are categorized by type and severity.

---

## 2. Risk Register

### 2.1 High Risks

| ID | Risk | Impact | Likelihood | Mitigation |
|----|------|--------|------------|------------|
| R001 | Intent Engine misparses natural language, causing unwanted preference changes | High | Medium | All intents require approval; low-confidence intents are suppressed |
| R002 | Model routing silently switches to a more expensive model | High | Low | Cost Policy intercepts all model changes; approval required before switch |
| R003 | MCP server installation introduces security vulnerability | High | Low | Sandbox validation; blocked patterns; no sudo/privileged access allowed |
| R004 | Cost tracking inaccuracy leads to budget overruns | High | Medium | Conservative estimates; unknown models treated as free; hard limit enforcement |
| R005 | Preference schema migration breaks existing user data | Medium | Low | Version field in all JSON files; migration scripts tested before deployment |

### 2.2 Medium Risks

| ID | Risk | Impact | Likelihood | Mitigation |
|----|------|--------|------------|------------|
| R006 | Notification fatigue from excessive recommendations | Medium | Medium | Deduplication; cooldown periods; silent mode option |
| R007 | Trust score manipulation (intentional or accidental) | Medium | Low | Deterministic formula; no external input to scoring |
| R008 | Workflow pattern false positives (suggesting non-repeated patterns) | Medium | Medium | Minimum 3 occurrences required; cooldown between suggestions |
| R009 | Profile switch overrides useful preferences | Medium | Low | Merge semantics; diff view before applying profile |
| R010 | Pricing table drift from actual provider prices | Medium | High | Periodic refresh mechanism; user override capability |
| R011 | Skill installation from untrusted sources | Medium | Low | Trust levels; sandbox validation; community rating |

### 2.3 Low Risks

| ID | Risk | Impact | Likelihood | Mitigation |
|----|------|--------|------------|------------|
| R012 | Performance impact of adaptive subsystems on TUI | Low | Low | Async event processing; bounded history sizes |
| R013 | Disk space growth from audit logs | Low | Medium | Log rotation; max entries per log |
| R014 | Edge cases in cost estimation for new models | Low | Medium | Conservative defaults; user override |
| R015 | Registry format incompatibility across versions | Low | Low | Version field; forward-compatible parsing |

---

## 3. Risk Scoring Matrix

```
                    Likelihood
              Low       Medium      High
           ┌────────┬──────────┬──────────┐
    High   │  R002  │   R001   │   R004   │
           │  M     │    H     │    H     │
           ├────────┼──────────┼──────────┤
   Medium  │  R012  │   R006   │   R007   │
           │  L     │    M     │    M     │
           ├────────┼──────────┼──────────┤
     Low   │  R014  │   R003   │   R008   │
           │  L     │    H     │    M     │
           └────────┴──────────┴──────────┘
```

---

## 4. Detailed Risk Analysis

### 4.1 R001: Intent Engine Misinterpretation

**Scenario:** User says "I prefer minimal code" and the engine interprets it as "disable all tests and comments" when the user meant "shorter function bodies."

**Impact:** User's preferences are changed in unintended ways.

**Mitigation:**
- Confidence threshold of 0.5 prevents low-confidence intents from proceeding
- All intent-driven changes require explicit approval
- Audit trail allows reversal of incorrect changes
- Clear reasoning is shown before approval

**Residual Risk:** Low — user has explicit control and can reverse changes.

### 4.2 R002: Silent Model Upgrade

**Scenario:** Model routing automatically selects a more expensive model without user knowledge, increasing costs.

**Impact:** Unexpected cost increase; potential budget violation.

**Mitigation:**
- Cost Policy evaluates all model changes
- Any model with higher cost requires approval
- Title bar shows current cost status
- Daily/ session limits are enforced

**Residual Risk:** Low — cost compliance check is a hard gate.

### 4.3 R003: MCP Security Vulnerability

**Scenario:** A malicious MCP server is discovered and installed, accessing sensitive files or making unauthorized network calls.

**Impact:** Data breach; system compromise.

**Mitigation:**
- Discovery requires approval
- Unverified servers run in sandbox before installation
- Forbidden patterns are automatically blocked
- No servers allowed to access sensitive paths
- Trust levels control capabilities

**Residual Risk:** Medium — sandbox may not catch all attack vectors; continuous monitoring required.

### 4.4 R004: Cost Tracking Inaccuracy

**Scenario:** Cost estimates are significantly lower than actual charges, leading to budget overruns.

**Impact:** User exceeds budget without warning.

**Mitigation:**
- Unknown models are treated as free (conservative)
- Estimates are over-estimates, not under-estimates
- Hard limit enforcement prevents execution past limit
- Regular cost reconciliation with actual charges

**Residual Risk:** Medium — estimates may still be inaccurate; hard limits provide final safeguard.

### 4.5 R006: Notification Fatigue

**Scenario:** Too many recommendations overwhelm the user, causing them to dismiss or ignore important ones.

**Impact:** Important recommendations are missed; user experience degrades.

**Mitigation:**
- Deduplication suppresses repeated recommendations
- Cooldown periods prevent rapid successive recommendations
- Silent mode available for power users
- Priority ranking surfaces most important recommendations

**Residual Risk:** Low — user has control over notification frequency.

---

## 5. Risk Acceptance

The following risks are accepted with their current mitigations:

| Risk | Acceptance Criteria | Acceptance Date |
|------|---------------------|-----------------|
| R001 | Approval gate prevents unwanted changes | 2026-08-06 |
| R002 | Cost Policy hard gate prevents budget violation | 2026-08-06 |
| R003 | Sandbox validation + forbidden patterns | 2026-08-06 |
| R004 | Conservative estimates + hard limits | 2026-08-06 |
| R006 | Deduplication + silent mode | 2026-08-06 |

---

## 6. Risk Monitoring

Risks should be monitored during P6 implementation and validation:

| Risk | Monitoring Method | Frequency |
|------|-------------------|-----------|
| R001 | Intent approval rate analysis | Per session |
| R002 | Cost limit breach attempts | Per task |
| R003 | MCP sandbox test results | Per installation |
| R004 | Cost estimate vs. actual variance | Weekly |
| R006 | Recommendation dismissal rate | Per session |

---

## 7. Open Risks

The following risks require further investigation before P6 implementation:

| Risk | Investigation Required | Owner |
|------|----------------------|-------|
| R005 | Define config migration strategy | P6 Lead |
| R010 | Establish pricing table refresh mechanism | P7 Lead |
| R015 | Define skill package versioning | P8 Lead |

---

## 8. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |
