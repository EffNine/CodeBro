# Prompt Builder v2 — Engineering Intelligence Compiler

**Version:** 1.0.0
**Status:** Active
**Sprint:** 21.0
**Module:** `src/prompt_builder/`

---

## Responsibilities

The Prompt Builder v2 transforms from a string concatenation utility into a
deterministic **Engineering Intelligence Compiler**.

It consumes typed inputs from upstream modules and produces a
`CompiledPrompt` — a structured, observable prompt ready for provider
submission.

### What It Owns

- Section assembly from typed context inputs
- Template selection based on intent classification
- Deterministic section ordering
- Diagnostics and statistics tracking
- Context budget respect for memory injection

### What It Does NOT Own

- Context budget decisions (consumes already-selected fragments)
- Intent classification (consumes `IntentPlan` from intent engine)
- Memory selection (consumes `MemoryResolution` from memory runtime)
- Provider selection (output feeds into provider runtime)

---

## Pipeline

```
User Request
    ↓
Intent Classification (IntentPlan)
    ↓
Context Assembly (Context)
    ↓
Engineering Memory (MemoryResolution)
    ↓
Project Identity (ProjectInfo)
    ↓
Prompt Builder v2
    ↓
Compiled Prompt
    ↓
Provider Runtime
```

The compiler is a **pure function**: same inputs → same output, every time.
No random ordering. No HashMap iteration ordering. No timestamps in output.

---

## Section Ordering

Sections are emitted in a **canonical deterministic order** defined by
`PromptOrdering`. Templates select a subset but never reorder.

### Canonical Order

| # | Section Key | Description |
|---|-------------|-------------|
| 1 | SystemIdentity | Core system prompt and role definition |
| 2 | ProjectIdentity | Project metadata from scanner |
| 3 | CurrentTask | Intent plan: goal, type, confidence, ambiguity |
| 4 | EngineeringConstraints | Project constraints derived from identity |
| 5 | RelevantContext | Conversation history + relevant files |
| 6 | EngineeringMemory | Relevant memory fragments (budget-respecting) |
| 7 | ArchitectureDecisions | Architecture rules from engineering facts |
| 8 | WorkspaceFacts | Fact count + diagnostics summary |
| 9 | ActiveFiles | Currently active file paths |
| 10 | UserRequest | Raw user request (trimmed) |
| 11 | ResponseInstructions | Template-aware output guidance |

### Template-Selected Orderings

**Engineering / Default / Architecture**
```
SystemIdentity → ProjectIdentity → CurrentTask → Constraints → Context
→ Memory → WorkspaceFacts → UserRequest → ResponseInstructions
```

**Debugging**
```
SystemIdentity → ProjectIdentity → CurrentTask → Constraints
→ ActiveFiles → Context → Memory → UserRequest → ResponseInstructions
```

**Review**
```
SystemIdentity → ProjectIdentity → CurrentTask → ArchitectureDecisions
→ Context → Memory → ActiveFiles → UserRequest → ResponseInstructions
```

**Planning**
```
SystemIdentity → ProjectIdentity → CurrentTask → Constraints
→ Context → Memory → UserRequest → ResponseInstructions
```

**Refactoring**
```
SystemIdentity → ProjectIdentity → CurrentTask → ArchitectureDecisions
→ Context → Memory → ActiveFiles → UserRequest → ResponseInstructions
```

**Testing**
```
SystemIdentity → ProjectIdentity → CurrentTask → Constraints
→ Context → Memory → ActiveFiles → UserRequest → ResponseInstructions
```

**Documentation**
```
SystemIdentity → ProjectIdentity → CurrentTask → Context
→ Memory → UserRequest → ResponseInstructions
```

---

## Template Selection

Templates are selected deterministically from `IntentPlan` properties.

| Intent Type | Goal Keywords | Selected Template |
|-------------|---------------|-------------------|
| Execution | debug, fix, error | Debugging |
| Execution | test | Testing |
| Execution | refactor, restructure | Refactoring |
| Execution | document, readme | Documentation |
| Execution | architecture, design | Architecture |
| Execution | plan | Planning |
| Question | — | Review |
| Preference / Configuration | — | Default |
| Help | — | Default |
| Unknown | — | Default |
| (no intent) | — | Engineering |

The `select_template()` function is a pure function with no side effects.

---

## Diagnostics

`PromptDiagnostics` exposes per-compilation observability:

