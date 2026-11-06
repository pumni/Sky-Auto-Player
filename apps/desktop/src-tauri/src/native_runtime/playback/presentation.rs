use super::*;

impl PlaybackPresentationMessage {
    pub(in crate::native_runtime) fn order(&self) -> u64 {
        match self {
            Self::State { order, .. } | Self::Finalize { order, .. } => *order,
        }
    }
}

impl PlaybackPresentationQueue {
    pub(in crate::native_runtime) fn record_snapshot_drop(&self) {
        self.snapshot_dropped.fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::native_runtime) fn metrics(&self) -> PlaybackPresentationMetrics {
        PlaybackPresentationMetrics {
            snapshot_coalesced: self.snapshot_coalesced.load(Ordering::Acquire),
            snapshot_dropped: self.snapshot_dropped.load(Ordering::Acquire),
        }
    }

    pub(in crate::native_runtime) fn record_failure(&self, reason: &str) {
        self.failed.store(true, Ordering::Release);
        self.closed.store(true, Ordering::Release);
        if let Ok(mut failure_reason) = self.failure_reason.try_lock() {
            failure_reason.get_or_insert_with(|| reason.to_string());
        }
    }

    pub(in crate::native_runtime) fn failure_reason(&self) -> Option<String> {
        self.failure_reason
            .try_lock()
            .ok()
            .and_then(|reason| reason.clone())
    }
}

impl NativePlaybackPresentation {
    pub(in crate::native_runtime) fn start(
        active: &Arc<NativeActivePlayback>,
        events: Arc<Mutex<NativeEventHub>>,
        diagnostics_gate: Arc<DiagnosticsPublicationGate>,
        #[cfg(all(test, feature = "tauri-test"))] test_hook: Option<PlaybackPresentationTestHook>,
    ) -> Result<Arc<Self>, String> {
        let (critical, receiver) = mpsc::sync_channel(PLAYBACK_PRESENTATION_CRITICAL_CAPACITY);
        let queue = Arc::new(PlaybackPresentationQueue::default());
        let weak_active = Arc::downgrade(active);
        let worker_queue = Arc::clone(&queue);
        let worker = thread::Builder::new()
            .name("sky-native-playback-presentation".into())
            .spawn(move || {
                let current_thread = thread::current();
                let _ = worker_queue.worker_thread.set(current_thread);
                playback_presentation_worker(
                    weak_active,
                    events,
                    diagnostics_gate,
                    worker_queue,
                    receiver,
                    #[cfg(all(test, feature = "tauri-test"))]
                    test_hook,
                );
            })
            .map_err(|error| format!("could not start playback presentation consumer: {error}"))?;
        Ok(Arc::new(Self {
            critical,
            queue,
            worker: Mutex::new(Some(worker)),
        }))
    }

    pub(in crate::native_runtime) fn publish_state(
        &self,
        active: &NativeActivePlayback,
        state: PlaybackEventState,
        message: Option<String>,
        outcome: Option<String>,
    ) -> Result<(), String> {
        let order = self.queue.next_order.fetch_add(1, Ordering::Relaxed) + 1;
        self.try_publish_critical(
            active,
            PlaybackPresentationMessage::State {
                order,
                state,
                message,
                outcome,
            },
        )
    }

