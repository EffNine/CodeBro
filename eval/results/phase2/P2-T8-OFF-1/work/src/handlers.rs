//! Command handlers wired into the registry in lib.rs.
use super::Ctx;

pub fn standard(_ctx: &Ctx) -> String {
    format!("fare {}", super::pricing::price(10))
}

pub fn express(_ctx: &Ctx) -> String {
    // Resolves pricing through the crate-root re-export.
    format!("fare {}", crate::price(20))
}

pub fn staff_meal(_ctx: &Ctx) -> String {
    format!("meal {}", super::staff::price(5))
}
