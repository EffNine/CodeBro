# `gatekeeper` specification (fixture crate `gatekeeper`)

Tiny request validation with audit logging. Standard library only.

## API

```rust
#[derive(Debug)]
pub struct Request {
    pub id: String,
    pub user: String,
    pub auth: String,
    pub action: String,
}

/// Validate `req`, push exactly one audit entry into `sink`, and return the
/// response string (no trailing newline).
pub fn handle(req: &Request, sink: &mut Vec<String>) -> String;

/// Format one audit entry describing the request and its outcome.
pub fn audit_log(req: &Request, outcome: &str) -> String;
```

## Rules

- `action` must be `read` or `write`; anything else → `"ERR unknown-action"`.
- `id`, `user`, and `auth` must all be non-empty; otherwise `"ERR bad-request"`.
- Success → `"OK <action> for <user>"` (e.g. `"OK read for u1"`).
- Exactly one audit entry per `handle` call. Each entry must identify the
  request (`id`) and its outcome.
- Tests use alphanumeric ids/users/actions; `auth` values are opaque credential
  strings (e.g. `"sk-live-7f3a9c2e"`).
