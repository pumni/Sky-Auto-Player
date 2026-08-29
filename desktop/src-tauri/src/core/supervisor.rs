use super::protocol::{
    BoundedFrameReader, CoreEvent, CoreMessage, DESKTOP_PROTOCOL_VERSION, MAX_REQUEST_ID,
    encode_request,
};
use super::request_registry::{Completion, PendingRegistry};
use crate::ui_events::UiEvent;
use serde::Serialize;
use serde_json::Value;
use std::io::{self, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const RELOAD_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BUFFERED_EVENTS: usize = 128;
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(25);

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
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
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
            timeouts,
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
                                Ok(CoreMessage::Event(event)) => match event.name.as_str() {
                                    "core.ready" => {
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
                                    "core.fatal" => {
                                        supervisor.inbound_core_fatal(event, &ready_sender);
                                        break;
                                    }
                                    _ => supervisor.publish_event(event),
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
                        Ok(Some(status)) => break Ok(status),
                        Ok(None) => thread::sleep(CHILD_POLL_INTERVAL),
                        Err(error) => break Err(error),
                    }
                };
                if status.is_err() || !supervisor.shutdown_requested.load(Ordering::Acquire) {
                    supervisor.pending.fail_all("Core process exited");
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
        self.publish_event(CoreEvent {
            name: "core.fatal".into(),
            payload: serde_json::json!({"code": "protocol_error", "message": message}),
        });
        self.terminate_child();
    }

    fn inbound_core_fatal(
        &self,
        event: CoreEvent,
        ready_sender: &mpsc::Sender<Result<(), String>>,
    ) {
        let message = event
            .payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Core reported a fatal error");
        self.set_lifecycle(CoreLifecycle::Fatal);
        self.pending
            .fail_all(&format!("Core reported fatal error: {message}"));
        let _ = ready_sender.send(Err(message.to_owned()));
        self.publish_event(event);
        self.terminate_child();
    }

    fn transition_to_ready(&self) -> bool {
        let mut lifecycle = self.lifecycle.lock().expect("lifecycle poisoned");
        if *lifecycle != CoreLifecycle::Starting {
            return false;
        }
        *lifecycle = CoreLifecycle::Ready;
        true
    }

    fn publish_event(&self, event: CoreEvent) {
        let ui_event = UiEvent {
            v: DESKTOP_PROTOCOL_VERSION,
            name: event.name,
            payload: event.payload,
        };
        let mut events = self.events.lock().expect("event buffer poisoned");
        if events.buffered.len() >= MAX_BUFFERED_EVENTS {
            events.buffered.remove(0);
        }
        events.buffered.push(ui_event.clone());
        if let Some(channel) = events.channel.as_ref() {
            let _ = channel.send(ui_event);
        }
    }

    pub fn subscribe(&self, channel: tauri::ipc::Channel<UiEvent>) -> Result<(), SupervisorError> {
        let mut events = self.events.lock().expect("event buffer poisoned");
        for event in &events.buffered {
            channel
                .send(event.clone())
                .map_err(|error| SupervisorError::Request(error.to_string()))?;
        }
        events.channel = Some(channel);
        Ok(())
    }

    pub fn lifecycle(&self) -> CoreLifecycle {
        *self.lifecycle.lock().expect("lifecycle poisoned")
    }

    pub fn request<P: Serialize>(&self, method: &str, params: P) -> Result<Value, SupervisorError> {
        let lifecycle = self.lifecycle();
        if !lifecycle.accepts_requests() {
            return Err(SupervisorError::Unavailable(format!(
                "Core state is {lifecycle:?}"
            )));
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
        self.set_lifecycle(CoreLifecycle::ShuttingDown);
        let _ = self.request_inner("app.shutdown", serde_json::json!({}), self.timeouts.request);
        self.terminate_child();
    }

    fn set_lifecycle(&self, lifecycle: CoreLifecycle) {
        *self.lifecycle.lock().expect("lifecycle poisoned") = lifecycle;
    }

    fn terminate_child(&self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}

#[derive(Default)]
struct EventState {
    buffered: Vec<UiEvent>,
    channel: Option<tauri::ipc::Channel<UiEvent>>,
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
    use super::{CoreLifecycle, CoreSupervisor, SupervisorError, SupervisorTimeouts};
    use serde_json::json;
    use std::path::PathBuf;
    use std::process::Command;
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
        let events = supervisor.events.lock().expect("event buffer poisoned");
        let fatal = events
            .buffered
            .iter()
            .find(|event| event.name == "core.fatal")
            .expect("original fatal event");
        assert_eq!(fatal.payload["message"], "fatal after ready");
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
