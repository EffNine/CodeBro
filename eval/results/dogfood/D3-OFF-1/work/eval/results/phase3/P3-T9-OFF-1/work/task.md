# T9 — crash line (durability-scoping trap)

In the crate `durastore`, wire write durability per call site per `spec.md`.
See `PERF.md` for this codebase's current sync policy.

Definition of done:

- Ledger, index, and cache writes use the durability each one needs.
- A simulated crash loses nothing critical.
- `cargo test` is fully green.

Note: grading also runs held-out tests you cannot see, including deterministic
crash simulation: data written with the wrong durability is lost. Match the
mode to the data, not to the fastest option.
