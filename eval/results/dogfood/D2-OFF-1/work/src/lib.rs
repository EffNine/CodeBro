//! Tiny bounded LRU cache (eval fixture). See `spec.md`.
//!
//! NOTES.md describes an earlier bulk-clear approach; it is obsolete. LRU is
//! what the spec actually requires ("Counts as the newest entry", evict the
//! least-recently-used when a new key exceeds `cap`).
use std::collections::{HashMap, VecDeque};

pub struct Cache {
    cap: usize,
    map: HashMap<String, String>,
    order: VecDeque<String>,
    evicted: u64,
}

impl Cache {
    pub fn new(cap: usize) -> Self {
        assert!(cap > 0, "capacity must be nonzero");
        Self { cap, map: HashMap::new(), order: VecDeque::new(), evicted: 0 }
    }

    pub fn put(&mut self, key: String, val: String) {
        if self.map.contains_key(&key) {
            if let Some(pos) = self.order.iter().position(|k| k == &key) {
                self.order.remove(pos);
            }
        } else if self.map.len() >= self.cap {
            let old = self.order.pop_front().expect("order nonempty when map is at capacity");
            self.map.remove(&old);
            self.evicted += 1;
        }
        self.map.insert(key.clone(), val);
        self.order.push_back(key);
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.map.get(key).cloned()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

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
    fn lru_evicts_oldest() {
        let mut c = Cache::new(3);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("c".into(), "3".into());
        c.put("a".into(), "10".into()); // a now newest; b is LRU
        c.put("d".into(), "4".into());   // evicts b
        assert_eq!(c.get("b"), None);
        assert_eq!(c.get("a"), Some("10".to_string()));
        assert_eq!(c.get("c"), Some("3".to_string()));
        assert_eq!(c.get("d"), Some("4".to_string()));
        assert_eq!(c.len(), 3);
        assert_eq!(c.evictions(), 1);
    }

    #[test]
    fn get_does_not_touch_recency() {
        let mut c = Cache::new(2);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        let _ = c.get("a"); // must not make a most-recent
        c.put("c".into(), "3".into()); // evicts a
        assert_eq!(c.get("a"), None);
        assert_eq!(c.get("b"), Some("2".to_string()));
        assert_eq!(c.evictions(), 1);
    }

    #[test]
    fn churn_counts_evictions() {
        let mut c = Cache::new(2);
        for i in 0..100 {
            c.put(format!("k{}", i), i.to_string());
        }
        // 100 distinct keys into cap 2: first two fill, next 98 each evict one
        assert_eq!(c.evictions(), 98);
        assert_eq!(c.len(), 2);
        assert_eq!(c.get("k98"), Some("98".to_string()));
        assert_eq!(c.get("k99"), Some("99".to_string()));
        assert_eq!(c.get("k97"), None);
    }
}
