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
