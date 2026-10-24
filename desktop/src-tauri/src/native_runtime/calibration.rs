use super::{NativeEventHub, NativePlaybackService, TestSeams, bounded_text, opaque_native_id};
use crate::app_state::{ActivityCoordinator, ActivityReservationError};
use crate::commands::{
    CalibrationCancelAckDto, CalibrationCancelRequest, CalibrationStartAckDto,
    CalibrationStartRequest,
};
use crate::ui_events::{
    CalibrationFinishedPayload, CalibrationMode, CalibrationOutcome, CalibrationProgressPayload,
    CalibrationState, UiEvent,
};
use serde_json::Value;
use sky_native_adapters::{
    AppPaths, CALIBRATION_ARTIFACT_SCHEMA_VERSION, CALIBRATION_CACHE_VERSION,
    CALIBRATION_EVIDENCE_KIND, CALIBRATION_HOST_FINGERPRINT_VERSION, CALIBRATION_MAX_SHRINK_US,
    CALIBRATION_MEASUREMENT_PROTOCOL_VERSION, CALIBRATION_NATIVE_VERSION,
    CALIBRATION_REQUIRED_BUCKETS, CALIBRATION_SAMPLE_COUNT, CALIBRATION_SOURCE_FORMULA_VERSION,
    load_calibration_resolution,
};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
const MAX_CALIBRATION_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_CALIBRATION_ERROR_BYTES: usize = 64 * 1024;
pub(super) const CALIBRATION_DEFAULT_TIMEOUT_SECONDS: f64 = 120.0;
const CALIBRATION_GLOBAL_MAX_SECONDS: f64 = 120.0;
pub(super) const CALIBRATION_TIMING_MARGIN_GUARD_US: u64 = 100;
const CALIBRATION_PUBLICATION_RESERVE_SECONDS: f64 = 5.0;
const CALIBRATION_PARENT_EXIT_RESERVE_SECONDS: f64 = 1.0;
const CALIBRATION_NATIVE_EXIT_RESERVE_SECONDS: f64 = 1.0;
const CALIBRATION_NATIVE_CLEANUP_RESERVE_SECONDS: f64 = 5.0;

const CALIBRATION_MIN_MEASUREMENT_SECONDS: f64 = 1.0;
pub(super) const CALIBRATION_MIN_NATIVE_TOTAL_SECONDS: f64 =
    CALIBRATION_NATIVE_CLEANUP_RESERVE_SECONDS + CALIBRATION_MIN_MEASUREMENT_SECONDS;
pub(super) const CALIBRATION_MIN_SINGLE_TIMEOUT_SECONDS: f64 =
    CALIBRATION_PARENT_EXIT_RESERVE_SECONDS
        + CALIBRATION_NATIVE_EXIT_RESERVE_SECONDS
        + CALIBRATION_MIN_NATIVE_TOTAL_SECONDS;
pub(super) const CALIBRATION_MIN_FULL_TIMEOUT_SECONDS: f64 = CALIBRATION_PUBLICATION_RESERVE_SECONDS
    + CALIBRATION_NATIVE_EXIT_RESERVE_SECONDS
    + CALIBRATION_MIN_NATIVE_TOTAL_SECONDS;
const CALIBRATION_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Default)]
struct CalibrationPublicationGate {
    state: Mutex<CalibrationPublicationState>,
    changed: Condvar,
    #[cfg(test)]
    before_admission: Mutex<Option<(Arc<std::sync::Barrier>, Arc<std::sync::Barrier>)>>,
    #[cfg(test)]
    after_admission: Mutex<Option<(Arc<std::sync::Barrier>, Arc<std::sync::Barrier>)>>,
}

#[derive(Default)]
struct CalibrationPublicationState {
    closing: bool,
    commit_in_progress: bool,
}

struct CalibrationPublicationAdmission {
    gate: Arc<CalibrationPublicationGate>,
}

impl Drop for CalibrationPublicationAdmission {
    fn drop(&mut self) {
        if let Ok(mut state) = self.gate.state.lock() {
            state.commit_in_progress = false;
            self.gate.changed.notify_all();
        }
    }
}

impl CalibrationPublicationGate {
    fn try_admit(self: &Arc<Self>) -> Option<CalibrationPublicationAdmission> {
        #[cfg(test)]
        let before_admission = self
            .before_admission
            .lock()
            .ok()
            .and_then(|mut hook| hook.take());
        #[cfg(test)]
        if let Some((entered, release)) = before_admission {
            entered.wait();
            release.wait();
        }

        let mut state = self.state.lock().ok()?;
        if state.closing || state.commit_in_progress {
            return None;
        }
        state.commit_in_progress = true;
        drop(state);

        #[cfg(test)]
        let after_admission = self
            .after_admission
            .lock()
            .ok()
            .and_then(|mut hook| hook.take());
        #[cfg(test)]
        if let Some((entered, release)) = after_admission {
            entered.wait();
            release.wait();
        }

        Some(CalibrationPublicationAdmission {
            gate: Arc::clone(self),
        })
    }

    fn close_until(&self, deadline: Instant) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        state.closing = true;
        while state.commit_in_progress {
            if Instant::now() >= deadline {
                return false;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.changed.wait_timeout(state, remaining) {
                Ok((next, _)) => state = next,
                Err(_) => return false,
            }
        }
        true
    }

    #[cfg(test)]
    fn is_closing(&self) -> bool {
        self.state.lock().map(|state| state.closing).unwrap_or(true)
    }

    #[cfg(test)]
    fn pause_before_next_admission(
        &self,
        entered: Arc<std::sync::Barrier>,
        release: Arc<std::sync::Barrier>,
    ) {
        *self.before_admission.lock().expect("publication hook") = Some((entered, release));
    }

