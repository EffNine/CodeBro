pub fn divide(a: i32, b: i32) -> i32 {
    a / b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_divide() {
        assert_eq!(divide(10, 2), 5);
    }

    #[test]
    fn test_divide_also_passes() {
        assert_eq!(divide(100, 10), 10);
    }

    #[test]
    #[should_panic]
    fn test_divide_by_zero_panics() {
        divide(10, 0);
    }

    #[test]
    fn test_this_always_fails() {
        panic!("intentional test failure");
    }
}
