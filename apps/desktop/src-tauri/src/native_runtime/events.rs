use crate::ui_events::UiEvent;
use sky_player::engine::NativeDispatchSession;
use std::collections::VecDeque;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;

pub(crate) const MAX_NATIVE_EVENTS: usize = 128;
const EVENT_PUBLISHER_CLOSE_BUDGET: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::native_runtime) struct NativeEventSnapshotMetrics {
    pub(in crate::native_runtime) snapshot_coalesced: u64,
    pub(in crate::native_runtime) snapshot_dropped: u64,
}

#[derive(Default)]
struct NativeEventHubState {
    buffered: VecDeque<UiEvent>,
    channel: Option<Channel<UiEvent>>,
    closed: bool,
    in_flight: bool,
    in_flight_snapshot: bool,
    delivery_failure: Option<String>,
    playback_player: Option<std::sync::Weak<NativeDispatchSession>>,
}

#[derive(Default)]
struct NativeEventQueue {
    state: Mutex<NativeEventHubState>,
    changed: Condvar,
    publisher_finished: AtomicBool,
    delivery_failure: Arc<AtomicBool>,
    pub(in crate::native_runtime) snapshot_coalesced: AtomicU64,
    pub(in crate::native_runtime) snapshot_dropped: AtomicU64,
}

impl NativeEventQueue {
    fn fail_delivery(
        &self,
        reason: &str,
        in_flight_send_failed: bool,
    ) -> Option<Arc<NativeDispatchSession>> {
        self.delivery_failure.store(true, Ordering::Release);
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state
            .delivery_failure
            .get_or_insert_with(|| reason.to_string());
        state.closed = true;
        let buffered_snapshots = count_buffered_snapshots(&state.buffered);
        if buffered_snapshots > 0 {
            self.snapshot_dropped
                .fetch_add(buffered_snapshots, Ordering::Relaxed);
        }
        if in_flight_send_failed && state.in_flight_snapshot {
            self.snapshot_dropped.fetch_add(1, Ordering::Relaxed);
        }
        if in_flight_send_failed {
            state.in_flight = false;
            state.in_flight_snapshot = false;
        }
        state.channel = None;
        state.buffered.clear();
        let player = state
            .playback_player
            .as_ref()
            .and_then(std::sync::Weak::upgrade);
        drop(state);
        self.changed.notify_all();
        player
    }
}

pub(crate) struct NativeEventHub {
    queue: Arc<NativeEventQueue>,
    publisher: Option<thread::JoinHandle<()>>,
}

impl Default for NativeEventHub {
    fn default() -> Self {
        Self {
            queue: Arc::new(NativeEventQueue::default()),
            publisher: None,
        }
    }
}

impl NativeEventHub {
    pub(in crate::native_runtime) fn publish(&mut self, event: UiEvent) -> Result<(), String> {
        validate_ui_event(&event)?;
        let is_snapshot = snapshot_key(&event).is_some();
        let mut failed_player = None;
        let result = {
            let mut state = match self.queue.state.lock() {
                Ok(state) => state,
                Err(_) => {
                    let player = self
                        .queue
                        .fail_delivery("native event queue lock poisoned", false);
                    if let Some(player) = player {
                        request_event_delivery_termination(&player);
                    }
                    return Err("native event queue lock poisoned".into());
                }
            };
            if state.closed {
                state
                    .delivery_failure
                    .get_or_insert_with(|| "native event hub is closed".into());
                self.queue.delivery_failure.store(true, Ordering::Release);
                return Err("native event hub is closed".into());
            }
            if let Some(error) = &state.delivery_failure {
                self.queue.delivery_failure.store(true, Ordering::Release);
                return Err(error.clone());
            }

            if let Some(key) = snapshot_key(&event)
                && let Some(existing) = state
                    .buffered
                    .iter_mut()
                    .find(|candidate| snapshot_key(candidate).as_ref() == Some(&key))
            {
                *existing = event;
                self.queue
                    .snapshot_coalesced
                    .fetch_add(1, Ordering::Relaxed);
                self.queue.snapshot_dropped.fetch_add(1, Ordering::Relaxed);
                Ok(())
            } else {
                let occupied = state.buffered.len() + usize::from(state.in_flight);
                let at_capacity = occupied >= MAX_NATIVE_EVENTS;
                let evicted_snapshot = at_capacity && remove_oldest_snapshot(&mut state.buffered);
                if evicted_snapshot {
                    self.queue.snapshot_dropped.fetch_add(1, Ordering::Relaxed);
                }
                let has_room = !at_capacity || evicted_snapshot;
                if has_room {
                    state.buffered.push_back(event);
                    Ok(())
                } else if is_snapshot {
                    // Snapshots are best-effort. A full queue containing only
                    // ordered lifecycle events leaves no older snapshot to
                    // evict, so this update is dropped without affecting the
                    // lifecycle delivery contract.
                    self.queue.snapshot_dropped.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                } else {
                    let error = "native event hub lifecycle buffer overflow".to_string();
                    state.delivery_failure = Some(error.clone());
                    self.queue.delivery_failure.store(true, Ordering::Release);
                    failed_player = state
                        .playback_player
                        .as_ref()
                        .and_then(std::sync::Weak::upgrade);
                    Err(error)
                }
            }
        };
        self.queue.changed.notify_one();
        if let Some(player) = failed_player {
            request_event_delivery_termination(&player);
        }
        result
    }

