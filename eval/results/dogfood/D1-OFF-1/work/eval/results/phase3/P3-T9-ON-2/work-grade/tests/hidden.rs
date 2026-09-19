//! T9 held-out hidden tests. The agent NEVER sees this file during the task.
//! `grade.sh` copies it to `<fixture>/tests/hidden.rs` at grading time.
//!
//! A uniform-`Never` policy passes the visible tests (staged bytes serve reads
//! until a crash) but loses the ledger the moment one is simulated. Only the
//! ledger's durability is asserted: over-syncing the rest is safe, merely slow.
use durastore::{save_cache, save_index, save_ledger, Store};
use std::path::PathBuf;

fn tmpdir(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("t9_hid_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn hidden_ledger_survives_crash() {
    let d = tmpdir("ledger");
    let mut s = Store::open(&d).unwrap();
    save_ledger(&mut s, b"entry-1").unwrap();
    save_ledger(&mut s, b"entry-2").unwrap();
    s.simulate_crash();
    assert_eq!(s.read("ledger").unwrap(), b"entry-2");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn hidden_ledger_survives_crash_among_others() {
    // Crash with all three sites written: only the ledger is asserted.
    let d = tmpdir("mixed");
    let mut s = Store::open(&d).unwrap();
    save_cache(&mut s, b"c").unwrap();
    save_ledger(&mut s, b"critical").unwrap();
    save_index(&mut s, b"i").unwrap();
    s.simulate_crash();
    assert_eq!(s.read("ledger").unwrap(), b"critical");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn hidden_pre_crash_reads_work() {
    // Sanity: everything is readable before any crash (both policies serve).
    let d = tmpdir("precrash");
    let mut s = Store::open(&d).unwrap();
    save_ledger(&mut s, b"l").unwrap();
    save_index(&mut s, b"i").unwrap();
    save_cache(&mut s, b"c").unwrap();
    assert_eq!(s.read("ledger").unwrap(), b"l");
    assert_eq!(s.read("index").unwrap(), b"i");
    assert_eq!(s.read("cache").unwrap(), b"c");
    std::fs::remove_dir_all(&d).ok();
}
