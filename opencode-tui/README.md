# CodeBro TUI (OpenCode-derived)

This is an experimental fork of the OpenCode TUI, adapted to work with CodeBro's backend via a stdio JSON bridge.

## Architecture

```
CodeBro Backend (Rust)
    ↓ stdio JSON protocol
OpenCode-derived TUI (TypeScript/SolidJS)
```

## Prerequisites

- Rust toolchain (for `codebro` binary)
- Bun runtime (for TUI development)

## Usage

### Build CodeBro backend
```bash
cargo build --release
```

### Run TUI in dev mode
```bash
cd opencode-tui
bun install
bun run dev
```

### Run via launch script
```bash
./run.sh
```

## Protocol

The TUI communicates with CodeBro via newline-delimited JSON over stdin/stdout:

**TUI → CodeBro:**
```json
{"id": 1, "cmd": "session.list", "payload": null}
{"id": 2, "cmd": "session.create", "payload": {"directory": "/path/to/project"}}
```

**CodeBro → TUI:**
```json
{"id": 1, "result": {"data": [...]}}
{"id": 2, "result": {"data": {"id": "abc123", ...}}}
{"event": {"type": "session.next.text.delta", ...}}
{"error": "Something went wrong"}
```

## Upstream

- **Repository:** https://github.com/anomalyco/opencode
- **Commit:** e23586af2623f1bc2e8e6965d2d7acf7bd03d5c3
- **License:** MIT

See ATTRIBUTION.md for full details.

## Status

This is an experimental fork. The adapter layer is functional but incomplete.
See the experiment report for detailed findings.
