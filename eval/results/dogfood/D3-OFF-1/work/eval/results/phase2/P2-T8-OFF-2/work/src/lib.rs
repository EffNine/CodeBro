//! Tiny command fleet (eval fixture). See `spec.md`.
pub mod handlers;
pub mod pricing;
pub mod staff;

pub use pricing::price;

pub struct Ctx;

pub fn inventory() -> Vec<(&'static str, fn(&Ctx) -> String)> {
    vec![
        ("standard", handlers::standard as fn(&Ctx) -> String),
        ("express", handlers::express as fn(&Ctx) -> String),
        ("staff", handlers::staff_meal as fn(&Ctx) -> String),
    ]
}
