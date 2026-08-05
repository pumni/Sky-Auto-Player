#![cfg(any(test, feature = "test-support"))]

use super::fault_injection::{FaultInjectionScript, InjectedSendOutcome};
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::{QpcClock, QpcError, QpcTicks};
use sky_dispatch_win32::input::{
    PacketClockFailurePhase, PhysicalPacket, PhysicalSendOutcome, PlatformSendResult,
    TrackedKeyState,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub(crate) fn create_mock_backend(
    qpc_clock: QpcClock,
    latency_base_us: u64,
    latency_per_key_us: u64,
    fault_script: FaultInjectionScript,
) -> TrackedKeyState {
    let script = Arc::new(fault_script);
    let call_index = Arc::new(AtomicU64::new(0));
    let script_emitter = Arc::clone(&script);
    let call_index_emitter = Arc::clone(&call_index);
    let mut backend = TrackedKeyState::with_emitter(move |codes, _key_up| {
        let idx = call_index_emitter.fetch_add(1, Ordering::Relaxed) as usize;

        // Keep the artificial sender work after the sender start boundary so
        // test-support timing matches the real SendInput seam.
        let base_latency_us =
            latency_base_us.saturating_add(latency_per_key_us.saturating_mul(codes.len() as u64));
        let sender_started_ticks = match qpc_clock.now() {
            Ok(ticks) => ticks,
            Err(error) => {
                return mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Err(error),
                    codes.len() as u32,
                    0,
                    0,
                    0,
                );
            }
        };
        if base_latency_us > 0 {
            std::thread::sleep(Duration::from_micros(base_latency_us));
        }

        match script_emitter.resolve(idx) {
            None | Some(InjectedSendOutcome::Full { latency_ticks: 0 }) => {
                mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Ok(sender_started_ticks),
                    codes.len() as u32,
                    codes.len() as u32,
                    0,
                    0,
                )
            }
            Some(InjectedSendOutcome::Full { latency_ticks }) => {
                mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Ok(sender_started_ticks),
                    codes.len() as u32,
                    codes.len() as u32,
                    0,
                    *latency_ticks,
                )
            }
            Some(InjectedSendOutcome::Zero {
                latency_ticks,
                win32_error,
            }) => mock_platform_send_result_from_started_ticks(
                qpc_clock,
                Ok(sender_started_ticks),
                codes.len() as u32,
                0,
                *win32_error,
                *latency_ticks,
            ),
            Some(InjectedSendOutcome::Partial {
                inserted,
                latency_ticks,
                win32_error,
            }) => {
                let inserted = (*inserted as u32).min(codes.len() as u32);
                mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Ok(sender_started_ticks),
                    codes.len() as u32,
                    inserted,
                    *win32_error,
                    *latency_ticks,
                )
            }
            Some(InjectedSendOutcome::Stall { duration_ticks }) => {
                mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Ok(sender_started_ticks),
                    codes.len() as u32,
                    0,
                    0,
                    *duration_ticks,
                )
            }
            Some(InjectedSendOutcome::PanicAfterSend) => {
                let _ = mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Ok(sender_started_ticks),
                    codes.len() as u32,
                    codes.len() as u32,
                    0,
                    0,
                );
                panic!("fault injection: panic after send before commit");
            }
            Some(InjectedSendOutcome::QpcFailureAfterSend) => {
                let mut result = mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Ok(sender_started_ticks),
                    codes.len() as u32,
                    codes.len() as u32,
                    0,
                    0,
                );
                result.timing_error = Some(QpcError::CounterUnavailable);
                result
            }
        }
    });
    let script_packet = Arc::clone(&script);
    let call_index_packet = Arc::clone(&call_index);
    backend.set_packet_emitter(move |packet| {
        let idx = call_index_packet.fetch_add(1, Ordering::Relaxed) as usize;
        physical_packet_outcome(
            qpc_clock,
            &script_packet,
            idx,
            packet,
            latency_base_us,
            latency_per_key_us,
        )
    });
    backend
}

