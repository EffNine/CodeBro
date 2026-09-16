//! Tiny bounded LRU cache. Evictions drop the least-recently-used entry one
//! at a time. The old bulk-clear approach documented in NOTES.md (2026-02)
//! was rejected in 2026-08 (never mass-delete); it kept here only as history.
//! See `spec.md` for the public API.
use std::collections::{HashMap, VecDeque};

pub struct Cache {
    cap: usize,
    map: HashMap<String, String>,
    recency: VecDeque<String>,
    evicted: u64,
}

impl Cache {
    pub fn new(cap: usize) -> Self {
        assert!(cap > 0, "capacity must be nonzero");
        Self {
            cap,
            map: HashMap::new(),
            recency: VecDeque::new(),
            evicted: 0,
        }
    }

    pub fn put(&mut self, key: String, val: String) {
        if self.map.contains_key(&key) {
            self.map.insert(key.clone(), val);
            self.mark_newest(&key);
            return;
        }
        while self.recency.len() >= self.cap {
            if let Some(dropped) = self.recency.pop_front() {
                self.map.remove(&dropped);
                self.evicted += 1;
            }
        }
        self.map.insert(key.clone(), val);
        self.recency.push_back(key);
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

    fn mark_newest(&mut self, key: &str) {
        if let Some(pos) = self.recency.iter().position(|k| k == key) {
            let item = self.recency.remove(pos).unwrap();
            self.recency.push_back(item);
        }
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
    fn lru_eviction_drops_oldest() {
        let mut c = Cache::new(3);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("c".into(), "3".into());
        c.put("d".into(), "4".into());
        assert_eq!(c.get("a"), None);
        assert_eq!(c.get("b"), Some("2".to_string()));
        assert_eq!(c.get("c"), Some("3".to_string()));
        assert_eq!(c.get("d"), Some("4".to_string()));
        assert_eq!(c.len(), 3);
        assert_eq!(c.evictions(), 1);
    }

    #[test]
    fn update_moves_to_newest() {
        let mut c = Cache::new(3);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("c".into(), "3".into());
        c.put("a".into(), "11".into()); // a becomes newest: [b, c, a]
        c.put("d".into(), "4".into()); // evicts b -> [c, a, d]
        assert_eq!(c.get("a"), Some("11".to_string()));
        assert_eq!(c.get("b"), None);
        assert_eq!(c.get("c"), Some("3".to_string()));
        assert_eq!(c.get("d"), Some("4".to_string()));
        assert_eq!(c.evictions(), 1);
        c.put("e".into(), "5".into()); // evicts c -> [a, d, e]
        assert_eq!(c.get("c"), None);
        assert_eq!(c.evictions(), 2);
    }

    #[test]
    fn get_does_not_affect_recency() {
        let mut c = Cache::new(3);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("c".into(), "3".into());
        assert_eq!(c.get("a"), Some("1".to_string())); // read only
        c.put("d".into(), "4".into()); // evicts a (still oldest)
        assert_eq!(c.get("a"), None);
        assert_eq!(c.get("b"), Some("2".to_string()));
        assert_eq!(c.evictions(), 1);
    }

    #[test]
    fn churn_many_puts_counts_evictions() {
        let mut c = Cache::new(10);
        for i in 0..100 {
            c.put(format!("k{i}"), format!("v{i}"));
        }
        assert_eq!(c.len(), 10);
        assert_eq!(c.evictions(), 90);
        for i in 90..100 {
            assert_eq!(c.get(&format!("k{i}")), Some(format!("v{i}")));
        }
        for i in 0..90 {
            assert_eq!(c.get(&format!("k{i}")), None);
        }
    }

    #[test]
    fn large_scale_churn() {
        let mut c = Cache::new(1_000_000);
        let n = 5_000_000usize;
        let _ = c.get("");
        for i in 0..n {
            c.put(format!("key-{i}"), format!("value-{i}"));
        }
        assert_eq!(c.len(), 1_000_000);
        assert_eq!(c.evictions(), (n - 1_000_000) as u64);
        assert_eq!(c.get(&format!("key-{}", n - 1)), Some(format!("value-{}", n - 1)));
        // key-4000000 is exactly the oldest surviving entry (the boundary)
        assert_eq!(c.get(&format!("key-{}", n - 1_000_000)), Some(format!("value-{}", n - 1_000_000)));
        // the entry just before the boundary was evicted
        assert_eq!(c.get(&format!("key-{}", n - 1_000_000 - 1)), None);
        assert_eq!(c.get("key-0"), None);
    }

    #[test]
    fn replace_then_churn() {
        let mut c = Cache::new(2);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("a".into(), "3".into()); // a newest; evictions still 0
        c.put("c".into(), "4".into()); // evicts b
        assert_eq!(c.evictions(), 1);
        c.put("d".into(), "5".into()); // evicts a? no: a was newest, c is newer now? order: c newest, then a
        assert_eq!(c.evictions(), 2);
        assert_eq!(c.get("a"), None);
        assert_eq!(c.get("b"), None);
        assert_eq!(c.get("c"), Some("4".to_string()));
        assert_eq!(c.get("d"), Some("5".to_string()));
        assert_eq!(c.len(), 2);
    }

    #[test]
    #[should_panic]
    fn zero_capacity_panics() {
        Cache::new(0);
    }
}
