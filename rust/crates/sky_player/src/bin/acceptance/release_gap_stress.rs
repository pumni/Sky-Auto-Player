use super::{ACCEPTANCE_HOLD_FRAMES, ACCEPTANCE_FRAME_US, Scenario, ScenarioPlan, Verdict, action};
use sky_dispatch_core::coordinator::GenerationAccounting;
use sky_dispatch_core::model::ActionKind;
use sky_dispatch_win32::input::PHYSICAL_INSTRUMENT_SCAN_CODES;
use sky_player::adapter_support::compile_runtime_intents;
use sky_player::engine::NativeDispatchSession;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub(super) const RELEASE_GAP_STRESS_CYCLES: usize = 513;
pub(super) const RELEASE_GAP_STRESS_MIN_SAMPLES: u64 = 512;

pub(super) struct IndependentHelperLivenessProbe {
    stop: Arc<AtomicBool>,
    ticks: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl IndependentHelperLivenessProbe {
    pub(super) fn start() -> Result<Self, std::io::Error> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let ticks = Arc::new(AtomicU64::new(0));
        let thread_ticks = Arc::clone(&ticks);
        let thread = std::thread::Builder::new()
            .name("rt-native-acceptance-independent-helper".into())
            .spawn(move || {
                while !thread_stop.load(Ordering::Acquire) {
                    thread_ticks.fetch_add(1, Ordering::Relaxed);
                    std::thread::sleep(Duration::from_millis(20));
                }
            })?;
        Ok(Self {
            stop,
            ticks,
            thread: Some(thread),
        })
    }

    pub(super) fn stop_and_read(mut self) -> u64 {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.ticks.load(Ordering::Acquire)
    }
}

pub(super) fn healthy_generation_qualification(
    scenario: Scenario,
    accounting: GenerationAccounting,
) -> (Verdict, &'static str) {
    if !matches!(
        scenario,
        Scenario::CanonicalSingle
            | Scenario::CanonicalChord
            | Scenario::CanonicalMaxChord
            | Scenario::Hold
            | Scenario::LongSingleSequence
            | Scenario::DenseAlternating
            | Scenario::ChordSweep
            | Scenario::NearMinimumRetrigger
            | Scenario::RapidRetrigger
            | Scenario::ReleaseGapStress
            | Scenario::MixedUpDown
            | Scenario::W4Noncanonical
            | Scenario::TimingMarginSweep
    ) {
        return (
            Verdict::Pass,
            "lifecycle/fault scenario reports authoritative generation accounting without natural-completion gating",
        );
    }
    if accounting.total != accounting.activated
        || accounting.activated != accounting.released
        || accounting.scheduled != 0
        || accounting.active != 0
        || accounting.dropped_conflict != 0
        || accounting.dropped_backend != 0
        || accounting.dropped_expired != 0
        || accounting.cancelled != 0
    {
        return (
            Verdict::Fail,
            "healthy native acceptance violated authoritative generation accounting",
        );
    }
    (Verdict::Pass, "authoritative generation accounting is clean")
}

pub(super) fn scenario_plan(timing_margin_us: u64) -> Result<ScenarioPlan, String> {
    let hold_us = ACCEPTANCE_HOLD_FRAMES * ACCEPTANCE_FRAME_US + timing_margin_us;
    let gap_us = ACCEPTANCE_FRAME_US + timing_margin_us;
    let mut actions = Vec::with_capacity(RELEASE_GAP_STRESS_CYCLES * 2);
    let mut next_down_us = 50_000_u64;
    for cycle in 0..RELEASE_GAP_STRESS_CYCLES {
        let down_index = u32::try_from(cycle * 2).map_err(|error| error.to_string())?;
        let up_us = next_down_us.checked_add(hold_us).ok_or("stress schedule overflow")?;
        actions.push(action(down_index, ActionKind::Down, next_down_us, &[0]));
        actions.push(action(down_index + 1, ActionKind::Up, up_us, &[0]));
        next_down_us = up_us.checked_add(gap_us).ok_or("stress schedule overflow")?;
    }
    let schedule = compile_runtime_intents(&actions, &PHYSICAL_INSTRUMENT_SCAN_CODES)
        .map_err(|error| format!("release-gap stress schedule compilation failed: {error}"))?;
    Ok(ScenarioPlan {
        schedule,
        profile: None,
        expected_down_slots: vec![0; RELEASE_GAP_STRESS_CYCLES],
        expected_up_slots: vec![0; RELEASE_GAP_STRESS_CYCLES],
        allow_unpaired_cleanup_ups: false,
    })
}

