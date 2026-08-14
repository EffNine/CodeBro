# Third-Party Attribution

This product incorporates code inspired by and adapted from
OpenCode (https://github.com/anomalyco/opencode), licensed under MIT.

**Upstream commit:** e23586af2623f1bc2e8e6965d2d7acf7bd03d5c3
**Retrieval date:** 2026-08-14

## Components Used

| Component | Source | License | Copyright |
|-----------|--------|---------|-----------|
| TUI source (packages/tui/) | opencode monorepo | MIT | 2025 opencode |
| @opentui/core | npm registry | MIT | sst / contributors |
| @opentui/keymap | npm registry | MIT | sst / contributors |
| @opentui/solid | npm registry | MIT | sst / contributors |
| solid-js | npm registry | MIT | Ryan Carniato |
| effect | npm registry | MIT | Effect.ts |
| fuzzysort | npm registry | MIT | Gordon Wang |
| clipboardy | npm registry | MIT | Sindre Sorhus |
| strip-ansi | npm registry | MIT | Sindre Sorhus |
| diff | npm registry | BSD-3 | Kevin Mårtensson |

## Modifications

The OpenCode TUI source has been adapted to:
- Replace the HTTP/SSE backend with a stdio JSON bridge to CodeBro
- Remove dependencies on OpenCode-specific server APIs
- Add CodeBro branding and configuration

## License

The OpenCode TUI source is covered by the MIT License (see LICENSE-OPENCODE).
CodeBro itself is also MIT licensed.

## Trademark Note

CodeBro is not affiliated with, endorsed by, or sponsored by the OpenCode project.
