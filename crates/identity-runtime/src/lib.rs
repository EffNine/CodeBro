//! CodeBro identity runtime — persistent project identity.
//!
//! Medium-high-trust "declared intent" store: project description,
//! constraints, engineering decisions, roadmap, conventions. Distinct from
//! agent-recorded memory (low trust) and verified facts (high trust).

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

pub mod project_identity;