    #[cfg(test)]
    fn pause_after_next_admission(
        &self,
        entered: Arc<std::sync::Barrier>,
        release: Arc<std::sync::Barrier>,
    ) {
        *self.after_admission.lock().expect("publication hook") = Some((entered, release));
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CalibrationBudget {
    pub(super) child_timeout_seconds: f64,
    pub(super) native_budget_seconds: u64,
}

pub(super) fn calibration_budget(
    timeout_seconds: f64,
    mode: CalibrationMode,
) -> Result<CalibrationBudget, String> {
    let minimum = if mode == CalibrationMode::Full {
        CALIBRATION_MIN_FULL_TIMEOUT_SECONDS
    } else {
        CALIBRATION_MIN_SINGLE_TIMEOUT_SECONDS
    };
    if !timeout_seconds.is_finite()
        || timeout_seconds < minimum
        || timeout_seconds > CALIBRATION_GLOBAL_MAX_SECONDS
    {
        return Err(format!(
            "invalid_params: timeout_seconds must be finite and in [{minimum}, {CALIBRATION_GLOBAL_MAX_SECONDS}] for this calibration mode"
        ));
    }
    let child_timeout_seconds = if mode == CalibrationMode::Full {
        timeout_seconds - CALIBRATION_PUBLICATION_RESERVE_SECONDS
    } else {
        timeout_seconds - CALIBRATION_PARENT_EXIT_RESERVE_SECONDS
    };
    let native_budget_seconds =
        (child_timeout_seconds - CALIBRATION_NATIVE_EXIT_RESERVE_SECONDS).floor() as u64;
    if (native_budget_seconds as f64) < CALIBRATION_MIN_NATIVE_TOTAL_SECONDS {
        return Err(
            "invalid_params: timeout leaves no useful calibration measurement budget".into(),
        );
    }
    Ok(CalibrationBudget {
        child_timeout_seconds,
        native_budget_seconds: native_budget_seconds.min(CALIBRATION_GLOBAL_MAX_SECONDS as u64),
    })
}

pub(super) struct NativeCalibrationService {
    paths: AppPaths,
    activity: ActivityCoordinator,
    events: Arc<Mutex<NativeEventHub>>,
    playback: Arc<NativePlaybackService>,
    test_seams: TestSeams,
    operation: Mutex<Option<NativeCalibrationOperation>>,
    publication: Arc<CalibrationPublicationGate>,
    closed: AtomicBool,
}

struct NativeCalibrationOperation {
    operation_id: String,
    state: CalibrationState,
    cancel: Arc<AtomicBool>,
    child: Arc<Mutex<Option<std::process::Child>>>,
    worker: Option<thread::JoinHandle<()>>,
    done: Arc<(Mutex<bool>, Condvar)>,
    reservation: Option<crate::app_state::CalibrationReservation>,
}

impl NativeCalibrationService {
    pub(super) fn new(
        paths: AppPaths,
        activity: ActivityCoordinator,
        events: Arc<Mutex<NativeEventHub>>,
        playback: Arc<NativePlaybackService>,
        test_seams: TestSeams,
    ) -> Self {
        Self {
            paths,
            activity,
            events,
            playback,
            test_seams,
            operation: Mutex::new(None),
            publication: Arc::new(CalibrationPublicationGate::default()),
            closed: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub(super) fn from_test_sandbox(
        root: &Path,
        activity: ActivityCoordinator,
        events: Arc<Mutex<NativeEventHub>>,
        playback: Arc<NativePlaybackService>,
        test_seams: TestSeams,
    ) -> Self {
        let paths = AppPaths::from_test_sandbox(root);
        Self::new(paths, activity, events, playback, test_seams)
    }

    #[cfg(test)]
    pub(super) fn set_running_operation_for_test(
        &self,
        operation_id: String,
        done: Arc<(Mutex<bool>, Condvar)>,
        reservation: crate::app_state::CalibrationReservation,
        worker: Option<thread::JoinHandle<()>>,
    ) {
        *self.operation.lock().expect("calibration operation") = Some(NativeCalibrationOperation {
            operation_id,
            state: CalibrationState::Running,
            cancel: Arc::new(AtomicBool::new(false)),
            child: Arc::new(Mutex::new(None)),
            worker,
            done,
            reservation: Some(reservation),
        });
    }

    #[cfg(test)]
    pub(super) fn pause_before_admission_for_test(
        &self,
        entered: Arc<std::sync::Barrier>,
        release: Arc<std::sync::Barrier>,
    ) {
        self.publication
            .pause_before_next_admission(entered, release);
    }

    #[cfg(test)]
    pub(super) fn pause_after_admission_for_test(
        &self,
        entered: Arc<std::sync::Barrier>,
        release: Arc<std::sync::Barrier>,
    ) {
        self.publication
            .pause_after_next_admission(entered, release);
    }

    #[cfg(test)]
    pub(super) fn publication_is_closing_for_test(&self) -> bool {
        self.publication.is_closing()
    }

    #[cfg(test)]
    pub(super) fn is_calibration_active_for_test(&self) -> bool {
        self.activity.is_calibration_active()
    }

    pub(super) fn start(
        self: &Arc<Self>,
        request: CalibrationStartRequest,
    ) -> Result<CalibrationStartAckDto, String> {
        validate_calibration_request(&request)?;
        let operation_id = opaque_native_id()?;
        let cancel = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(None));
        let done = Arc::new((Mutex::new(false), Condvar::new()));
        {
            let mut current = self
                .operation
                .lock()
                .map_err(|_| "native calibration state lock poisoned".to_string())?;
            if current.as_ref().is_some_and(|value| {
                matches!(
                    value.state,
                    CalibrationState::Starting
                        | CalibrationState::Running
                        | CalibrationState::Cancelling
                )
            }) {
                return Err("already_running: a calibration operation is already active".into());
            }
            // Hold the operation lock while acquiring the shared activity
            // reservation.  This gives a second calibration start a stable
            // `already_running` linearization point instead of exposing the
            // generic activity error from a duplicate request.
            let reservation = self
                .activity
                .reserve_calibration()
                .map_err(calibration_activity_error)?;
            *current = Some(NativeCalibrationOperation {
                operation_id: operation_id.clone(),
                state: CalibrationState::Running,
                cancel: cancel.clone(),
                child: child.clone(),
                worker: None,
                done: done.clone(),
                reservation: Some(reservation),
            });
        }
        if let Err(error) = self.publish_progress(
            &operation_id,
            CalibrationState::Running,
            "starting",
            0,
            1,
            "Starting calibration",
        ) {
            if let Ok(mut current) = self.operation.lock() {
                current.take();
            }
            return Err(error);
        }
        let service = Arc::clone(self);
        let worker_id = operation_id.clone();
        let worker_done = done.clone();
        let handle = match thread::Builder::new()
            .name("native-calibration-orchestrator".into())
            .spawn(move || {
                service.run(worker_id, request, cancel, child);
                if let Ok(mut finished) = worker_done.0.lock() {
                    *finished = true;
                    worker_done.1.notify_all();
                }
            }) {
            Ok(handle) => handle,
            Err(error) => {
                if let Ok(mut current) = self.operation.lock() {
                    current.take();
                }
                return Err(format!("could not start calibration worker: {error}"));
            }
        };
        if let Ok(mut current) = self.operation.lock()
            && let Some(operation) = current.as_mut()
            && operation.operation_id == operation_id
        {
            operation.worker = Some(handle);
        }
        Ok(CalibrationStartAckDto {
            operation_id,
            state: CalibrationState::Running,
        })
    }

    pub(super) fn cancel(
        &self,
        request: CalibrationCancelRequest,
    ) -> Result<CalibrationCancelAckDto, String> {
        let mut operation = self
            .operation
            .lock()
            .map_err(|_| "native calibration state lock poisoned".to_string())?;
        let Some(current) = operation.as_mut() else {
            return Err("stale_operation: calibration operation is stale".into());
        };
        if current.operation_id != request.operation_id {
            return Err("stale_operation: calibration operation is stale".into());
        }
        if matches!(
            current.state,
            CalibrationState::Starting | CalibrationState::Running
        ) {
            current.state = CalibrationState::Cancelling;
            current.cancel.store(true, Ordering::Release);
            if let Ok(mut child) = current.child.lock()
                && let Some(child) = child.as_mut()
            {
                let _ = child.kill();
            }
            return Ok(CalibrationCancelAckDto {
                operation_id: request.operation_id,
                state: CalibrationState::Cancelling,
                accepted: true,
            });
        }
        if current.state == CalibrationState::Cancelling {
            return Ok(CalibrationCancelAckDto {
                operation_id: request.operation_id,
                state: CalibrationState::Cancelling,
                accepted: true,
            });
        }
        Ok(CalibrationCancelAckDto {
            operation_id: request.operation_id,
            state: current.state,
            accepted: false,
        })
    }

    pub(super) fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
        let deadline = Instant::now() + CALIBRATION_SHUTDOWN_TIMEOUT;
        if !self.publication.close_until(deadline) {
            // A commit admitted before Closing owns the final production
            // mutation.  Returning while it is still in flight would permit a
            // post-shutdown cache writer, so the process boundary fails closed
            // if that bounded admission contract cannot complete.
            std::process::abort();
        }
        let _ = self.shutdown_worker_until(deadline);
    }

    #[cfg(test)]
    pub(super) fn shutdown_with_timeout(&self, timeout: Duration) -> bool {
        self.closed.store(true, Ordering::Release);
        let deadline = Instant::now() + timeout;
        if !self.publication.close_until(deadline) {
            return false;
        }
        self.shutdown_worker_until(deadline)
    }

    fn shutdown_worker_until(&self, deadline: Instant) -> bool {
        let worker_and_done = if let Ok(mut operation) = self.operation.lock() {
            if let Some(current) = operation.as_mut()
                && matches!(
                    current.state,
                    CalibrationState::Starting
                        | CalibrationState::Running
                        | CalibrationState::Cancelling
                )
            {
                current.state = CalibrationState::Cancelling;
                current.cancel.store(true, Ordering::Release);
                if let Ok(mut child) = current.child.lock()
                    && let Some(child) = child.as_mut()
                {
                    let _ = child.kill();
                }
            }
            operation.as_mut().map(|value| {
                (
                    value.worker.take(),
                    value.done.clone(),
                    value.reservation.take(),
                )
            })
        } else {
            None
        };
        if let Some((worker, done, reservation)) = worker_and_done {
            if let Some(worker) = worker {
                let mut finished = done.0.lock().ok();
                while let Some(value) = finished.take() {
                    if *value || Instant::now() >= deadline {
                        finished = Some(value);
                        break;
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    match done.1.wait_timeout(value, remaining) {
                        Ok((next, _)) => finished = Some(next),
                        Err(_) => break,
                    }
                }
                if finished.as_deref() == Some(&true) {
                    let _ = worker.join();
                    drop(reservation);
                    return true;
                }
                // A worker that did not acknowledge completion within the
                // bounded shutdown budget cannot be allowed to publish a cache
                // or terminal event after shutdown.  The publication gate is
                // already Closing, so dropping this handle is a safe,
                // non-publishing containment path rather than an unbounded join.
                drop(reservation);
                return false;
            }
            drop(reservation);
        }
        true
    }

    pub(super) fn wait_for_terminal(&self, timeout: Duration) -> Result<CalibrationState, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let state = self
                .operation
                .lock()
                .map_err(|_| "native calibration state lock poisoned".to_string())?
                .as_ref()
                .map(|operation| operation.state);
            if let Some(
                state @ (CalibrationState::Succeeded
                | CalibrationState::Failed
                | CalibrationState::Cancelled),
            ) = state
            {
                return Ok(state);
            }
            if Instant::now() >= deadline {
                return Err("calibration selftest timed out".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn run(
        &self,
        operation_id: String,
        request: CalibrationStartRequest,
        cancel: Arc<AtomicBool>,
        child_slot: Arc<Mutex<Option<std::process::Child>>>,
    ) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let result = self.run_child(&request, &cancel, child_slot);
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        if cancel.load(Ordering::Acquire) {
            self.finish(
                &operation_id,
                CalibrationState::Cancelled,
                CalibrationOutcome::Cancelled,
                "cancelled",
                None,
                0,
                "none",
                "Calibration was cancelled.",
                false,
            );
            return;
        }
        match result {
            Ok(_raw) if request.mode == CalibrationMode::Diagnostic => self.finish(
                &operation_id,
                CalibrationState::Succeeded,
                CalibrationOutcome::Succeeded,
                "succeeded",
                None,
                request.samples.unwrap_or_default() as u64,
                "native",
                "Diagnostic calibration completed successfully.",
                false,
            ),
            Ok(raw) => {
                // Evidence validation and temporary-cache generation happen
                // before commit admission.  Only the final production rename,
                // prepared-plan invalidation, and terminal publication are
                // protected by the shared shutdown/publication gate.
                let prepared = match prepare_calibration_cache(self.paths.cache_root(), &raw) {
                    Ok(value) => value,
                    Err(error) => {
                        self.finish(
                            &operation_id,
                            CalibrationState::Failed,
                            CalibrationOutcome::Failed,
                            "failed",
                            None,
                            0,
                            "none",
                            &error,
                            false,
                        );
                        return;
                    }
                };
                let Some(publication) = self.publication.try_admit() else {
                    drop(prepared);
                    return;
                };
                // Calibration changes the advisory input read by Prepare.
                // Serialize its small cache rename and Prepare invalidation
                // against source capture and cache publication.
                let cache_commit = match self.playback.prepare_publication.lock() {
                    Ok(mut prepare_publication) => match prepared.commit() {
                        Ok(committed) => {
                            self.playback
                                .invalidate_settings_locked(&mut prepare_publication, None);
                            Ok(committed)
                        }
                        Err(error) => Err(error),
                    },
                    Err(_) => Err("native Prepare publication lock poisoned".to_string()),
                };
                match cache_commit {
                    Ok((margin, samples)) => {
                        // Cache publication and prepared-plan invalidation precede
                        // the terminal event and release of the activity lease.
                        self.finish_admitted(
                            publication,
                            &operation_id,
                            CalibrationState::Succeeded,
                            CalibrationOutcome::Succeeded,
                            "succeeded",
                            margin,
                            samples,
                            "native",
                            "Calibration completed successfully.",
                            margin.is_some(),
                        );
                    }
                    Err(error) => self.finish_admitted(
                        publication,
                        &operation_id,
                        CalibrationState::Failed,
                        CalibrationOutcome::Failed,
                        "failed",
                        None,
                        0,
                        "none",
                        &error,
                        false,
                    ),
                }
            }
            Err(CalibrationRunError::Cancelled) => self.finish(
                &operation_id,
                CalibrationState::Cancelled,
                CalibrationOutcome::Cancelled,
                "cancelled",
                None,
                0,
                "none",
                "Calibration was cancelled.",
                false,
            ),
            Err(CalibrationRunError::Failed(error)) => self.finish(
                &operation_id,
                CalibrationState::Failed,
                CalibrationOutcome::Failed,
                "failed",
                None,
                0,
                "none",
                &error,
                false,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        operation_id: &str,
        state: CalibrationState,
        outcome: CalibrationOutcome,
        status: &str,
        margin_us: Option<u64>,
        sample_count: u64,
        source: &str,
        message: &str,
        applied: bool,
    ) {
        let Some(publication) = self.publication.try_admit() else {
            return;
        };
        self.finish_admitted(
            publication,
            operation_id,
            state,
            outcome,
            status,
            margin_us,
            sample_count,
            source,
            message,
            applied,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_admitted(
        &self,
        _publication: CalibrationPublicationAdmission,
        operation_id: &str,
        state: CalibrationState,
        outcome: CalibrationOutcome,
        status: &str,
        margin_us: Option<u64>,
        sample_count: u64,
        _source: &str,
        message: &str,
        applied: bool,
    ) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let reservation = if let Ok(mut operation) = self.operation.lock() {
            let Some(current) = operation.as_mut() else {
                return;
            };
            if current.operation_id != operation_id
                || matches!(
                    current.state,
                    CalibrationState::Succeeded
                        | CalibrationState::Failed
                        | CalibrationState::Cancelled
                )
            {
                return;
            }
            current.state = state;
            current.reservation.take()
        } else {
            None
        };
        let recommendation_available = outcome == CalibrationOutcome::Succeeded;
        let recommended_timing_margin_us = if !recommendation_available {
            None
        } else if applied {
            recommended_calibrated_timing_margin_us(
                margin_us.unwrap_or(sky_native_adapters::DEFAULT_TRANSPORT_MARGIN_US),
            )
        } else {
            Some(sky_app_core::settings::DEFAULT_TIMING_MARGIN_US)
        };
        let insufficient_headroom =
            recommendation_available && applied && recommended_timing_margin_us.is_none();
        let recommendation_qualified =
            recommendation_available && applied && !insufficient_headroom;
        let recommendation_source = if insufficient_headroom {
            "insufficient_headroom"
        } else if recommendation_qualified {
            "qualified_calibration"
        } else if recommendation_available {
            "out_of_envelope_fallback"
        } else {
            "unavailable"
        };
        let _ = self.publish_finished(CalibrationFinishedPayload {
            operation_id: operation_id.to_owned(),
            outcome,
            status: bounded_text(status),
            recommended_timing_margin_us,
            recommendation_qualified,
            sample_count,
            source: recommendation_source.into(),
            message: bounded_text(message),
        });
        drop(reservation);
    }

    fn publish_progress(
        &self,
        operation_id: &str,
        state: CalibrationState,
        phase: &str,
        completed: u64,
        total: u64,
        message: &str,
    ) -> Result<(), String> {
        self.publish(UiEvent::CalibrationProgress {
            v: crate::DESKTOP_PROTOCOL_VERSION,
            payload: CalibrationProgressPayload {
                operation_id: operation_id.to_owned(),
                state,
                phase: bounded_text(phase),
                completed: completed.min(10_000),
                total: total.clamp(1, 10_000),
                message: bounded_text(message),
            },
        })
    }

    fn publish_finished(&self, payload: CalibrationFinishedPayload) -> Result<(), String> {
        self.publish(UiEvent::CalibrationFinished {
            v: crate::DESKTOP_PROTOCOL_VERSION,
            payload,
        })
    }

    fn publish(&self, event: UiEvent) -> Result<(), String> {
        self.events
            .lock()
            .map_err(|_| "native event hub lock poisoned".to_string())?
            .publish(event)
    }

    pub(super) fn run_child(
        &self,
        request: &CalibrationStartRequest,
        cancel: &AtomicBool,
        child_slot: Arc<Mutex<Option<std::process::Child>>>,
    ) -> Result<Value, CalibrationRunError> {
        if cancel.load(Ordering::Acquire) {
            return Err(CalibrationRunError::Cancelled);
        }
        if self.test_seams == TestSeams::SafePackage {
            return Ok(safe_calibration_evidence());
        }
        let binary = self.paths.calibration_binary_path();
        let mode = match request.mode {
            CalibrationMode::Quick => "quick",
            CalibrationMode::Full => "full",
            CalibrationMode::Diagnostic => "bucket",
        };
        let timeout = request
            .timeout_seconds
            .unwrap_or(CALIBRATION_DEFAULT_TIMEOUT_SECONDS);
        let budget =
            calibration_budget(timeout, request.mode).map_err(CalibrationRunError::Failed)?;
        let mut command = std::process::Command::new(&binary);
        command
            .arg("--mode")
            .arg(mode)
            .arg("--budget-seconds")
            .arg(budget.native_budget_seconds.to_string());
        if request.mode == CalibrationMode::Diagnostic {
            command
                .arg("--class")
                .arg(request.class_name.as_deref().unwrap_or_default())
                .arg("--polyphony")
                .arg(request.polyphony.unwrap_or_default().to_string())
                .arg("--samples")
                .arg(request.samples.unwrap_or_default().to_string());
        }
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().map_err(|error| {
            CalibrationRunError::Failed(format!("calibration child failed to start: {error}"))
        })?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let out_reader = thread::spawn(move || read_bounded(stdout, MAX_CALIBRATION_OUTPUT_BYTES));
        let err_reader = thread::spawn(move || read_bounded(stderr, MAX_CALIBRATION_ERROR_BYTES));
        {
            let mut slot = child_slot.lock().map_err(|_| {
                CalibrationRunError::Failed("calibration child lock poisoned".into())
            })?;
            *slot = Some(child);
        }
        let deadline = Instant::now() + Duration::from_secs_f64(budget.child_timeout_seconds);
        let mut exited = false;
        while Instant::now() < deadline {
            if cancel.load(Ordering::Acquire) {
                if let Ok(mut slot) = child_slot.lock()
                    && let Some(child) = slot.as_mut()
                {
                    let _ = child.kill();
                }
                break;
            }
            if let Ok(mut slot) = child_slot.lock()
                && let Some(child) = slot.as_mut()
                && child.try_wait().ok().flatten().is_some()
            {
                exited = true;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if !exited
            && !cancel.load(Ordering::Acquire)
            && let Ok(mut slot) = child_slot.lock()
            && let Some(child) = slot.as_mut()
        {
            let _ = child.kill();
        }
        let status = child_slot
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
            .and_then(|mut child| child.wait().ok());
        let stdout = out_reader
            .join()
            .unwrap_or_else(|_| Err("stdout reader panicked".into()))
            .map_err(CalibrationRunError::Failed)?;
        let stderr = err_reader
            .join()
            .unwrap_or_else(|_| Err("stderr reader panicked".into()))
            .map_err(CalibrationRunError::Failed)?;
        if cancel.load(Ordering::Acquire) {
            return Err(CalibrationRunError::Cancelled);
        }
        if status.is_none_or(|value| !value.success()) {
            return Err(CalibrationRunError::Failed(format!(
                "native calibration failed: {}",
                bounded_text(String::from_utf8_lossy(&stderr))
            )));
        }
        serde_json::from_slice(&stdout).map_err(|error| {
            CalibrationRunError::Failed(format!("native calibration output invalid: {error}"))
        })
    }
}

pub(super) enum CalibrationRunError {
    Cancelled,
    Failed(String),
}

pub(super) fn safe_calibration_evidence() -> Value {
    let quantiles = || {
        serde_json::json!({
            "min": 0,
            "p50": 0,
            "p90": 0,
            "p95": 0,
            "p99": 0,
            "max": 0,
            "mean": 0
        })
    };
    let bucket = || {
        serde_json::json!({
            "attempted": 100,
            "clean": 100,
            "clean_pair_count": 100,
            "clean_sample_count": 100,
            "rejected": 0,
            "partial_send": 0,
            "sample_count": 100,
            "anomaly_count": 0,
            "pairing_anomaly_count": 0,
            "duplicate_receipt_count": 0,
            "unexpected_scan_code_count": 0,
            "direction_mismatch_count": 0,
            "reordered_receipt_count": 0,
            "timeout_count": 0,
            "class_mismatch_count": 0,
            "receipt_before_completion_count": 0,
            "pair_sender_hold_shrink_us": quantiles(),
            "scheduler_shrink_us": quantiles(),
            "sendinput_shrink_us": quantiles(),
            "down_call_duration_us": quantiles(),
            "up_call_duration_us": quantiles()
        })
    };
    let host = sky_dispatch_win32::calibration::build_host_fingerprint()
        .ok()
        .and_then(|value| serde_json::to_value(value).ok())
        .unwrap_or_else(|| {
            serde_json::json!({
                "host_fingerprint_version": CALIBRATION_HOST_FINGERPRINT_VERSION,
                "qpc_frequency_hz": 1,
                "win32_build": "safe-packaged-selftest",
                "processor_architecture": std::env::consts::ARCH,
                "cpu_vendor": "safe-packaged-selftest",
                "cpu_family": 0,
                "cpu_model": 0,
                "cpu_stepping": 0,
                "logical_processor_count": 1,
                "processor_group_count": 1,
                "cpu_set_efficiency_classes": [0]
            })
        });
    serde_json::json!({
        "version": CALIBRATION_NATIVE_VERSION,
        "calibration_schema_version": CALIBRATION_NATIVE_VERSION,
        "measurement_protocol_version": CALIBRATION_MEASUREMENT_PROTOCOL_VERSION,
        "source_git_sha": "safe-packaged-selftest",
        "native_build_id": "safe-packaged-selftest",
        "dirty_worktree": false,
        "native_source_fingerprint": "safe-packaged-selftest",
        "rustc_version": "safe-packaged-selftest",
        "evidence_kind": CALIBRATION_EVIDENCE_KIND,
        "host_fingerprint": host,
        "scheduling_aids": {
            "mmcss_acquired": "off",
            "mmcss_active": false,
            "power_throttling_active": false,
            "waiter_mode": "event+high_resolution_timer"
        },
        "configuration": {
            "polyphonies": [1, 5, 15],
            "samples_per_hot_bucket": 100,
            "samples_per_cold_bucket": 100,
            "warmup_samples": 0,
            "hot_gap_target_us": 5000,
            "cold_idle_gap_us": 25000,
            "cold_threshold_us": 25000,
            "budget_seconds": 6
        },
        "pair_buckets": {
            "1": {"hot": bucket(), "cold": bucket()},
            "5": {"hot": bucket(), "cold": bucket()},
            "15": {"hot": bucket(), "cold": bucket()}
        }
    })
}

fn validate_calibration_request(request: &CalibrationStartRequest) -> Result<(), String> {
    let timeout = request
        .timeout_seconds
        .unwrap_or(CALIBRATION_DEFAULT_TIMEOUT_SECONDS);
    calibration_budget(timeout, request.mode)?;
    if request.mode == CalibrationMode::Diagnostic {
        if !matches!(request.class_name.as_deref(), Some("hot") | Some("cold")) {
            return Err("invalid_params: diagnostic class_name must be hot or cold".into());
        }
        if !matches!(request.polyphony, Some(1 | 5 | 15)) {
            return Err("invalid_params: diagnostic polyphony must be 1, 5, or 15".into());
        }
        if !matches!(request.samples, Some(1..=5000)) {
            return Err("invalid_params: diagnostic samples must be 1..5000".into());
        }
    } else if request.class_name.is_some()
        || request.polyphony.is_some()
        || request.samples.is_some()
    {
        return Err("invalid_params: diagnostic fields are only valid in diagnostic mode".into());
    }
    Ok(())
}

fn read_bounded<R: Read>(reader: Option<R>, limit: usize) -> Result<Vec<u8>, String> {
    let Some(reader) = reader else {
        return Ok(Vec::new());
    };
    let mut bytes = Vec::new();
    reader
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > limit {
        return Err("calibration child output exceeds bound".into());
    }
    Ok(bytes)
}

struct PreparedCalibrationCache {
    temp_path: PathBuf,
    cache_path: PathBuf,
    margin_us: Option<u64>,
    sample_count: u64,
    committed: bool,
}

impl PreparedCalibrationCache {
    fn commit(mut self) -> Result<(Option<u64>, u64), String> {
        fs::rename(&self.temp_path, &self.cache_path).map_err(|error| error.to_string())?;
        self.committed = true;
        Ok((self.margin_us, self.sample_count))
    }
}

impl Drop for PreparedCalibrationCache {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.temp_path);
        }
    }
}

/// Convert the child protocol-vNext evidence into the accepted cache-v8
/// shape.  All required production buckets and publishability counters are
/// checked before the temporary cache is exposed to the commit gate; a
/// zero/partial/malformed child result can never become a timing policy.
fn prepare_calibration_cache(
    cache_root: &Path,
    raw: &Value,
) -> Result<PreparedCalibrationCache, String> {
    let root = raw
        .as_object()
        .ok_or("calibration evidence root must be an object")?;
    if root.get("version").and_then(Value::as_u64) != Some(CALIBRATION_NATIVE_VERSION)
        || root
            .get("measurement_protocol_version")
            .and_then(Value::as_u64)
            != Some(CALIBRATION_MEASUREMENT_PROTOCOL_VERSION)
        || root.get("evidence_kind").and_then(Value::as_str) != Some(CALIBRATION_EVIDENCE_KIND)
        || root.get("dirty_worktree").and_then(Value::as_bool) != Some(false)
    {
        return Err("calibration evidence metadata is incompatible".into());
    }
    let source = root
        .get("pair_buckets")
        .and_then(Value::as_object)
        .ok_or("calibration pair buckets are missing")?;
    let mut flattened = serde_json::Map::new();
    let mut worst = 0_i64;
    let mut worst_bucket = CALIBRATION_REQUIRED_BUCKETS[0];
    let mut samples = 0_u64;
    for key in CALIBRATION_REQUIRED_BUCKETS {
        let (polyphony, class_name) = key.split_once('/').expect("required bucket shape");
        let value = source
            .get(polyphony)
            .and_then(Value::as_object)
            .and_then(|value| value.get(class_name))
            .and_then(Value::as_object)
            .ok_or_else(|| format!("calibration bucket is missing: {key}"))?;
        let mut bucket = value.clone();
        let clean = bucket
            .get("clean")
            .and_then(Value::as_u64)
            .or_else(|| bucket.get("clean_pair_count").and_then(Value::as_u64))
            .ok_or_else(|| format!("calibration clean count is missing: {key}"))?;
        let attempted = bucket
            .get("attempted")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("calibration attempted count is missing: {key}"))?;
        if !(CALIBRATION_SAMPLE_COUNT..=CALIBRATION_SAMPLE_COUNT * 2).contains(&attempted)
            || clean != CALIBRATION_SAMPLE_COUNT
        {
            return Err(format!("calibration bucket is not publishable: {key}"));
        }
        let rejected = bucket
            .get("rejected")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("calibration rejected count is missing: {key}"))?;
        if rejected != attempted.saturating_sub(clean)
            || bucket.get("anomaly_count").and_then(Value::as_u64) != Some(rejected)
            || bucket.get("class_mismatch_count").and_then(Value::as_u64) != Some(rejected)
            || bucket.get("timeout_count").and_then(Value::as_u64) != Some(0)
            || bucket.get("partial_send").and_then(Value::as_u64) != Some(0)
        {
            return Err(format!(
                "calibration bucket counters are not publishable: {key}"
            ));
        }
        for quantiles in [
            "pair_sender_hold_shrink_us",
            "scheduler_shrink_us",
            "sendinput_shrink_us",
        ] {
            validate_signed_quantiles(
                bucket.get(quantiles),
                &format!("calibration {quantiles} is invalid: {key}"),
            )?;
        }
        for quantiles in ["down_call_duration_us", "up_call_duration_us"] {
            validate_unsigned_quantiles(
                bucket.get(quantiles),
                &format!("calibration {quantiles} is invalid: {key}"),
            )?;
        }
        bucket.insert("clean_pair_count".into(), Value::from(clean));
        let shrink = bucket
            .get("sendinput_shrink_us")
            .and_then(Value::as_object)
            .and_then(|value| value.get("max"))
            .and_then(Value::as_i64)
            .ok_or_else(|| format!("calibration sendinput quantile is missing: {key}"))?;
        if shrink > worst {
            worst = shrink;
            worst_bucket = key;
        }
        samples = samples.saturating_add(clean);
        flattened.insert(key.to_owned(), Value::Object(bucket));
    }
    let candidate = worst.saturating_add(100);
    let valid = candidate <= 2_000;
    let applied = candidate.clamp(300, 2_000);
    let mut cache = root.clone();
    cache.insert("version".into(), Value::from(CALIBRATION_CACHE_VERSION));
    cache.insert(
        "calibration_schema_version".into(),
        Value::from(CALIBRATION_NATIVE_VERSION),
    );
    cache.insert(
        "artifact_schema_version".into(),
        Value::from(CALIBRATION_ARTIFACT_SCHEMA_VERSION),
    );
    cache.insert(
        "native_calibration_version".into(),
        Value::from(CALIBRATION_NATIVE_VERSION),
    );
    cache.insert(
        "source_formula_version".into(),
        Value::from(CALIBRATION_SOURCE_FORMULA_VERSION),
    );
    cache.insert(
        "source".into(),
        Value::from(sky_native_adapters::CALIBRATION_MARGIN_SOURCE_DEVICE),
    );
    cache.insert(
        "status".into(),
        Value::from(if valid { "valid" } else { "out_of_envelope" }),
    );
    cache.insert(
        "transport_margin_us".into(),
        if valid {
            Value::from(applied)
        } else {
            Value::Null
        },
    );
    cache.insert(
        "transport_margin_source".into(),
        Value::from("device_cache"),
    );
    cache.insert(
        "transport_worst_positive_us".into(),
        Value::from(worst.max(0)),
    );
    cache.insert("transport_guard_us".into(), Value::from(100));
    cache.insert("transport_floor_us".into(), Value::from(300));
    cache.insert("transport_ceiling_us".into(), Value::from(2000));
    cache.insert("calibration_timing_qualified".into(), Value::from(valid));
    cache.insert(
        "required_buckets".into(),
        serde_json::json!(CALIBRATION_REQUIRED_BUCKETS),
    );
    cache.insert("pair_buckets".into(), Value::Object(flattened));
    cache.insert(
        "qualification".into(),
        serde_json::json!({
            "basis": "max_required_bucket_max_positive_sendinput_shrink",
            "worst_bucket": worst_bucket,
            "transport_worst_positive_us": worst.max(0),
            "guard_us": 100,
            "floor_us": 300,
            "ceiling_us": 2000,
            "candidate_transport_margin_us": candidate,
            "applied_transport_margin_us": if valid { Value::from(applied) } else { Value::Null }
        }),
    );
    let cache_path = cache_root.join(sky_native_adapters::paths::CALIBRATION_CACHE_FILE);
    fs::create_dir_all(cache_path.parent().expect("cache parent"))
        .map_err(|error| error.to_string())?;
    let temp = cache_path.with_extension(format!("json.{}.tmp", opaque_native_id()?));
    let serialized =
        serde_json::to_vec_pretty(&Value::Object(cache)).map_err(|error| error.to_string())?;
    if let Err(error) = fs::write(&temp, serialized) {
        let _ = fs::remove_file(&temp);
        return Err(error.to_string());
    }
    let resolution = load_calibration_resolution(&temp);
    if !matches!(
        resolution.source.as_str(),
        sky_native_adapters::CALIBRATION_MARGIN_SOURCE_DEVICE
            | sky_native_adapters::CALIBRATION_MARGIN_SOURCE_OUT_OF_ENVELOPE
    ) {
        let _ = fs::remove_file(&temp);
        return Err("published calibration cache failed its compatibility validation".into());
    }
    Ok(PreparedCalibrationCache {
        temp_path: temp,
        cache_path,
        margin_us: valid.then_some(applied as u64),
        sample_count: samples,
        committed: false,
    })
}

#[cfg(test)]
pub(super) fn publish_calibration_cache(
    cache_root: &Path,
    raw: &Value,
) -> Result<(Option<u64>, u64), String> {
    prepare_calibration_cache(cache_root, raw)?.commit()
}

fn validate_signed_quantiles(value: Option<&Value>, message: &str) -> Result<(), String> {
    let Some(object) = value.and_then(Value::as_object) else {
        return Err(message.into());
    };
    let fields = ["min", "p50", "p90", "p95", "p99", "max", "mean"];
    let Some(values) = fields
        .iter()
        .map(|field| object.get(*field).and_then(Value::as_i64))
        .collect::<Option<Vec<_>>>()
    else {
        return Err(message.into());
    };
    if !values[..6].windows(2).all(|pair| pair[0] <= pair[1])
        || values[0] > values[6]
        || values[6] > values[5]
        || values[..6]
            .iter()
            .any(|value| value.unsigned_abs() > CALIBRATION_MAX_SHRINK_US as u64)
    {
        return Err(message.into());
    }
    Ok(())
}

fn validate_unsigned_quantiles(value: Option<&Value>, message: &str) -> Result<(), String> {
    let Some(object) = value.and_then(Value::as_object) else {
        return Err(message.into());
    };
    let fields = ["min", "p50", "p90", "p95", "p99", "max", "mean"];
    let Some(values) = fields
        .iter()
        .map(|field| object.get(*field).and_then(Value::as_u64))
        .collect::<Option<Vec<_>>>()
    else {
        return Err(message.into());
    };
    if !values[..6].windows(2).all(|pair| pair[0] <= pair[1])
        || values[0] > values[6]
        || values[6] > values[5]
    {
        return Err(message.into());
    }
    Ok(())
}

pub(super) fn timing_margin_recommendation(
    calibration_cache_path: impl AsRef<Path>,
) -> crate::commands::TimingMarginRecommendationDto {
    let resolution = load_calibration_resolution(calibration_cache_path);
    let recommended_timing_margin_us = if resolution.qualified {
        recommended_calibrated_timing_margin_us(resolution.transport_reserve_us)
    } else {
        Some(sky_app_core::settings::DEFAULT_TIMING_MARGIN_US)
    };
    let source = if resolution.qualified && recommended_timing_margin_us.is_none() {
        "insufficient_headroom"
    } else {
        match resolution.source.as_str() {
            sky_native_adapters::CALIBRATION_MARGIN_SOURCE_DEVICE => "qualified_calibration",
            sky_native_adapters::CALIBRATION_MARGIN_SOURCE_OUT_OF_ENVELOPE => {
                "out_of_envelope_fallback"
            }
            sky_native_adapters::CALIBRATION_MARGIN_SOURCE_INVALID => "invalid_cache_fallback",
            sky_native_adapters::CALIBRATION_MARGIN_SOURCE_INCOMPATIBLE => {
                "incompatible_calibration_fallback"
            }
            _ => "default_fallback",
        }
    };
    crate::commands::TimingMarginRecommendationDto {
        recommended_timing_margin_us,
        qualified: resolution.qualified && recommended_timing_margin_us.is_some(),
        source: source.into(),
    }
}

pub(super) fn recommended_calibrated_timing_margin_us(transport_reserve_us: u64) -> Option<u64> {
    let evidence_budget_us =
        transport_reserve_us.checked_add(CALIBRATION_TIMING_MARGIN_GUARD_US)?;
    let step_us = sky_app_core::settings::TIMING_MARGIN_STEP_US;
    let recommendation_us = evidence_budget_us.div_ceil(step_us).checked_mul(step_us)?;
    (recommendation_us <= sky_app_core::settings::MAX_TIMING_MARGIN_US).then_some(recommendation_us)
}

fn calibration_activity_error(error: ActivityReservationError) -> String {
    match error {
        ActivityReservationError::Closing => "closing: desktop application is closing".into(),
        ActivityReservationError::CalibrationAlreadyActive => {
            "already_running: a calibration operation is already active".into()
        }
        ActivityReservationError::PhysicalPlaybackActive => {
            "playback_active: calibration cannot run during physical playback".into()
        }
        ActivityReservationError::PhysicalReleaseUnverified => {
            "playback_release_unverified: calibration is blocked because the previous physical key release could not be verified; restart the application to restore physical input".into()
        }
        ActivityReservationError::UpdateAlreadyActive => {
            "update_active: calibration cannot run during update installation".into()
        }
    }
}
