#![cfg(any(test, feature = "test-support"))]

use super::fault_injection::{FaultInjectionScript, InjectedSendOutcome};
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::{QpcClock, QpcError, QpcTicks};
use sky_dispatch_win32::input::{
    InstrumentPhysicalState, PacketRetryReason, PhysicalPacket, PlatformSendResult, SendEvidence,
    SendTransactionOutcome, SendTransactionStatus, TrackedKeyState, scan_codes_from_mask,
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
    let send_call_count = script.send_call_count.clone();
    let script_emitter = Arc::clone(&script);
    let call_index_emitter = Arc::clone(&call_index);
    let mut backend = TrackedKeyState::with_qpc_clock(qpc_clock);
    backend.set_emitter(move |codes, _key_up| {
        if let Some(counter) = &send_call_count {
            counter.fetch_add(1, Ordering::SeqCst);
        }
        let idx = call_index_emitter.fetch_add(1, Ordering::Relaxed) as usize;

        let base_latency_us =
            latency_base_us.saturating_add(latency_per_key_us.saturating_mul(codes.len() as u64));
        let sender_started_ticks = match qpc_clock.now() {
            Ok(ticks) => ticks,
            Err(error) => {
                return mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Err(error),
                    codes.len() as u8,
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
                    codes.len() as u8,
                    codes.len() as u8,
                    0,
                    0,
                )
            }
            Some(InjectedSendOutcome::Full { latency_ticks }) => {
                mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Ok(sender_started_ticks),
                    codes.len() as u8,
                    codes.len() as u8,
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
                codes.len() as u8,
                0,
                *win32_error,
                *latency_ticks,
            ),
            Some(InjectedSendOutcome::Partial {
                inserted,
                latency_ticks,
                win32_error,
            }) => {
                let inserted = (*inserted).min(codes.len() as u8);
                mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Ok(sender_started_ticks),
                    codes.len() as u8,
                    inserted,
                    *win32_error,
                    *latency_ticks,
                )
            }
            Some(InjectedSendOutcome::Stall { duration_ticks }) => {
                mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Ok(sender_started_ticks),
                    codes.len() as u8,
                    0,
                    0,
                    *duration_ticks,
                )
            }
            Some(InjectedSendOutcome::PanicAfterSend) => {
                let _ = mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Ok(sender_started_ticks),
                    codes.len() as u8,
                    codes.len() as u8,
                    0,
                    0,
                );
                panic!("fault injection: panic after send before commit");
            }
            Some(InjectedSendOutcome::QpcFailureAfterSend) => {
                let mut result = mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Ok(sender_started_ticks),
                    codes.len() as u8,
                    codes.len() as u8,
                    0,
                    0,
                );
                result.timing_error = Some(QpcError::CounterUnavailable);
                result
            }
            // The packet-emitter path below is the only mock path that can
            // model this typed no-syscall sender result. Keep the legacy
            // scan-code emitter total for tests that exercise it directly.
            Some(InjectedSendOutcome::DeadlineMissedBeforeSend) => {
                mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Ok(sender_started_ticks),
                    codes.len() as u8,
                    codes.len() as u8,
                    0,
                    0,
                )
            }
        }
    });
    if let Some(counter) = script.full_instrument_release_calls.clone() {
        backend.set_full_instrument_release_counter(counter);
    }
    if let Some(flag) = script.force_preflight_failure.clone() {
        backend.set_force_preflight_failure(flag);
    }
    let script_packet = Arc::clone(&script);
    let call_index_packet = Arc::clone(&call_index);
    let send_call_count_packet = script.send_call_count.clone();
    backend.set_packet_emitter(move |packet| {
        if let Some(counter) = &send_call_count_packet {
            counter.fetch_add(1, Ordering::SeqCst);
        }
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
    // The mock has no real keyboard, so a deterministic physical probe mirrors
    // transport confirmation: keys the transport confirmed as released read as
    // AllUp; anything still unconfirmed reads Held. This keeps the cleanup FSM
    // synthesized-verification semantics while retaining the V3-1 guarantee
    // that a bespoke test probe must be explicit rather than invented by the
    // emitter itself.
    let probe_control = script.force_inconclusive_probe.clone();
    backend.set_probe(move |unresolved_mask, confirmed_mask| {
        if probe_control
            .as_ref()
            .is_some_and(|force| force.load(Ordering::Acquire))
        {
            InstrumentPhysicalState::Inconclusive
        } else if confirmed_mask == unresolved_mask {
            InstrumentPhysicalState::AllUp
        } else {
            InstrumentPhysicalState::Held(scan_codes_from_mask(unresolved_mask & !confirmed_mask))
        }
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
) -> SendTransactionOutcome {
    let requested_mask = packet.up_mask | packet.down_mask;
    let requested = packet.event_count();
    let total_latency_us =
        latency_base_us.saturating_add(latency_per_key_us.saturating_mul(u64::from(requested)));
    let started_ticks = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return SendTransactionOutcome {
                status: SendTransactionStatus::ClockFailureBeforeSend,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: 0,
                    attempts: 0,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: None,
                    completed_ticks: None,
                    timing_error: Some(error),
                },
            };
        }
    };
    if total_latency_us > 0 {
        std::thread::sleep(Duration::from_micros(total_latency_us));
    }
    let scripted = script.resolve(call_index);
    if matches!(
        scripted,
        Some(InjectedSendOutcome::DeadlineMissedBeforeSend)
    ) {
        return SendTransactionOutcome {
            status: SendTransactionStatus::DeadlineMissedBeforeSend,
            evidence: SendEvidence {
                requested_mask,
                confirmed_mask: 0,
                skipped_mask: 0,
                first_inserted: 0,
                attempts: 0,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: None,
                last_win32_error: None,
                started_ticks: Some(started_ticks),
                completed_ticks: None,
                timing_error: None,
            },
        };
    }
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
            return SendTransactionOutcome {
                status: SendTransactionStatus::ClockFailureAfterSend,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: requested,
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(started_ticks),
                    completed_ticks: None,
                    timing_error: Some(QpcError::CounterUnavailable),
                },
            };
        }
        Some(InjectedSendOutcome::DeadlineMissedBeforeSend) => {
            unreachable!("DeadlineMissedBeforeSend handled before the transport outcome match")
        }
    };
    let deadline = match started_ticks.checked_add_duration(DurationTicks::from_raw(latency_ticks))
    {
        Ok(deadline) => deadline,
        Err(_) => {
            return SendTransactionOutcome {
                status: SendTransactionStatus::ClockFailureAfterSend,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: inserted,
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(started_ticks),
                    completed_ticks: None,
                    timing_error: Some(QpcError::DeadlineOverflow),
                },
            };
        }
    };
    let completed_ticks = loop {
        match qpc_clock.now() {
            Ok(now) if now >= deadline => break now,
            Ok(_) => std::hint::spin_loop(),
            Err(error) => {
                return SendTransactionOutcome {
                    status: SendTransactionStatus::ClockFailureAfterSend,
                    evidence: SendEvidence {
                        requested_mask,
                        confirmed_mask: 0,
                        skipped_mask: 0,
                        first_inserted: inserted,
                        attempts: 1,
                        zero_progress_retries: 0,
                        retry_reason: PacketRetryReason::None,
                        first_win32_error: None,
                        last_win32_error: None,
                        started_ticks: Some(started_ticks),
                        completed_ticks: None,
                        timing_error: Some(error),
                    },
                };
            }
        }
    };

    let status = if inserted >= requested {
        SendTransactionStatus::Complete
    } else if inserted == 0 {
        SendTransactionStatus::ZeroProgress
    } else if !packet.is_up_only() {
        SendTransactionStatus::IntegrityLost
    } else {
        SendTransactionStatus::PartialProgress
    };

    let err_opt = (win32_error != 0).then_some(win32_error);

    SendTransactionOutcome {
        status,
        evidence: SendEvidence {
            requested_mask,
            confirmed_mask: if matches!(status, SendTransactionStatus::Complete) {
                requested_mask
            } else {
                0
            },
            skipped_mask: 0,
            first_inserted: inserted,
            attempts: 1,
            zero_progress_retries: 0,
            retry_reason: PacketRetryReason::None,
            first_win32_error: err_opt,
            last_win32_error: err_opt,
            started_ticks: Some(started_ticks),
            completed_ticks: Some(completed_ticks),
            timing_error: None,
        },
    }
}

fn mock_platform_send_result_from_started_ticks(
    qpc_clock: QpcClock,
    started_ticks: Result<QpcTicks, QpcError>,
    requested: u8,
    inserted: u8,
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
                win32_error,
                timing_error: Some(QpcError::DeadlineOverflow),
            };
        }
    };
    loop {
        match qpc_clock.now() {
            Ok(now) if now >= deadline => {
                return PlatformSendResult {
                    requested,
                    inserted,
                    started_ticks,
                    completed_ticks: Some(now),
                    win32_error,
                    timing_error: None,
                };
            }
            Ok(_) => std::hint::spin_loop(),
            Err(error) => {
                return PlatformSendResult {
                    requested,
                    inserted: 0,
                    started_ticks,
                    completed_ticks: None,
                    win32_error,
                    timing_error: Some(error),
                };
            }
        }
    }
}
