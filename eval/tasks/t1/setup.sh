#!/usr/bin/env bash
# T1 fixture setup: generate a tiny deterministic `stats` crate with a seeded bug.
#
# The fixture contains the buggy implementation plus VISIBLE tests only.
# Held-out hidden tests live in `hidden_tests.rs` next to this script and are
# NEVER copied here — `grade.sh` installs them at grading time.
#
# Usage: setup.sh <dest-dir>
set -euo pipefail

dest="${1:?usage: setup.sh <dest-dir>}"
rm -rf "$dest"
mkdir -p "$dest/src"

cat > "$dest/Cargo.toml" <<'EOF'
[package]
name = "stats"
version = "0.1.0"
edition = "2021"
EOF

cat > "$dest/src/lib.rs" <<'EOF'
//! Tiny integer statistics helpers (eval fixture).

/// Median of `v`.
///
/// Returns `None` for an empty vector. Otherwise sorts a copy and returns the
/// middle element (odd length) or the integer mean of the two middle elements
/// (even length): `(v[n / 2 - 1] + v[n / 2]) / 2`.
pub fn median(mut v: Vec<i32>) -> Option<i32> {
    if v.is_empty() {
        return None;
    }
    v.sort();
    let n = v.len();
    if n % 2 == 1 {
        Some(v[n / 2])
    } else {
        // SEEDED BUG (T1): wrong indices for the even-length case.
        Some((v[n / 2] + v[n / 2 + 1]) / 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_median_odd() {
        assert_eq!(median(vec![3, 1, 2]), Some(2));
    }

    #[test]
    fn visible_median_even() {
        assert_eq!(median(vec![1, 2, 3, 4]), Some(2));
    }
}
EOF
