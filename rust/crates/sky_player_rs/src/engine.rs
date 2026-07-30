//! End-to-End Real-Time Native Dispatch Session Engine for sky_player_rs.

use parking_lot::Mutex;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::coordinator::RuntimeDispatchCoordinator;
use sky_dispatch_core::estimator::SendLatencyEstimator;
use sky_dispatch_core::model::{ActionKind, RuntimeSchedule};
use sky_dispatch_win32::clock::qpc_now_us;
use sky_dispatch_win32::input::TrackedKeyState;
use sky_dispatch_win32::mmcss::MmcssGuard;
use sky_dispatch_win32::sleeper::sleep_until_us;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineSnapshot {
    pub elapsed_us: u64,
    pub total_us: u64,
    pub lateness_us: u64,
    pub max_lateness_us: u64,
    pub late_2ms: u64,
    pub late_5ms: u64,
    pub late_10ms: u64,
    pub is_running: bool,
    pub is_finished: bool,
    pub is_paused: bool,
    pub status: String,
    pub active_count: usize,
    pub keys_dropped: u64,
    pub chord_split_events: u64,
}

pub struct NativeDispatchSession {
    schedule: RuntimeSchedule,
    min_hold_us: u64,
    estimator: Arc<Mutex<SendLatencyEstimator>>,
    backend: Arc<Mutex<TrackedKeyState>>,

    pause_requested: Arc<AtomicBool>,
    quit_requested: Arc<AtomicBool>,
    skip_requested: Arc<AtomicBool>,
    is_running: Arc<AtomicBool>,
    is_finished: Arc<AtomicBool>,
    is_paused: Arc<AtomicBool>,

    elapsed_us: Arc<AtomicU64>,
    total_us: u64,
    lateness_us: Arc<AtomicU64>,
    max_lateness_us: Arc<AtomicU64>,
    late_2ms: Arc<AtomicU64>,
    late_5ms: Arc<AtomicU64>,
    late_10ms: Arc<AtomicU64>,

