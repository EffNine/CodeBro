//! Fare pricing.
// Formula: base + per-km (5 + 3 * km).
pub fn price(km: u32) -> u32 {
    5 + 3 * km
}
