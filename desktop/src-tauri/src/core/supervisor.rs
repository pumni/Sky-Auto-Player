use super::protocol::{BoundedFrameReader, CoreEvent, CoreMessage, MAX_REQUEST_ID, encode_request};
use super::request_registry::{Completion, PendingRegistry};
use crate::ui_events::{CoreFatalPayload, UiEvent};
use serde::Serialize;
use serde_json::Value;
use sky_dispatch_win32::emergency_release_canonical;
use std::io::{self, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const RELOAD_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BUFFERED_EVENTS: usize = 128;
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(25);
const CHILD_TERMINATION_TIMEOUT: Duration = Duration::from_secs(1);

#[cfg(test)]
static TEST_CHILD_REAPED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy)]
struct SupervisorTimeouts {
    startup: Duration,
    request: Duration,
    reload: Duration,
}

const DEFAULT_TIMEOUTS: SupervisorTimeouts = SupervisorTimeouts {
    startup: STARTUP_TIMEOUT,
    request: REQUEST_TIMEOUT,
    reload: RELOAD_TIMEOUT,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreLifecycle {
    Starting,
    Ready,
    ShuttingDown,
    Exited,
    Fatal,
}

impl CoreLifecycle {
    fn accepts_requests(self) -> bool {
        matches!(self, Self::Ready)
    }

    #[cfg(test)]
    fn is_terminal(self) -> bool {
        matches!(self, Self::Exited | Self::Fatal)
    }
}

#[derive(Debug, thiserror::Error, Clone)]
pub enum SupervisorError {
    #[error("Core launch failed: {0}")]
    Launch(String),
    #[error("Core is not available: {0}")]
    Unavailable(String),
    #[error("Core request failed: {0}")]
    Request(String),
    #[error("Core request timed out")]
    Timeout,
    #[error("Core returned {code}: {message}")]
    Core { code: String, message: String },
}

pub struct CoreSupervisor {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    pending: PendingRegistry,
    next_id: AtomicU64,
    lifecycle: Mutex<CoreLifecycle>,
    ready: Mutex<Option<Receiver<Result<(), String>>>>,
    events: Mutex<EventState>,
    shutdown_requested: AtomicBool,
    physical_session_active: AtomicBool,
    emergency_release_done: AtomicBool,
    emergency_release: fn() -> sky_dispatch_win32::input::ReleaseAllOutcome,
    #[cfg(test)]
    track_child_reaped: bool,
    timeouts: SupervisorTimeouts,
}

impl CoreSupervisor {
    pub fn spawn() -> Result<Arc<Self>, SupervisorError> {
        let command = super::build_core_command()?;
        Self::spawn_with_command(command)
    }

    #[cfg(test)]
    pub(crate) fn spawn_with_command(mut command: Command) -> Result<Arc<Self>, SupervisorError> {
        Self::spawn_process(&mut command, DEFAULT_TIMEOUTS)
    }

    #[cfg(not(test))]
    pub(crate) fn spawn_with_command(mut command: Command) -> Result<Arc<Self>, SupervisorError> {
        Self::spawn_process(&mut command, DEFAULT_TIMEOUTS)
    }

    #[cfg(test)]
    fn spawn_with_command_and_timeouts(
        mut command: Command,
        timeouts: SupervisorTimeouts,
    ) -> Result<Arc<Self>, SupervisorError> {
        Self::spawn_process(&mut command, timeouts)
    }

    fn spawn_process(
        command: &mut Command,
        timeouts: SupervisorTimeouts,
    ) -> Result<Arc<Self>, SupervisorError> {
        Self::spawn_process_with_release(command, timeouts, emergency_release_canonical, false)
    }

    fn spawn_process_with_release(
        command: &mut Command,
        timeouts: SupervisorTimeouts,
        emergency_release: fn() -> sky_dispatch_win32::input::ReleaseAllOutcome,
        track_child_reaped: bool,
    ) -> Result<Arc<Self>, SupervisorError> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(not(test))]
        let _ = track_child_reaped;
        let mut child = command
            .spawn()
            .map_err(|error| SupervisorError::Launch(error.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SupervisorError::Launch("Core stdin was not piped".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SupervisorError::Launch("Core stdout was not piped".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SupervisorError::Launch("Core stderr was not piped".into()))?;
        let (ready_sender, ready_receiver) = mpsc::channel();
        let supervisor = Arc::new(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            pending: PendingRegistry::new(),
            next_id: AtomicU64::new(1),
            lifecycle: Mutex::new(CoreLifecycle::Starting),
            ready: Mutex::new(Some(ready_receiver)),
            events: Mutex::new(EventState::default()),
            shutdown_requested: AtomicBool::new(false),
            physical_session_active: AtomicBool::new(false),
            emergency_release_done: AtomicBool::new(false),
            emergency_release,
            timeouts,
            #[cfg(test)]
            track_child_reaped,
        });
        Self::spawn_reader(Arc::clone(&supervisor), stdout, ready_sender);
        Self::spawn_stderr_drainer(stderr);
        Self::spawn_waiter(Arc::clone(&supervisor));

        let receiver = supervisor
            .ready
            .lock()
            .expect("ready receiver poisoned")
            .take()
            .expect("ready receiver missing");
        match receiver.recv_timeout(supervisor.timeouts.startup) {
            Ok(Ok(())) => Ok(supervisor),
            Ok(Err(error)) => {
                supervisor.terminate_child();
                Err(SupervisorError::Unavailable(error))
            }
            Err(RecvTimeoutError::Timeout) => {
                supervisor.terminate_child();
                Err(SupervisorError::Timeout)
            }
            Err(RecvTimeoutError::Disconnected) => {
                supervisor.terminate_child();
                Err(SupervisorError::Unavailable(
                    "Core exited before ready".into(),
                ))
            }
        }
    }

    fn spawn_reader(
        supervisor: Arc<Self>,
        stdout: ChildStdout,
        ready_sender: mpsc::Sender<Result<(), String>>,
    ) {
        thread::Builder::new()
            .name("sky-desktop-core-reader".into())
            .spawn(move || {
                let mut frames = BoundedFrameReader::new(stdout);
                loop {
                    match frames.next_frame() {
                        Ok(Some(frame)) => {
                            match super::protocol::parse_message(&frame) {
                                Ok(CoreMessage::Response(response)) => {
                                    // Timed-out IDs are retained as bounded tombstones. A response
                                    // for one of them is a harmless late response; any other unknown
                                    // ID remains a protocol violation and fails closed.
                                    match supervisor.pending.complete(response.id, Ok(response)) {
                                        Completion::Delivered | Completion::LateAfterTimeout => {}
                                        Completion::Unknown => {
                                            supervisor.protocol_fatal(
                                                "response used an unknown request id",
                                                &ready_sender,
                                            );
                                            break;
                                        }
                                    }
                                }
                                Ok(CoreMessage::Event(event)) => match event {
                                    CoreEvent::Ready(_) => {
                                        if supervisor.transition_to_ready() {
                                            supervisor.publish_event(event);
                                            let _ = ready_sender.send(Ok(()));
                                        } else {
                                            supervisor.protocol_fatal(
                                                "core.ready is only valid once while Starting",
                                                &ready_sender,
                                            );
                                            break;
                                        }
                                    }
                                    CoreEvent::Fatal(_) => {
                                        supervisor.inbound_core_fatal(event, &ready_sender);
                                        break;
                                    }
                                    CoreEvent::CatalogChanged(_) => supervisor.publish_event(event),
                                    CoreEvent::PlaybackStateChanged(ref payload) => {
                                        supervisor.update_physical_session(
                                            payload.physical,
                                            payload.state.as_str(),
                                        );
                                        supervisor.publish_event(event);
                                    }
                                    CoreEvent::PlaybackSnapshot(_)
                                    | CoreEvent::PlaybackFinished(_)
                                    | CoreEvent::PlaybackFailed(_)
                                    | CoreEvent::DiagnosticsSnapshot(_)
                                    | CoreEvent::CalibrationProgress(_)
                                    | CoreEvent::CalibrationFinished(_) => {
                                        supervisor.publish_event(event)
                                    }
                                },
                                Err(error) => {
                                    supervisor.protocol_fatal(&error.to_string(), &ready_sender);
                                    break;
                                }
                            }
                        }
                        Ok(None) => {
                            if supervisor.shutdown_requested.load(Ordering::Acquire) {
                                supervisor.set_lifecycle(CoreLifecycle::Exited);
                            } else {
                                supervisor.protocol_fatal(
                                    "Core stdout closed unexpectedly",
                                    &ready_sender,
                                );
                            }
                            break;
                        }
                        Err(error) => {
                            supervisor.protocol_fatal(&error.to_string(), &ready_sender);
                            break;
                        }
                    }
                }
            })
            .expect("failed to start Core reader");
    }

    fn spawn_stderr_drainer(mut stderr: impl io::Read + Send + 'static) {
        thread::Builder::new()
            .name("sky-desktop-core-stderr".into())
            .spawn(move || {
                let mut buffer = [0_u8; 4096];
                loop {
                    match stderr.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            })
            .expect("failed to start Core stderr drainer");
    }

    fn spawn_waiter(supervisor: Arc<Self>) {
        thread::Builder::new()
            .name("sky-desktop-core-waiter".into())
            .spawn(move || {
                let status = loop {
                    let status = supervisor
                        .child
                        .lock()
                        .expect("Core child poisoned")
                        .try_wait();
                    match status {
                        Ok(Some(status)) => {
                            supervisor.note_child_termination();
                            break Ok(status);
                        }
                        Ok(None) => thread::sleep(CHILD_POLL_INTERVAL),
                        Err(error) => break Err(error),
                    }
                };
                if status.is_err() || !supervisor.shutdown_requested.load(Ordering::Acquire) {
                    supervisor.pending.fail_all("Core process exited");
                    supervisor.emergency_release_if_needed();
                    let mut lifecycle = supervisor.lifecycle.lock().expect("lifecycle poisoned");
                    if *lifecycle != CoreLifecycle::Fatal {
                        *lifecycle = CoreLifecycle::Exited;
                    }
                }
            })
            .expect("failed to start Core waiter");
    }

    fn protocol_fatal(&self, message: &str, ready_sender: &mpsc::Sender<Result<(), String>>) {
        self.set_lifecycle(CoreLifecycle::Fatal);
        self.pending.fail_all(message);
        let _ = ready_sender.send(Err(message.to_owned()));
        self.publish_event(CoreEvent::Fatal(CoreFatalPayload {
            code: "protocol_error".into(),
            message: message.to_owned(),
        }));
        self.terminate_child_before_emergency_release();
    }

    fn inbound_core_fatal(
        &self,
        event: CoreEvent,
        ready_sender: &mpsc::Sender<Result<(), String>>,
    ) {
        let message = match &event {
            CoreEvent::Fatal(payload) => payload.message.as_str(),
            _ => "Core reported a fatal error",
        };
        self.set_lifecycle(CoreLifecycle::Fatal);
        self.pending
            .fail_all(&format!("Core reported fatal error: {message}"));
        let _ = ready_sender.send(Err(message.to_owned()));
        self.publish_event(event);
        self.terminate_child_before_emergency_release();
    }

    fn transition_to_ready(&self) -> bool {
        let mut lifecycle = self.lifecycle.lock().expect("lifecycle poisoned");
        if *lifecycle != CoreLifecycle::Starting {
            return false;
        }
        *lifecycle = CoreLifecycle::Ready;
        true
    }

    fn update_physical_session(&self, physical: bool, state: &str) {
        if !physical {
            return;
        }
        if matches!(state, "starting" | "playing" | "paused" | "stopping") {
            if state == "starting" {
                self.emergency_release_done.store(false, Ordering::Release);
            }
            self.physical_session_active.store(true, Ordering::Release);
        } else if matches!(state, "finished" | "failed" | "cancelled") {
            self.physical_session_active.store(false, Ordering::Release);
        }
    }

    fn emergency_release_if_needed(&self) {
        if !self.physical_session_active.load(Ordering::Acquire) {
            return;
        }
        if self
            .emergency_release_done
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let _ = (self.emergency_release)();
            self.physical_session_active.store(false, Ordering::Release);
        }
    }

    fn publish_event(&self, event: CoreEvent) {
        let ui_event = event.into_ui_event();
        let mut events = self.events.lock().expect("event buffer poisoned");
        if events.overflowed {
            return;
        }

        // The replay buffer is only for events produced before a usable UI
        // subscriber exists. Once live delivery is installed, retaining a
        // second copy would turn the bounded backlog into lifetime history.
        if let Some(channel) = events.channel.clone() {
            if channel.send(ui_event).is_err() {
                events.channel = None;
                events.overflowed = true;
                drop(events);
                self.event_delivery_fatal();
            }
            return;
        }

        let snapshot_session_id = match &ui_event {
            UiEvent::PlaybackSnapshot { payload, .. } => Some(payload.session_id.as_str()),
            _ => None,
        };
        if let Some(session_id) = snapshot_session_id {
            if let Some(index) = events.buffered.iter().rposition(|buffered| {
                matches!(
                    buffered,
                    UiEvent::PlaybackSnapshot { payload, .. }
                        if payload.session_id == session_id
                )
            }) {
                events.buffered[index] = ui_event;
            } else if events.buffered.len() < MAX_BUFFERED_EVENTS {
                events.buffered.push(ui_event);
            }
            return;
        }

        // Before subscription, lifecycle events are retained in order. A
        // buffered snapshot may be reclaimed to preserve that lifecycle
        // ordering, but lifecycle events themselves are never silently lost.
        if events.buffered.len() >= MAX_BUFFERED_EVENTS
            && let Some(index) = events
                .buffered
                .iter()
                .position(|buffered| matches!(buffered, UiEvent::PlaybackSnapshot { .. }))
        {
            events.buffered.remove(index);
        }
        if events.buffered.len() < MAX_BUFFERED_EVENTS {
            events.buffered.push(ui_event);
        } else {
            // There is no safe way to drop a lifecycle event. Mark the
            // replay history unusable and fail closed for future command
            // and subscription calls instead of silently losing state.
            events.overflowed = true;
        }
    }

    pub fn subscribe(&self, channel: tauri::ipc::Channel<UiEvent>) -> Result<(), SupervisorError> {
        let mut events = self.events.lock().expect("event buffer poisoned");
        if events.overflowed {
            return Err(SupervisorError::Unavailable(
                "Core event history overflowed its bounded capacity".into(),
            ));
        }
        for event in &events.buffered {
            if let Err(error) = channel.send(event.clone()) {
                events.channel = None;
                events.overflowed = true;
                let message = error.to_string();
                drop(events);
                self.event_delivery_fatal();
                return Err(SupervisorError::Request(message));
            }
        }
        events.channel = Some(channel);
        events.buffered.clear();
        Ok(())
    }

    fn event_delivery_fatal(&self) {
        self.set_lifecycle(CoreLifecycle::Fatal);
        self.pending.fail_all("UI event channel delivery failed");
        self.terminate_child_before_emergency_release();
    }

    pub fn lifecycle(&self) -> CoreLifecycle {
        *self.lifecycle.lock().expect("lifecycle poisoned")
    }

    #[cfg(test)]
    fn event_history_overflowed(&self) -> bool {
        self.events
            .lock()
            .expect("event buffer poisoned")
            .overflowed
    }

    pub fn request<P: Serialize>(&self, method: &str, params: P) -> Result<Value, SupervisorError> {
        let lifecycle = self.lifecycle();
        if !lifecycle.accepts_requests() {
            return Err(SupervisorError::Unavailable(format!(
                "Core state is {lifecycle:?}"
            )));
        }
        if self
            .events
            .lock()
            .expect("event buffer poisoned")
            .overflowed
        {
            return Err(SupervisorError::Unavailable(
                "Core event history overflowed its bounded capacity".into(),
            ));
        }
        let timeout = if method == "catalog.reload" {
            self.timeouts.reload
        } else {
            self.timeouts.request
        };
        self.request_inner(method, params, timeout)
    }

    fn request_inner<P: Serialize>(
        &self,
        method: &str,
        params: P,
        timeout: Duration,
    ) -> Result<Value, SupervisorError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if id > MAX_REQUEST_ID {
            return Err(SupervisorError::Request(
                "request ID exhausted safe JSON range".into(),
            ));
        }
        let params = serde_json::to_value(params)
            .map_err(|error| SupervisorError::Request(error.to_string()))?;
        let frame = encode_request(id, method, params)
            .map_err(|error| SupervisorError::Request(error.to_string()))?;
        let receiver = self.pending.register(id);
        if let Err(error) = self.write_frame(&frame) {
            self.pending.remove(id);
            return Err(error);
        }
        let response = match receiver.recv_timeout(timeout) {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => return Err(SupervisorError::Unavailable(error)),
            Err(RecvTimeoutError::Timeout) => {
                self.pending.expire(id);
                return Err(SupervisorError::Timeout);
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(SupervisorError::Unavailable(
                    "Core response channel closed".into(),
                ));
            }
        };
        if response.ok {
            response
                .result
                .ok_or_else(|| SupervisorError::Request("Core response omitted result".into()))
        } else if let Some(error) = response.error {
            Err(SupervisorError::Core {
                code: error.code,
                message: error.message,
            })
        } else {
            Err(SupervisorError::Request(
                "Core failure response omitted error".into(),
            ))
        }
    }

    fn write_frame(&self, frame: &[u8]) -> Result<(), SupervisorError> {
        let mut stdin = self.stdin.lock().expect("Core stdin poisoned");
        stdin
            .write_all(frame)
            .and_then(|_| stdin.flush())
            .map_err(|error| SupervisorError::Request(error.to_string()))
    }

    pub fn shutdown(&self) {
        if self.shutdown_requested.swap(true, Ordering::AcqRel) {
            return;
        }
        if matches!(
            self.lifecycle(),
            CoreLifecycle::Fatal | CoreLifecycle::Exited
        ) {
            self.terminate_child();
            return;
        }
        self.set_lifecycle(CoreLifecycle::ShuttingDown);
        let shutdown_result =
            self.request_inner("app.shutdown", serde_json::json!({}), self.timeouts.request);
        if shutdown_result.is_err() {
            // Normal app shutdown lets the Core/native session clean itself up.
            // The canonical allowlisted release is only a fallback when that
            // graceful boundary is unavailable.
            self.terminate_child_before_emergency_release();
        } else {
            self.terminate_child();
        }
    }

    fn set_lifecycle(&self, lifecycle: CoreLifecycle) {
        *self.lifecycle.lock().expect("lifecycle poisoned") = lifecycle;
    }

    fn terminate_child(&self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }

    fn terminate_child_before_emergency_release(&self) {
        // A physical Core may still be dispatching while a fail-closed path is
        // unwinding. Stop and boundedly reap it before sending the final
        // canonical key-up, so the worker cannot issue another key-down after
        // emergency cleanup. If termination cannot be confirmed before the
        // budget expires, emergency release is still the last-resort safety
        // action.
        let _terminated = self.terminate_child_bounded();
        self.emergency_release_if_needed();
    }

    fn terminate_child_bounded(&self) -> bool {
        let deadline = Instant::now() + CHILD_TERMINATION_TIMEOUT;
        let mut kill_attempted = false;

        loop {
            let status = match self.child.lock() {
                Ok(mut child) => child.try_wait(),
                Err(_) => return false,
            };
            match status {
                Ok(Some(_)) => {
                    self.note_child_termination();
                    return true;
                }
                Ok(None) => {}
                Err(_) => return false,
            }

            if !kill_attempted {
                if let Ok(mut child) = self.child.lock() {
                    let _ = child.kill();
                } else {
                    return false;
                }
                kill_attempted = true;
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            thread::sleep(CHILD_POLL_INTERVAL.min(remaining));
        }
    }

    fn note_child_termination(&self) {
        #[cfg(test)]
        if self.track_child_reaped {
            TEST_CHILD_REAPED.store(true, Ordering::SeqCst);
        }
    }
}

#[derive(Default)]
struct EventState {
    buffered: Vec<UiEvent>,
    channel: Option<tauri::ipc::Channel<UiEvent>>,
    overflowed: bool,
}

impl Drop for CoreSupervisor {
    fn drop(&mut self) {
        self.shutdown_requested.store(true, Ordering::Release);
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::protocol::CoreEvent;
    use super::{
        CoreLifecycle, CoreSupervisor, MAX_BUFFERED_EVENTS, SupervisorError, SupervisorTimeouts,
    };
    use crate::ui_events::{
        PlaybackEventState, PlaybackFinishedPayload, PlaybackFocusState, PlaybackHealthState,
        PlaybackSnapshotPayload, UiEvent,
    };
    use serde_json::json;
    use sky_dispatch_win32::input::ReleaseAllOutcome;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    fn fake_core(mode: &str) -> Command {
        let python = std::env::var("SKY_PYTHON").unwrap_or_else(|_| "python".into());
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("fake_core.py");
        let mut command = Command::new(python);
        command
            .arg("-u")
            .arg(fixture)
            .arg(mode)
            .env("PYTHONUNBUFFERED", "1");
        command
    }

    fn short_timeouts() -> SupervisorTimeouts {
        SupervisorTimeouts {
            // Windows can start several Python fixtures concurrently when the
            // test harness runs these cases in parallel. Keep the request
            // budgets short, but leave startup enough room for process launch.
            startup: Duration::from_secs(2),
            request: Duration::from_millis(150),
            reload: Duration::from_millis(250),
        }
    }

    fn start(mode: &str) -> std::sync::Arc<CoreSupervisor> {
        CoreSupervisor::spawn_with_command_and_timeouts(fake_core(mode), short_timeouts())
            .unwrap_or_else(|error| panic!("fake Core did not become ready: {error}"))
    }

    static EMERGENCY_RELEASE_CALLS: AtomicUsize = AtomicUsize::new(0);
    static EMERGENCY_RELEASE_ORDER_VIOLATIONS: AtomicUsize = AtomicUsize::new(0);
    static EMERGENCY_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn test_emergency_release() -> ReleaseAllOutcome {
        if !super::TEST_CHILD_REAPED.load(Ordering::SeqCst) {
            EMERGENCY_RELEASE_ORDER_VIOLATIONS.fetch_add(1, Ordering::SeqCst);
        }
        EMERGENCY_RELEASE_CALLS.fetch_add(1, Ordering::SeqCst);
        ReleaseAllOutcome {
            attempted_mask: 0,
            transport_anomaly: false,
            released_successfully: true,
            stuck_mask: 0,
            verification_inconclusive: false,
            attempts: 1,
        }
    }

    fn start_with_test_release(mode: &str) -> std::sync::Arc<CoreSupervisor> {
        super::TEST_CHILD_REAPED.store(false, Ordering::SeqCst);
        EMERGENCY_RELEASE_ORDER_VIOLATIONS.store(0, Ordering::SeqCst);
        let mut command = fake_core(mode);
        CoreSupervisor::spawn_process_with_release(
            &mut command,
            short_timeouts(),
            test_emergency_release,
            true,
        )
        .unwrap_or_else(|error| panic!("fake Core did not become ready: {error}"))
    }

    fn eventually(timeout: Duration, predicate: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if predicate() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        predicate()
    }

    fn finished_event(total_us: u64) -> CoreEvent {
        CoreEvent::PlaybackFinished(PlaybackFinishedPayload {
            session_id: "b".repeat(32),
            song_id: "c".repeat(32),
            outcome: "finished".into(),
            total_us,
            message: "finished".into(),
        })
    }

    #[test]
    fn lifecycle_only_accepts_requests_after_ready() {
        assert!(!CoreLifecycle::Starting.accepts_requests());
        assert!(CoreLifecycle::Ready.accepts_requests());
        assert!(!CoreLifecycle::ShuttingDown.accepts_requests());
        assert!(!CoreLifecycle::Exited.accepts_requests());
        assert!(!CoreLifecycle::Fatal.accepts_requests());
    }

    #[test]
    fn lifecycle_terminal_states_are_not_recoverable() {
        assert!(!CoreLifecycle::Starting.is_terminal());
        assert!(!CoreLifecycle::Ready.is_terminal());
        assert!(!CoreLifecycle::ShuttingDown.is_terminal());
        assert!(CoreLifecycle::Exited.is_terminal());
        assert!(CoreLifecycle::Fatal.is_terminal());
    }

    #[test]
    fn fake_core_request_round_trip_uses_real_pipes() {
        let supervisor = start("normal");
        let result = supervisor
            .request("catalog.search", json!({"query": "Aurora"}))
            .expect("fake Core response");
        assert_eq!(result["method"], "catalog.search");
        assert_eq!(result["params"]["query"], "Aurora");
        supervisor.shutdown();
    }

    #[test]
    fn core_fatal_before_ready_fails_startup() {
        let result = CoreSupervisor::spawn_with_command_and_timeouts(
            fake_core("fatal_before_ready"),
            short_timeouts(),
        );
        let error = match result {
            Ok(_) => panic!("fatal Core must not become ready"),
            Err(error) => error,
        };
        assert!(
            matches!(error, SupervisorError::Unavailable(message) if message.contains("fatal before ready"))
        );
    }

    #[test]
    fn core_fatal_after_ready_is_immediately_terminal_and_preserves_event() {
        let supervisor = start("fatal_after_ready");
        assert!(eventually(Duration::from_secs(1), || {
            supervisor.lifecycle() == CoreLifecycle::Fatal
        }));
        assert!(!supervisor.lifecycle().accepts_requests());
        assert!(eventually(Duration::from_secs(1), || {
            supervisor
                .events
                .lock()
                .expect("event buffer poisoned")
                .buffered
                .iter()
                .any(|event| matches!(event, UiEvent::CoreFatal { .. }))
        }));
        let events = supervisor.events.lock().expect("event buffer poisoned");
        let fatal = events
            .buffered
            .iter()
            .find_map(|event| match event {
                UiEvent::CoreFatal { payload, .. } => Some(payload),
                _ => None,
            })
            .expect("original fatal event");
        assert_eq!(fatal.message, "fatal after ready");
    }

    #[test]
    fn shutdown_of_terminal_core_is_immediate_and_idempotent() {
        let fatal = start("fatal_after_ready");
        assert!(eventually(Duration::from_secs(1), || {
            fatal.lifecycle() == CoreLifecycle::Fatal
        }));
        let started = Instant::now();
        fatal.shutdown();
        fatal.shutdown();
        assert!(started.elapsed() < Duration::from_millis(250));

        let exited = start("eof_after_ready");
        assert!(eventually(Duration::from_secs(1), || {
            exited.lifecycle() == CoreLifecycle::Fatal
        }));
        let started = Instant::now();
        exited.shutdown();
        assert!(started.elapsed() < Duration::from_millis(250));
    }

    #[test]
    fn duplicate_ready_is_a_protocol_fatal() {
        let supervisor = start("duplicate_ready");
        assert!(eventually(Duration::from_secs(1), || {
            supervisor.lifecycle() == CoreLifecycle::Fatal
        }));
    }

    #[test]
    fn malformed_duplicate_and_oversized_output_fail_closed() {
        for mode in ["malformed", "duplicate_output", "oversized_output"] {
            let result =
                CoreSupervisor::spawn_with_command_and_timeouts(fake_core(mode), short_timeouts());
            assert!(result.is_err(), "mode {mode} unexpectedly became ready");
        }
    }

    #[test]
    fn startup_timeout_and_eof_before_ready_are_errors() {
        let timeout = CoreSupervisor::spawn_with_command_and_timeouts(
            fake_core("startup_timeout"),
            short_timeouts(),
        );
        assert!(matches!(timeout, Err(SupervisorError::Timeout)));

        let eof = CoreSupervisor::spawn_with_command_and_timeouts(
            fake_core("eof_before_ready"),
            short_timeouts(),
        );
        assert!(eof.is_err());
    }

    #[test]
    fn eof_after_ready_and_unknown_response_are_terminal() {
        let eof = start("eof_after_ready");
        assert!(eventually(Duration::from_secs(1), || {
            eof.lifecycle() == CoreLifecycle::Fatal
        }));

        let unknown = start("unknown_id");
        let result = unknown.request("test.echo", json!({}));
        assert!(result.is_err());
        assert!(eventually(Duration::from_secs(1), || {
            unknown.lifecycle() == CoreLifecycle::Fatal
        }));
    }

    #[test]
    fn request_timeout_ignores_one_late_response_without_poisoning_core() {
        let supervisor = start("request_timeout");
        let error = supervisor
            .request("slow.operation", json!({}))
            .expect_err("request should time out");
        assert!(matches!(error, SupervisorError::Timeout));
        thread::sleep(Duration::from_millis(350));
        assert_eq!(supervisor.lifecycle(), CoreLifecycle::Ready);
        supervisor.shutdown();
    }

    #[test]
    fn child_exit_during_pending_request_fails_the_request() {
        let supervisor = start("child_pending");
        let result = supervisor.request("pending.operation", json!({}));
        assert!(result.is_err());
        assert!(eventually(Duration::from_secs(1), || {
            supervisor.lifecycle().is_terminal()
        }));
    }

    #[test]
    fn unexpected_active_physical_core_loss_releases_once() {
        let _guard = EMERGENCY_TEST_LOCK.lock().expect("emergency test lock");
        EMERGENCY_RELEASE_CALLS.store(0, Ordering::SeqCst);
        let supervisor = start_with_test_release("physical_active_exit");
        assert!(eventually(Duration::from_secs(1), || {
            supervisor.lifecycle() == CoreLifecycle::Fatal
        }));
        assert!(eventually(Duration::from_secs(1), || {
            EMERGENCY_RELEASE_CALLS.load(Ordering::SeqCst) == 1
        }));
        supervisor.shutdown();
        supervisor.shutdown();
        assert_eq!(EMERGENCY_RELEASE_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(EMERGENCY_RELEASE_ORDER_VIOLATIONS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn fatal_active_physical_core_loss_releases_once() {
        let _guard = EMERGENCY_TEST_LOCK.lock().expect("emergency test lock");
        EMERGENCY_RELEASE_CALLS.store(0, Ordering::SeqCst);
        let supervisor = start_with_test_release("physical_active_fatal");
        assert!(eventually(Duration::from_secs(1), || {
            supervisor.lifecycle() == CoreLifecycle::Fatal
        }));
        assert!(eventually(Duration::from_secs(1), || {
            EMERGENCY_RELEASE_CALLS.load(Ordering::SeqCst) == 1
        }));
        supervisor.shutdown();
        assert_eq!(EMERGENCY_RELEASE_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(EMERGENCY_RELEASE_ORDER_VIOLATIONS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn protocol_fatal_reaps_before_emergency_release() {
        let _guard = EMERGENCY_TEST_LOCK.lock().expect("emergency test lock");
        EMERGENCY_RELEASE_CALLS.store(0, Ordering::SeqCst);
        let supervisor = start_with_test_release("normal");
        supervisor.update_physical_session(true, "playing");
        let (ready_sender, ready_receiver) = mpsc::channel();

        supervisor.protocol_fatal("malformed protocol", &ready_sender);

        assert!(matches!(
            ready_receiver.try_recv(),
            Ok(Err(message)) if message == "malformed protocol"
        ));
        assert_eq!(supervisor.lifecycle(), CoreLifecycle::Fatal);
        assert_eq!(EMERGENCY_RELEASE_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(EMERGENCY_RELEASE_ORDER_VIOLATIONS.load(Ordering::SeqCst), 0);
        supervisor.shutdown();
        assert_eq!(EMERGENCY_RELEASE_CALLS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dry_run_core_loss_does_not_use_emergency_release() {
        let _guard = EMERGENCY_TEST_LOCK.lock().expect("emergency test lock");
        EMERGENCY_RELEASE_CALLS.store(0, Ordering::SeqCst);
        let supervisor = start_with_test_release("dry_run_active_exit");
        assert!(eventually(Duration::from_secs(1), || {
            supervisor.lifecycle() == CoreLifecycle::Fatal
        }));
        supervisor.shutdown();
        assert_eq!(EMERGENCY_RELEASE_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn failed_graceful_shutdown_reaps_before_emergency_release() {
        let _guard = EMERGENCY_TEST_LOCK.lock().expect("emergency test lock");
        EMERGENCY_RELEASE_CALLS.store(0, Ordering::SeqCst);
        let supervisor = start_with_test_release("force_shutdown");
        supervisor.update_physical_session(true, "playing");

        supervisor.shutdown();

        assert_eq!(EMERGENCY_RELEASE_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(EMERGENCY_RELEASE_ORDER_VIOLATIONS.load(Ordering::SeqCst), 0);
        supervisor.shutdown();
        assert_eq!(EMERGENCY_RELEASE_CALLS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn normal_core_shutdown_does_not_use_emergency_release() {
        let _guard = EMERGENCY_TEST_LOCK.lock().expect("emergency test lock");
        EMERGENCY_RELEASE_CALLS.store(0, Ordering::SeqCst);
        let supervisor = start_with_test_release("normal");
        supervisor.shutdown();
        assert_eq!(EMERGENCY_RELEASE_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn snapshot_history_is_coalesced_without_evicting_the_lifecycle_budget() {
        let supervisor = start("normal");
        for seq in 1..=1_000 {
            supervisor.publish_event(CoreEvent::PlaybackSnapshot(PlaybackSnapshotPayload {
                session_id: "b".repeat(32),
                seq,
                state: PlaybackEventState::Playing,
                song_id: "c".repeat(32),
                title: "Fake Song".into(),
                current_us: seq,
                total_us: 1_000,
                pre_roll_remaining_us: 0,
                focus_state: PlaybackFocusState::Focused,
                health: PlaybackHealthState::Healthy,
                input_path_degraded: false,
                message: None,
            }));
        }
        let events = supervisor.events.lock().expect("event buffer poisoned");
        assert!(events.buffered.len() <= 128);
        let snapshots: Vec<_> = events
            .buffered
            .iter()
            .filter_map(|event| match event {
                UiEvent::PlaybackSnapshot { payload, .. } => Some(payload),
                _ => None,
            })
            .collect();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].seq, 1_000);
        drop(events);
        supervisor.shutdown();
    }

    #[test]
    fn successful_subscription_replays_and_clears_backlog() {
        let supervisor = start("normal");
        supervisor
            .events
            .lock()
            .expect("event buffer poisoned")
            .buffered
            .clear();
        for total_us in 1..=3 {
            supervisor.publish_event(finished_event(total_us));
        }

        let delivered = Arc::new(Mutex::new(Vec::<u64>::new()));
        let delivered_for_channel = Arc::clone(&delivered);
        let channel = tauri::ipc::Channel::<UiEvent>::new(move |body| {
            let payload = match body {
                tauri::ipc::InvokeResponseBody::Json(raw) => {
                    serde_json::from_str::<serde_json::Value>(&raw)?
                }
                tauri::ipc::InvokeResponseBody::Raw(raw) => {
                    serde_json::from_slice::<serde_json::Value>(&raw)?
                }
            };
            delivered_for_channel
                .lock()
                .expect("delivered events poisoned")
                .push(payload["payload"]["total_us"].as_u64().expect("total_us"));
            Ok(())
        });

        supervisor
            .subscribe(channel)
            .expect("subscription succeeds");
        assert_eq!(
            *delivered.lock().expect("delivered events poisoned"),
            [1, 2, 3]
        );
        assert!(
            supervisor
                .events
                .lock()
                .expect("event buffer poisoned")
                .buffered
                .is_empty()
        );
        assert!(!supervisor.event_history_overflowed());

        supervisor.publish_event(finished_event(4));
        assert_eq!(
            *delivered.lock().expect("delivered events poisoned"),
            [1, 2, 3, 4]
        );
        assert!(!supervisor.event_history_overflowed());
        supervisor.shutdown();
    }

    #[test]
    fn live_subscription_does_not_fill_replay_history_or_block_stop() {
        let supervisor = start("tauri_commands");
        supervisor
            .events
            .lock()
            .expect("event buffer poisoned")
            .buffered
            .clear();
        let delivered = Arc::new(AtomicUsize::new(0));
        let delivered_for_channel = Arc::clone(&delivered);
        let channel = tauri::ipc::Channel::<UiEvent>::new(move |_| {
            delivered_for_channel.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        supervisor
            .subscribe(channel)
            .expect("subscription succeeds");

        for total_us in 1..=(MAX_BUFFERED_EVENTS * 2 + 1) as u64 {
            supervisor.publish_event(finished_event(total_us));
        }

        assert_eq!(
            delivered.load(Ordering::SeqCst),
            MAX_BUFFERED_EVENTS * 2 + 1
        );
        assert!(
            supervisor
                .events
                .lock()
                .expect("event buffer poisoned")
                .buffered
                .is_empty()
        );
        assert!(!supervisor.event_history_overflowed());
        let stop = supervisor
            .request("playback.stop", json!({"session_id": "b".repeat(32)}))
            .expect("stop remains usable after live delivery");
        assert_eq!(stop["accepted"], true);
        supervisor.shutdown();
    }

    #[test]
    fn failed_live_subscription_delivery_fails_closed_and_releases_active_session() {
        let _guard = EMERGENCY_TEST_LOCK.lock().expect("emergency test lock");
        EMERGENCY_RELEASE_CALLS.store(0, Ordering::SeqCst);
        let supervisor = start_with_test_release("normal");
        let fail_delivery = Arc::new(AtomicBool::new(false));
        let fail_delivery_for_channel = Arc::clone(&fail_delivery);
        let channel = tauri::ipc::Channel::<UiEvent>::new(move |_| {
            if fail_delivery_for_channel.load(Ordering::Acquire) {
                Err(tauri::Error::Io(std::io::Error::other("channel closed")))
            } else {
                Ok(())
            }
        });
        supervisor
            .subscribe(channel)
            .expect("subscription succeeds");
        supervisor.update_physical_session(true, "playing");
        fail_delivery.store(true, Ordering::Release);

        supervisor.publish_event(finished_event(1));

        assert_eq!(supervisor.lifecycle(), CoreLifecycle::Fatal);
        assert!(supervisor.event_history_overflowed());
        assert_eq!(EMERGENCY_RELEASE_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(EMERGENCY_RELEASE_ORDER_VIOLATIONS.load(Ordering::SeqCst), 0);
        assert!(supervisor.request("playback.stop", json!({})).is_err());
        supervisor.shutdown();
        assert_eq!(EMERGENCY_RELEASE_CALLS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn lifecycle_history_overflow_fails_closed_without_growing() {
        let supervisor = start("normal");
        for _ in 0..(MAX_BUFFERED_EVENTS + 1) {
            supervisor.publish_event(CoreEvent::PlaybackFinished(PlaybackFinishedPayload {
                session_id: "b".repeat(32),
                song_id: "c".repeat(32),
                outcome: "finished".into(),
                total_us: 0,
                message: "finished".into(),
            }));
        }
        let events = supervisor.events.lock().expect("event buffer poisoned");
        assert_eq!(events.buffered.len(), MAX_BUFFERED_EVENTS);
        drop(events);
        assert!(supervisor.event_history_overflowed());
        assert!(matches!(
            supervisor.request("catalog.search", json!({})),
            Err(SupervisorError::Unavailable(message)) if message.contains("overflowed")
        ));
        supervisor.shutdown();
    }

    #[test]
    fn stderr_flood_does_not_deadlock_stdout_requests() {
        let supervisor = start("stderr_flood");
        let result = supervisor
            .request("catalog.search", json!({"query": "flood"}))
            .expect("response after stderr flood");
        assert_eq!(result["method"], "catalog.search");
        supervisor.shutdown();
    }

    #[test]
    fn shutdown_is_graceful_and_forced_fallback_is_bounded() {
        let graceful = start("normal");
        graceful.shutdown();
        assert!(eventually(Duration::from_secs(1), || {
            graceful.lifecycle() == CoreLifecycle::Exited
        }));

        let forced = start("force_shutdown");
        let started = Instant::now();
        forced.shutdown();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(eventually(Duration::from_secs(1), || {
            forced.lifecycle() == CoreLifecycle::Exited
        }));
    }
}
