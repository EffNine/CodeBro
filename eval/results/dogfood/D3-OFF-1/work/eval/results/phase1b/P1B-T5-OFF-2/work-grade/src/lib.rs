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
    fn eviction_counts_each_dropped_entry() {
        let mut c = Cache::new(2);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("c".into(), "3".into()); // full -> clear (evict a,b) then insert c
        assert_eq!(c.len(), 1);
        assert_eq!(c.evictions(), 2);
        assert_eq!(c.get("c"), Some("3".to_string()));
        assert_eq!(c.get("a"), None);
    }

    #[test]
    fn repeated_new_puts_keep_making_room() {
        let mut c = Cache::new(1);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into()); // evict a
        c.put("c".into(), "3".into()); // evict b
        assert_eq!(c.len(), 1);
        assert_eq!(c.evictions(), 2);
        assert_eq!(c.get("c"), Some("3".to_string()));
    }

    #[test]
    fn replace_never_evicts_or_counts() {
        let mut c = Cache::new(1);
        c.put("a".into(), "1".into());
        c.put("a".into(), "2".into());
        c.put("a".into(), "3".into());
        assert_eq!(c.len(), 1);
        assert_eq!(c.evictions(), 0);
        assert_eq!(c.get("a"), Some("3".to_string()));
    }

    #[test]
    fn new_panics_at_zero() {
        let result = std::panic::catch_unwind(|| Cache::new(0));
        assert!(result.is_err());
    }

    #[test]
    fn heavy_churn_counts_every_clear() {
        let mut c = Cache::new(3);
        // 9 distinct keys, each new put at full state clears 3
        for i in 0..9 {
            c.put(format!("k{}", i), i.to_string());
        }
        // Puts 0,1,2: len grows 1->2->3, no clear.
        // Puts 3..8: each new key finds len==3 -> clear 3, insert -> len 1.
        // But put 4 sees len 1 (not full), put 5 len 2, put 6 len 3 (not full...
        // wait put 6: after put 5 len==2, not full, insert -> len 3.
        // Let's trace: k3:clear3(=3),len1; k4:len2; k5:len3; k6:clear3(=6),len1;
        // k7:len2; k8:len3.
        assert_eq!(c.len(), 3);
        assert_eq!(c.evictions(), 6);
        assert_eq!(c.get("k8"), Some("8".to_string()));
        assert_eq!(c.get("k5"), None);
    }
}
