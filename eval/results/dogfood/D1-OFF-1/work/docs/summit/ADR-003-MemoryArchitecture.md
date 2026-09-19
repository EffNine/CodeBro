# ADR-003: Memory Architecture

**ADR Number:** ADR-003
**Title:** Memory Architecture
**Author:** CodeBro Engineering
**Status:** Proposed
**Created:** 2026-08-07
**Updated:** 2026-08-07
**Part of:** Design Summit v2
**Supersedes:** None
**Related:** ADR-001, ADR-002, ADR-004

---

## 1. Context

### 1.1 Background

The v1.0 Memory system (`src/agent/memory.rs`) provides basic JSON persistence:

```rust
pub struct Memory {
    pub short_term: Vec<MemoryEntry>,
    pub project: ProjectMemory,
    pub global: GlobalMemory,
    pub sessions: Vec<Session>,
    pub current_session_id: Option<String>,
}
```

Storage locations:
- Global: `~/.codebro/memory.json`
- Project: `~/.codebro/project_memory.json`

### 1.2 Problem

The v1.0 memory system lacks:

1. **Explicit tiers** — Short-term, project, and global are in one struct
2. **Eviction policy** — Short-term uses simple truncation (remove oldest)
3. **Summarization** — No way to compress old memory
4. **Separation of concerns** — Memory is coupled to agent module
5. **Persistence granularity** — All-or-nothing save

### 1.3 Constraints

- Existing memory format must be migratable
- JSON persistence format is frozen (deterministic output)
- No embedding models or AI-based summarization
- Memory must remain read-only from intelligence layer

### 1.4 Stakeholders

- **Agent** — Reads and writes memory
- **Context Runtime** — Reads memory for context assembly
- **AI Runtime** — Consumes memory-augmented context
- **TUI** — Displays memory status

---

## 2. Decision

### 2.1 Decision Statement

The Memory Runtime adopts a **multi-tier, policy-driven, explicitly managed** architecture. Memory is separated into three independent tiers (short-term, project, global) with distinct retention and eviction policies. The existing Memory struct is migrated to the new tier system.

### 2.2 Rationale

1. **Tiered memory** matches natural knowledge lifetimes
2. **Policy-driven eviction** ensures determinism
3. **Explicit management** makes persistence auditable
4. **Separation from agent** enables reuse by context runtime
5. **Backward compatibility** preserves existing user data

### 2.3 Principles Applied

- **Principle 7 (Modular Architecture)** — Memory is a distinct runtime module
- **Principle 8 (Observable AI Actions)** — Memory operations are logged
- **Principle 10 (Small, Composable Components)** — Each tier is independently managed

---

## 3. Architecture

### 3.1 Memory Runtime Module

```
src/runtime/memory/
├── mod.rs              # Module assembly
├── tiers.rs            # Tier definitions and management
├── evictor.rs          # Deterministic eviction policy
├── summarizer.rs       # Context summarization (rule-based)
└── persistence.rs      # JSON persistence layer
```

### 3.2 Tier Definitions

| Tier | Scope | Storage | Retention | Eviction |
|------|-------|---------|-----------|----------|
| Short-term | Session | `~/.codebro/sessions/{id}/messages.json` | Until session ends | LRU, max 100 entries |
| Project | Project | `~/.codebro/projects/{project_id}/memory.json` | Until project change | Importance-weighted, max 500 entries |
| Global | System | `~/.codebro/memory/global.json` | Indefinite | Confidence threshold (< 0.3), max 1000 entries |

### 3.3 Memory Store Trait

```rust
pub trait MemoryStore: Send + Sync {
    /// Save memory entries for a scope.
    async fn save(&self, scope: MemoryScope, entries: Vec<MemoryEntry>) -> Result<()>;

    /// Load memory entries for a scope.
    async fn load(&self, scope: MemoryScope) -> Result<Vec<MemoryEntry>>;

    /// Search memory with a query.
    async fn search(&self, query: &str, scope: MemoryScope, limit: usize) -> Result<Vec<MemoryEntry>>;

    /// Evict entries based on policy.
    async fn evict(&self, scope: MemoryScope, policy: EvictionPolicy) -> Result<Vec<MemoryKey>>;

    /// Get memory statistics.
    async fn stats(&self, scope: MemoryScope) -> Result<MemoryStats>;
}
```

### 3.4 Memory Entry

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub scope: MemoryScope,
    pub entry_type: MemoryType,
    pub content: String,
    pub importance: f32,      // 0.0 to 1.0
    pub confidence: f32,      // 0.0 to 1.0
    pub timestamp: String,
    pub usage_count: u32,
    pub last_used: Option<String>,
    pub metadata: HashMap<String, String>,
}

pub enum MemoryScope {
    Session(String),
    Project(String),
    Global,
}

pub enum MemoryType {
    Message { user_input: String, response: String },
    File { path: String, summary: String },
    Command { command: String, output: String },
    Plan { plan: String },
    Decision { context: String, decision: String, rationale: String },
    Lesson { lesson: String, context: String },
    Solution { problem: String, solution: String },
    Custom { typ: String, data: String },
}
```

### 3.5 Eviction Policies

| Policy | Description | Applied To |
|--------|-------------|------------|
| LRU | Remove least recently used | Short-term |
| Low Importance | Remove entries with importance < threshold | Project |
| Low Confidence | Remove entries with confidence < 0.3 | Global |
| Capacity | Remove oldest when at max size | All tiers |
| Age | Remove entries older than threshold | Global |

### 3.6 Summarization

Rule-based summarization (no AI):

```rust
pub struct Summarizer;

