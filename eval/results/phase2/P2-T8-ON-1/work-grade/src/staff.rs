//! Staff canteen meal pricing — unrelated to fare pricing. Do not change
//! this module for the fare formula migration.
pub fn price(km: u32) -> u32 {
    km * 2
}
