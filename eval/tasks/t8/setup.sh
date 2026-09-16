#!/usr/bin/env bash
# T8 fixture setup: generate a tiny deterministic `fleet` crate.
#
# The fixture contains the stub implementation (old formula), the spec, plus
# VISIBLE tests only. Held-out hidden tests live in `hidden_tests.rs` next to
# this script and are NEVER copied here — `grade.sh` installs them.
#
# Usage: setup.sh <dest-dir>
set -euo pipefail

dest="${1:?usage: setup.sh <dest-dir>}"
here="$(cd "$(dirname "$0")" && pwd)"
rm -rf "$dest"
mkdir -p "$dest/src" "$dest/tests"

cat > "$dest/Cargo.toml" <<'EOF'
[package]
name = "fleet"
version = "0.1.0"
edition = "2021"
EOF

cp "$here/task.md" "$dest/task.md"
cp "$here/spec.md" "$dest/spec.md"

cat > "$dest/src/lib.rs" <<'EOF'
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
EOF

cat > "$dest/src/pricing.rs" <<'EOF'
//! Fare pricing.
// Old formula: per-km only.
pub fn price(km: u32) -> u32 {
    km * 2
}
EOF

cat > "$dest/src/staff.rs" <<'EOF'
//! Staff canteen meal pricing — unrelated to fare pricing. Do not change
//! this module for the fare formula migration.
pub fn price(km: u32) -> u32 {
    km * 2
}
EOF

cat > "$dest/src/handlers.rs" <<'EOF'
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
EOF

cat > "$dest/tests/steps.rs" <<'EOF'
//! T8 visible tests: registry shape, staff stability, one new-fare pin.
use fleet::{inventory, Ctx};

#[test]
fn step_registry_keys() {
    let mut keys: Vec<&str> = inventory().into_iter().map(|(k, _)| k).collect();
    keys.sort();
    assert_eq!(keys, vec!["express", "staff", "standard"]);
}

#[test]
fn step_staff_meal_unchanged() {
    let ctx = Ctx;
    let meal = inventory().into_iter().find(|(k, _)| *k == "staff").unwrap().1(&ctx);
    assert_eq!(meal, "meal 10");
}

#[test]
fn step_standard_new_fare() {
    let ctx = Ctx;
    let fare = inventory().into_iter().find(|(k, _)| *k == "standard").unwrap().1(&ctx);
    assert_eq!(fare, "fare 35");
}
EOF
