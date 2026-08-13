# Grok Build Third-Party Components

This directory documents upstream components adopted from the Grok Build
project under the audit documented in `OPENCODE_AS_FRONTEND_AUDIT.md`.

## Adopted Component

### xai-ratatui-textarea

- **Source**: [SylphxAI/spiron](https://github.com/SylphxAI/spiron)
- **Audited revision**: `ea094a8c369475f97c85540d01730baec0dce5d6`
- **Crates.io version**: `0.1.0` (published from the audited revision)
- **License**: Apache-2.0
- **Purpose**: Production-grade multiline textarea widget for ratatui, used as
  the replacement for CodeBro's hand-rolled chat input implementation.
- **Audit findings**: Zero Grok backend coupling; pure ratatui widget with
  host-driven event handling, mouse selection, undo/redo, grapheme safety,
  wide-character safety, wrapping, viewport handling, and clipboard abstraction.

Only `xai-ratatui-textarea` was adopted. No other Grok crates (pager, shell,
config, tools, inline, auth) were integrated.
