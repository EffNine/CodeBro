# ADR-004: Agent Architecture

**ADR Number:** ADR-004
**Title:** Agent Architecture
**Author:** CodeBro Engineering
**Status:** Proposed
**Created:** 2026-08-07
**Updated:** 2026-08-07
**Part of:** Design Summit v2
**Supersedes:** None
**Related:** ADR-001, ADR-002, ADR-003

---

## 1. Context

### 1.1 Background

The v1.0 Agent system (`src/agent/`) provides:

- `AgentCoordinator` — Orchestrates subagent execution
- `SubAgent` trait — Interface for agent implementations
- `AgentMessageBus` — Inter-agent communication
- Built-in agents: Research, Planning, Coding, Testing, Review

The coordinator runs agents sequentially with a task graph for dependency tracking.

### 1.2 Problem

The v1.0 agent system lacks:

1. **Parallel execution** — Agents run sequentially, not in parallel
2. **Resource management** — No limits on concurrent agents
3. **Lifecycle management** — Agents are spawned but not tracked
4. **Advanced communication** — Basic message bus, no channels or groups
5. **Observability** — Limited agent-level metrics

### 1.3 Constraints

- SubAgent trait is frozen — cannot modify the interface
- AgentCoordinator must continue to work
- Communication must use existing AgentMessageBus where possible
- Resource limits must not block legitimate execution

### 1.4 Stakeholders

- **AI Runtime** — Spawns agents for task decomposition
- **Integration Pipeline** — Uses agents for reasoning phase
- **TUI** — Displays agent status and messages
- **Plugin SDK** — Enables plugin-provided agents

---

## 2. Decision

### 2.1 Decision Statement

The Agent Runtime wraps the frozen AgentCoordinator and SubAgent trait with orchestration capabilities: parallel execution, resource management, lifecycle tracking, and enhanced communication. The existing coordinator is preserved; the runtime adds infrastructure around it.

### 2.2 Rationale

1. **Wrap, don't replace** — Preserves v1.0 compatibility
2. **Parallel execution** — Improves throughput on independent tasks
3. **Resource management** — Prevents overload
4. **Lifecycle tracking** — Enables debugging and recovery
5. **Enhanced communication** — Supports complex agent interactions

### 2.3 Principles Applied

- **Principle 7 (Modular Architecture)** — Agent runtime is a distinct module
- **Principle 8 (Observable AI Actions)** — Agent events are emitted
- **Principle 10 (Small, Composable Components)** — Agents are focused and composable

---

## 3. Architecture

### 3.1 Agent Runtime Module

```
src/runtime/agent/
├── mod.rs              # Module assembly
├── orchestrator.rs     # Multi-agent orchestration
├── communication.rs    # Enhanced messaging
├── lifecycle.rs        # Agent lifecycle management
└── resource.rs         # Resource management
```

### 3.2 Agent Runtime Trait

```rust
pub trait AgentRuntime: Send + Sync {
    /// Register a new agent type.
    fn register_agent(&mut self, agent: Box<dyn SubAgent>);

    /// Spawn an agent for a task.
    async fn spawn(
        &mut self,
        agent_id: &str,
        task: &str,
        context: AgentContext,
    ) -> Result<AgentHandle>;

    /// Spawn multiple agents in parallel.
    async fn spawn_parallel(
        &mut self,
        agents: Vec<AgentSpawnRequest>,
    ) -> Result<Vec<AgentHandle>>;

    /// Wait for agent completion.
    async fn wait(&self, handle: &AgentHandle) -> Result<AgentResult>;

    /// Wait for all agents to complete.
    async fn wait_all(&self, handles: &[AgentHandle]) -> Result<Vec<AgentResult>>;

    /// Get agent status.
    fn status(&self, agent_id: &str) -> AgentStatus;

    /// Get all agent statuses.
    fn all_statuses(&self) -> HashMap<String, AgentStatus>;

    /// Send message between agents.
    async fn send_message(&self, message: AgentMessage);

    /// Get message history.
    async fn message_history(&self, limit: usize) -> Vec<AgentMessage>;

    /// Shutdown all agents gracefully.
    async fn shutdown(&mut self);

    /// Get resource usage.
    fn resource_usage(&self) -> ResourceUsage;
}
```

### 3.3 Agent Lifecycle States

```
┌──────────┐   spawn()    ┌──────────┐   start()   ┌──────────┐
│  Created │ ──────────→ │ Pending  │ ──────────→ │ Running  │
└──────────┘              └──────────┘            └────┬─────┘
                                                       │
                    ┌────────────────────────────────────┘
                    │
               complete()│                    fail()│
                    ▼                              ▼
            ┌──────────┐                    ┌──────────┐
            │Completed │                    │  Failed  │
            └────┬─────┘                    └────┬─────┘
                 │                               │
                 │ cleanup()                     │ cleanup()
                 ▼                               ▼
            ┌──────────┐                    ┌──────────┐
            │  Done    │                    │  Done    │
            └──────────┘                    └──────────┘
```

### 3.4 Parallel Execution Model

```
Task: "Implement feature X"
    │
    ├──→ Agent 1 (Research): "Research existing implementations"
    │       └──→ completes → result 1
    │
    ├──→ Agent 2 (Planning): "Plan implementation"
    │       └──→ completes → result 2
    │
    └──→ Agent 3 (Review): "Review existing code"
            └──→ completes → result 3

    ↓ (all complete)
    
    Agent 4 (Coding): "Implement feature X"
    └──→ completes → result 4
```

