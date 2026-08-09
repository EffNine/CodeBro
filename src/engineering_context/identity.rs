//! Project identity — re-exported from `project_identity`.
//!
//! `EngineeringContext` depends on `ProjectIdentity` from the
//! `project_identity` module. This file exists for backward
//! compatibility with existing import paths.

pub use crate::project_identity::identity::{
    CURRENT_SCHEMA_VERSION, DecisionStatus, EngineeringDecision, ProjectIdentity,
    RoadmapItem, RoadmapStatus,
};
