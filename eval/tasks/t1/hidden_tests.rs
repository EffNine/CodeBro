//! T1 held-out hidden tests. The agent NEVER sees this file during the task.
//! `grade.sh` copies it to `<fixture>/tests/hidden.rs` at grading time.
use stats::median;

#[test]
fn hidden_empty_is_none() {
    assert_eq!(median(vec![]), None);
}

#[test]
fn hidden_single() {
    assert_eq!(median(vec![42]), Some(42));
}

#[test]
fn hidden_two_elements() {
    // (10 + 20) / 2 = 15. The seeded bug panics here (index out of bounds).
    assert_eq!(median(vec![20, 10]), Some(15));
}

#[test]
fn hidden_odd() {
    assert_eq!(median(vec![3, 1, 2]), Some(2));
}

#[test]
fn hidden_even_1234() {
    // Sorted [1,2,3,4]: (v[1] + v[2]) / 2 = (2 + 3) / 2 = 2.
    assert_eq!(median(vec![1, 2, 3, 4]), Some(2));
}

#[test]
fn hidden_negative_even() {
    // Sorted [-4,-2,0,2]: (v[1] + v[2]) / 2 = (-2 + 0) / 2 = -1.
    assert_eq!(median(vec![2, -4, 0, -2]), Some(-1));
}

#[test]
fn hidden_sorted_input() {
    assert_eq!(median(vec![1, 2, 3, 4, 5]), Some(3));
}

#[test]
fn hidden_unsorted_same_as_sorted() {
    assert_eq!(
        median(vec![5, 3, 1, 4, 2]),
        median(vec![1, 2, 3, 4, 5])
    );
}

#[test]
fn hidden_large_odd() {
    let v: Vec<i32> = (1..=1001).rev().collect();
    assert_eq!(median(v), Some(501));
}

#[test]
fn hidden_large_even() {
    // Sorted 1..=1000: (v[499] + v[500]) / 2 = (500 + 501) / 2 = 500.
    let v: Vec<i32> = (1..=1000).rev().collect();
    assert_eq!(median(v), Some(500));
}
