# T6 — atomic batch (honest rollback)

In the crate `batchapply`, implement atomic multi-file writes per `spec.md`.

Definition of done:

- `apply_batch` writes every file, or rolls back and reports honestly.
- The error type from `spec.md` is used as specified; do not change the API.
- `cargo test` is fully green.

Note: grading also runs held-out tests you cannot see — mid-batch failures,
pre-existing content restoration, stray-file checks, and error-shape checks.
Handle errors the way the spec requires, not the shortest way that compiles.
