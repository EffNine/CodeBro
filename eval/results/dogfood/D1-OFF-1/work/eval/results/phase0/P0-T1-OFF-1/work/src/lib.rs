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
        Some((v[n / 2 - 1] + v[n / 2]) / 2)
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