| Field | Type | Description |
|-------|------|-------------|
| `total_length` | usize | Total character count of rendered prompt |
| `section_sizes` | Vec<(String, usize)> | Per-section character counts |
| `template_used` | String | Template name used |
| `estimated_tokens` | usize | Token estimate (chars / 4) |
| `dropped_sections` | Vec<String> | Sections skipped (empty content) |
| `compile_duration_ms` | u64 | Wall-clock compilation time |

---

## Statistics

`PromptStatistics` exposes aggregate compile metrics:

| Field | Type | Description |
|-------|------|-------------|
| `section_count` | usize | Number of emitted sections |
| `estimated_tokens` | usize | Total estimated tokens |
| `compile_time_ns` | u64 | Nanoseconds elapsed |
| `memory_fragments` | usize | Memory entries injected |
| `context_fragments` | usize | Context files injected |
| `template` | String | Template name |

---

## Tradeoffs

### Determinism vs. Flexibility

The compiler is fully deterministic. This means:
- **Pro:** Reproducible prompts enable debugging and testing
- **Con:** Cannot adapt to runtime conditions not captured in inputs

### Section Dropping

Empty sections are dropped (not rendered). This:
- **Pro:** Keeps prompts concise
- **Con:** Section count varies between compilations

### Memory Budget Respect

Memory injection respects `context_budget_remaining`:
- **Pro:** Prevents prompt bloat
- **Con:** Some relevant memories may be omitted under tight budgets

### Template Selection Heuristics

Keyword-based selection is simple and deterministic:
- **Pro:** No LLM dependency, fully auditable
- **Con:** May miss nuanced intent classifications

---

## Future Extension Points (Sprint 22)

The following extension points are exposed but not implemented:

1. **Custom Section Builders** — `sections.rs` module is designed for
   additional section builders without modifying the compiler core.

2. **Template Registry** — `PromptTemplate` enum can be extended with new
   variants. The `section_order()` method must be updated accordingly.

3. **Token-Aware Truncation** — The compiler currently uses char/4 as a
   rough estimate. A more accurate tokenizer (e.g., tiktoken) can be
   plugged in via the `estimate_tokens` function.

4. **Multi-Turn Optimization** — The compiler treats each compilation
   independently. Future work can cache previous compilations and
   diff against them.

5. **Conditional Sections** — Some sections could be gated on runtime
   conditions (e.g., only show WorkspaceFacts when facts exist).
   The `build_section()` method is the extension point.

6. **Prompt Validation** — A pre-flight validation pass could check
   section content against token budgets before compilation.

---

## Constitution Compliance

This implementation complies with all documents under `docs/vision/`:

| Principle | Compliance |
|-----------|------------|
| Engineering First | Sections reflect engineering context, not conversation |
| Project Awareness | Project identity and constraints are first-class sections |
| Provider Agnostic | Output is plain text; no provider-specific logic |
| Deterministic by Default | Same inputs → same output, verified by tests |
| Explainability | Diagnostics expose every decision (template, sections, drops) |
| Progressive Disclosure | Empty sections are dropped; only relevant content rendered |
| Extensibility | Module is designed for extension without core changes |

---

## Files

| File | Purpose |
|------|---------|
| `mod.rs` | Module declaration and re-exports |
| `builder.rs` | Public `PromptBuilder` API |
| `compiler.rs` | Core compilation logic + template selection |
| `template.rs` | `PromptSection`, `PromptTemplate`, `SectionKey` types |
| `sections.rs` | Per-section content builders |
| `ordering.rs` | Canonical section ordering |
| `diagnostics.rs` | Per-compilation diagnostics |
| `statistics.rs` | Aggregate compile statistics |

---

## Public API

```rust
// Entry point
let builder = PromptBuilder::new();

// Compile
let compiled: CompiledPrompt = builder.compile(
    system_prompt,
    project_name,
    project_info,       // Option<&ProjectInfoLike>
    intent_plan,        // Option<&IntentPlanLike>
    relevant_files,     // &[ContextFileLike]
    conversation,       // &[ConversationMsgLike]
    memories,           // &[MemoryFragment]
    arch_rules,         // &[ArchitectureRuleLike]
    fact_count,
    diagnostics,        // &[DiagnosticLike]
    active_files,       // &[String]
    user_request,
    context_budget_remaining,
);

// Access results
compiled.prompt          // String
compiled.statistics      // PromptStatistics
compiled.diagnostics     // PromptDiagnostics
compiled.template_selection  // TemplateSelection
```
