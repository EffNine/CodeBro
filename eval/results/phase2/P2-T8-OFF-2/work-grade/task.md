# T8 — fare change (impact/disambiguation refactor)

In the crate `fleet`, apply the new fare formula per `spec.md`: change the fare
calculation and every behavior it affects, and nothing else.

Definition of done:

- The new formula from `spec.md` is implemented exactly once, in the right place.
- Every affected command behavior is updated; unrelated behavior is byte-identical.
- `cargo test` is fully green.

Note: grading also runs held-out tests you cannot see, covering commands
reachable only through the registry and the re-export, plus behavior that must
not change. Trace the real call graph — text search alone over- and
under-matches in this codebase.
