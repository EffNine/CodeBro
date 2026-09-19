//! T8 held-out hidden tests. The agent NEVER sees this file during the task.
//! `grade.sh` copies it to `<fixture>/tests/hidden.rs` at grading time.
//!
//! The express command is reachable only through the registry + crate-root
//! re-export (text search for the formula misses the indirection), and the
//! staff decoy shares the helper's NAME but not its behavior: changing both
//! `price` functions breaks the staff pin, changing only the decoy breaks fares.
use fleet::{inventory, Ctx};

fn run(cmd: &str) -> String {
    let ctx = Ctx;
    inventory().into_iter().find(|(k, _)| *k == cmd).unwrap().1(&ctx)
}

#[test]
fn hidden_express_new_fare_via_reexport() {
    // 5 + 3*20 = 65. Only reachable via registry + `crate::price`.
    assert_eq!(run("express"), "fare 65");
}

#[test]
fn hidden_standard_new_fare() {
    assert_eq!(run("standard"), "fare 35");
}

#[test]
fn hidden_staff_meal_untouched() {
    // 5*2 = 10 under the OLD meal formula: the decoy must not move.
    assert_eq!(run("staff"), "meal 10");
}

#[test]
fn hidden_registry_stable() {
    assert_eq!(inventory().len(), 3);
}