    pub(crate) fn subscribe(&mut self, channel: Channel<UiEvent>) -> Result<(), String> {
        {
            let state = match self.queue.state.lock() {
                Ok(state) => state,
                Err(_) => {
                    if let Some(player) = self.queue.fail_delivery(
                        "native event queue lock poisoned during subscription",
                        false,
                    ) {
                        request_event_delivery_termination(&player);
                    }
                    return Err("native event queue lock poisoned".into());
                }
            };
            if state.closed {
                return Err("native event hub is closed".into());
            }
            if state.channel.is_some() {
                return Err("native event hub already has a subscriber".into());
            }
        }

        if self.publisher.is_none() {
            let queue = Arc::clone(&self.queue);
            match thread::Builder::new()
                .name("sky-native-ui-event-publisher".into())
                .spawn(move || publish_native_events(queue))
            {
                Ok(publisher) => self.publisher = Some(publisher),
                Err(error) => {
                    let failure = format!("could not start native UI event publisher: {error}");
                    let player = self.queue.fail_delivery(&failure, false);
                    if let Some(player) = player {
                        request_event_delivery_termination(&player);
                    }
                    return Err(failure);
                }
            }
        }

        let mut state = match self.queue.state.lock() {
            Ok(state) => state,
            Err(_) => {
                if let Some(player) = self
                    .queue
                    .fail_delivery("native event queue lock poisoned while subscribing", false)
                {
                    request_event_delivery_termination(&player);
                }
                return Err("native event queue lock poisoned".into());
            }
        };
        if state.closed {
            return Err("native event hub is closed".into());
        }
        state.channel = Some(channel);
        drop(state);
        self.queue.changed.notify_one();
        Ok(())
    }

    pub(in crate::native_runtime) fn register_playback_player(
        &mut self,
        player: std::sync::Weak<NativeDispatchSession>,
    ) -> Result<(), String> {
        let mut state = match self.queue.state.lock() {
            Ok(state) => state,
            Err(_) => {
                self.queue.fail_delivery(
                    "native event queue lock poisoned while registering playback",
                    false,
                );
                return Err("native event queue lock poisoned".into());
            }
        };
        if state.closed {
            return Err("native event hub is closed".into());
        }
        state.playback_player = Some(player);
        Ok(())
    }

