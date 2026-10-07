/// Offset for a 1-based page. Callers retain their own nonnegative size policy.
pub(super) fn page_offset(page: i64, page_size: i64) -> i64 {
    debug_assert!(page_size >= 0);
    // SQLite uses signed 64-bit offsets. Anything larger is beyond its possible
    // row count; saturate rather than wrapping back to an earlier page.
    (page.max(1) - 1).saturating_mul(page_size)
}

#[cfg(test)]
mod tests {
    use super::page_offset;

    #[test]
    fn offset_is_saturating_and_one_based() {
        assert_eq!(page_offset(i64::MIN, 20), 0);
        assert_eq!(page_offset(0, 20), 0);
        assert_eq!(page_offset(1, 20), 0);
        assert_eq!(page_offset(2, 20), 20);
        assert_eq!(page_offset(i64::MAX, 0), 0);
        assert_eq!(page_offset(i64::MAX, 1), i64::MAX - 1);
        assert_eq!(page_offset(i64::MAX, 2), i64::MAX);
        assert_eq!(page_offset(i64::from(u32::MAX), i64::from(u32::MAX)), i64::MAX);
    }
}