pub(super) fn production_visibility_qualification(
    scenario: Scenario,
    hold_samples: u64,
    hold_floor_violations: u64,
    release_samples: u64,
    release_floor_violations: u64,
) -> (Verdict, &'static str) {
    if hold_floor_violations > 0 || release_floor_violations > 0 {
        return (
            Verdict::Fail,
            "physical hold or one-frame release floor violation",
        );
    }
    if scenario == Scenario::ReleaseGapStress
        && (hold_samples < RELEASE_GAP_STRESS_MIN_SAMPLES
            || release_samples < RELEASE_GAP_STRESS_MIN_SAMPLES)
    {
        (Verdict::NonQualifying, "stress collected fewer than 512 qualifying hold or release samples")
    } else {
        (Verdict::Pass, "normal delivery passed; completion-relative hold/release floors remain diagnostic evidence")
    }
}

#[cfg(test)]
pub(super) fn strict_physical_forensics_qualification(
    hold_floor_violations: u64,
    release_floor_violations: u64,
) -> (Verdict, &'static str) {
    if hold_floor_violations > 0 || release_floor_violations > 0 {
        (
            Verdict::Fail,
            "strict physical timing forensics violated an intentional floor",
        )
    } else {
        (
            Verdict::Pass,
            "strict physical timing forensics meet the intentional floor",
        )
    }
}

pub(super) fn attach_sink_window_provenance(
    details: &mut Value,
    sink: &super::ReadyRecord,
    cursor: super::LogCursor,
    events: &[super::EventRecord],
    expected_event_count: usize,
    scenario: Scenario,
) {
    let Some(object) = details.as_object_mut() else { return; };
    object.insert("sink_event_log_id".to_string(), json!(sink.event_log_id));
    object.insert("sink_pid".to_string(), json!(sink.pid));
    object.insert("sink_hwnd".to_string(), json!(sink.hwnd));
    object.insert("sink_process_start_time_filetime".to_string(), json!(sink.process_start_time_filetime));
    object.insert("sink_cursor_sequence_before_arm".to_string(), json!(cursor.sequence));
    object.insert("sink_cursor_offset_before_arm".to_string(), json!(cursor.offset));
    object.insert("first_authorized_sequence".to_string(), json!(events.first().map(|event| event.sequence)));
    object.insert("last_authorized_sequence".to_string(), json!(events.last().map(|event| event.sequence)));
    object.insert("sink_event_count".to_string(), json!(events.len()));
    object.insert("expected_sink_event_count".to_string(), json!(expected_event_count));
    object.insert("observed_sink_event_count".to_string(), json!(events.len()));
    if scenario == Scenario::ReleaseGapStress {
        object.insert("minimum_qualifying_hold_pair_samples".to_string(), json!(RELEASE_GAP_STRESS_MIN_SAMPLES));
        object.insert("minimum_qualifying_release_gap_samples".to_string(), json!(RELEASE_GAP_STRESS_MIN_SAMPLES));
    }
}

pub(super) fn start_heartbeat(session: Arc<NativeDispatchSession>) -> Result<(), String> {
    let heartbeat_session = Arc::clone(&session);
    let result = std::thread::Builder::new()
        .name("rt-native-acceptance-heartbeat".into())
        .spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(60);
            while !heartbeat_session.snapshot().is_finished && Instant::now() < deadline {
                let _ = heartbeat_session.publish_supervisor_progress();
                std::thread::sleep(Duration::from_millis(250));
            }
        });
    result.map(|_| ()).map_err(|error| {
        let _ = session.quit();
        let _ = session.join(Duration::from_secs(5));
        format!("failed to start acceptance heartbeat: {error}")
    })
}