    pub(in crate::native_runtime) fn publish_snapshot(
        &self,
        active: &NativeActivePlayback,
        elapsed_us: u64,
        pre_roll_remaining_us: u64,
        status: EnginePollStatus,
    ) -> bool {
        if self.queue.closed.load(Ordering::Acquire) || active.done.load(Ordering::Acquire) {
            self.queue.snapshot_dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let frame = PlaybackPresentationFrame {
            order: self.queue.next_order.fetch_add(1, Ordering::Relaxed) + 1,
            elapsed_us,
            pre_roll_remaining_us,
            status,
        };
        match self.queue.latest_snapshot.try_lock() {
            Ok(mut latest) => {
                if self.queue.closed.load(Ordering::Acquire) || active.done.load(Ordering::Acquire)
                {
                    self.queue.record_snapshot_drop();
                    return false;
                }
                if latest.replace(frame).is_some() {
                    self.queue
                        .snapshot_coalesced
                        .fetch_add(1, Ordering::Relaxed);
                    self.queue.snapshot_dropped.fetch_add(1, Ordering::Relaxed);
                }
                if let Some(worker) = self.queue.worker_thread.get() {
                    worker.unpark();
                }
                true
            }
            Err(TryLockError::WouldBlock | TryLockError::Poisoned(_)) => {
                self.queue.snapshot_dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    pub(in crate::native_runtime) fn finalize(
        &self,
        active: &NativeActivePlayback,
        finalization: PlaybackPresentationFinalization,
        completed: PlaybackPresentationCompletion,
    ) -> Result<(), String> {
        let order = self.queue.next_order.fetch_add(1, Ordering::Relaxed) + 1;
        self.try_publish_critical(
            active,
            PlaybackPresentationMessage::Finalize {
                order,
                finalization,
                completed,
            },
        )
    }

    pub(in crate::native_runtime) fn try_publish_critical(
        &self,
        active: &NativeActivePlayback,
        message: PlaybackPresentationMessage,
    ) -> Result<(), String> {
        if self.queue.closed.load(Ordering::Acquire) || self.queue.failed.load(Ordering::Acquire) {
            let error = self
                .queue
                .failure_reason()
                .unwrap_or_else(|| "playback presentation consumer is closed".into());
            self.fail(active, &error);
            reject_presentation_message(message, &error);
            return Err(error);
        }
        match self.critical.try_send(message) {
            Ok(()) => {
                if let Some(worker) = self.queue.worker_thread.get() {
                    worker.unpark();
                }
                Ok(())
            }
            Err(mpsc::TrySendError::Full(message)) => {
                let error = "playback presentation lifecycle queue overflow".to_string();
                self.fail(active, &error);
                reject_presentation_message(message, &error);
                Err(error)
            }
            Err(mpsc::TrySendError::Disconnected(message)) => {
                let error = "playback presentation consumer is unavailable".to_string();
                self.fail(active, &error);
                reject_presentation_message(message, &error);
                Err(error)
            }
        }
    }

    pub(in crate::native_runtime) fn fail(&self, active: &NativeActivePlayback, reason: &str) {
        self.queue.record_failure(reason);
        active.event_delivery_failure.store(true, Ordering::Release);
        cleanup_failed_event_delivery(active);
        if let Some(worker) = self.queue.worker_thread.get() {
            worker.unpark();
        }
    }
}

pub(in crate::native_runtime) fn reject_presentation_message(
    message: PlaybackPresentationMessage,
    error: &str,
) {
    if let PlaybackPresentationMessage::Finalize { completed, .. } = message {
        completed(Err(error.to_string()));
    }
}

impl Drop for NativePlaybackPresentation {
    fn drop(&mut self) {
        self.queue.closed.store(true, Ordering::Release);
        if let Some(worker) = self.queue.worker_thread.get() {
            worker.unpark();
        }
        if let Ok(worker) = self.worker.get_mut()
            && worker.as_ref().is_some_and(thread::JoinHandle::is_finished)
            && let Some(worker) = worker.take()
        {
            let _ = worker.join();
        }
    }
}

pub(in crate::native_runtime) fn playback_presentation_worker(
    active: std::sync::Weak<NativeActivePlayback>,
    events: Arc<Mutex<NativeEventHub>>,
    diagnostics_gate: Arc<DiagnosticsPublicationGate>,
    queue: Arc<PlaybackPresentationQueue>,
    receiver: mpsc::Receiver<PlaybackPresentationMessage>,
    #[cfg(all(test, feature = "tauri-test"))] test_hook: Option<PlaybackPresentationTestHook>,
) {
    let mut pending_critical = None;
    let mut pending_snapshot = None;
    loop {
        #[cfg(all(test, feature = "tauri-test"))]
        if let Some(hook) = &test_hook {
            hook(PlaybackPresentationStage::BeforeWorkSelection);
        }

        if pending_critical.is_none() {
            pending_critical = receiver.try_recv().ok();
        }
        match queue.latest_snapshot.try_lock() {
            Ok(mut latest) => {
                if let Some(snapshot) = latest.take()
                    && pending_snapshot.replace(snapshot).is_some()
                {
                    queue.snapshot_coalesced.fetch_add(1, Ordering::Relaxed);
                    queue.record_snapshot_drop();
                }
            }
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Poisoned(poisoned)) => {
                if poisoned.into_inner().take().is_some() {
                    queue.record_snapshot_drop();
                }
                discard_pending_presentation_snapshot(&queue, &mut pending_snapshot);
                queue.record_failure("playback presentation snapshot slot was poisoned");
                if let Some(active) = active.upgrade() {
                    active.event_delivery_failure.store(true, Ordering::Release);
                    cleanup_failed_event_delivery(&active);
                }
                reject_queued_presentation_finalizers(
                    &receiver,
                    "playback presentation snapshot slot was poisoned",
                );
                report_presentation_snapshot_metrics(&queue, &events);
                return;
            }
        }

        let work = match (pending_critical.as_ref(), pending_snapshot.as_ref()) {
            (Some(critical), Some(snapshot)) if critical.order() <= snapshot.order => {
                Some(PresentationWork::Critical(
                    pending_critical.take().expect("critical work is present"),
                ))
            }
            (Some(_), Some(_)) => Some(PresentationWork::Snapshot(
                pending_snapshot.take().expect("snapshot work is present"),
            )),
            (Some(_), None) => Some(PresentationWork::Critical(
                pending_critical.take().expect("critical work is present"),
            )),
            (None, Some(_)) => Some(PresentationWork::Snapshot(
                pending_snapshot.take().expect("snapshot work is present"),
            )),
            (None, None) => None,
        };

        let Some(work) = work else {
            if queue.closed.load(Ordering::Acquire) {
                discard_pending_presentation_snapshot(&queue, &mut pending_snapshot);
                discard_latest_presentation_snapshot(&queue);
                report_presentation_snapshot_metrics(&queue, &events);
                return;
            }
            thread::park_timeout(Duration::from_millis(50));
            continue;
        };

        #[cfg(all(test, feature = "tauri-test"))]
        if let Some(hook) = &test_hook {
            hook(PlaybackPresentationStage::AfterWorkSelection);
        }

        let snapshot_work = matches!(&work, PresentationWork::Snapshot(_));
        let mut snapshot_published = false;
        let result = catch_unwind(AssertUnwindSafe(|| {
            process_playback_presentation_work(
                work,
                &active,
                &events,
                &diagnostics_gate,
                &queue,
                &mut snapshot_published,
                #[cfg(all(test, feature = "tauri-test"))]
                &test_hook,
            )
        }));
        match result {
            Ok(Ok(true)) => {
                discard_pending_presentation_snapshot(&queue, &mut pending_snapshot);
                discard_latest_presentation_snapshot(&queue);
                report_presentation_snapshot_metrics(&queue, &events);
                return;
            }
            Ok(Ok(false)) => {}
            Ok(Err(error)) => {
                if snapshot_work && !snapshot_published {
                    queue.record_snapshot_drop();
                }
                queue.record_failure(&error);
                if let Some(active) = active.upgrade() {
                    active.event_delivery_failure.store(true, Ordering::Release);
                    cleanup_failed_event_delivery(&active);
                }
                if let Some(PlaybackPresentationMessage::Finalize { completed, .. }) =
                    pending_critical.take()
                {
                    completed(Err(error));
                }
                reject_queued_presentation_finalizers(
                    &receiver,
                    "playback presentation consumer failed before finalization",
                );
                discard_pending_presentation_snapshot(&queue, &mut pending_snapshot);
                discard_latest_presentation_snapshot(&queue);
                report_presentation_snapshot_metrics(&queue, &events);
                return;
            }
            Err(_) => {
                if snapshot_work && !snapshot_published {
                    queue.record_snapshot_drop();
                }
                queue.record_failure("playback presentation consumer panicked");
                if let Some(active) = active.upgrade() {
                    active.event_delivery_failure.store(true, Ordering::Release);
                    cleanup_failed_event_delivery(&active);
                }
                if let Some(PlaybackPresentationMessage::Finalize { completed, .. }) =
                    pending_critical.take()
                {
                    completed(Err(
                        "playback presentation consumer panicked during finalization".into(),
                    ));
                }
                reject_queued_presentation_finalizers(
                    &receiver,
                    "playback presentation consumer panicked before finalization",
                );
                discard_pending_presentation_snapshot(&queue, &mut pending_snapshot);
                discard_latest_presentation_snapshot(&queue);
                report_presentation_snapshot_metrics(&queue, &events);
                return;
            }
        }
    }
}

pub(in crate::native_runtime) fn discard_pending_presentation_snapshot(
    queue: &PlaybackPresentationQueue,
    pending_snapshot: &mut Option<PlaybackPresentationFrame>,
) {
    if pending_snapshot.take().is_some() {
        queue.record_snapshot_drop();
    }
}

pub(in crate::native_runtime) fn discard_latest_presentation_snapshot(
    queue: &PlaybackPresentationQueue,
) {
    match queue.latest_snapshot.lock() {
        Ok(mut snapshot) => {
            if snapshot.take().is_some() {
                queue.record_snapshot_drop();
            }
        }
        Err(poisoned) => {
            if poisoned.into_inner().take().is_some() {
                queue.record_snapshot_drop();
            }
        }
    }
}

pub(in crate::native_runtime) fn report_presentation_snapshot_metrics(
    queue: &PlaybackPresentationQueue,
    events: &Arc<Mutex<NativeEventHub>>,
) {
    let session_metrics = queue.metrics();
    let publisher_metrics = match events.lock() {
        Ok(hub) => hub.snapshot_metrics(),
        Err(poisoned) => poisoned.into_inner().snapshot_metrics(),
    };
    let _ = writeln!(
        std::io::stderr().lock(),
        "playback_presentation_snapshots session_coalesced={} session_dropped={} publisher_coalesced={} publisher_dropped={}",
        session_metrics.snapshot_coalesced,
        session_metrics.snapshot_dropped,
        publisher_metrics.snapshot_coalesced,
        publisher_metrics.snapshot_dropped,
    );
}

pub(in crate::native_runtime) fn reject_queued_presentation_finalizers(
    receiver: &mpsc::Receiver<PlaybackPresentationMessage>,
    error: &str,
) {
    while let Ok(message) = receiver.try_recv() {
        reject_presentation_message(message, error);
    }
}

pub(in crate::native_runtime) enum PresentationWork {
    Critical(PlaybackPresentationMessage),
    Snapshot(PlaybackPresentationFrame),
}

pub(in crate::native_runtime) fn process_playback_presentation_work(
    work: PresentationWork,
    active: &std::sync::Weak<NativeActivePlayback>,
    events: &Arc<Mutex<NativeEventHub>>,
    diagnostics_gate: &DiagnosticsPublicationGate,
    queue: &PlaybackPresentationQueue,
    snapshot_published: &mut bool,
    #[cfg(all(test, feature = "tauri-test"))] test_hook: &Option<PlaybackPresentationTestHook>,
) -> Result<bool, String> {
    let Some(active) = active.upgrade() else {
        if matches!(&work, PresentationWork::Snapshot(_)) {
            queue.record_snapshot_drop();
        }
        return Ok(true);
    };
    match work {
        PresentationWork::Snapshot(frame) => {
            if queue.closed.load(Ordering::Acquire)
                || active.done.load(Ordering::Acquire)
                || active.event_delivery_failure.load(Ordering::Acquire)
            {
                queue.record_snapshot_drop();
                return Ok(false);
            }
            #[cfg(all(test, feature = "tauri-test"))]
            if let Some(hook) = test_hook {
                hook(PlaybackPresentationStage::SnapshotProjection);
            }
            if queue.closed.load(Ordering::Acquire)
                || active.done.load(Ordering::Acquire)
                || active.event_delivery_failure.load(Ordering::Acquire)
            {
                queue.record_snapshot_drop();
                return Ok(false);
            }
            publish_playback_snapshot_cancellable(
                events,
                &active,
                frame.elapsed_us,
                frame.pre_roll_remaining_us,
                frame.status,
                &queue.closed,
            )?;
            *snapshot_published = true;
            if queue.closed.load(Ordering::Acquire)
                || active.event_delivery_failure.load(Ordering::Acquire)
            {
                return Ok(false);
            }
            #[cfg(all(test, feature = "tauri-test"))]
            if let Some(hook) = test_hook {
                hook(PlaybackPresentationStage::DiagnosticsProjection);
            }
            if queue.closed.load(Ordering::Acquire) {
                return Ok(false);
            }
            publish_diagnostics_snapshot_inner_cancellable(
                events,
                &active,
                diagnostics_gate,
                false,
                &queue.closed,
            )?;
            Ok(false)
        }
        PresentationWork::Critical(PlaybackPresentationMessage::State {
            state,
            message,
            outcome,
            ..
        }) => {
            if queue.closed.load(Ordering::Acquire)
                || active.event_delivery_failure.load(Ordering::Acquire)
            {
                return Ok(false);
            }
            publish_playback_state(events, &active, state, message, outcome)?;
            Ok(false)
        }
        PresentationWork::Critical(PlaybackPresentationMessage::Finalize {
            finalization,
            completed,
            ..
        }) => {
            if queue.closed.load(Ordering::Acquire)
                || active.event_delivery_failure.load(Ordering::Acquire)
            {
                let error = queue.failure_reason().unwrap_or_else(|| {
                    "playback presentation was already failed before finalization".into()
                });
                completed(Err(error.clone()));
                return Err(error);
            }
            let finalization = catch_unwind(AssertUnwindSafe(|| {
                #[cfg(all(test, feature = "tauri-test"))]
                if let Some(hook) = test_hook {
                    hook(PlaybackPresentationStage::DiagnosticsProjection);
                }
                publish_diagnostics_snapshot_inner_cancellable(
                    events,
                    &active,
                    diagnostics_gate,
                    true,
                    &queue.closed,
                )?;
                if queue.closed.load(Ordering::Acquire) {
                    return Err("playback presentation closed during final diagnostics".into());
                }
                publish_playback_state(
                    events,
                    &active,
                    finalization.state,
                    finalization.message,
                    finalization.outcome,
                )?;
                publish_terminal_event(events, &active, &finalization.publication)
            }))
            .unwrap_or_else(|_| Err("playback presentation panicked during finalization".into()));
            let completed_successfully = finalization.is_ok();
            completed(finalization.clone());
            if completed_successfully {
                queue.closed.store(true, Ordering::Release);
                Ok(true)
            } else {
                let error = finalization.expect_err("presentation finalization failed");
                queue.record_failure(&error);
                Err(error)
            }
        }
    }
}

#[cfg(all(test, feature = "tauri-test"))]
pub(in crate::native_runtime) fn native_event_delivery_failure(
    events: &Arc<Mutex<NativeEventHub>>,
) -> Option<String> {
    match events.lock() {
        Ok(hub) => hub.delivery_failure(),
        Err(_) => Some("native event hub lock poisoned".into()),
    }
}

pub(in crate::native_runtime) fn set_playback_state(
    active: &NativeActivePlayback,
    state: PlaybackSessionState,
) -> Result<(), String> {
    *active
        .state
        .lock()
        .map_err(|_| "native playback state lock poisoned".to_string())? = state;
    Ok(())
}

pub(in crate::native_runtime) fn dry_run_elapsed(active: &NativeActivePlayback) -> u64 {
    let paused_total = active
        .paused_total
        .lock()
        .map(|value| *value)
        .unwrap_or_default();
    let current_pause = active
        .paused_since
        .lock()
        .ok()
        .and_then(|value| *value)
        .map(|value| Instant::now().saturating_duration_since(value))
        .unwrap_or_default();
    active
        .started_at
        .elapsed()
        .saturating_sub(paused_total)
        .saturating_sub(current_pause)
        .as_micros()
        .min(u128::from(active.total_us)) as u64
}

pub(in crate::native_runtime) fn publish_playback_state(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    state: PlaybackEventState,
    message: Option<String>,
    outcome: Option<String>,
) -> Result<(), String> {
    let event = UiEvent::PlaybackStateChanged {
        v: crate::DESKTOP_PROTOCOL_VERSION,
        payload: PlaybackStateChangedPayload {
            session_id: active.session_id.clone(),
            song_id: active.song_id.clone(),
            state,
            physical: active.physical,
            message,
            outcome,
        },
    };
    events
        .lock()
        .map_err(|_| "native event hub lock poisoned".to_string())?
        .publish(event)
}

pub(in crate::native_runtime) fn publish_active_playback_state(
    _events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    state: PlaybackEventState,
    message: Option<String>,
    outcome: Option<String>,
) -> Result<(), String> {
    if let Some(presentation) = active.presentation.get() {
        presentation.publish_state(active, state, message, outcome)
    } else {
        let _ = (state, message, outcome);
        Err("playback presentation consumer is unavailable".into())
    }
}

pub(in crate::native_runtime) fn finalize_active_presentation(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    diagnostics_gate: &DiagnosticsPublicationGate,
    finalization: PlaybackPresentationFinalization,
    completed: PlaybackPresentationCompletion,
) -> Result<(), String> {
    if let Some(presentation) = active.presentation.get() {
        presentation.finalize(active, finalization, completed)
    } else {
        let _ = (events, diagnostics_gate, finalization);
        let error = "playback presentation consumer is unavailable".to_string();
        completed(Err(error.clone()));
        Err(error)
    }
}

#[cfg(all(test, feature = "tauri-test"))]
pub(in crate::native_runtime) fn publish_playback_snapshot(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    elapsed_us: u64,
    pre_roll_remaining_us: u64,
    status: EnginePollStatus,
) -> Result<(), String> {
    publish_playback_snapshot_inner(
        events,
        active,
        elapsed_us,
        pre_roll_remaining_us,
        status,
        None,
    )
}

pub(in crate::native_runtime) fn publish_playback_snapshot_cancellable(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    elapsed_us: u64,
    pre_roll_remaining_us: u64,
    status: EnginePollStatus,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    publish_playback_snapshot_inner(
        events,
        active,
        elapsed_us,
        pre_roll_remaining_us,
        status,
        Some(cancelled),
    )
}

pub(in crate::native_runtime) fn publish_playback_snapshot_inner(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    elapsed_us: u64,
    pre_roll_remaining_us: u64,
    status: EnginePollStatus,
    cancelled: Option<&AtomicBool>,
) -> Result<(), String> {
    let (focus_state, health, input_path_degraded, message) = if let Some(player) = &active.player {
        let snapshot = player.snapshot_lite();
        let focused = active
            .target_hwnd
            .is_some_and(sky_dispatch_win32::focus::foreground_window_matches);
        let focus_state = if focused {
            PlaybackFocusState::Focused
        } else if matches!(
            active.state.lock().ok().as_deref(),
            Some(PlaybackSessionState::Starting)
        ) {
            PlaybackFocusState::Waiting
        } else {
            PlaybackFocusState::Unfocused
        };
        let health = playback_health_from_snapshot(&snapshot);
        (
            focus_state,
            health,
            snapshot.input_path_degraded,
            snapshot.last_error,
        )
    } else {
        (
            // Dry-run has no HWND or physical backend.  Keep the delivery
            // state honest instead of claiming a focus admission that was
            // never performed; once the preview leaves Starting, the
            // non-physical path is considered focus-neutral.
            if matches!(
                active.state.lock().ok().as_deref(),
                Some(PlaybackSessionState::Starting)
            ) {
                PlaybackFocusState::Waiting
            } else {
                PlaybackFocusState::Focused
            },
            PlaybackHealthState::Healthy,
            false,
            None,
        )
    };
    let event = UiEvent::PlaybackSnapshot {
        v: crate::DESKTOP_PROTOCOL_VERSION,
        payload: PlaybackSnapshotPayload {
            session_id: active.session_id.clone(),
            seq: active.sequence.fetch_add(1, Ordering::Relaxed) + 1,
            state: playback_event_state(status),
            song_id: active.song_id.clone(),
            title: active.title.clone(),
            current_us: elapsed_us,
            total_us: active.total_us,
            pre_roll_remaining_us,
            focus_state,
            health,
            input_path_degraded,
            message,
        },
    };
    if cancelled.is_some_and(|cancelled| cancelled.load(Ordering::Acquire)) {
        return Ok(());
    }
    events
        .lock()
        .map_err(|_| "native event hub lock poisoned".to_string())?
        .publish(event)
}

pub(in crate::native_runtime) fn playback_health_from_snapshot(
    snapshot: &EngineProgressSnapshot,
) -> PlaybackHealthState {
    if snapshot
        .terminal_outcome
        .is_some_and(EnginePollStatus::is_terminal_failure)
        || snapshot.has_terminal_error
        || snapshot.last_error.is_some()
        || snapshot.failed_release_count > 0
    {
        PlaybackHealthState::Error
    } else if snapshot.input_path_degraded
        || snapshot.keys_dropped > 0
        || snapshot.chord_split_events > 0
        || snapshot.possibly_active_count > snapshot.active_count
    {
        PlaybackHealthState::Degraded
    } else {
        PlaybackHealthState::Healthy
    }
}
