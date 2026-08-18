use super::down_late_grace_reached;
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use std::num::NonZeroU64;

#[test]
fn down_late_grace_allows_exact_target_and_in_grace_lateness() {
    let clock = QpcClock::from_frequency_hz(NonZeroU64::new(3_125_000).unwrap());
    let target = QpcTicks::from_raw(10_000);
    let grace = clock.duration_from_us(500).unwrap();
    let latest = target.checked_add_duration(grace).unwrap();

    assert!(!down_late_grace_reached(target, Some(latest)));
    assert!(!down_late_grace_reached(
        target
            .checked_add_duration(clock.duration_from_us(499).unwrap())
            .unwrap(),
        Some(latest)
    ));
    assert!(!down_late_grace_reached(latest, Some(latest)));
}

#[test]
fn down_late_grace_rejects_first_tick_after_cutoff() {
    let clock = QpcClock::from_frequency_hz(NonZeroU64::new(3_125_000).unwrap());
    let target = QpcTicks::from_raw(10_000);
    let grace = clock.duration_from_us(500).unwrap();
    let latest = target.checked_add_duration(grace).unwrap();

    assert!(down_late_grace_reached(
        QpcTicks::from_raw(latest.as_u64() + 1),
        Some(latest)
    ));
}

#[test]
fn five_millisecond_lateness_is_outside_a_five_hundred_microsecond_grace() {
    let clock = QpcClock::from_frequency_hz(NonZeroU64::new(3_125_000).unwrap());
    let target = QpcTicks::from_raw(10_000);
    let grace = clock.duration_from_us(500).unwrap();
    let latest = target.checked_add_duration(grace).unwrap();
    let five_milliseconds_late = target
        .checked_add_duration(clock.duration_from_us(5_000).unwrap())
        .unwrap();

    assert!(down_late_grace_reached(
        five_milliseconds_late,
        Some(latest)
    ));
}

#[test]
fn up_only_dispatch_has_no_down_late_grace_cutoff() {
    assert!(!down_late_grace_reached(QpcTicks::from_raw(u64::MAX), None));
}