impl Summarizer {
    /// Summarize a group of entries by extracting key patterns.
    pub fn summarize(&self, entries: &[MemoryEntry], max_length: usize) -> String {
        // Extract unique patterns
        // Count frequency
        // Return top patterns condensed
    }
}
```

---

## 4. Migration from v1.0

### 4.1 Data Migration

```
v1.0 Format                          v2.0 Format
─────────────────────────────────────────────────────────────────
memory.json                          sessions/{id}/messages.json
├── short_term: [...]       →        └── entries: [...]
├── project: {...}               →    projects/{project}/memory.json
│   ├── recent_files               ├── entries: [...]
│   ├── tasks                      └── stats: {...}
│   └── preferences              →    global.json
├── global: {...}                  ├── entries: [...]
│   ├── skills                     └── stats: {...}
│   ├── reflections
│   └── lessons
└── sessions: [...]
    └── current_session_id
```

### 4.2 Migration Script

```rust
pub fn migrate_v1_to_v2(v1_memory: Memory) -> Result<MemoryV2> {
    let mut v2 = MemoryV2::default();

    // Migrate short-term to session storage
    for entry in v1_memory.short_term {
        v2.add_entry(
            MemoryScope::Session(
                v1_memory.current_session_id.clone().unwrap_or_default()
            ),
            entry.into_v2_entry(),
        );
    }

    // Migrate project memory
    v2.save_project_memory(
        v1_memory.project,
        detect_project_id(),
    );

    // Migrate global memory
    v2.save_global_memory(v1_memory.global);

    Ok(v2)
}
```

### 4.3 Compatibility

- v1.0 memory format is read during migration
- v2.0 writes new format
- Old format is preserved as backup for one release cycle

---

## 5. Integration with Other Runtimes

### 5.1 Context Runtime Integration

```rust
// Context runtime reads from memory
let relevant_entries = memory
    .search(&query, MemoryScope::Project(project_id), 20)
    .await?;

// Entries are included in context assembly
let context = context_assembler.build(&relevant_entries)?;
```

### 5.2 Agent Integration

```rust
// Agent writes to memory
memory.add_entry(
    MemoryScope::Session(session_id.clone()),
    MemoryEntry::message(user_input, response),
).await?;

// Agent reads from memory
let history = memory.load(MemoryScope::Session(session_id)).await?;
```

### 5.3 Event Emissions

| Event | Trigger |
|-------|---------|
| `MemorySaved(scope)` | Save succeeds |
| `MemoryLoaded(scope)` | Load succeeds |
| `MemoryEvicted(scope, keys)` | Eviction runs |
| `MemorySearch(query, results)` | Search completes |

---

## 6. Consequences

### 6.1 Positive Consequences

- Clear separation of memory concerns
- Deterministic eviction prevents data loss surprises
- Migration preserves existing user data
- Context runtime can reuse memory without agent coupling

### 6.2 Negative Consequences

- Migration adds startup complexity
- New storage locations require directory creation
- Some v1.0 memory is lost if migration fails

### 6.3 Trade-offs

| Aspect | Trade-off | Mitigation |
|--------|-----------|------------|
| Migration | Startup delay | Run asynchronously, log progress |
| Storage layout | More files | Organized by scope |
| Summarization | Rule-based only | Acceptable for v2; AI in v3 |
| Backward compat | Dual format support | One release cycle only |

---

## 7. Alternatives Considered

| Alternative | Description | Pros | Cons | Why Rejected |
|-------------|-------------|------|------|--------------|
| In-place upgrade | Modify Memory struct directly | Simple | Breaks format compatibility | Data loss risk |
| SQLite backend | Relational storage | Queryable | New dependency | Violates no-new-deps |
| Embedding-based search | Semantic search | Better relevance | AI dependency | Out of scope for v2 |
| Single storage | One file for all memory | Simple | No tiering | Violates tier principle |

---

## 8. Implementation Notes

### 8.1 Code Patterns

```rust
// Load memory
let memory = MemoryRuntime::load().await?;

// Save memory
memory.save().await?;

// Search memory
let results = memory.search("rust error handling", MemoryScope::Project, 10).await?;

// Evict old entries
memory.evict(MemoryScope::Global, EvictionPolicy::LowConfidence).await?;
```

### 8.2 Anti-Patterns

```rust
// NEVER: Direct file manipulation
fs::write("memory.json", content)?;

// ALWAYS: Use MemoryRuntime
memory.save().await?;
```

### 8.3 Persistence Format

```json
{
  "version": "2.0.0",
  "scope": "project",
  "project_id": "codebro",
  "entries": [
    {
      "id": "uuid-v4",
      "type": "lesson",
      "content": "Use Result for error handling",
      "importance": 0.8,
      "confidence": 0.9,
      "timestamp": "2026-08-07T12:00:00Z",
      "usage_count": 5,
      "last_used": "2026-08-07T12:00:00Z"
    }
  ],
  "stats": {
    "total_entries": 1,
    "total_tokens_estimate": 50
  }
}
```

---

## 9. References

- [ADR-001: Runtime Architecture](./ADR-001-RuntimeArchitecture.md)
- [Memory Contract](../contracts/memory_contract.md)
- [Runtime Architecture](../summit/RuntimeArchitecture.md)
- [Memory Principles](../summit/RuntimePrinciples.md) §4

---

## 10. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-07 | Created | CodeBro Engineering |
