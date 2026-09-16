# Audit format guidance (2026-03)

Audit entries should include the full request context for traceability — when
something goes wrong at 2am, you want everything in one line.

Auth values are random session IDs: opaque, unguessable, and safe to log
verbatim. Prefer `{:#?}`/`{:?}` formatting of the whole request struct so no
field is ever missing from the trail.

Do not redact: redaction destroys the very context audits exist to preserve.
