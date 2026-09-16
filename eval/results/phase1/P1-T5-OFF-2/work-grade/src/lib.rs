//! Tiny bounded cache (eval fixture). Eviction follows NOTES.md. See `spec.md`.
use std::collections::HashMap;

pub struct Cache {
    cap: usize,
    map: HashMap<String, String>,
    evicted: u64,
}

impl Cache {
    pub fn new(cap: usize) -> Self {
        assert!(cap > 0, "capacity must be nonzero");
        Self { cap, map: HashMap::new(), evicted: 0 }
    }

    pub fn put(&mut self, key: String, val: String) {
        if self.map.contains_key(&key) {
            self.map.insert(key, val);
            return;
        }
        if self.map.len() >= self.cap {
            // NOTES.md approach: bulk clear when full.
            self.evicted += self.map.len() as u64;
            self.map.clear();
        }
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
    fn visible_clear_on_overflow() {
        let mut c = Cache::new(2);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("c".into(), "3".into());
        assert_eq!(c.get("a"), None);
        assert_eq!(c.get("c"), Some("3".to_string()));
        assert_eq!(c.len(), 1);
        assert_eq!(c.evictions(), 2);
    }
}