fn physical_packet_outcome(
    qpc_clock: QpcClock,
    script: &FaultInjectionScript,
    call_index: usize,
    packet: PhysicalPacket,
    latency_base_us: u64,
    latency_per_key_us: u64,
) -> PhysicalSendOutcome {
    let requested = packet.event_count();
    let total_latency_us =
        latency_base_us.saturating_add(latency_per_key_us.saturating_mul(u64::from(requested)));
    let started_ticks = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return PhysicalSendOutcome::ClockFailure {
                phase: PacketClockFailurePhase::BeforeSend,
                send_was_called: false,
                inserted_count: None,
                started_ticks: None,
                error,
            };
        }
    };
    if total_latency_us > 0 {
        std::thread::sleep(Duration::from_micros(total_latency_us));
    }
    let scripted = script.resolve(call_index);
    let (inserted, latency_ticks, win32_error) = match scripted {
        None | Some(InjectedSendOutcome::Full { latency_ticks: 0 }) => (requested, 0, 0),
        Some(InjectedSendOutcome::Full { latency_ticks }) => (requested, *latency_ticks, 0),
        Some(InjectedSendOutcome::Zero {
            latency_ticks,
            win32_error,
        }) => (0, *latency_ticks, *win32_error),
        Some(InjectedSendOutcome::Partial {
            inserted,
            latency_ticks,
            win32_error,
        }) => ((*inserted).min(requested), *latency_ticks, *win32_error),
        Some(InjectedSendOutcome::Stall { duration_ticks }) => (0, *duration_ticks, 0),
        Some(InjectedSendOutcome::PanicAfterSend) => {
            panic!("fault injection: panic after send before commit")
        }
        Some(InjectedSendOutcome::QpcFailureAfterSend) => {
            return PhysicalSendOutcome::ClockFailure {
                phase: PacketClockFailurePhase::AfterSend,
                send_was_called: true,
                inserted_count: Some(requested),
                started_ticks: Some(started_ticks),
                error: QpcError::CounterUnavailable,
            };
        }
    };
    let deadline = match started_ticks.checked_add_duration(DurationTicks::from_raw(latency_ticks))
    {
        Ok(deadline) => deadline,
        Err(_) => {
            return PhysicalSendOutcome::ClockFailure {
                phase: PacketClockFailurePhase::AfterSend,
                send_was_called: true,
                inserted_count: Some(inserted),
                started_ticks: Some(started_ticks),
                error: QpcError::DeadlineOverflow,
            };
        }
    };
    let completed_ticks = loop {
        match qpc_clock.now() {
            Ok(now) if now >= deadline => break now,
            Ok(_) => std::hint::spin_loop(),
            Err(error) => {
                return PhysicalSendOutcome::ClockFailure {
                    phase: PacketClockFailurePhase::AfterSend,
                    send_was_called: true,
                    inserted_count: Some(inserted),
                    started_ticks: Some(started_ticks),
                    error,
                };
            }
        }
    };
    if inserted >= requested {
        PhysicalSendOutcome::Complete {
            requested,
            inserted,
            attempts: 1,
            started_ticks,
            completed_ticks,
        }
    } else if inserted == 0 {
        PhysicalSendOutcome::ZeroProgress {
            requested,
            attempts: 1,
            first_error: win32_error,
            last_error: win32_error,
            started_ticks,
            completed_ticks,
        }
    } else {
        PhysicalSendOutcome::Partial {
            requested,
            inserted_count: inserted,
            attempts: 1,
            first_error: win32_error,
            last_error: win32_error,
            started_ticks,
            completed_ticks,
        }
    }
}

fn mock_platform_send_result_from_started_ticks(
    qpc_clock: QpcClock,
    started_ticks: Result<QpcTicks, QpcError>,
    requested: u32,
    inserted: u32,
    win32_error: u32,
    latency_ticks: u64,
) -> PlatformSendResult {
    let started_ticks = match started_ticks {
        Ok(ticks) => ticks,
        Err(error) => {
            return PlatformSendResult {
                requested,
                inserted: 0,
                started_ticks: QpcTicks::ZERO,
                completed_ticks: None,
                completed_us: 0,
                win32_error,
                timing_error: Some(error),
            };
        }
    };
    let deadline = match started_ticks.checked_add_duration(DurationTicks::from_raw(latency_ticks))
    {
        Ok(deadline) => deadline,
        Err(_) => {
            return PlatformSendResult {
                requested,
                inserted: 0,
                started_ticks,
                completed_ticks: None,
                completed_us: 0,
                win32_error,
                timing_error: Some(QpcError::DeadlineOverflow),
            };
        }
    };
    loop {
        match qpc_clock.now() {
            Ok(now) if now >= deadline => {
                let (completed_us, timing_error) =
                    match qpc_clock.duration_to_us(DurationTicks::from_raw(now.as_u64())) {
                        Ok(micros) => (micros, None),
                        Err(_) => (0, Some(QpcError::ConversionOverflow)),
                    };
                return PlatformSendResult {
                    requested,
                    inserted,
                    started_ticks,
                    completed_ticks: Some(now),
                    completed_us,
                    win32_error,
                    timing_error,
                };
            }
            Ok(_) => std::hint::spin_loop(),
            Err(error) => {
                return PlatformSendResult {
                    requested,
                    inserted: 0,
                    started_ticks,
                    completed_ticks: None,
                    completed_us: 0,
                    win32_error,
                    timing_error: Some(error),
                };
            }
        }
    }
}