### 3.5 Resource Management

```rust
pub struct ResourceLimits {
    pub max_concurrent_agents: usize,
    pub max_memory_per_agent: usize,
    pub max_duration_per_agent: Duration,
    pub total_budget: Option<f64>,
}

pub struct ResourceUsage {
    pub active_agents: usize,
    pub pending_agents: usize,
    pub total_memory: usize,
    pub elapsed_time: Duration,
}
```

### 3.6 Communication Channels

| Channel Type | Description | Use Case |
|-------------|-------------|----------|
| Public | Broadcast to all agents | Status updates |
| Direct | Point-to-point message | Coordination |
| Group | Message to named group | Team collaboration |
| Request/Reply | Synchronous request with response | Decision queries |

---

## 4. Integration with v1.0

### 4.1 AgentCoordinator Compatibility

The v1.0 AgentCoordinator is wrapped, not replaced:

```rust
// v1.0: Direct usage
let mut coordinator = AgentCoordinator::new(4);
let report = coordinator.run_task(task, Some(project_root), &emit).await;

// v2.0: Through runtime
runtime.register_agent(Box::new(coordinator));
let handle = runtime.spawn("coordinator", task, context).await?;
let result = runtime.wait(&handle).await?;
```

### 4.2 SubAgent Compatibility

Built-in agents continue to implement SubAgent:

```rust
// These remain unchanged
pub struct ResearchAgent { ... }
impl SubAgent for ResearchAgent { ... }

pub struct PlanningAgent { ... }
impl SubAgent for PlanningAgent { ... }

pub struct CodingAgent { ... }
impl SubAgent for CodingAgent { ... }

pub struct TestingAgent { ... }
impl SubAgent for TestingAgent { ... }

pub struct ReviewAgent { ... }
impl SubAgent for ReviewAgent { ... }
```

### 4.3 MessageBus Compatibility

The existing AgentMessageBus is used as the underlying transport:

```rust
pub struct AgentRuntime {
    coordinator: AgentCoordinator,
    message_bus: AgentMessageBus,
    // ...
}
```

---

## 5. Consequences

### 5.1 Positive Consequences

- Parallel execution improves throughput
- Resource limits prevent overload
- Lifecycle tracking enables debugging
- Enhanced communication supports complex workflows

### 5.2 Negative Consequences

- Additional abstraction layer
- Parallel execution requires careful synchronization
- Resource tracking adds overhead

### 5.3 Trade-offs

| Aspect | Trade-off | Mitigation |
|--------|-----------|------------|
| Parallelism | Race conditions | Mutex-protected shared state |
| Resource tracking | Overhead | Sampling-based tracking |
| Lifecycle states | Complexity | Clear state machine |
| Backward compat | Wrapper layer | Thin wrapper, delegate to coordinator |

---

## 6. Alternatives Considered

| Alternative | Description | Pros | Cons | Why Rejected |
|-------------|-------------|------|------|--------------|
| Replace coordinator | New agent system | Clean design | Breaking change | Frozen API |
| No parallelism | Keep sequential | Simple | Slow on multi-step tasks | Poor UX |
| Thread-per-agent | One thread per agent | Simple | Resource intensive | No limits |
| Actor model | Full actor system | Flexible | Heavy dependency | Out of scope |

---

## 7. Implementation Notes

### 7.1 Code Patterns

```rust
// Register agents
runtime.register_agent(Box::new(ResearchAgent::new()));
runtime.register_agent(Box::new(PlanningAgent::new()));
runtime.register_agent(Box::new(CodingAgent::new()));

// Spawn in parallel
let handles = runtime.spawn_parallel(vec![
    AgentSpawnRequest::new("research", task, context.clone()),
    AgentSpawnRequest::new("planning", task, context.clone()),
]).await?;

// Wait for all
let results = runtime.wait_all(&handles).await?;

// Send message
runtime.send_message(AgentMessage::new(
    "research", "coordinator", MessageType::ResearchResult(result),
)).await;
```

### 7.2 Anti-Patterns

```rust
// NEVER: Spawn without tracking
tokio::spawn(async { agent.execute().await });

// ALWAYS: Use runtime spawn
let handle = runtime.spawn("agent_id", task, context).await?;
```

### 7.3 Event Emissions

| Event | Trigger |
|-------|---------|
| `AgentSpawned(id)` | Agent spawned |
| `AgentStarted(id)` | Agent begins execution |
| `AgentCompleted(id, result)` | Agent finishes successfully |
| `AgentFailed(id, error)` | Agent fails |
| `AgentMessage(from, to, type)` | Message sent |
| `AgentStatusChanged(id, status)` | Status changes |

---

## 8. References

- [ADR-001: Runtime Architecture](./ADR-001-RuntimeArchitecture.md)
- [Agent Module](../../src/agent/mod.rs)
- [Coordinator](../../src/agent/coordinator.rs)
- [SubAgent Trait](../../src/agent/subagent/trait_agent.rs)
- [Runtime Architecture](../summit/RuntimeArchitecture.md)
- [Agent Principles](../summit/RuntimePrinciples.md) §6

---

## 9. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-07 | Created | CodeBro Engineering |
