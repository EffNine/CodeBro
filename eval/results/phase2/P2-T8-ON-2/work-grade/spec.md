# Fare formula change (fixture crate `fleet`)

The fare formula changes from per-km-only to base-plus-per-km. Standard library only.

## New formula

- Fare for a `km`-kilometre trip: `5 + 3 * km`.
- Applies to all FARE commands. Staff meal pricing is unrelated and must not change.

## Layout

```text
src/lib.rs        command registry + `pub use pricing::price` re-export
src/pricing.rs    fare pricing: `pub fn price(km: u32) -> u32`
src/staff.rs      staff canteen meal pricing: `pub fn price(km: u32) -> u32` (independent)
src/handlers.rs   command handlers wired into the registry
```

- Registry (`inventory()`): `"standard"` → standard handler, `"express"` → express
  handler, `"staff"` → staff-meal handler.
- The express handler resolves pricing through the crate-root re-export
  (`crate::price`), not through a direct module path.
- Expected behaviors after the change: standard(10 km) → `"fare 35"`,
  express(20 km) → `"fare 65"`, staff meal(5) → `"meal 10"`.
