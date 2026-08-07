#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
pub mod agent;
pub mod communication;
pub mod coordinator;
pub mod decision;
pub mod events;
pub mod experience;
pub mod memory;
pub mod memory_manager;
pub mod performance;
pub mod permissions;
pub mod plan_memory;
pub mod planner;
pub mod recovery;
pub mod reflection;
pub mod resources;
pub mod router;
pub mod skill;
pub mod status;
pub mod subagent;
pub mod task_graph;
pub mod trace;
pub mod workspace;

#[allow(unused_imports)]
pub use communication::{
    AgentMessageBus, CodeChangeProposal, DecisionRequest, Information, MessageChannel,
    MessagePriority, MessageType, PlanningUpdate, RecoveryRequest, ResearchResult, ReviewFeedback,
    ReviewSeverity, StatusUpdate, TestResult,
};
#[allow(unused_imports)]
pub use coordinator::AgentCoordinator;
#[allow(unused_imports)]
pub use decision::{Decision, DecisionContext, DecisionEngine};
#[allow(unused_imports)]
pub use events::{AgentEvent, AgentEventBus, EventHistory, EventSubscriber};
#[allow(unused_imports)]
pub use experience::{
    Experience, ExperienceContext, ExperienceReplay, ExperienceResult, ExperienceStatistics,
};
pub use memory::Memory;
#[allow(unused_imports)]
pub use memory_manager::MemoryConsolidationEngine;
#[allow(unused_imports)]
pub use performance::PerformanceLogger;
#[allow(unused_imports)]
pub use permissions::{PermissionDecision, PermissionLevel, PermissionManager};
#[allow(unused_imports)]
pub use plan_memory::{PlanMemoryStore, PlanRecord};
#[allow(unused_imports)]
pub use planner::CodeIntelligenceInsight;
#[allow(unused_imports)]
pub use planner::Plan;
pub use planner::Planner;
#[allow(unused_imports)]
pub use recovery::{
    FailureEvent, FailureType, RecoveryAction, RecoveryEngine, RecoveryPlan, RecoveryPolicy,
    RetryStats,
};
#[allow(unused_imports)]
pub use reflection::{Reflection, ReflectionEngine, ReflectionStore};
#[allow(unused_imports)]
pub use resources::{
    PerformanceProfile, PriorityLevel, ResourceLimits, ResourceManager, ResourceUsage, TaskPriority,
};
#[allow(unused_imports)]
pub use router::{TaskAnalysis, TaskComplexity, TaskRouter, TaskRouting};
pub use skill::SkillManager;
#[allow(unused_imports)]
pub use skill::SkillStatus;
#[allow(unused_imports)]
pub use status::{AgentState, AgentStatus, AgentStatusMonitor};
#[allow(unused_imports)]
pub use subagent::CodingAgent;
#[allow(unused_imports)]
pub use subagent::PlanningAgent;
#[allow(unused_imports)]
pub use subagent::ResearchAgent;
#[allow(unused_imports)]
pub use subagent::ReviewAgent;
pub use subagent::SubAgent;
#[allow(unused_imports)]
pub use subagent::SubAgentCapability;
#[allow(unused_imports)]
pub use subagent::SubAgentContext;
#[allow(unused_imports)]
pub use subagent::SubAgentResult;
#[allow(unused_imports)]
pub use subagent::TestingAgent;
#[allow(unused_imports)]
pub use task_graph::{TaskGraph, TaskNode, TaskStatus};
#[allow(unused_imports)]
pub use trace::OperationTrace;
pub use trace::TraceStore;
#[allow(unused_imports)]
pub use workspace::{
    get_workspace_path, Workspace, WorkspaceArtifact, WorkspaceInfo, WorkspaceManager,
};
