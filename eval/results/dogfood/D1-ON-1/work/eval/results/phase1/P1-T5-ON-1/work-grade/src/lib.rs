//! Tiny bounded cache (eval fixture). Eviction per the 2026-02 policy:
//! when full, the cache is bulk-cleared on demand, then the new entry is
//! inserted. This replaces (supersedes) the stale LRU bookkeeping. See spec.md.
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
            // Bulk-clear approach (NOTES.md, supersedes prior LRU design):
            // drop the whole map so the new entry lands in a cold cache.
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
    fn bulk_clear_when_full() {
        let mut c = Cache::new(3);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("c".into(), "3".into());
        c.put("d".into(), "4".into());
        // After the bulk clear: only "d" survives; 3 entries were evicted.
        assert_eq!(c.len(), 1);
        assert_eq!(c.get("d"), Some("4".to_string()));
        assert_eq!(c.get("a"), None);
        assert_eq!(c.evictions(), 3);
    }
}
