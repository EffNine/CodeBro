# Product Decision Filter

**Version:** 1.1.0
**Status:** Active
**Date:** 2026-08-08

---

## Purpose

This document provides a decision framework for evaluating every feature request, design proposal, and code change. It exists to ensure that CodeBro remains focused, disciplined, and true to its identity as a terminal-native engineering intelligence runtime.

When in doubt, run the proposal through this filter.

---

## The Filter

```
New Feature Request
        │
        ▼
┌─────────────────────────────────────────────────────┐
│  1. Does this make CodeBro a better                │
│     engineering intelligence runtime?               │
│                                                     │
│  • Does it help the developer write correct code?   │
│  • Does it improve understanding of the codebase?   │
│  • Does it make the engineering workflow faster?    │
│  • Does it increase trust in the tool's output?     │
│                                                     │
│  If NO → Go to Step 2                               │
│  If YES → Implement in core                         │
└─────────────────────────────────────────────────────┘
        │
        ▼ (NO)
┌─────────────────────────────────────────────────────┐
│  2. Can this be expressed as a                      │
│     Skill, MCP Server, or Plugin?                   │
│                                                     │
│  • Is it a reusable workflow pattern? → Skill       │
│  • Is it an external tool integration? → MCP Server │
│  • Is it a cross-cutting lifecycle hook? → Plugin   │
│                                                     │
│  If YES → Implement as extension                    │
│  If NO → Go to Step 3                               │
└─────────────────────────────────────────────────────┘
        │
        ▼ (NO)
┌─────────────────────────────────────────────────────┐
│  3. Reject.                                         │
│                                                     │
│  The feature does not serve CodeBro's identity.     │
│  It may be useful. It may be popular.               │
│  It does not belong in CodeBro.                     │
│                                                     │
│  Document the rejection reason.                     │
│  Point the requester to the appropriate             │
│  extension mechanism or alternative tool.             │
└─────────────────────────────────────────────────────┘
```

---

## Decision Examples

### Example 1: "Add a web dashboard so users can interact with CodeBro from a browser"

**Step 1:** Does this make CodeBro a better engineering intelligence runtime?

No. A web dashboard is a different interface paradigm. It does not improve code understanding, engineering speed, or trust. It changes the product from a terminal tool to a web application.

**Step 2:** Can this be a Skill, MCP, or Plugin?

No. A web dashboard is a fundamental interface change, not an extension. It would require a web server, authentication, session management, and a completely different architecture.

**Step 3: Reject.** This belongs in a separate project — a web-based IDE companion, perhaps. It is not CodeBro.

---

### Example 2: "Add Playwright test generation"

**Step 1:** Does this make CodeBro a better engineering intelligence runtime?

Yes. Playwright tests are code. Generating them is an engineering task. It helps the developer write correct, tested code. It improves the engineering workflow.

**Step 2:** N/A — proceeds to core implementation.

**Result: Implement in core.** This is `/playwright` — an engineering command that operates on the project.

---

### Example 3: "Add Slack integration to notify the team when a task completes"

**Step 1:** Does this make CodeBro a better engineering intelligence runtime?

No. Slack notifications are a team coordination feature. They do not improve code understanding, engineering speed, or trust. They are a personal assistant / project management function.

**Step 2:** Can this be an MCP Server?

Yes. An MCP server could expose a Slack notification tool. The server would handle authentication, message formatting, and delivery. CodeBro would simply call the tool.

**Result: Implement as MCP Server.** The integration belongs in the extension layer, not the core.

---

### Example 4: "Add a /docs command that generates documentation for the current project"

**Step 1:** Does this make CodeBro a better engineering intelligence runtime?

Debatable. Documentation is related to code, but generating documentation is not the same as writing or understanding code. It is a secondary activity. The primary engineering task is the code itself.

**Step 2:** Can this be a Skill?

Yes. A skill could encode the workflow for generating documentation: scan the codebase, identify public APIs, generate markdown, apply formatting conventions. This is a reusable workflow pattern.

**Result: Implement as a Skill.** The workflow is project-specific and belongs in the extension layer. If many users find it valuable, it can be shared as a community skill.

---

### Example 5: "Add automatic dependency vulnerability scanning"

**Step 1:** Does this make CodeBro a better engineering intelligence runtime?

Yes. Vulnerability scanning is an engineering concern. It affects code correctness, security, and maintainability. A developer who knows about vulnerabilities in their dependencies is making better engineering decisions.

**Step 2:** Can this be an MCP Server?

Yes. An MCP server could wrap `cargo audit`, `npm audit`, or `pip-audit` and expose the results as tools.

**Step 3:** Or should it be in the core?

Consider: is this a common enough workflow that it deserves core support? If the runtime can run vulnerability scans as part of a `/test` or `/doctor` workflow, it may belong in the core. If it is a specialized external tool, MCP is appropriate.

**Result: Implement as MCP Server (with core integration via `/doctor`).** The scanning logic lives in the MCP server; the core invokes it as part of the project health workflow.

---

### Example 6: "Add a chat interface for non-technical stakeholders to ask about the codebase"

**Step 1:** Does this make CodeBro a better engineering intelligence runtime?

No. This changes the user from a developer to a stakeholder. It changes the interface from a terminal to a chat. It changes the purpose from engineering to communication.

**Step 2:** Can this be an MCP Server or Plugin?

No. This is a fundamentally different product — a codebase Q&A web interface. It is not an extension of CodeBro; it is a different application built on top of CodeBro's intelligence layer.

**Result: Reject.** The intelligence layer could theoretically be exposed as an API, but the chat interface for stakeholders is a separate product.

---

### Example 7: "Add a /benchmark command to compare model performance on a benchmark suite"

**Step 1:** Does this make CodeBro a better engineering intelligence runtime?

Yes. Benchmarking is an engineering activity. It helps the developer evaluate model quality, compare providers, and make informed decisions about which model to use for which task. It improves the engineering workflow.

**Step 2:** N/A — proceeds to core implementation.

**Result: Implement in core.** This is `/benchmark` — an engineering command that operates on the project and the runtime's configuration.

---

### Example 8: "Add a /calendar command that shows the developer's upcoming meetings"

**Step 1:** Does this make CodeBro a better engineering intelligence runtime?

No. Calendar management is a personal assistant function. It does not improve code understanding, engineering speed, or trust.

**Step 2:** Can this be an MCP Server?

Yes. An MCP server could connect to a calendar API and expose scheduling tools.

**Step 3:** Should it be in CodeBro?

Even as an MCP server, this is a personal assistant integration. CodeBro's extension layer is for engineering extensions. A calendar MCP server belongs in a general-purpose AI assistant, not an engineering runtime.

**Result: Reject.** Even though it could be an MCP server, it does not serve CodeBro's engineering identity.

---

## The Filter in Practice

Use this filter at three stages:

1. **Before writing code** — Run the feature request through the filter. If it fails Step 1 and Step 2, do not start implementation.
2. **During code review** — If a change adds functionality that fails the filter, request revision.
3. **When reviewing existing code** — If a module exists that fails the filter, flag it for removal or migration to an extension.

---

## Summary

The filter has three outcomes:

| Outcome | Action |
|---------|--------|
| **Core** | The feature makes CodeBro a better engineering intelligence runtime. Implement it. |
| **Extension** | The feature does not belong in the core, but can be expressed as a Skill, MCP Server, or Plugin. Implement it as an extension. |
| **Reject** | The feature does not serve CodeBro's identity. Document why and close the request. |

The filter is not a barrier to innovation. It is a compass. It ensures that every addition — whether in the core or the extension layer — serves the same purpose: making the developer's engineering work better.
