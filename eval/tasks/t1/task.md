# T1 — fix `median` (calibration: simple bug)

In this crate (`stats`), `median` returns the wrong result for even-length vectors.

Definition of done for this task:

- `median` sorts a copy of the input and returns:
  - `None` for an empty vector,
  - the middle element for odd length,
  - the integer mean of the two middle elements for even length,
    i.e. `(v[n / 2 - 1] + v[n / 2]) / 2` on the sorted vector.
- Do not change the function signature.
- `cargo test` is fully green.

Note: grading also runs held-out tests you cannot see. Implement the general
fix, not a special case for the visible test.
