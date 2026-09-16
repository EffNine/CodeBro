//! Tiny bounded cache (eval fixture). See `spec.md`. The 2026-08 record in CodeBro
//! memory documents that mass eviction was rejected project-wide, so eviction is
//! bounded LRU: each over-capacity `put` evicts exactly one oldest entry.
//! NOTES.md's bulk-clear approach is a stale document and was not followed.
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
            self.touch(&key);
            return;
        }
        while self.map.len() >= self.cap {
            let evicted_key = self
                .recency
                .pop_front()
                .expect("full cache implies recorded recency entries");
            self.map.remove(&evicted_key);
            self.evicted += 1;
        }
        self.recency.push_back(key.clone());
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

    fn touch(&mut self, key: &str) {
        if let Some(pos) = self.recency.iter().position(|k| k == key) {
            self.recency.rotate_left(pos);
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
}
