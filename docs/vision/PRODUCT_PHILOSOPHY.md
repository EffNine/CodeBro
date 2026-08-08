# Product Philosophy

**Version:** 1.1.0
**Status:** Active
**Date:** 2026-08-08

---

## Why CodeBro Exists

Software engineering is a cognitive act. The hardest part is not typing syntax — it is understanding the codebase, planning the change, and executing it correctly. Most AI coding tools focus on the typing. CodeBro focuses on the understanding.

CodeBro exists because developers spend more time reading and reasoning about code than writing it. An AI assistant that only helps you generate code is a typewriter with a personality. An AI assistant that understands your project, remembers your conventions, and proposes changes you can trust is an engineering partner.

CodeBro is the latter.

---

## Why CodeBro Is Not Another AI Chatbot

A chatbot's goal is to be conversational. It optimizes for engagement, variety, and friendly output. This is the wrong optimization for a coding tool.

CodeBro optimizes for **correctness, traceability, and efficiency**. It does not chat. It observes, plans, executes, and reports. When it speaks, it speaks in the language of diffs, commands, and symbols — not in paragraphs of explanation.

The differences are structural:

| Chatbot | CodeBro |
|---------|---------|
| Optimizes for conversation flow | Optimizes for engineering task correctness |
| Forgets the project between sessions | Remembers the project across sessions |
| Generates text | Proposes and applies code changes |
| No approval gates | Explicit approval on every file write |
| Black-box reasoning | Observable tool calls and trace logs |
| Stateless or session-scoped memory | Project-level and global long-term memory |

CodeBro is not a chatbot with file access. It is an engineering intelligence runtime with natural language as its input surface.

---

## Why CodeBro Focuses on Engineering Intelligence

Engineering intelligence is the ability to understand a codebase well enough to make correct changes. It requires:

- **Symbol awareness** — knowing what `AuthService` is, not just where it lives
- **Dependency tracing** — knowing what breaks when you change a function signature
- **Convention recognition** — knowing that this project uses `Result<T, E>` not `Option<T>`
- **Pattern reuse** — knowing that the test pattern used last week works here too
- **Project memory** — remembering decisions made in previous sessions

Most AI coding tools skip the intelligence and go straight to generation. They send the whole repository to the LLM and hope for the best. CodeBro indexes, searches, and reasons before it generates. The intelligence layer exists so the generation layer is informed.

Engineering intelligence is what separates a tool that writes correct code from a tool that writes plausible code. Plausible code passes local review. Correct code passes CI.

---

## Why Core Stays Intentionally Small

The core of CodeBro contains only what is necessary to operate as a terminal-native engineering runtime:

- A TUI for interaction
- An agent loop for orchestration
- A provider abstraction for LLM communication
- A tool system for file and shell operations
- A memory system for persistence
- A skill system for reusable workflows
- An intelligence layer for code understanding

Everything else is an extension.

This constraint exists for three reasons:

1. **Maintainability** — Every module added to the core must be maintained, tested, and documented forever. Small cores stay small. Large cores become swamps.

2. **Speed** — The core determines startup time, memory usage, and response latency. Bloat in the core bloats everything.

3. **Focus** — A small core forces difficult prioritization. If a feature does not fit, it goes elsewhere. This keeps the product sharp.

The core is the instrument. Skills and MCP servers are the attachments. The instrument stays clean.

---

## Why Skills Exist

Skills are reusable workflows encoded as markdown files. They capture the pattern of a successful task — the steps, the tools used, the files touched — so that future similar tasks can be executed faster and more reliably.

Skills exist because:

- **Repetition is waste** — If you solved a problem once, you should not solve it the same way from scratch every time.
- **Context is expensive** — Each skill encodes project-specific knowledge that would otherwise need to be re-established.
- **Confidence grows with reuse** — Skills track success rates. A skill used 20 times with 95% success is more trustworthy than a fresh LLM guess.

Skills live in `.codebro/skills/`. They are plain markdown. They are editable by hand. They are discovered automatically by the runtime.

Skills are the primary extension mechanism for workflow-level intelligence.

---

## Why MCP Exists

MCP (Model Context Protocol) servers are external tool providers that extend CodeBro's capabilities beyond its built-in tools. An MCP server can expose filesystem operations, database queries, API calls, or any stateful interaction as a set of tools.

MCP exists because:

- **Not all tools belong in the core** — Database access, cloud APIs, and specialized integrations are project-specific. They do not belong in the universal runtime.
- **The community builds better integrations** — MCP servers can be shared, versioned, and discovered independently of CodeBro releases.
- **Security through sandboxing** — MCP servers run as separate processes. A misbehaving server cannot crash the runtime or access sensitive paths without explicit configuration.

MCP is the extension mechanism for tool-level capabilities. It is governed by a lifecycle system that requires explicit user approval for installation, updates, and removal.

---

## The Extension Stack

```
┌─────────────────────────────────────────────────────┐
│                   Extension Layer                    │
│  Skills (workflow patterns)    MCP Servers (tools)   │
├─────────────────────────────────────────────────────┤
│                   Intelligence Layer                 │
│  Symbol search · Dependency graphs · Context build   │
├─────────────────────────────────────────────────────┤
│                      Core                            │
│  TUI · Agent · Memory · Tools · Providers · Config  │
└─────────────────────────────────────────────────────┘
```

The core is frozen in responsibility. The intelligence layer grows with the codebase. The extension layer grows with the community. No layer encroaches on another.
