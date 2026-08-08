# Engineering Principles

**Version:** 1.1.0
**Status:** Active
**Date:** 2026-08-08

---

## 1. Engineering First

**The runtime exists to serve the engineering workflow, not the other way around.**

### Purpose
CodeBro's primary loop is not conversation — it is the engineering cycle: understand, plan, execute, verify, reflect. Every subsystem is designed to serve this cycle. Features that optimize for conversational smoothness at the expense of engineering correctness are rejected.

### Benefits
- Shorter feedback loops between proposal and verification
- Changes are grounded in codebase reality, not language model guesswork
- The tool feels like an extension of the developer's existing workflow

### Tradeoffs
- Development is slower — engineering intelligence requires indexing, parsing, and reasoning infrastructure
- The TUI is less "friendly" than a chatbot — it prioritizes information density over warmth
- Some features that would make the tool feel more conversational are deliberately excluded

---

## 2. Project Awareness

**The runtime understands the project, not just the current task.**

### Purpose
A coding runtime that only sees the files it reads is flying blind. Project awareness means the runtime knows the language, framework, build system, directory structure, conventions, and dependencies. This context is gathered once (during indexing) and reused across all tasks.

### Benefits
- Recommendations respect project conventions (e.g., `cargo test` not `pytest`)
- Context selection is targeted — the runtime sends relevant files, not the whole repo
- The runtime remembers decisions and patterns from previous sessions

### Tradeoffs
- Indexing adds startup latency (~500ms for mid-size projects)
- Index must be kept fresh — file changes require re-indexing
- Project memory grows over time and must be consolidated to prevent bloat

---

## 3. Provider Agnostic

**The provider is a detail. The runtime does not care which one you use.**

### Purpose
Locking CodeBro to a single provider (OpenAI, Anthropic, etc.) would make it fragile and expensive. The `Provider` trait is the single interface to LLM communication. Adding a new provider requires implementing the trait — no changes to the runtime, TUI, or tools.

### Benefits
- Users can switch providers without changing workflows
- Local providers (Ollama, LM Studio) are first-class citizens
- Cost optimization is possible by routing tasks to the cheapest capable model
- The runtime remains functional even if a provider changes pricing or deprecates models

### Tradeoffs
- Provider-specific features (streaming format, error codes, rate limits) require abstraction layers
- Testing against multiple providers increases CI surface
- Some provider quirks leak through despite the abstraction

---

## 4. Deterministic by Default

**Given the same input, CodeBro should produce the same output — every time.**

### Purpose
Non-determinism is the enemy of debugging. If a tool call produces different results on different runs, or if memory consolidation removes different entries, the runtime becomes unpredictable. Determinism applies to all layers except the LLM itself.

### Benefits
- Tasks are reproducible — bugs can be replayed and diagnosed
- Testing is meaningful — test failures indicate real regressions, not randomness
- Users can reason about cause and effect — if X changed, Y is the result

### Tradeoffs
- Some operations (e.g., timestamp logging) are inherently non-deterministic and must be handled explicitly
- Seeded randomness is required for any probabilistic operation
- The LLM layer remains non-deterministic by nature; the runtime must be resilient to this

---

## 5. Explainability

**Every action the runtime takes must be explainable to the user.**

### Purpose
A black-box runtime that modifies files without explanation destroys trust. Explainability means the user can see — in real time — what the runtime is doing, why it is doing it, and what the expected outcome is.

### Benefits
- Users can catch mistakes before they are applied (diff preview)
- Users learn about the codebase through the runtime's reasoning
- Trust is built through transparency, not opacity

### Tradeoffs
- Explaining every action adds UI complexity and latency
- Some explanations are noisy — the runtime must balance detail with clarity
- Chain-of-thought reasoning is private; only the conclusion and supporting evidence are shown

---

## 6. Offline First

**CodeBro must function without an internet connection, except where the LLM is required.**

### Purpose
Developers work in airports, on trains, and in buildings with poor connectivity. The runtime's core functions — reading files, running shell commands, managing memory, displaying the TUI — must work offline. Only LLM communication requires a network.

