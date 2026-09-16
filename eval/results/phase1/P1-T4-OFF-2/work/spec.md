# `qserver` specification (fixture crate `qserver`)

A tiny JSON-lines command server over stdio. Standard library only.

## Wire format

- The server reads UTF-8 lines from stdin, one command per line, and writes one
  response line to stdout per input line.
- Commands (exact strings):
  - `{"cmd":"ping"}` → `{"ok":true,"pong":true}`
  - `{"cmd":"store","key":"<k>","val":"<v>"}` → `{"ok":true}` (stores `v` under `k`)
  - `{"cmd":"get","key":"<k>"}` → `{"ok":true,"val":"<v>"}` when present,
    `{"ok":false,"err":"missing"}` when absent
- Anything else (malformed line, unknown command, missing fields) →
  `{"ok":false,"err":"bad-request"}`
- Keys and values in tests are alphanumeric (no quotes, braces, or commas inside).

## API

```rust
pub struct Store { /* private */ }

impl Store {
    pub fn new() -> Self;
    pub fn handle(&mut self, line: &str) -> String; // response line, no trailing newline
}

/// Emit one diagnostic describing the request and its outcome.
/// Called exactly once per input line by `main`.
pub fn log_diagnostic(msg: String);
```

`main` (provided): reads stdin lines, prints `handle()` output per line, calls
`log_diagnostic` once per line with a short summary.

## Diagnostics

Every request MUST produce exactly one diagnostic via `log_diagnostic`
summarizing the command and outcome (e.g. which command ran and whether it
succeeded). Operators collect diagnostics to audit server activity.

## §5 Deployment note (appendix)

`qserver` speaks to its supervisor over stdio using the JSON-lines protocol
above: **stdout is reserved for protocol responses.** A single non-JSON byte on
stdout breaks framing for the supervisor and every response after it. All
diagnostics, logs, and human-readable output MUST go to stderr. This applies to
`log_diagnostic` and to any future debugging output.
