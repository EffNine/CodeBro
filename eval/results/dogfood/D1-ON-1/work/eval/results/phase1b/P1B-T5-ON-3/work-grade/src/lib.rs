//! Tiny bounded cache (eval fixture). LRU eviction. See `spec.md`.
use std::collections::{HashMap, VecDeque};

pub struct Cache {
    cap: usize,
    map: HashMap<String, String>,
    // Recency chain: front = oldest, back = newest.
    chain: VecDeque<String>,
    evicted: u64,
}

impl Cache {
    pub fn new(cap: usize) -> Self {
        assert!(cap > 0, "capacity must be nonzero");
        Self { cap, map: HashMap::new(), chain: VecDeque::new(), evicted: 0 }
    }

    pub fn put(&mut self, key: String, val: String) {
        if self.map.contains_key(&key) {
            // Replace + mark newest. Never evicts.
            *self.map.get_mut(&key).unwrap() = val;
            let pos = self.chain.iter().position(|k| k == &key).unwrap();
            let k = self.chain.remove(pos).unwrap();
            self.chain.push_back(k);
            return;
        }
        // New key: make room if at capacity (evict oldest first).
        if self.map.len() >= self.cap {
            let oldest = self.chain.pop_front().unwrap();
            self.map.remove(&oldest);
            self.evicted += 1;
        }
        self.map.insert(key.clone(), val);
        self.chain.push_back(key);
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
    fn lru_eviction_order() {
        let mut c = Cache::new(3);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("c".into(), "3".into());
        // Update b so it becomes newest: order is a, c, b.
        c.put("b".into(), "2x".into());
        // Add d: evict oldest = a.
        c.put("d".into(), "4".into());
        assert_eq!(c.get("a"), None);
        assert_eq!(c.get("b"), Some("2x".to_string()));
        assert_eq!(c.get("d"), Some("4".to_string()));
        assert_eq!(c.evictions(), 1);
        // Add e: evict oldest = c.
        c.put("e".into(), "5".into());
        assert_eq!(c.get("c"), None);
        assert_eq!(c.get("e"), Some("5".to_string()));
        assert_eq!(c.get("d"), Some("4".to_string()));
        assert_eq!(c.evictions(), 2);
    }

    #[test]
    fn update_does_not_evict_even_when_full() {
        let mut c = Cache::new(2);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("b".into(), "3".into()); // replace, no eviction
        assert_eq!(c.get("a"), Some("1".to_string()));
        assert_eq!(c.get("b"), Some("3".to_string()));
        assert_eq!(c.evictions(), 0);
    }

    #[test]
    fn churn_counts_each_eviction() {
        let mut c = Cache::new(2);
        for i in 0..100u64 {
            c.put(format!("k{i}"), i.to_string());
        }
        assert_eq!(c.len(), 2);
        assert_eq!(c.evictions(), 98);
        assert_eq!(c.get("k98"), Some("98".to_string()));
        assert_eq!(c.get("k99"), Some("99".to_string()));
        assert_eq!(c.get("k97"), None);
    }
}
