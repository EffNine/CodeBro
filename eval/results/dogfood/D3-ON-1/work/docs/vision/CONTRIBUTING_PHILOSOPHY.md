# Contributing Philosophy

**Version:** 1.1.0
**Status:** Active
**Date:** 2026-08-08

---

## The First Question

Before implementing any feature, every contributor must ask:

> **Does this make CodeBro a better engineering intelligence runtime?**

Not a better chatbot. Not a better project manager. Not a better documentation generator. A better **engineering intelligence runtime** — a tool that helps developers write correct code, understand codebases, and ship with confidence.

If the answer is **yes**, the feature belongs in the core.

If the answer is **no**, the feature belongs in one of three places:

| Destination | When to Use |
|-------------|-------------|
| **Skill** | A reusable workflow pattern (e.g., "add a REST endpoint", "migrate a database") |
| **MCP Server** | An external tool integration (e.g., "Query PostgreSQL", "Interact with GitHub API") |
| **Plugin** | A cross-cutting concern that hooks into the runtime lifecycle (e.g., a custom permission checker) |
| **Reject** | A feature that does not serve the engineering workflow |

This question is the gate. It prevents feature creep from the start.

---

## Why This Question Matters

CodeBro's identity is narrow and deliberate. It is a terminal-native engineering intelligence runtime for professional software engineers. Every feature that does not serve this identity dilutes the product.

Consider what happens when a team stops asking this question:

1. A feature request comes in: "Can CodeBro write README files?"
2. The answer "yes" seems reasonable — it is related to code, after all.
3. Another: "Can CodeBro generate unit tests automatically?"
4. Another: "Can CodeBro explain code to junior developers?"
5. Another: "Can CodeBro track sprint progress?"

Each request is reasonable in isolation. Together, they transform CodeBro from a focused engineering instrument into a general-purpose AI assistant — and a mediocre one at that, since it now competes with tools that were designed for those purposes from the start.

The question protects the product's focus.

---

## Anti-Patterns

The following patterns are the enemy of a disciplined codebase. Recognize them, name them, and reject them.

### 1. Feature Creep

**Definition:** Adding functionality that is tangentially related to the core purpose.

**Example:** Adding a web dashboard so users can interact with CodeBro from a browser. The reasoning: "Some users prefer GUIs." The reality: This pulls the project into a completely different architectural domain, requires a completely different codebase, and serves a different user need than the terminal runtime.

**Detection:** Ask: "Would a user of this feature also be a user of the terminal runtime?" If the answer is no, it is feature creep.

**Resolution:** Move to a separate project or reject.

---

### 2. Overengineering

**Definition:** Building a general solution when a specific one would suffice. Introducing abstractions, frameworks, or architectures that add complexity without proportional benefit.

**Example:** Creating a full plugin framework with a custom language and runtime before there is more than one plugin. The reasoning: "We should be prepared for future plugins." The reality: The first plugin will teach you what the framework actually needs. Designing for ten plugins when you have zero is waste.

**Detection:** Ask: "Has this abstraction been proven by at least two real use cases?" If the answer is no, it is overengineering.

**Resolution:** Implement the simplest thing that works. Refactor when the second use case demands it.

---

### 3. Magic Behavior

**Definition:** Hidden logic that produces unexpected results. The user cannot predict the outcome from the input because the behavior is not visible.

**Example:** A skill that auto-applies itself when triggered, without showing the user what it will do. The reasoning: "It will save the user time." The reality: The user has no way to verify that the skill is doing the right thing. Trust is destroyed.

**Detection:** Ask: "Can the user predict this behavior by reading the code or the documentation?" If the answer is no, it is magic.

**Resolution:** Make the behavior visible. Show what the system is doing and why. Require approval for destructive actions.

---

### 4. Hidden Execution

**Definition:** Running code, making network calls, or modifying state without the user's knowledge.

