//! Tiny bounded cache (eval fixture). See `spec.md`.
//!
//! Eviction (per this codebase's documented approach): when a `put` of a
//! NEW key would exceed capacity, the cache clears the entire map, then
//! inserts the new entry. `get` never affects recency. `evictions()`
//! counts every entry dropped by a capacity-triggered clear, from creation.

use std::collections::HashMap;

pub struct Cache {
    cap: usize,
    map: HashMap<String, String>,
    evicted: u64,
}

impl Cache {
    /// Create an empty cache holding at most `cap` entries. Panics if `cap == 0`.
    pub fn new(cap: usize) -> Self {
        assert!(cap > 0, "capacity must be nonzero");
        Self { cap, map: HashMap::new(), evicted: 0 }
    }

    /// Insert or replace `key`. Counts as the newest entry.
    pub fn put(&mut self, key: String, val: String) {
        if self.map.contains_key(&key) {
            // Replace: no eviction, no counter change.
            self.map.insert(key, val);
            return;
        }
        if self.map.len() >= self.cap {
            // Full: drop every stored entry (bulk clear), count them.
            self.evicted += self.map.len() as u64;
            self.map.clear();
        }
        self.map.insert(key, val);
    }

    /// Look up `key`. Does not affect recency.
    pub fn get(&self, key: &str) -> Option<String> {
        self.map.get(key).cloned()
    }

    /// Number of entries currently stored.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Total entries removed due to capacity since creation.
    pub fn evictions(&self) -> u64 {
        self.evicted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_put_get() {
        let mut c = Cache::new(4);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        assert_eq!(c.get("a"), Some("1".to_string()));
        assert_eq!(c.get("zz"), None);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn visible_update_no_evict() {
        let mut c = Cache::new(2);
        c.put("a".into(), "1".into());
        c.put("a".into(), "2".into());
        assert_eq!(c.get("a"), Some("2".to_string()));
        assert_eq!(c.len(), 1);
        assert_eq!(c.evictions(), 0);
    }

    #[test]
    fn full_put_clears_then_inserts_new_only() {
        let mut c = Cache::new(2);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("c".into(), "3".into()); // full -> bulk clear, then insert c
        assert_eq!(c.get("a"), None);
        assert_eq!(c.get("b"), None);
        assert_eq!(c.get("c"), Some("3".to_string()));
        assert_eq!(c.len(), 1);
        assert_eq!(c.evictions(), 2);
    }

    #[test]
    fn updates_dont_evict_even_when_full() {
        let mut c = Cache::new(2);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("a".into(), "x".into()); // replace, no eviction
        assert_eq!(c.len(), 2);
        assert_eq!(c.evictions(), 0);
        assert_eq!(c.get("a"), Some("x".to_string()));
    }

    #[test]
    fn chained_full_puts_accumulate_evictions() {
        let mut c = Cache::new(1);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into()); // clears a, inserts b
        c.put("c".into(), "3".into()); // clears b, inserts c
        assert_eq!(c.get("a"), None);
        assert_eq!(c.get("b"), None);
        assert_eq!(c.get("c"), Some("3".to_string()));
        assert_eq!(c.len(), 1);
        assert_eq!(c.evictions(), 2);
    }

    #[test]
    fn cap3_partial_refill_after_clear() {
        let mut c = Cache::new(3);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("c".into(), "3".into());
        c.put("d".into(), "4".into()); // full -> clears a,b,c -> d
        assert_eq!(c.get("d"), Some("4".to_string()));
        assert_eq!(c.get("a"), None);
        c.put("e".into(), "5".into()); // not full: map = {d,e}
        c.put("f".into(), "6".into()); // not full: map = {d,e,f}
        assert_eq!(c.get("f"), Some("6".to_string()));
        assert_eq!(c.get("d"), Some("4".to_string()));
        assert_eq!(c.len(), 3);
        assert_eq!(c.evictions(), 3);
    }

    #[test]
    fn cap1_churn_is_pure_churn() {
        let mut c = Cache::new(1);
        for i in 0..100 {
            c.put(format!("k{}", i), format!("v{}", i));
        }
        // Every put except the first cleared 1 entry: 99 evictions, last key kept.
        assert_eq!(c.get("k99"), Some("v99".to_string()));
        assert_eq!(c.get("k98"), None);
        assert_eq!(c.len(), 1);
        assert_eq!(c.evictions(), 99);
    }

    #[test]
    fn zero_cap_panics() {
        std::panic::catch_unwind(|| {
            let _ = Cache::new(0);
        })
        .unwrap_err();
    }
}
