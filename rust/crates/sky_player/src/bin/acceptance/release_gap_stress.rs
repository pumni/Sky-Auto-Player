use super::{ACCEPTANCE_HOLD_FRAMES, ACCEPTANCE_FRAME_US, Scenario, ScenarioPlan, Verdict, action};
use sky_dispatch_core::model::ActionKind;
use sky_dispatch_win32::input::PHYSICAL_INSTRUMENT_SCAN_CODES;
use sky_player::adapter_support::compile_runtime_intents;
use sky_player::engine::NativeDispatchSession;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) const RELEASE_GAP_STRESS_CYCLES: usize = 513;
pub(super) const RELEASE_GAP_STRESS_MIN_SAMPLES: u64 = 512;

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

pub(super) fn release_gap_qualification(
    scenario: Scenario,
    samples: u64,
    below_frame_floor: u64,
) -> (Verdict, &'static str) {
    if below_frame_floor > 0 {
        (Verdict::Fail, "observed release gap fell below the fixed one-frame floor")
    } else if scenario == Scenario::ReleaseGapStress && samples < RELEASE_GAP_STRESS_MIN_SAMPLES {
        (Verdict::NonQualifying, "release-gap stress collected fewer than 512 qualifying samples")
    } else {
        (Verdict::Pass, "release-gap forensics meet the scenario qualification threshold")
    }
}

pub(super) fn start_heartbeat(session: Arc<NativeDispatchSession>) -> Result<(), String> {
    let heartbeat_session = Arc::clone(&session);
    let result = std::thread::Builder::new()
        .name("rt-native-acceptance-heartbeat".into())
        .spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(60);
            while !heartbeat_session.snapshot().is_finished && Instant::now() < deadline {
                let _ = heartbeat_session.heartbeat();
                std::thread::sleep(Duration::from_millis(250));
            }
        });
    result.map(|_| ()).map_err(|error| {
        let _ = session.quit();
        let _ = session.join(Duration::from_secs(5));
        format!("failed to start acceptance heartbeat: {error}")
    })
}
