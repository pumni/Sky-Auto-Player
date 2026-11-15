use smallvec::SmallVec;

const RETAINED_ANOMALY_LIMIT: usize = 16;
const RETAINED_FIRST_ANOMALIES: usize = RETAINED_ANOMALY_LIMIT / 2;
const MAX_ANOMALY_RECORD_BYTES: usize = 512;

/// Bounded terminal evidence for post-call attempted transport anomalies.
///
/// Storage is inline for the common session path and retains the first eight
/// and rolling last eight records. Detailed strings are only created when a
/// physical attempt is anomalous.
#[derive(Debug, Default)]
pub(crate) struct AttemptedDispatchAnomalies {
    total_count: u64,
    omitted_count: u64,
    counter_saturated: bool,
    details: SmallVec<[String; RETAINED_ANOMALY_LIMIT]>,
}

impl AttemptedDispatchAnomalies {
    pub(crate) fn record(&mut self, mut detail: String) {
        truncate_utf8(&mut detail, MAX_ANOMALY_RECORD_BYTES);
        self.total_count = match self.total_count.checked_add(1) {
            Some(count) => count,
            None => {
                self.counter_saturated = true;
                u64::MAX
            }
        };

        if self.details.len() < RETAINED_ANOMALY_LIMIT {
            self.details.push(detail);
            return;
        }

        // Keep the earliest eight records and replace only the oldest member
        // of the rolling tail with each newer anomaly.
        self.details.remove(RETAINED_FIRST_ANOMALIES);
        self.details.push(detail);
        self.omitted_count = match self.omitted_count.checked_add(1) {
            Some(count) => count,
            None => {
                self.counter_saturated = true;
                u64::MAX
            }
        };
    }

    #[cfg(test)]
    pub(crate) fn total_count(&self) -> u64 {
        self.total_count
    }

    #[cfg(test)]
    pub(crate) fn omitted_count(&self) -> u64 {
        self.omitted_count
    }

    #[cfg(test)]
    pub(crate) fn retained_count(&self) -> usize {
        self.details.len()
    }

    #[cfg(test)]
    pub(crate) fn details(&self) -> &[String] {
        &self.details
    }

    pub(crate) fn terminal_error_summary(&self) -> Option<String> {
        (self.total_count > 0).then(|| {
            let saturation = if self.counter_saturated {
                " counter_saturated=true"
            } else {
                ""
            };
            format!(
                "post-call attempted transport anomaly: total={} omitted={} retained={}{}",
                self.total_count,
                self.omitted_count,
                self.details.len(),
                saturation,
            )
        })
    }

    pub(crate) fn append_details_to(&mut self, destination: &mut Vec<String>) {
        destination.extend(self.details.drain(..));
    }
}

fn truncate_utf8(value: &mut String, maximum_bytes: usize) {
    if value.len() <= maximum_bytes {
        return;
    }
    let mut boundary = maximum_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}

#[cfg(test)]
mod tests {
    use super::{AttemptedDispatchAnomalies, MAX_ANOMALY_RECORD_BYTES, RETAINED_ANOMALY_LIMIT};

    #[test]
    fn retains_first_and_last_details_with_exact_omitted_count() {
        let mut history = AttemptedDispatchAnomalies::default();
        for index in 0..40 {
            history.record(format!("attempt={index}"));
        }

        assert_eq!(history.total_count(), 40);
        assert_eq!(history.retained_count(), RETAINED_ANOMALY_LIMIT);
        assert_eq!(history.omitted_count(), 24);
        assert_eq!(
            history.details().first().map(String::as_str),
            Some("attempt=0")
        );
        assert_eq!(
            history.details().get(7).map(String::as_str),
            Some("attempt=7")
        );
        assert_eq!(
            history.details().get(8).map(String::as_str),
            Some("attempt=32")
        );
        assert_eq!(
            history.details().last().map(String::as_str),
            Some("attempt=39")
        );
        assert!(
            history
                .terminal_error_summary()
                .is_some_and(|summary| summary.contains("total=40 omitted=24 retained=16"))
        );
    }

    #[test]
    fn individual_diagnostic_records_have_a_fixed_byte_bound() {
        let mut history = AttemptedDispatchAnomalies::default();
        history.record("x".repeat(MAX_ANOMALY_RECORD_BYTES + 20));

        assert_eq!(history.details()[0].len(), MAX_ANOMALY_RECORD_BYTES);
    }

    #[test]
    fn terminal_error_latch_survives_detail_drain() {
        let mut history = AttemptedDispatchAnomalies::default();
        for index in 0..40 {
            history.record(format!("attempt={index}"));
        }
        let mut details = Vec::new();
        history.append_details_to(&mut details);

        assert_eq!(details.len(), 16);
        assert_eq!(history.retained_count(), 0);
        assert!(
            history
                .terminal_error_summary()
                .is_some_and(|summary| summary.contains("total=40 omitted=24 retained=0"))
        );
    }
}
