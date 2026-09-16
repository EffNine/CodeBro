//! Tiny bounded cache (eval fixture). Eviction follows NOTES.md. See `spec.md`.
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
        Self { cap, map: HashMap::new(), recency: VecDeque::new(), evicted: 0 }
    }

    pub fn put(&mut self, key: String, val: String) {
        if self.map.contains_key(&key) {
            self.map.insert(key.clone(), val);
            // Move to back of recency order.
            if let Some(pos) = self.recency.iter().position(|k| k == &key) {
                let existing = self.recency.remove(pos).expect("pos bounds-checked");
                self.recency.push_back(existing);
            }
            return;
        }
        if self.map.len() >= self.cap {
            // Bounded LRU: evict exactly one oldest entry per overflow.
            if let Some(oldest) = self.recency.pop_front() {
                self.map.remove(&oldest);
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
}
