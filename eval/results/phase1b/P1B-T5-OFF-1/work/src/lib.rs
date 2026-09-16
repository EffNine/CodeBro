//! Tiny bounded cache (eval fixture). Eviction follows NOTES.md. See `spec.md`.
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
            self.order.push_back(key.clone());
            self.map.insert(key, val);
            return;
        }
        if self.map.len() >= self.cap {
            // NOTES.md approach: drop the least-recently-used entry.
            let old = self.order.pop_front().expect("order and map stay in sync");
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
}