### Benefits
- The tool is available wherever the developer is available
- Offline tasks build project memory that persists when connectivity returns
- No dependency on cloud infrastructure for core functionality

### Tradeoffs
- Offline mode cannot use LLM-based features (semantic search, intelligence queries)
- The runtime must degrade gracefully — show a clear message when the provider is unreachable
- Local model support (Ollama, LM Studio) is the bridge between offline and online

---

## 7. Inspectability

**Every state in CodeBro is inspectable, queryable, and auditable.**

### Purpose
If you cannot inspect the runtime's state, you cannot trust it. Memory entries, skill confidence scores, tool traces, session histories, and preference changes are all persisted as human-readable JSON. The user can open any of these files and understand exactly what the runtime knows and has done.

### Benefits
- Bugs can be diagnosed by examining state files
- Users can edit memory, skills, or preferences by hand if needed
- No hidden state — everything is either in a file or in memory with a clear lifecycle

### Tradeoffs
- State files consume disk space — consolidation and rotation are required
- Human-readable formats are less compact than binary — acceptable for the scale of data involved
- Exposing state invites users to edit it; edits must be validated to prevent corruption

---

## 8. Progressive Disclosure

**Show only what is needed, when it is needed.**

### Purpose
The terminal has limited screen space. Showing everything at once creates noise. Progressive disclosure means panels, details, and options appear only when relevant and collapse when idle. The default view is the task output. Everything else is a layer deeper.

### Benefits
- The TUI stays calm and focused on the task at hand
- New users are not overwhelmed by available features
- Expert users can access advanced panels without cluttering the default view

### Tradeoffs
- Users must discover how to access hidden panels (keyboard shortcuts, command palette)
- Some users prefer always-visible information — toggles must be intuitive
- Layout computation adds complexity to the TUI rendering engine

---

## 9. Extensibility

**The core provides the framework. The community provides the specialization.**

### Purpose
CodeBro's core is intentionally minimal. Specialized workflows belong in Skills. External tool integrations belong in MCP servers. This keeps the core small, testable, and maintainable while allowing the ecosystem to grow independently.

### Benefits
- Community contributions do not increase core maintenance burden
- Specialized workflows can be developed and distributed without CodeBro releases
- Users choose their extensions — they are not forced into a one-size-fits-all feature set

### Tradeoffs
- Extension development has a higher barrier to entry than core development
- Core APIs must be stable enough to support extensions across versions
- Extension quality varies; the core cannot guarantee extension behavior

---

## 10. Terminal Muscle Memory

**CodeBro extends terminal workflows. It does not invent new interaction patterns.**

### Purpose
Developers have years of terminal muscle memory — `Ctrl+C` to interrupt, `Ctrl+L` to clear, `Ctrl+R` to search history, `Up/Down` to scroll, `Tab` to complete, `Escape` to cancel. CodeBro must respect these. Overriding them creates friction and frustration. The default input scheme is arrow keys and Tab, not Vim bindings.

### Benefits
- No re-learning required — developers use CodeBro with muscle memory they already have
- The TUI feels like a natural extension of the shell, not a foreign interface
- Transition from shell to CodeBro is seamless

### Tradeoffs
- Vim bindings must be supported as an opt-in scheme, not the default
- Some TUI frameworks default to Vim-style bindings; CodeBro must override this
- Documentation must clearly state the default navigation scheme

---

## Summary

| Principle | One-Line Statement |
|-----------|-------------------|
| Engineering First | The runtime serves the engineering workflow |
| Project Awareness | The runtime understands the project, not just the task |
| Provider Agnostic | The provider is a detail, not a dependency |
| Deterministic by Default | Same input produces same output — except the LLM |
| Explainability | Every action is visible and justifiable |
| Offline First | Core functions work without internet |
| Inspectability | Every state file is human-readable and editable |
| Progressive Disclosure | Information appears only when relevant |
| Extensibility | Skills and MCP grow the tool; the core stays small |
| Terminal Muscle Memory | CodeBro extends terminal habits; it does not replace them |