    thread_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl NativeDispatchSession {
    pub fn new(
        schedule: RuntimeSchedule,
        min_hold_us: u64,
        max_lead_us: u64,
        allowed_scan_codes: Vec<u16>,
        mock_backend: bool,
    ) -> Self {
        let total_us = schedule.batches.last().map_or(0, |b| b.scheduled_us);
        let backend_state = if mock_backend {
            TrackedKeyState::with_emitter(|codes, _key_up| {
                sky_dispatch_win32::input::PlatformSendResult {
                    requested: codes.len() as u32,
                    inserted: codes.len() as u32,
                    completed_us: qpc_now_us(),
                    win32_error: 0,
                }
            })
        } else {
            TrackedKeyState::new()
        };

        Self {
            schedule,
            min_hold_us,
            estimator: Arc::new(Mutex::new(SendLatencyEstimator::new(
                0.2,
                max_lead_us,
                allowed_scan_codes.len(),
            ))),
            backend: Arc::new(Mutex::new(backend_state)),

            pause_requested: Arc::new(AtomicBool::new(false)),
            quit_requested: Arc::new(AtomicBool::new(false)),
            skip_requested: Arc::new(AtomicBool::new(false)),
            is_running: Arc::new(AtomicBool::new(false)),
            is_finished: Arc::new(AtomicBool::new(false)),
            is_paused: Arc::new(AtomicBool::new(false)),

            elapsed_us: Arc::new(AtomicU64::new(0)),
            total_us,
            lateness_us: Arc::new(AtomicU64::new(0)),
            max_lateness_us: Arc::new(AtomicU64::new(0)),
            late_2ms: Arc::new(AtomicU64::new(0)),
            late_5ms: Arc::new(AtomicU64::new(0)),
            late_10ms: Arc::new(AtomicU64::new(0)),

            thread_handle: Mutex::new(None),
        }
    }

    pub fn start(&self) {
        if self.is_running.swap(true, Ordering::SeqCst) {
            return;
        }

        let schedule = self.schedule.clone();
        let min_hold_us = self.min_hold_us;
        let estimator = Arc::clone(&self.estimator);
        let backend = Arc::clone(&self.backend);

        let pause_requested = Arc::clone(&self.pause_requested);
        let quit_requested = Arc::clone(&self.quit_requested);
        let skip_requested = Arc::clone(&self.skip_requested);
        let is_running = Arc::clone(&self.is_running);
        let is_finished = Arc::clone(&self.is_finished);
        let is_paused = Arc::clone(&self.is_paused);

        let elapsed_us = Arc::clone(&self.elapsed_us);
        let lateness_us = Arc::clone(&self.lateness_us);
        let max_lateness_us = Arc::clone(&self.max_lateness_us);
        let late_2ms = Arc::clone(&self.late_2ms);
        let late_5ms = Arc::clone(&self.late_5ms);
        let late_10ms = Arc::clone(&self.late_10ms);

        let handle = std::thread::spawn(move || {
            let _mmcss = MmcssGuard::join_pro_audio();
            let mut coordinator = RuntimeDispatchCoordinator::new(schedule.clone(), min_hold_us);
            let session_start_us = qpc_now_us();
            let mut clock_state = PlaybackClockState::new(session_start_us, 0);

            while !coordinator.is_finished() {
                let now_us = qpc_now_us();

                if quit_requested.load(Ordering::Relaxed) || skip_requested.load(Ordering::Relaxed)
                {
                    break;
                }

                if pause_requested.load(Ordering::Relaxed) {
                    if !clock_state.is_paused() {
                        clock_state.enter_pause("pause", now_us);
                        is_paused.store(true, Ordering::Relaxed);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                } else if clock_state.is_paused() {
                    clock_state.exit_pause("pause", now_us);
                    is_paused.store(false, Ordering::Relaxed);
                }

                let effective_now_us = clock_state.get_elapsed_us(now_us);
                elapsed_us.store(effective_now_us, Ordering::Relaxed);

                // 1. Check and drain due pending releases
                let due_pending = coordinator.pop_due_pending(effective_now_us, 0);
                if !due_pending.is_empty() {
                    let scan_codes: Vec<u16> = due_pending.iter().map(|p| p.scan_code).collect();
                    let res = backend.lock().key_up(&scan_codes);
                    let sent_codes = res.sent;

                    let dispatch_completed_us = res.send_completed_us;
                    let comp_effective = clock_state.get_elapsed_us(dispatch_completed_us);

                    coordinator.complete_releases(&due_pending, &sent_codes, &[]);
                    estimator.lock().update(
                        ActionKind::Up,
                        dispatch_completed_us.saturating_sub(now_us),
                        sent_codes.len(),
                    );

                    let late = comp_effective.saturating_sub(due_pending[0].scheduled_release_us);
                    lateness_us.store(late, Ordering::Relaxed);
                    max_lateness_us.fetch_max(late, Ordering::Relaxed);

                    if late >= 10_000 {
                        late_10ms.fetch_add(1, Ordering::Relaxed);
                    } else if late >= 5_000 {
                        late_5ms.fetch_add(1, Ordering::Relaxed);
                    } else if late >= 2_000 {
                        late_2ms.fetch_add(1, Ordering::Relaxed);
                    }
                    continue;
                }

                // 2. Check authored batch
                let popped = coordinator.pop_next_due_authored(effective_now_us, 0);
                if let Some((batch, _lead)) = popped {
                    if batch.kind == ActionKind::Down {
                        let (playable, _conflicts) = coordinator.split_down_intents(&batch.intents);
                        if !playable.is_empty() {
                            let scan_codes: Vec<u16> =
                                playable.iter().map(|i| i.scan_code).collect();
                            let res = backend.lock().key_down(&scan_codes);
                            let sent_codes = res.sent;
                            let dispatch_completed_us = res.send_completed_us;
                            let comp_effective = clock_state.get_elapsed_us(dispatch_completed_us);

                            coordinator.activate_sent_downs(
                                &playable,
                                &sent_codes,
                                effective_now_us,
                                comp_effective,
                            );
                            estimator.lock().update(
                                ActionKind::Down,
                                dispatch_completed_us.saturating_sub(now_us),
                                sent_codes.len(),
                            );

                            let late = comp_effective.saturating_sub(batch.scheduled_us);
                            lateness_us.store(late, Ordering::Relaxed);
                            max_lateness_us.fetch_max(late, Ordering::Relaxed);

                            if late >= 10_000 {
                                late_10ms.fetch_add(1, Ordering::Relaxed);
                            } else if late >= 5_000 {
                                late_5ms.fetch_add(1, Ordering::Relaxed);
                            } else if late >= 2_000 {
                                late_2ms.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    } else if batch.kind == ActionKind::Up {
                        let (_requested, suppressed) = coordinator.request_releases(&batch.intents);
                        if !suppressed.is_empty() {
                            // Stale releases suppressed
                        }
                    }
                    continue;
                }

                // Sleep to next deadline
                let next_dl = coordinator.next_deadline_us(0, 0);
                if let Some(dl_us) = next_dl {
                    if dl_us > effective_now_us {
                        let wait_us = dl_us - effective_now_us;
                        let target_qpc = now_us + wait_us;
                        sleep_until_us(target_qpc, 150);
                    }
                } else {
                    std::thread::sleep(std::time::Duration::from_micros(200));
                }
            }

            // Cleanup on termination
            let _ = backend.lock().release_all_full_instrument();
            is_running.store(false, Ordering::SeqCst);
            is_finished.store(true, Ordering::SeqCst);
        });

        *self.thread_handle.lock() = Some(handle);
    }

    pub fn pause(&self) {
        self.pause_requested.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.pause_requested.store(false, Ordering::SeqCst);
    }

    pub fn skip(&self) {
        self.skip_requested.store(true, Ordering::SeqCst);
    }

    pub fn quit(&self) {
        self.quit_requested.store(true, Ordering::SeqCst);
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        let b = self.backend.lock();
        EngineSnapshot {
            elapsed_us: self.elapsed_us.load(Ordering::Relaxed),
            total_us: self.total_us,
            lateness_us: self.lateness_us.load(Ordering::Relaxed),
            max_lateness_us: self.max_lateness_us.load(Ordering::Relaxed),
            late_2ms: self.late_2ms.load(Ordering::Relaxed),
            late_5ms: self.late_5ms.load(Ordering::Relaxed),
            late_10ms: self.late_10ms.load(Ordering::Relaxed),
            is_running: self.is_running.load(Ordering::Relaxed),
            is_finished: self.is_finished.load(Ordering::Relaxed),
            is_paused: self.is_paused.load(Ordering::Relaxed),
            status: if self.is_finished.load(Ordering::Relaxed) {
                "finished".to_string()
            } else if self.is_paused.load(Ordering::Relaxed) {
                "paused".to_string()
            } else {
                "playing".to_string()
            },
            active_count: b.active_keys.len(),
            keys_dropped: b.keys_dropped,
            chord_split_events: b.chord_split_events,
        }
    }

    pub fn join(&self) {
        let handle = self.thread_handle.lock().take();
        if let Some(h) = handle {
            let _ = h.join();
        }
    }
}
