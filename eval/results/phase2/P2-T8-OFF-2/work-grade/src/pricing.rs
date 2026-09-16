//! Fare pricing.
// New formula: base-plus-per-km.
pub fn price(km: u32) -> u32 {
    5 + 3 * km
}