    pub(in crate::native_runtime) fn failure_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.queue.delivery_failure)
    }

    pub(in crate::native_runtime) fn snapshot_metrics(&self) -> NativeEventSnapshotMetrics {
        NativeEventSnapshotMetrics {
            snapshot_coalesced: self.queue.snapshot_coalesced.load(Ordering::Acquire),
            snapshot_dropped: self.queue.snapshot_dropped.load(Ordering::Acquire),
        }
    }

    #[cfg(test)]
    pub(in crate::native_runtime) fn delivery_failure(&self) -> Option<String> {
        match self.queue.state.lock() {
            Ok(state) => state.delivery_failure.clone().or_else(|| {
                self.queue
                    .delivery_failure
                    .load(Ordering::Acquire)
                    .then(|| "native lifecycle event delivery failed".into())
            }),
            Err(_) => {
                self.queue
                    .fail_delivery("native event queue lock poisoned", false);
                Some("native event queue lock poisoned".to_string())
            }
        }
    }

    pub(crate) fn close(&mut self) {
        let player = match self.queue.state.lock() {
            Ok(mut state) => {
                state.closed = true;
                let buffered_snapshots = count_buffered_snapshots(&state.buffered);
                if buffered_snapshots > 0 {
                    self.queue
                        .snapshot_dropped
                        .fetch_add(buffered_snapshots, Ordering::Relaxed);
                }
                state.buffered.clear();
                let player = state
                    .playback_player
                    .as_ref()
                    .and_then(std::sync::Weak::upgrade);
                state.playback_player = None;
                player
            }
            Err(poisoned) => {
                self.queue.delivery_failure.store(true, Ordering::Release);
                let mut state = poisoned.into_inner();
                state.closed = true;
                let buffered_snapshots = count_buffered_snapshots(&state.buffered);
                if buffered_snapshots > 0 {
                    self.queue
                        .snapshot_dropped
                        .fetch_add(buffered_snapshots, Ordering::Relaxed);
                }
                state.buffered.clear();
                let player = state
                    .playback_player
                    .as_ref()
                    .and_then(std::sync::Weak::upgrade);
                state.playback_player = None;
                player
            }
        };
        if player.is_some() {
            self.queue.delivery_failure.store(true, Ordering::Release);
        }
        if let Some(player) = player {
            request_event_delivery_termination(&player);
        }
        self.queue.changed.notify_all();

        // A platform channel send is not cancellable. Never make physical
        // shutdown wait indefinitely for it; this is one app-scoped publisher,
        // not a per-session thread or a physical input owner.
        if let Some(publisher) = self.publisher.take() {
            let deadline = Instant::now() + EVENT_PUBLISHER_CLOSE_BUDGET;
            while !publisher.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            if publisher.is_finished() {
                let _ = publisher.join();
            }
        }
        report_native_event_snapshot_metrics(&self.queue);
    }

    #[cfg(test)]
    pub(in crate::native_runtime) fn buffered_for_test(&self) -> VecDeque<UiEvent> {
        self.queue
            .state
            .lock()
            .expect("event queue")
            .buffered
            .iter()
            .cloned()
            .collect()
    }

    #[cfg(test)]
    pub(in crate::native_runtime) fn publisher_finished_for_test(&self) -> bool {
        self.queue.publisher_finished.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(in crate::native_runtime) fn wait_idle_for_test(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let Ok(mut state) = self.queue.state.lock() else {
            return false;
        };
        loop {
            if state.buffered.is_empty() && !state.in_flight {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            match self.queue.changed.wait_timeout(state, remaining) {
                Ok((next, result)) => {
                    state = next;
                    if result.timed_out() {
                        return state.buffered.is_empty() && !state.in_flight;
                    }
                }
                Err(_) => return false,
            }
        }
    }
}

impl Drop for NativeEventHub {
    fn drop(&mut self) {
        self.close();
    }
}

fn publish_native_events(queue: Arc<NativeEventQueue>) {
    loop {
        let (channel, event) = {
            let mut state = match queue.state.lock() {
                Ok(state) => state,
                Err(_) => {
                    fail_native_event_publisher(&queue, "native event queue lock poisoned");
                    return;
                }
            };
            loop {
                if state.closed && state.buffered.is_empty() {
                    let channel = state.channel.take();
                    drop(state);
                    drop(channel);
                    queue.publisher_finished.store(true, Ordering::Release);
                    queue.changed.notify_all();
                    return;
                }
                if !state.buffered.is_empty()
                    && let Some(channel) = state.channel.clone()
                    && let Some(event) = state.buffered.pop_front()
                {
                    state.in_flight = true;
                    state.in_flight_snapshot = snapshot_key(&event).is_some();
                    break (channel, event);
                }
                state = match queue.changed.wait(state) {
                    Ok(state) => state,
                    Err(_) => {
                        fail_native_event_publisher(
                            &queue,
                            "native event queue lock poisoned while waiting",
                        );
                        return;
                    }
                };
            }
        };

        let sent = catch_unwind(AssertUnwindSafe(|| channel.send(event)))
            .map_err(|_| "native UI event delivery panicked".to_string())
            .and_then(|result| {
                result.map_err(|error| format!("native UI event delivery failed: {error}"))
            });
        match sent {
            Ok(()) => {
                match queue.state.lock() {
                    Ok(mut state) => {
                        state.in_flight = false;
                        state.in_flight_snapshot = false;
                    }
                    Err(poisoned) => {
                        let mut state = poisoned.into_inner();
                        state.in_flight = false;
                        state.in_flight_snapshot = false;
                        drop(state);
                        fail_native_event_publisher(
                            &queue,
                            "native event queue lock poisoned after channel delivery",
                        );
                        return;
                    }
                }
                queue.changed.notify_all();
            }
            Err(error) => {
                fail_native_event_publisher(&queue, &error);
                return;
            }
        }
    }
}

fn fail_native_event_publisher(queue: &NativeEventQueue, reason: &str) {
    if let Some(player) = queue.fail_delivery(reason, true) {
        request_event_delivery_termination(&player);
    }
    queue.publisher_finished.store(true, Ordering::Release);
    report_native_event_snapshot_metrics(queue);
}

fn report_native_event_snapshot_metrics(queue: &NativeEventQueue) {
    let _ = writeln!(
        std::io::stderr().lock(),
        "native_event_snapshots coalesced={} dropped={}",
        queue.snapshot_coalesced.load(Ordering::Acquire),
        queue.snapshot_dropped.load(Ordering::Acquire),
    );
}

fn request_event_delivery_termination(player: &NativeDispatchSession) {
    let _ = player.panic_release();
    let _ = player.quit();
}

fn validate_ui_event(event: &UiEvent) -> Result<(), String> {
    match event {
        UiEvent::CoreReady { payload, .. } => UiEvent::validate_ready(payload),
        UiEvent::CoreFatal { payload, .. } => UiEvent::validate_fatal(payload),
        UiEvent::CatalogChanged { payload, .. } => UiEvent::validate_catalog_changed(payload),
        UiEvent::CatalogLoadFailed { payload, .. } => {
            UiEvent::validate_catalog_load_failed(payload)
        }
        UiEvent::PlaybackStateChanged { payload, .. } => {
            UiEvent::validate_playback_state_changed(payload)
        }
        UiEvent::PlaybackSnapshot { payload, .. } => UiEvent::validate_playback_snapshot(payload),
        UiEvent::PlaybackFinished { payload, .. } => UiEvent::validate_playback_finished(payload),
        UiEvent::PlaybackFailed { payload, .. } => UiEvent::validate_playback_failed(payload),
        UiEvent::DiagnosticsSnapshot { payload, .. } => {
            UiEvent::validate_diagnostics_snapshot(payload)
        }
        UiEvent::UpdateChanged { payload, .. } => UiEvent::validate_update_changed(payload),
    }
}

fn snapshot_key(event: &UiEvent) -> Option<(u8, String)> {
    match event {
        UiEvent::PlaybackSnapshot { payload, .. } => Some((1, payload.session_id.clone())),
        UiEvent::DiagnosticsSnapshot { payload, .. } => {
            Some((2, payload.session_id.clone().unwrap_or_default()))
        }
        _ => None,
    }
}

fn count_buffered_snapshots(buffered: &VecDeque<UiEvent>) -> u64 {
    buffered
        .iter()
        .filter(|event| snapshot_key(event).is_some())
        .count() as u64
}

/// Select a percentile with nearest-even index rounding for half-index cases.
pub(in crate::native_runtime) fn percentile_ms(values: &[i64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let position = fraction * (ordered.len() - 1) as f64;
    let index = (position.round_ties_even() as usize).min(ordered.len() - 1);
    ordered[index] as f64 / 1000.0
}

pub(in crate::native_runtime) fn population_sigma_ms(values: &[i64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().map(|value| *value as f64).sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| {
            let delta = *value as f64 - mean;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt() / 1000.0
}

pub(in crate::native_runtime) fn remove_oldest_snapshot(buffered: &mut VecDeque<UiEvent>) -> bool {
    let Some(index) = buffered
        .iter()
        .position(|event| snapshot_key(event).is_some())
    else {
        return false;
    };
    buffered.remove(index).is_some()
}
