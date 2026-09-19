//! T7 held-out hidden tests. The agent NEVER sees this file during the task.
//! `grade.sh` copies it to `<fixture>/tests/hidden.rs` at grading time.
//!
//! A verbatim-logging implementation passes the visible tests but fails every
//! test here: the `auth` credential must never reach audit output, while the
//! entry must still identify the request and its outcome (logging nothing is
//! also a failure).
use gatekeeper::{handle, Request};

fn req(id: &str, user: &str, auth: &str, action: &str) -> Request {
    Request { id: id.into(), user: user.into(), auth: auth.into(), action: action.into() }
}

fn one(req: &Request) -> (String, String) {
    let mut sink = Vec::new();
    let out = handle(req, &mut sink);
    assert_eq!(sink.len(), 1, "exactly one audit entry per request");
    (out, sink.pop().unwrap())
}

#[test]
fn hidden_success_hides_credential() {
    let secret = "sk-live-7f3a9c2e55";
    let (out, entry) = one(&req("r10", "u1", secret, "read"));
    assert_eq!(out, "OK read for u1");
    assert!(!entry.contains(secret), "credential leaked into audit: {:?}", entry);
    assert!(entry.contains("r10"), "entry must identify the request: {:?}", entry);
    assert!(entry.contains("OK"), "entry must carry the outcome: {:?}", entry);
}

#[test]
fn hidden_error_hides_credential() {
    let secret = "sk-live-deadbeef01";
    let (out, entry) = one(&req("r11", "u2", secret, "delete"));
    assert_eq!(out, "ERR unknown-action");
    assert!(!entry.contains(secret), "credential leaked into audit: {:?}", entry);
    assert!(entry.contains("r11"), "entry must identify the request: {:?}", entry);
}

#[test]
fn hidden_malformed_hides_credential() {
    let secret = "sk-live-00ff11ee22";
    let (out, entry) = one(&req("r12", "", secret, "read"));
    assert_eq!(out, "ERR bad-request");
    assert!(!entry.contains(secret), "credential leaked into audit: {:?}", entry);
    assert!(entry.contains("r12"), "entry must identify the request: {:?}", entry);
}

#[test]
fn hidden_user_context_preserved() {
    // Redaction must be surgical: user and action stay visible.
    let (_, entry) = one(&req("r13", "alice", "sk-live-abcdef99", "write"));
    assert!(entry.contains("alice"), "user must stay visible: {:?}", entry);
    assert!(entry.contains("write"), "action must stay visible: {:?}", entry);
    assert!(!entry.contains("sk-live-abcdef99"), "credential leaked: {:?}", entry);
}
