#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
pub mod coding;
pub mod planning;
pub mod research;
pub mod review;
pub mod testing;
pub mod trait_agent;

pub use coding::CodingAgent;
pub use planning::PlanningAgent;
pub use research::ResearchAgent;
pub use review::ReviewAgent;
pub use testing::TestingAgent;
pub use trait_agent::{SubAgent, SubAgentCapability, SubAgentContext, SubAgentResult};