**Example:** An MCP server that runs a background process to fetch updates without notifying the user. The reasoning: "It keeps the data fresh." The reality: The user has no awareness of the network traffic, the data being sent, or the additional latency being introduced.

**Detection:** Ask: "If the user ran `strace` or a network monitor, would they see this execution?" If the answer is no, it is hidden.

**Resolution:** All external execution must be logged, visible in the activity stream, and (for destructive actions) subject to approval.

---

### 5. UI Clutter

**Definition:** Adding visual elements to the TUI that do not directly support the user's current task.

**Example:** Adding a persistent sidebar with 12 icons, a bottom status bar with 5 metrics, and a right panel showing runtime history — all visible by default. The reasoning: "Users might want to see this information." The reality: Most users never look at it. It occupies terminal space that could be used for the task output.

**Detection:** Ask: "Does the average user interact with this element more than once per session?" If the answer is no, it is clutter.

**Resolution:** Apply progressive disclosure. Hide the element by default. Make it available via keyboard shortcut or command.

---

### 6. Unnecessary Abstraction

**Definition:** Introducing a layer of indirection that does not solve a real problem.

**Example:** Creating a new trait (`FileEditStrategy`) when a simple function parameter (`diff: bool`) would suffice. The reasoning: "We might need different edit strategies in the future." The reality: The future never arrives on schedule. The abstraction adds complexity today for a benefit that may never materialize.

**Detection:** Ask: "Is there a concrete, present need for this abstraction?" If the answer is no, it is unnecessary.

**Resolution:** YAGNI (You Aren't Gonna Need It). Implement the direct solution. Abstract only when the second use case demands it.

---

## The Review Process

Every contribution is reviewed against this checklist:

1. **Identity check** — Does this make CodeBro a better engineering intelligence runtime?
2. **Scope check** — Does this belong in the core, or does it belong in a Skill, MCP, or Plugin?
3. **Simplicity check** — Is this the simplest solution that satisfies the requirement?
4. **Visibility check** — Is every behavior of this change visible to the user?
5. **Test check** — Does this change have tests that verify both the happy path and the failure path?
6. **Documentation check** — Is this change documented in the relevant `docs/vision/` or `docs/design/` file?

If any check fails, the contribution is returned for revision. This is not a barrier — it is a quality gate.

---

## Adding to the Core

The core is the smallest possible set of functionality that makes CodeBro useful. To add to the core:

1. Write an RFC describing the problem, the proposed solution, and the tradeoffs.
2. File an ADR if the RFC requires an architectural decision.
3. Implement the change with tests.
4. Update documentation.
5. Submit for review against the checklist above.

The bar for core changes is high because the core is permanent. Every line added to the core must be maintained forever.

---

## Adding a Skill

Skills are the preferred extension mechanism. They are markdown files that encode workflows. To add a skill:

1. Create a new markdown file in `.codebro/skills/` (project-level) or `~/.codebro/skills/` (global).
2. Follow the skill template: description, triggers, workflow, tools used, examples.
3. Test the skill by triggering it with the specified trigger phrases.
4. Share the skill with the community if it is generally useful.

Skills require no core changes, no reviews, and no releases. They are the fastest path from idea to execution.

---

## Adding an MCP Server

MCP servers extend CodeBro's tool surface. To add an MCP server:

1. Ensure the server implements the Model Context Protocol specification.
2. Install it via the `//mcp` command (approval required).
3. Validate that the server's tools are accessible from the runtime.
4. Document the server's capabilities in the project README or a provider card.

MCP servers are maintained by their authors, not by CodeBro. CodeBro's responsibility ends at discovery, validation, and installation.

---

## Summary

CodeBro's strength is its focus. The first question — "Does this make CodeBro a better engineering intelligence runtime?" — is the mechanism that protects that focus.

When in doubt, ask the question. When the answer is unclear, choose the path that keeps the core small and the extensions rich.
