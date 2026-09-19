//! Tiny bounded cache (eval fixture). Bounded LRU eviction per `spec.md`.
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
            self.order.retain(|k| k != &key);
            self.order.push_back(key.clone());
            self.map.insert(key, val);
            return;
        }
        if self.map.len() >= self.cap {
            // Bounded LRU: drop the least-recently-used entry to make room.
            let evicted_key = self.order.pop_front().expect("order never empty while full");
            self.order.retain(|k| k != &evicted_key);
            self.map.remove(&evicted_key);
            self.evicted += 1;
        }
        self.order.push_back(key.clone());
        self.map.insert(key, val);
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
    fn visible_lru_eviction() {
        let mut c = Cache::new(3);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("c".into(), "3".into());
        c.put("a".into(), "1x".into()); // a becomes MRU, b is LRU
        c.put("d".into(), "4".into()); // evicts b
        assert_eq!(c.get("b"), None);
        assert_eq!(c.get("a"), Some("1x".to_string()));
        assert_eq!(c.get("c"), Some("3".to_string()));
        assert_eq!(c.get("d"), Some("4".to_string()));
        assert_eq!(c.len(), 3);
        assert_eq!(c.evictions(), 1);
    }
}
