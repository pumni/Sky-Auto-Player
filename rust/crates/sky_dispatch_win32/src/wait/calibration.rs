pub(crate) fn median_sorted(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        values[middle - 1].saturating_add(values[middle]) / 2
    } else {
        values[middle]
    }
}

pub(crate) fn robust_wake_error_us(sorted_errors: &[u64]) -> u64 {
    let median = median_sorted(sorted_errors);
    let mut deviations: Vec<u64> = sorted_errors
        .iter()
        .map(|value| value.abs_diff(median))
        .collect();
    deviations.sort_unstable();
    median.saturating_add(median_sorted(&deviations).saturating_mul(6))
}
