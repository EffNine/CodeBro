# Context Compiler

**Phase**: P10.5 — Engineering Runtime Design Summit
**Status**: APPROVED TO DESIGN — NO IMPLEMENTATION
**Version**: 1.0.0

---

## 1. Purpose

The Context Compiler produces **minimal, maximally-relevant, token-efficient
prompt fragments** for the AI Runtime. It is the last deterministic stage in
the Engineering Runtime before any LLM call.

It does **not** assemble the full conversation (that is the Context
Runtime's job). It produces engineering-aware fragments: compact, factual,
graph-derived summaries that the Context Runtime can slot into the prompt.

---

## 2. Inputs

| Input | Source | Content |
|-------|--------|---------|
| Engineering Graphs | Engineering Runtime | dependency, module, call, test-impact, architecture facts |
| Workspace Runtime | Workspace Runtime | file list, change events, build-system profile, active files |
| Memory Runtime | Memory Runtime | persistent project knowledge (patterns, past impact, conventions) |
| Query intent | Caller | the question/task being answered |

All inputs are read-only. The Compiler never writes memory, never walks the
filesystem, and never invokes an LLM.

---

## 3. Output

A single `ContextFragment` (or a set, for batched requests):

| Field | Description |
|-------|-------------|
| `fragments` | ordered, typed prompt fragments |
| `token_estimate` | computed token count |
| `sources` | graph/file references backing each fragment |
| `confidence` | deterministic — based on graph freshness, not probability |
| `truncated` | whether budget enforcement dropped fragments |

---

## 4. Compilation Pipeline

```
Request / Intent
    │
    ▼
┌───────────────────────┐
│ 1. Intent Mapping      │  question → which graphs to query, in order
│    (deterministic)     │  "rename X" → dependency + symbol registry
└───────────┬───────────┘
            ▼
┌───────────────────────┐
│ 2. Graph Queries       │  run only the minimal required queries
│    (batch, cached)     │  reuse in-flight results, no redundant traversal
└───────────┬───────────┘
            ▼
┌───────────────────────┐
│ 3. Relevance Ranking   │  rank raw facts by task relevance + locality
│    (deterministic)     │  close files > distant files; public API > private
└───────────┬───────────┘
            ▼
┌───────────────────────┐
│ 4. Fragment Assembly   │  emit typed fragments (symbol cards, dep lists,
│    (templates)         │  impact lists, test lists, arch notes)
└───────────┬───────────┘
            ▼
┌───────────────────────┐
│ 5. Token Budgeting     │  fit within caller budget; drop lowest value
│    (budget tracker)    │  fragments; mark truncated
└───────────┬───────────┘
            ▼
┌───────────────────────┐
│ 6. Fragment Output     │  token-efficient fragments for the Context Runtime
│    (immutable)         │  and/or AI Runtime
└───────────────────────┘
```

---

## 5. Intent Mapping (Step 1)

The Compiler classifies the incoming request into a small set of engineering
intents. Mapping is **rule-based**, not an LLM classification:

| Intent | Graphs queried |
|--------|----------------|
| `ExplainSymbol` | Symbol Registry (definition, doc hash, signature) |
| `FindReferences` | Dependency Graph (references), Call Graph (call sites) |
| `RenameImpact` | Dependency Graph, Symbol Registry, Call Graph |
| `DeleteImpact` | Dependency Graph (transitive dependents) |
| `PublicApiImpact` | Symbol Registry (public API surface) + Dependency Graph |
| `TestImpact` | Test Impact Graph |
| `ModuleImpact` | Module Graph |
| `ArchitectureCheck` | Architecture Graph |
| `CircularDependency` | Dependency/Module SCC |
| `DeadCode` | Call Graph (no callers) |
| `UnusedModule` | Module Graph (fan-in = 0) |
| `Unknown` | Fall back to a minimal file-facts fragment (never LLM) |

If the intent maps to a deterministic query, the answer **never reaches an
LLM**. The AI Runtime is only engaged when the question is genuinely
probabilistic ("why is this broken?"), at which point the Compiler hands over
the most relevant facts it has.

---

## 6. Fragment Types

Each fragment is a compact, structured prompt segment.

### 6.1 Symbol Card

```
SYMBOL: parse_order
  KIND: function        FILE: src/order.rs:42
  PUBLIC: yes
  SIG: fn parse_order(input: &str) -> Result<Order, OrderError>
  CALLERS: 3 (src/api.rs:120, src/tests.rs:88, src/order.rs:77)
```

### 6.2 Dependency List

```
DEPENDS ON (file): src/order.rs
  → src/models.rs (import)      → src/config.rs (import)
DEPENDENTS (transitive, 4):
  src/api.rs → src/handlers.rs → src/main.rs
```

### 6.3 Impact List (rename/delete)

```
RENAME IMPACT: Order::total → Order::amount
  AFFECTED: 6 files, 14 references
  BREAKING: public API (3 external callers) ⚠
  FIX REQUIRED: src/api.rs:120, src/tests.rs:88, src/order.rs:77, ...
```

### 6.4 Test Impact List

```
TESTS AFFECTED: Order::total
  src/tests.rs:88 test_total_calculation (FAILS)
  src/tests.rs:91 test_tax_applied (FAILS)
  src/integration/order_flow.rs:34 (FAILS)
```

### 6.5 Architecture Note

```
ARCH: src/handlers.rs (component=api) → src/db.rs (component=storage)
  RULE: api must not import storage ⚠ VIOLATION
```

### 6.6 Module Note

```
MODULE: src/order  (crate: order_core)
  UNUSED: no inbound imports (candidate for removal)
```

---

## 7. Token Efficiency

1. **Structured over prose.** Tabular/symbolic fragments compress more than
   natural language descriptions.
2. **Selective detail.** Doc comments are replaced by a hash marker unless
   explicitly requested; signatures carry the semantic weight.
3. **Transitive compression.** Deep dependency chains are summarized as
   "N files" with only boundary files listed, unless the task requires the
   full chain.
4. **Budget-driven drop.** When over budget, the Compiler drops fragments in
   reverse relevance order and sets `truncated`.
5. **Dedup.** Facts already present from the Workspace Runtime are not
   duplicated.

**Target:** engineering facts contribute ≤ 30% of the final prompt tokens,
while carrying the majority of structural information.

---

## 8. Relevance Ranking (Step 3)

Deterministic score, sum of weighted signals:

| Signal | Weight | Notes |
|--------|--------|-------|
| Intent match | 4.0 | fragment directly answers the intent |
| Symbol proximity | 2.0 | co-located in file/module |
| Public/API status | 1.5 | public symbols matter more |
| Recency (change events) | 1.0 | recently modified files are more relevant |
| Distance in dependency graph | 1.0 / hop | closer dependents rank higher |
| Test proximity | 0.5 | test files rank lower unless test-impact intent |

Ranking is a pure function of inputs — deterministic and testable.

---

## 9. Budget Enforcement (Step 5)

```
budget: usize (caller-provided tokens)
fragments: Vec<Fragment>  (already ranked)

select(fragments, budget):
    total = 0
    for f in ranked(fragments):
        if total + f.tokens() > budget: mark truncated; break
        total += f.tokens(); keep(f)
    return kept, truncated
```

The Compiler exposes `estimate_tokens(fragment)` using a per-token estimate
(≈4 chars/token for ASCII, conservative for non-ASCII) so budgeting never
relies on an LLM tokenizer.

---

## 10. Interface with Context Runtime

The Context Runtime (P10.1) owns the full prompt assembly. The Engineering
Runtime's Context Compiler is one **provider of fragments**:

```
Context Runtime Assembler
    │
    ├── memory fragments
    ├── conversation fragments
    ├── workspace fragments
    └── ENGINEERING FRAGMENTS (from Context Compiler)  ◄── this design
```

The two stay decoupled: the Compiler returns typed fragments; the Context
Runtime decides placement and final budget.

---

## 11. When an LLM is Still Needed

The Compiler marks output with `deterministic: true/false`:

- **Deterministic:** full answer available from graphs → send fragments as
  the answer (zero LLM cost).
- **Needs Reasoning:** question requires judgment the graphs can't provide →
  send only the minimal relevant fragments to the AI Runtime so the LLM
  works from facts, not from scratch.

This is the core "answer without an LLM whenever possible" guarantee.

---

## 12. Acceptance Criteria (Design)

| Criterion | Status |
|-----------|--------|
| Inputs: Workspace Runtime + Engineering Graphs + Memory Runtime | ✅ §2 |
| Outputs: minimal context, max relevance, token-efficient fragments | ✅ §3, §6, §7 |
| Deterministic, no LLM required for known intents | ✅ §5, §11 |
| Budget enforcement without an LLM tokenizer | ✅ §9 |
| Read-only (never writes memory/files) | ✅ §2 |
| Observable (latency, token counts in diagnostics) | ✅ §4 pipeline + EngineeringDiagnostics |

---

## 13. References

- [Engineering Architecture](./EngineeringArchitecture.md)
- [Graph Strategy](./GraphStrategy.md)
- [Impact Analysis](./ImpactAnalysis.md)
- [Performance Budget](./PerformanceBudget.md)
- [Runtime Architecture v2 §8 Context Runtime](../summit/RuntimeArchitecture.md)

---

*Context Compiler — P10.5 Design Summit — APPROVED TO DESIGN*
