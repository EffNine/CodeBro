//! T5 held-out hidden tests. The agent NEVER sees this file during the task.
//! `grade.sh` copies it to `<fixture>/tests/hidden.rs` at grading time.
//!
//! A bulk-clear eviction passes the visible tests but fails every test here:
//! after 1000 inserts at capacity 100 the cache must hold the 100 newest
//! entries with exactly 900 counted evictions — a clear-all design cannot.
use cachebox::Cache;
use std::collections::VecDeque;

#[test]
fn hidden_large_scale_newest_survive() {
    // 950 inserts at capacity 100 (deliberately NOT a multiple of capacity:
    // a bulk-clear design ends with the wrong length AND the wrong count).
    let mut c = Cache::new(100);
    for i in 0..950 {
        c.put(format!("k{:04}", i), format!("v{}", i));
    }
    assert_eq!(c.len(), 100);
    assert_eq!(c.evictions(), 850);
    for i in 850..950 {
        assert_eq!(c.get(&format!("k{:04}", i)), Some(format!("v{}", i)));
    }
    assert_eq!(c.get("k0000"), None);
    assert_eq!(c.get("k0849"), None);
}

#[test]
fn hidden_churn_matches_oracle() {
    // Independent oracle: insertion-ordered deque with oldest-first eviction.
    let mut c = Cache::new(16);
    let mut oracle: VecDeque<(String, String)> = VecDeque::new();
    let oracle_put = |oracle: &mut VecDeque<(String, String)>, k: String, v: String| {
        if let Some(pos) = oracle.iter().position(|(ek, _)| *ek == k) {
            oracle.remove(pos);
        } else if oracle.len() >= 16 {
            oracle.pop_front();
        }
        oracle.push_back((k, v));
    };
    // Deterministic churn: keys cycle over 3x capacity with periodic updates.
    for i in 0..500 {
        let k = format!("k{:02}", i % 48);
        let v = format!("v{}", i);
        c.put(k.clone(), v.clone());
        oracle_put(&mut oracle, k, v);
    }
    assert_eq!(c.len(), oracle.len());
    for (k, v) in oracle.iter() {
        assert_eq!(c.get(k), Some(v.clone()), "mismatch for key {}", k);
    }
    // 500 inserts, 16 slots, updates don't evict: evictions == 500 - 16 - (updates).
    // Updates happen for i >= 48 reusing keys: count exactly via oracle-independent math:
    // first 48 inserts fill+evict to 16 live; every later insert evicts exactly one
    // UNLESS its key is already live. Recompute directly:
    let mut live: VecDeque<String> = VecDeque::new();
    let mut ev = 0u64;
    for i in 0..500 {
        let k = format!("k{:02}", i % 48);
        if let Some(pos) = live.iter().position(|ek| *ek == k) {
            live.remove(pos);
        } else if live.len() >= 16 {
            live.pop_front();
            ev += 1;
        }
        live.push_back(k);
    }
    assert_eq!(c.evictions(), ev);
}

#[test]
fn hidden_update_never_evicts() {
    let mut c = Cache::new(2);
    c.put("a".into(), "1".into());
    c.put("b".into(), "2".into());
    for i in 0..50 {
        c.put("a".into(), format!("{}", i));
    }
    assert_eq!(c.get("a"), Some("49".to_string()));
    assert_eq!(c.get("b"), Some("2".to_string()));
    assert_eq!(c.evictions(), 0);
}

#[test]
fn hidden_single_slot() {
    let mut c = Cache::new(1);
    c.put("a".into(), "1".into());
    c.put("b".into(), "2".into());
    assert_eq!(c.get("a"), None);
    assert_eq!(c.get("b"), Some("2".to_string()));
    assert_eq!(c.evictions(), 1);
}
