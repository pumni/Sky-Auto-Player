//! Lightweight statistics collector for the P3.4 sender-side benchmark.
//!
//! No external dependencies — uses only `std`.  All computations are done
//! eagerly on the collected sample vectors at report time.

// ─── SenderStats ─────────────────────────────────────────────────────────────

/// Accumulated samples for one (kind × polyphony × class × load) cell.
#[derive(Debug, Default)]
pub struct SenderStats {
    /// Signed dispatch-error samples in microseconds.
    /// `actual_us - scheduled_us` — positive means late.
    pub signed_errors: Vec<i64>,
    /// Total coordinator API call duration per batch (µs).
    pub send_durations: Vec<u64>,
    /// Compile + coordinator-init duration per iteration (µs).
    pub bookkeepings: Vec<u64>,
    /// Wake jitter per batch (µs): |pop_time - next_deadline_us|.
    pub wake_errors: Vec<u64>,
}

impl SenderStats {
    pub fn push(&mut self, signed_err: i64, send_dur_us: u64, book_us: u64, wake_err_us: u64) {
        self.signed_errors.push(signed_err);
        self.send_durations.push(send_dur_us);
        self.bookkeepings.push(book_us);
        self.wake_errors.push(wake_err_us);
    }

    pub fn n(&self) -> usize {
        self.signed_errors.len()
    }

    // ── Timing error metrics ─────────────────────────────────────────────────

    pub fn mean_signed_error_us(&self) -> f64 {
        mean_i64(&self.signed_errors)
    }

    pub fn mean_absolute_error_us(&self) -> f64 {
        mean_f64(self.signed_errors.iter().map(|&e| e.unsigned_abs() as f64))
    }

    pub fn mean_late_only_us(&self) -> f64 {
        mean_f64(self.signed_errors.iter().map(|&e| e.max(0) as f64))
    }

    pub fn mean_early_only_us(&self) -> f64 {
        mean_f64(self.signed_errors.iter().map(|&e| (-e).max(0) as f64))
    }

    pub fn max_absolute_error_us(&self) -> i64 {
        self.signed_errors
            .iter()
            .map(|&e| e.unsigned_abs() as i64)
            .max()
            .unwrap_or(0)
    }

    // ── Percentile helpers ───────────────────────────────────────────────────

    /// Return `p`-th percentile of |signed_error| in µs (0.0 ≤ p ≤ 1.0).
    ///
    /// Sorts a temporary copy; call sparingly.
    pub fn percentile_abs_error_us(&self, p: f64) -> i64 {
        if self.signed_errors.is_empty() {
            return 0;
        }
        let mut abs: Vec<i64> = self
            .signed_errors
            .iter()
            .map(|&e| e.unsigned_abs() as i64)
            .collect();
        abs.sort_unstable();
        let idx = ((p * (abs.len() as f64 - 1.0)).round() as usize).min(abs.len() - 1);
        abs[idx]
    }

    /// Return `p`-th percentile of signed_error in µs.
    #[allow(dead_code)]
    pub fn percentile_signed_error_us(&self, p: f64) -> i64 {
        if self.signed_errors.is_empty() {
            return 0;
        }
        let mut sorted = self.signed_errors.clone();
        sorted.sort_unstable();
        let idx = ((p * (sorted.len() as f64 - 1.0)).round() as usize).min(sorted.len() - 1);
        sorted[idx]
    }

    // ── Overhead metrics ─────────────────────────────────────────────────────

    pub fn mean_send_duration_us(&self) -> f64 {
        mean_u64(&self.send_durations)
    }

    pub fn mean_bookkeeping_us(&self) -> f64 {
        mean_u64(&self.bookkeepings)
    }

    pub fn mean_wake_error_us(&self) -> f64 {
        mean_u64(&self.wake_errors)
    }
}

// ─── Free helpers ─────────────────────────────────────────────────────────────

fn mean_i64(v: &[i64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64
}

fn mean_u64(v: &[u64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64
}

fn mean_f64(it: impl Iterator<Item = f64>) -> f64 {
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for v in it {
        sum += v;
        count += 1;
    }
    if count == 0 { 0.0 } else { sum / count as f64 }
}

// ─── Report helpers ───────────────────────────────────────────────────────────

/// Format a percentile value, substituting "N/A" when `n < min_samples`.
#[allow(dead_code)]
pub fn fmt_pct(stats: &SenderStats, p: f64, min_samples: usize, label: &str) -> String {
    if stats.n() < min_samples {
        format!("{label}=N/A")
    } else {
        format!("{label}={}", stats.percentile_abs_error_us(p))
    }
}

/// Print the TSV header for the benchmark report.
pub fn print_header() {
    println!(
        "{:<6} {:<5} {:<5} {:<5}  {:>10} {:>10} {:>10} {:>10}  {:>5} {:>5} {:>5} {:>5} {:>5}  {:>9} {:>9} {:>9}",
        "kind",
        "poly",
        "class",
        "load",
        "signed_err",
        "abs_err",
        "late",
        "early",
        "p50",
        "p95",
        "p99",
        "p999",
        "max",
        "send_us",
        "book_us",
        "wake_us",
    );
    println!("{}", "-".repeat(120));
}

/// Print one TSV row for the given cell.
pub fn print_row(kind: &str, poly: usize, class: &str, load: &str, stats: &SenderStats) {
    let n = stats.n();
    let p999 = if n >= 1000 {
        format!("{}", stats.percentile_abs_error_us(0.999))
    } else {
        "N/A".to_string()
    };

    println!(
        "{:<6} {:<5} {:<5} {:<5}  {:>+10.1} {:>10.1} {:>10.1} {:>10.1}  {:>5} {:>5} {:>5} {:>5} {:>5}  {:>9.1} {:>9.1} {:>9.1}",
        kind,
        poly,
        class,
        load,
        stats.mean_signed_error_us(),
        stats.mean_absolute_error_us(),
        stats.mean_late_only_us(),
        stats.mean_early_only_us(),
        stats.percentile_abs_error_us(0.50),
        stats.percentile_abs_error_us(0.95),
        stats.percentile_abs_error_us(0.99),
        p999,
        stats.max_absolute_error_us(),
        stats.mean_send_duration_us(),
        stats.mean_bookkeeping_us(),
        stats.mean_wake_error_us(),
    );
}
