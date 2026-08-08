# CodeBro Manifesto

**Version:** 1.1.0
**Status:** Active
**Date:** 2026-08-08

---

## Identity

CodeBro is a terminal-native engineering intelligence runtime. It lives inside the developer's existing workflow — the terminal — and operates as an extension of their engineering judgment, not a replacement for it.

CodeBro is not a chatbot wrapped in a TUI. It is an engineering instrument: a tool that reads code, reasons about it, proposes changes, and executes under explicit human supervision.

Every design decision flows from this identity. If a feature does not make CodeBro a better engineering instrument, it does not belong in the core.

---

## Vision

A world where every developer has a patient, knowledgeable, and disciplined engineering partner that lives in the terminal, learns the project, and never surprises them.

CodeBro will be the tool senior engineers wish they had — one that understands context, respects boundaries, and makes the mundane visible and the complex manageable.

---

## Mission

To build the most trustworthy terminal engineering runtime for professional software engineers by:

- Understanding the codebase, not just the task input
- Proposing changes that are visible, diff-based, and reversible
- Learning from every session without losing the thread
- Remaining fast, deterministic, and observably correct

---

## North Star

**Every interaction with CodeBro should make the developer more confident in their code — not more dependent on the tool.**

Trust is the metric. If a developer second-guesses CodeBro's output, the system has failed. If a developer reviews a diff and says "yes, that's right," the system has succeeded.

---

## Product DNA

| Attribute | Expression |
|-----------|------------|
| **Engineering-first** | Built for people who write code for a living, not for casual users |
| **Terminal-native** | Keyboard-driven, fast, unobtrusive — respects the terminal environment |
| **Transparent by default** | Every action is visible, every decision is explainable |
| **Human in the loop** | Approval gates on all destructive or expensive operations |
| **Project-aware** | Remembers the codebase, not just the task input |
| **Provider-agnostic** | Works with any LLM provider; the provider is a detail, not a dependency |
| **Extensible** | Skills and MCP servers allow the community to extend without bloating the core |

---

## Long-Term Direction

CodeBro will remain a focused, disciplined tool. The trajectory is:

1. **Deepen project understanding** — better code indexing, symbol awareness, dependency graphs
2. **Improve trust through transparency** — clearer reasoning, better diffs, fuller observability
3. **Grow the extension surface** — skills and MCP servers for specialized workflows
4. **Sharpen the core** — remove complexity that does not serve the primary user

The product will not become a general-purpose AI assistant, a project management tool, or a documentation generator. Those belong in other tools. CodeBro's job is to help you write better code, faster, with more confidence.

---

## Anti-Goals

The following are explicitly out of scope. When a feature request aligns with any of these, the answer is **no** — unless it can be expressed as a Skill or MCP integration.

| Anti-Goal | Reason |
|-----------|--------|
| General chatbot behavior | CodeBro is an engineering runtime, not a conversationalist |
| GUI or web dashboard | The terminal is the interface; nothing more |
| Automatic code deployment | Deployment is a human decision, not an agent decision |
| Social features or leaderboards | This is a personal engineering tool |
| Real-time collaboration | CodeBro is a solo tool; collaboration happens in git |
| Replacing the developer's judgment | CodeBro assists; the developer decides |
| Accumulating unnecessary features | Each feature adds cognitive load and maintenance cost |

---

## Core Values

### 1. Trust Over Capability

A simpler tool that developers trust is more valuable than a complex tool they second-guess. We would rather have 80% coverage with 100% trust than 100% coverage with 80% trust.

### 2. Visibility Over Convenience

It is better to show the user what CodeBro is doing and ask for confirmation than to do something useful but invisible. Transparency is a feature.

### 3. Simplicity Over Cleverness

Simple, readable code beats clever, compact code. Clever code is hard to debug at 11 PM. Simple code is maintainable by anyone who inherits the project.

### 4. Project Memory Over Conversation Memory

A task input that forgets the project is useless. CodeBro must remember the codebase — its structure, conventions, patterns, and history — across sessions. Project context is the foundation; task input context is the surface.

### 5. Determinism Over Probability

Wherever possible, CodeBro's behavior should be deterministic. Non-determinism belongs in the LLM layer, not in the tool dispatch, file editing, or memory management layers. The user should be able to reproduce any session.

### 6. Extensibility Over Monolith

The core stays small. Specialized workflows live in Skills. External integrations live in MCP servers. The core provides the framework; the community provides the specialization.
