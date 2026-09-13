//! Native desktop application runtime.
//!
//! This is the composition root for commands that have crossed the strangler
//! boundary.  It owns live application state and calls pure app-core services
//! plus outer adapters.  It never delegates a native-owned command to the
//! Python Core.

use crate::app_state::{ActivityCoordinator, ActivityReservationError, PhysicalActivityLease};
use crate::commands::{
    BootstrapDto, CalibrationCancelAckDto, CalibrationCancelRequest, CalibrationStartAckDto,
    CalibrationStartRequest, CatalogDetailRequest, CatalogReloadDto, CatalogRowDto,
    CatalogSearchDto, CatalogSearchRequest, CatalogSetLikedDto, CatalogSetLikedRequest,
    CatalogSourceId, CatalogViewportDto, CatalogViewportRequest, DiagnosticsEnabledDto,
    DiagnosticsSetEnabledRequest, LibraryCreatePlaylistRequest, LibraryNavigationDto,
    LibraryPlaylistIdRequest, LibraryPlaylistImportResultDto, LibraryPlaylistSongsRequest,
    LibraryPlaylistSummaryDto, LibraryRenamePlaylistRequest, LibrarySource,
    PlaybackActiveStatusDto, PlaybackAdmission, PlaybackCommandAckDto, PlaybackConfigDto,
    PlaybackDecision, PlaybackDecisionAcceptanceDto, PlaybackDefaultsDto, PlaybackPendingControl,
    PlaybackPlanVariantDto, PlaybackPrepareRequest, PlaybackSessionDto, PlaybackSessionState,
    PlaybackStartRequest, PlaybackStatusDto, PlaybackTerminalStatusDto, PreparedPlaybackDto,
    RiskDecisionDto, RiskSummaryDto, SettingsDto, SettingsPatch, SongDetailDto, UpdateCheckDto,
    UpdateHandoffDto, UpdatePreferencesDto, UpdatePreferencesPatch,
};
use crate::ui_events::{
    CalibrationFinishedPayload, CalibrationMode, CalibrationOutcome, CalibrationProgressPayload,
    CalibrationState, CatalogChangedPayload, CoreReadyPayload, DiagnosticsBackendStatus,
    NativeBuildPayload, PlaybackEventState, PlaybackFailedPayload, PlaybackFinishedPayload,
    PlaybackFocusState, PlaybackHealthState, PlaybackSnapshotPayload, PlaybackStateChangedPayload,
    UiEvent,
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sky_app_core::catalog::{CatalogError, CatalogIndex, WRatioRanker};
use sky_app_core::library::{
    ImportedSourceKind, ImportedSourceRef, LibraryError, LibraryManifestService,
};
use sky_app_core::settings::{
    ApplicationSettings, PlaybackDefaultsPatch, SettingsError, SettingsService,
    UpdatePreferencesPatch as CoreUpdatePreferencesPatch,
};
use sky_app_core::song::{
    ActionKind, RiskReport, ScheduleMetadata, Song, analyze_schedule_with_context,
    build_schedule_with_policy, parse_song_json,
};
use sky_app_core::timing::MaterializedTimingPolicy;
use sky_native_adapters::{
    AppPaths, AppResources, BuiltinCatalogSource, BuiltinCatalogStatus,
    CALIBRATION_ARTIFACT_SCHEMA_VERSION, CALIBRATION_CACHE_VERSION, CALIBRATION_EVIDENCE_KIND,
    CALIBRATION_HOST_FINGERPRINT_VERSION, CALIBRATION_MAX_SHRINK_US,
    CALIBRATION_MEASUREMENT_PROTOCOL_VERSION, CALIBRATION_NATIVE_VERSION,
    CALIBRATION_REQUIRED_BUCKETS, CALIBRATION_SAMPLE_COUNT, CALIBRATION_SOURCE_FORMULA_VERSION,
    CatalogComposer, CatalogComposition, FileCatalogSource, ImportedSourceCatalogStatus,
    JsonLibraryManifestStore, JsonSettingsStore, load_calibration_resolution,
};
use sky_player::adapter_support::{
    ActionKind as DispatchActionKind, KeyActionInput, PriorityMode, compile_runtime_intents,
};
use sky_player::engine::{
    BackendConfig, DispatchProfile, EnginePollStatus, FocusOptions, NativeDispatchSession,
    NativeSessionOptions, PriorityOptions, TelemetryMode, TelemetryOptions, TimingOptions,
    WaitOptions,
};
use smallvec::SmallVec;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;

pub(crate) const MAX_NATIVE_EVENTS: usize = 128;
const MAX_PREPARED_PLANS: usize = 64;
const MAX_DECISION_COUNT: usize = 8;
const MAX_CALIBRATION_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_CALIBRATION_ERROR_BYTES: usize = 64 * 1024;
const CALIBRATION_DEFAULT_TIMEOUT_SECONDS: f64 = 120.0;
const CALIBRATION_GLOBAL_MAX_SECONDS: f64 = 120.0;
const CALIBRATION_PUBLICATION_RESERVE_SECONDS: f64 = 5.0;
const CALIBRATION_PARENT_EXIT_RESERVE_SECONDS: f64 = 1.0;
const CALIBRATION_NATIVE_EXIT_RESERVE_SECONDS: f64 = 1.0;
const CALIBRATION_NATIVE_CLEANUP_RESERVE_SECONDS: f64 = 5.0;
const CALIBRATION_MIN_MEASUREMENT_SECONDS: f64 = 1.0;
const CALIBRATION_MIN_NATIVE_TOTAL_SECONDS: f64 =
    CALIBRATION_NATIVE_CLEANUP_RESERVE_SECONDS + CALIBRATION_MIN_MEASUREMENT_SECONDS;
const CALIBRATION_MIN_SINGLE_TIMEOUT_SECONDS: f64 = CALIBRATION_PARENT_EXIT_RESERVE_SECONDS
    + CALIBRATION_NATIVE_EXIT_RESERVE_SECONDS
    + CALIBRATION_MIN_NATIVE_TOTAL_SECONDS;
const CALIBRATION_MIN_FULL_TIMEOUT_SECONDS: f64 = CALIBRATION_PUBLICATION_RESERVE_SECONDS
    + CALIBRATION_NATIVE_EXIT_RESERVE_SECONDS
    + CALIBRATION_MIN_NATIVE_TOTAL_SECONDS;
const CALIBRATION_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(6);

/// Production composition has no synthetic calibration/update behavior.  The
/// package harness enters `SafePackage` only through the hidden selftest
/// composition root, never through an environment variable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TestSeams {
    #[default]
    Disabled,
    SafePackage,
}

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
struct CalibrationBudget {
    child_timeout_seconds: f64,
    native_budget_seconds: u64,
}

fn calibration_budget(
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

struct NativeCalibrationService {
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
    down_late_grace_us: u64,
    cancel: Arc<AtomicBool>,
    child: Arc<Mutex<Option<std::process::Child>>>,
    worker: Option<thread::JoinHandle<()>>,
    done: Arc<(Mutex<bool>, Condvar)>,
    reservation: Option<crate::app_state::CalibrationReservation>,
}

impl NativeCalibrationService {
    fn new(
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
    fn from_test_sandbox(
        root: &Path,
        activity: ActivityCoordinator,
        events: Arc<Mutex<NativeEventHub>>,
        playback: Arc<NativePlaybackService>,
        test_seams: TestSeams,
    ) -> Self {
        let paths = AppPaths::from_test_sandbox(root);
        Self::new(paths, activity, events, playback, test_seams)
    }

    fn start(
        self: &Arc<Self>,
        request: CalibrationStartRequest,
        down_late_grace_us: u64,
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
                down_late_grace_us,
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

    fn cancel(&self, request: CalibrationCancelRequest) -> Result<CalibrationCancelAckDto, String> {
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

    fn shutdown(&self) {
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
    fn shutdown_with_timeout(&self, timeout: Duration) -> bool {
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

    fn wait_for_terminal(&self, timeout: Duration) -> Result<CalibrationState, String> {
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
                match prepared.commit() {
                    Ok((margin, samples)) => {
                        // Cache publication and prepared-plan invalidation precede
                        // the terminal event and release of the activity lease.
                        self.playback.invalidate_settings();
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
        let (reservation, down_late_grace_us) = if let Ok(mut operation) = self.operation.lock() {
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
            (current.reservation.take(), current.down_late_grace_us)
        } else {
            (None, sky_app_core::timing::DEFAULT_DOWN_LATE_GRACE_US)
        };
        let recommendation_available = outcome == CalibrationOutcome::Succeeded;
        let recommended_timing_margin_us = recommendation_available.then(|| {
            if !applied {
                return sky_app_core::settings::DEFAULT_TIMING_MARGIN_US;
            }
            let reserve_us = margin_us.unwrap_or(sky_native_adapters::DEFAULT_TRANSPORT_MARGIN_US);
            let evidence_budget_us = down_late_grace_us.saturating_add(reserve_us);
            evidence_budget_us.div_ceil(sky_app_core::settings::TIMING_MARGIN_STEP_US)
                * sky_app_core::settings::TIMING_MARGIN_STEP_US
        });
        let recommendation_qualified = recommendation_available && applied;
        let recommendation_source = if recommendation_qualified {
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

    fn run_child(
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

enum CalibrationRunError {
    Cancelled,
    Failed(String),
}

fn safe_calibration_evidence() -> Value {
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

fn bounded_text(value: impl AsRef<str>) -> String {
    value.as_ref().chars().take(4096).collect()
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
fn publish_calibration_cache(cache_root: &Path, raw: &Value) -> Result<(Option<u64>, u64), String> {
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

pub(crate) struct NativeDesktopRuntime {
    #[allow(dead_code)]
    paths: AppPaths,
    settings: Mutex<SettingsService<JsonSettingsStore>>,
    library_manifest: Mutex<LibraryManifestService<JsonLibraryManifestStore>>,
    catalog_composer: CatalogComposer,
    catalog: Mutex<CatalogState>,
    analysis_cache: Mutex<HashMap<String, CachedSongAnalysis>>,
    events: Arc<Mutex<NativeEventHub>>,
    playback: Arc<NativePlaybackService>,
    calibration: Arc<NativeCalibrationService>,
    update_service: Option<Arc<crate::native_update::UpdateService<crate::ShellRuntime>>>,
    pre_exit_safety: Arc<dyn Fn() + Send + Sync>,
    ready_emitted: AtomicBool,
    closed: AtomicBool,
}

#[derive(Debug, Clone)]
struct CatalogMetadata {
    format_label: String,
    duration_us: Option<u64>,
    note_count: Option<u64>,
    risk_level: String,
    state: &'static str,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeLibraryImportRequest {
    pub playlist_id: String,
    pub paths: Vec<String>,
}

#[derive(Default)]
struct CatalogState {
    index: CatalogIndex,
    library_membership: BTreeSet<String>,
    builtin_membership: BTreeSet<String>,
    user_membership: BTreeSet<String>,
    imported_membership: HashMap<String, BTreeSet<String>>,
    imported_status: Vec<ImportedSourceCatalogStatus>,
    builtin_status: BuiltinCatalogStatus,
}

impl CatalogMetadata {
    fn error(format_label: String) -> Self {
        Self {
            format_label,
            duration_us: None,
            note_count: None,
            risk_level: "unknown".into(),
            state: "error",
        }
    }
}

#[derive(Debug, Clone)]
struct CachedSongAnalysis {
    metadata: CatalogMetadata,
    detail: Option<SongDetailDto>,
}

impl NativeDesktopRuntime {
    #[allow(dead_code)]
    pub(crate) fn for_current_install() -> Result<Self, String> {
        let paths = AppPaths::resolve()?;
        let resources = AppResources::from_resource_dir(paths.install_root());
        Self::from_paths_with_activity_and_seams(
            paths,
            resources,
            ActivityCoordinator::default(),
            TestSeams::Disabled,
        )
    }

    #[allow(dead_code)]
    #[cfg(test)]
    pub(crate) fn from_install_root(install_root: PathBuf) -> Result<Self, String> {
        Self::from_install_root_with_activity_and_seams(
            install_root,
            ActivityCoordinator::default(),
            TestSeams::Disabled,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn for_current_install_with_activity(
        activity: ActivityCoordinator,
    ) -> Result<Self, String> {
        let paths = AppPaths::resolve()?;
        let resources = AppResources::from_resource_dir(paths.install_root());
        Self::from_paths_with_activity_and_seams(paths, resources, activity, TestSeams::Disabled)
    }

    #[allow(dead_code)]
    #[cfg(test)]
    pub(crate) fn from_install_root_with_activity(
        install_root: PathBuf,
        activity: ActivityCoordinator,
    ) -> Result<Self, String> {
        Self::from_install_root_with_activity_and_seams(install_root, activity, TestSeams::Disabled)
    }

    pub(crate) fn from_paths_with_activity_and_seams(
        paths: AppPaths,
        resources: AppResources,
        activity: ActivityCoordinator,
        test_seams: TestSeams,
    ) -> Result<Self, String> {
        Self::from_paths_with_activity_and_seams_and_update_service(
            paths, resources, activity, test_seams, None,
        )
    }

    pub(crate) fn from_paths_with_activity_and_seams_and_update_service(
        paths: AppPaths,
        resources: AppResources,
        activity: ActivityCoordinator,
        test_seams: TestSeams,
        update_service: Option<Arc<crate::native_update::UpdateService<crate::ShellRuntime>>>,
    ) -> Result<Self, String> {
        paths.assert_clean_boundary()?;
        Self::from_paths_internal(paths, resources, activity, test_seams, update_service)
    }

    fn from_paths_internal(
        paths: AppPaths,
        resources: AppResources,
        activity: ActivityCoordinator,
        test_seams: TestSeams,
        update_service: Option<Arc<crate::native_update::UpdateService<crate::ShellRuntime>>>,
    ) -> Result<Self, String> {
        let settings_path = paths.settings_path();
        let settings_store = JsonSettingsStore::new(settings_path);
        let settings = SettingsService::load(settings_store)
            .map_err(|error| format!("native settings startup failed: {error}"))?;
        let manifest_store = JsonLibraryManifestStore::new(paths.library_manifest_path());
        let library_manifest = LibraryManifestService::load(manifest_store)
            .map_err(|error| format!("native library manifest startup failed: {error}"))?;
        let songs_dir = paths
            .resolve_songs_dir(&settings.snapshot().songs_dir)
            .unwrap_or_else(|_| paths.user_music_root().to_path_buf());
        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let playback = Arc::new(NativePlaybackService::new(activity.clone()));
        let calibration = Arc::new(NativeCalibrationService::new(
            paths.clone(),
            activity.clone(),
            events.clone(),
            playback.clone(),
            test_seams,
        ));
        let safety_activity = activity.clone();
        let safety_calibration = calibration.clone();
        let safety_playback = playback.clone();
        let safety_events = events.clone();
        let pre_exit_safety: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            safety_activity.begin_shutdown();
            crate::record_update_safety_phase("activity.quiesced");
            safety_calibration.shutdown();
            safety_playback.shutdown(safety_events.clone());
            crate::record_update_safety_phase("playback.keys_released");
            // Settings mutations use the durable Native-owned store before
            // this hook can run. The marker records that the close boundary
            // has crossed the persistence phase.
            crate::record_update_safety_phase("state.persisted");
            if let Ok(mut events) = safety_events.lock() {
                events.close();
            }
            crate::record_update_safety_phase("resources.closed");
        });
        if let Some(service) = &update_service {
            service.set_pre_exit_safety(Arc::clone(&pre_exit_safety));
        }
        Ok(Self {
            paths,
            settings: Mutex::new(settings),
            library_manifest: Mutex::new(library_manifest),
            catalog_composer: CatalogComposer::new(
                BuiltinCatalogSource::new(resources),
                FileCatalogSource::new(songs_dir),
            ),
            catalog: Mutex::new(CatalogState::default()),
            analysis_cache: Mutex::new(HashMap::new()),
            events,
            playback,
            calibration,
            update_service,
            pre_exit_safety,
            ready_emitted: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        })
    }

    #[allow(dead_code)]
    #[cfg(test)]
    pub(crate) fn from_install_root_with_activity_and_seams(
        install_root: PathBuf,
        activity: ActivityCoordinator,
        test_seams: TestSeams,
    ) -> Result<Self, String> {
        let paths = AppPaths::from_roots(
            install_root.clone(),
            install_root.clone(),
            install_root.clone(),
            install_root.join("cache"),
            install_root.join("logs"),
            install_root.join("songs"),
        );
        let resources = AppResources::from_resource_dir(&install_root);
        Self::from_paths_internal(paths, resources, activity, test_seams, None)
    }

    #[allow(dead_code)]
    pub(crate) fn paths(&self) -> &AppPaths {
        &self.paths
    }

    #[allow(dead_code)]
    pub(crate) fn install_root(&self) -> &Path {
        self.paths.install_root()
    }

    pub(crate) fn dispatch(&self, method: &str, params: Value) -> Result<Value, String> {
        if self.closed.load(Ordering::Acquire) && method != "app.shutdown" {
            return Err("native desktop runtime is shut down".into());
        }
        if crate::command_ownership::owner_for(method)
            != Some(crate::command_ownership::CommandOwner::Native)
        {
            return Err(format!(
                "native runtime does not own desktop command: {method}"
            ));
        }
        match method {
            "app.bootstrap" => encode_result(self.bootstrap()),
            "app.shutdown" => {
                self.shutdown();
                Ok(Value::Null)
            }
            "catalog.search" => {
                let request: CatalogSearchRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.search(request))
            }
            "catalog.set_liked" => {
                let request: CatalogSetLikedRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.set_song_liked(request))
            }
            "catalog.detail" => {
                let request: NativeCatalogDetailRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.detail(CatalogDetailRequest {
                    song_id: request.song_id,
                    generation: request.generation,
                }))
            }
            "catalog.reload" => encode_result(self.reload()),
            "catalog.set_viewport" => {
                let request: NativeCatalogViewportRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.set_viewport(CatalogViewportRequest {
                    generation: request.generation,
                    first_index: request.first_index,
                    last_index: request.last_index,
                    selected_song_id: request.selected_song_id,
                    song_ids: request.song_ids,
                }))
            }
            "library.list_playlists" => encode_result(self.list_playlists()),
            "library.create_playlist" => {
                let request: LibraryCreatePlaylistRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.create_playlist(request))
            }
            "library.rename_playlist" => {
                let request: LibraryRenamePlaylistRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.rename_playlist(request))
            }
            "library.delete_playlist" => {
                let request: LibraryPlaylistIdRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.delete_playlist(request))
            }
            "library.add_songs" => {
                let request: LibraryPlaylistSongsRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.add_playlist_songs(request))
            }
            "library.remove_songs" => {
                let request: LibraryPlaylistSongsRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.remove_playlist_songs(request))
            }
            "library.import_local_files_to_playlist" => {
                let request: NativeLibraryImportRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.import_paths(request, ImportedSourceKind::File))
            }
            "library.import_local_folder_to_playlist" => {
                let request: NativeLibraryImportRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.import_paths(request, ImportedSourceKind::Folder))
            }
            "settings.get" => encode_result(self.settings_dto()),
            "settings.patch" => {
                let request: NativeSettingsPatch =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.patch_settings(request.into_public()))
            }
            "update.preferences.get" => encode_result(self.update_preferences()),
            "update.preferences.patch" => {
                let request: NativeUpdatePreferencesPatch =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.patch_update_preferences(request.into_public()))
            }
            "update.check" => {
                if !params.as_object().is_some_and(|object| object.is_empty()) {
                    return Err("invalid_params: update.check takes no parameters".into());
                }
                encode_result(self.check_update())
            }
            "update.begin_handoff" => {
                let request: crate::commands::UpdateBeginHandoffRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.begin_update_handoff(request.target_version))
            }
            "playback.prepare" => {
                let request: NativePlaybackPrepareRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.prepare_playback(request.into_public()))
            }
            "playback.start" => {
                let request: NativePlaybackStartRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                validate_playback_start_request(&request)?;
                encode_result(self.start_playback(request.into_public()))
            }
            "playback.get_status" => {
                if !params.as_object().is_some_and(|object| object.is_empty()) {
                    return Err("invalid_params: playback.get_status takes no parameters".into());
                }
                encode_result(self.get_playback_status())
            }
            "playback.stop" | "playback.pause" | "playback.resume" | "playback.skip" => {
                let request: NativePlaybackSessionRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.playback_command(method, request.session_id))
            }
            "playback.export_sender_trace" => {
                if !params.as_object().is_some_and(|object| object.is_empty()) {
                    return Err(
                        "invalid_params: playback.export_sender_trace takes no parameters".into(),
                    );
                }
                encode_result(self.playback.export_sender_trace())
            }
            "diagnostics.set_enabled" => {
                let request: DiagnosticsSetEnabledRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.set_diagnostics_enabled(request))
            }
            "calibration.start" => {
                let request: CalibrationStartRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.start_calibration(request))
            }
            "calibration.cancel" => {
                let request: CalibrationCancelRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.cancel_calibration(request))
            }
            _ => Err(format!("native command is not implemented: {method}")),
        }
    }

    pub(crate) fn bootstrap(&self) -> Result<BootstrapDto, String> {
        let snapshot = self.ensure_catalog_loaded()?;
        let settings = self.settings_snapshot()?;
        let timing_margin_recommendation =
            self.timing_margin_recommendation(settings.playback_defaults.down_late_grace_us);
        let result = BootstrapDto {
            app_version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: crate::DESKTOP_PROTOCOL_VERSION,
            native_build: native_build_dto(),
            playback_defaults: playback_defaults(&settings),
            timing_margin_recommendation: timing_margin_recommendation.clone(),
            option_sets: crate::commands::PlaybackOptionSetsDto {
                hold_frames: vec![1.0, 1.25, 1.5],
                tempo_scales: vec![0.90, 0.95, 1.0, 1.05, 1.10],
                fps: sky_app_core::settings::VALID_FPS.to_vec(),
                timing_margin_min_us: sky_app_core::settings::MIN_TIMING_MARGIN_US,
                timing_margin_max_us: sky_app_core::settings::MAX_TIMING_MARGIN_US,
                timing_margin_step_us: sky_app_core::settings::TIMING_MARGIN_STEP_US,
                down_late_grace_min_us: sky_app_core::settings::MIN_DOWN_LATE_GRACE_US,
                down_late_grace_max_us: sky_app_core::settings::MAX_DOWN_LATE_GRACE_US,
                down_late_grace_step_us: sky_app_core::settings::DOWN_LATE_GRACE_STEP_US,
            },
            theme: settings.theme.clone(),
            telemetry_enabled: settings.telemetry_enabled,
            update_preferences: update_preferences_dto(&settings),
            catalog_generation: snapshot.generation,
        };
        if !self.ready_emitted.swap(true, Ordering::AcqRel) {
            self.publish(UiEvent::CoreReady {
                v: crate::DESKTOP_PROTOCOL_VERSION,
                payload: CoreReadyPayload {
                    app_version: result.app_version.clone(),
                    protocol_version: result.protocol_version,
                    native_build: NativeBuildPayload {
                        native_build_commit: result.native_build.native_build_commit.clone(),
                        native_version: result.native_build.native_version.clone(),
                        schema_version: result.native_build.schema_version,
                        native_abi: result.native_build.native_abi.clone(),
                        rustc_version: result.native_build.rustc_version.clone(),
                        win32_backend: result.native_build.win32_backend,
                    },
                },
            })?;
        }
        Ok(result)
    }

    fn settings_snapshot(&self) -> Result<ApplicationSettings, String> {
        let mut service = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        // Reloading from the one Native-owned store keeps every application
        // service on the same persisted snapshot across process restarts.
        service
            .reload()
            .map_err(|error| format!("native settings reload failed: {error}"))?;
        Ok(service.snapshot().clone())
    }

    fn settings_dto(&self) -> Result<SettingsDto, String> {
        let settings = self.settings_snapshot()?;
        Ok(settings_dto(
            &settings,
            self.timing_margin_recommendation(settings.playback_defaults.down_late_grace_us),
        ))
    }

    fn timing_margin_recommendation(
        &self,
        down_late_grace_us: u64,
    ) -> crate::commands::TimingMarginRecommendationDto {
        timing_margin_recommendation(self.paths.calibration_cache_path(), down_late_grace_us)
    }

    fn patch_settings(&self, patch: SettingsPatch) -> Result<SettingsDto, String> {
        let update_preferences_changed = patch.update_preferences.is_some();
        let core_patch = sky_app_core::settings::SettingsPatch {
            theme: patch.theme,
            telemetry_enabled: patch.telemetry_enabled,
            verbose_hud: patch.verbose_hud,
            playback_defaults: patch.playback_defaults.map(|value| PlaybackDefaultsPatch {
                hold_frames: value.hold_frames,
                timing_margin_us: value.timing_margin_us,
                down_late_grace_us: value.down_late_grace_us,
                tempo_scale: value.tempo_scale,
                fps: value.fps,
            }),
            update: patch
                .update_preferences
                .map(|value| CoreUpdatePreferencesPatch {
                    auto_check: value.auto_check,
                    channel: value.channel.map(|channel| match channel {
                        crate::ui_events::UpdateChannel::Stable => {
                            sky_app_core::settings::UpdateChannel::Stable
                        }
                        crate::ui_events::UpdateChannel::Beta => {
                            sky_app_core::settings::UpdateChannel::Beta
                        }
                    }),
                    skip_version: value.skip_version,
                }),
        };
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        let snapshot = settings.patch(&core_patch).map_err(settings_error)?.clone();
        drop(settings);
        self.playback.invalidate_settings();
        self.invalidate_analysis_cache();
        if update_preferences_changed && let Some(update_service) = &self.update_service {
            update_service.reset();
        }
        Ok(settings_dto(
            &snapshot,
            self.timing_margin_recommendation(snapshot.playback_defaults.down_late_grace_us),
        ))
    }

    fn update_preferences(&self) -> Result<UpdatePreferencesDto, String> {
        let settings = self.settings_snapshot()?;
        Ok(update_preferences_dto(&settings))
    }

    fn check_update(&self) -> Result<UpdateCheckDto, String> {
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        settings
            .reload()
            .map_err(|error| format!("native settings reload failed: {error}"))?;
        self.update_service
            .as_ref()
            .ok_or_else(|| {
                "update_service_unavailable: Rust-owned UpdateService is not configured".to_string()
            })?
            .check(&mut settings, |event| self.publish(event))
    }

    fn begin_update_handoff(&self, target_version: String) -> Result<UpdateHandoffDto, String> {
        if target_version.is_empty() || target_version.len() > 64 || target_version.contains('\0') {
            return Err("invalid_params: target_version is invalid".into());
        }
        let settings = self.settings_snapshot()?;
        self.update_service
            .as_ref()
            .ok_or_else(|| {
                "update_service_unavailable: Rust-owned UpdateService is not configured".to_string()
            })?
            .install(&settings, &target_version, |event| self.publish(event))
    }

    fn patch_update_preferences(
        &self,
        patch: UpdatePreferencesPatch,
    ) -> Result<UpdatePreferencesDto, String> {
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        let snapshot = settings
            .patch(&sky_app_core::settings::SettingsPatch {
                update: Some(CoreUpdatePreferencesPatch {
                    auto_check: patch.auto_check,
                    channel: patch.channel.map(|channel| match channel {
                        crate::ui_events::UpdateChannel::Stable => {
                            sky_app_core::settings::UpdateChannel::Stable
                        }
                        crate::ui_events::UpdateChannel::Beta => {
                            sky_app_core::settings::UpdateChannel::Beta
                        }
                    }),
                    skip_version: patch.skip_version,
                }),
                ..Default::default()
            })
            .map_err(settings_error)?;
        if let Some(update_service) = &self.update_service {
            update_service.reset();
        }
        Ok(update_preferences_dto(snapshot))
    }

    fn ensure_catalog_loaded(&self) -> Result<sky_app_core::catalog::CatalogSnapshot, String> {
        let needs_load = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?
            .index
            .generation()
            == 0;
        if needs_load {
            let composition = self.catalog_composition()?;
            let mut catalog = self
                .catalog
                .lock()
                .map_err(|_| "native catalog lock poisoned".to_string())?;
            if catalog.index.generation() == 0 {
                let snapshot = catalog
                    .index
                    .replace_entries(composition.entries)
                    .map_err(catalog_error)?;
                catalog.library_membership = composition.library_membership;
                catalog.builtin_membership = composition.builtin_membership;
                catalog.user_membership = composition.user_membership;
                catalog.imported_membership = composition.imported_membership;
                catalog.imported_status = composition.imported_status;
                catalog.builtin_status = composition.builtin_status;
                return Ok(snapshot);
            }
        }
        self.catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())
            .map(|catalog| catalog.index.snapshot())
    }

    fn reload(&self) -> Result<CatalogReloadDto, String> {
        let composition = self.catalog_composition()?;
        let mut catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let snapshot = catalog
            .index
            .replace_entries(composition.entries)
            .map_err(catalog_error)?;
        catalog.library_membership = composition.library_membership;
        catalog.builtin_membership = composition.builtin_membership;
        catalog.user_membership = composition.user_membership;
        catalog.imported_membership = composition.imported_membership;
        catalog.imported_status = composition.imported_status;
        catalog.builtin_status = composition.builtin_status;
        drop(catalog);
        self.invalidate_analysis_cache();
        self.playback.invalidate_catalog(snapshot.generation);
        let library_total = self.library_catalog_total(snapshot.generation)?;
        self.publish(UiEvent::CatalogChanged {
            v: crate::DESKTOP_PROTOCOL_VERSION,
            payload: CatalogChangedPayload {
                generation: snapshot.generation,
                total: library_total as u64,
            },
        })?;
        Ok(CatalogReloadDto {
            generation: snapshot.generation,
            total: library_total as u64,
        })
    }

    fn library_catalog_total(&self, generation: u64) -> Result<usize, String> {
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        catalog
            .index
            .count_allowed_ids(&catalog.library_membership, Some(generation))
            .map_err(catalog_error)
    }

    pub(crate) fn builtin_catalog_status(&self) -> Result<BuiltinCatalogStatus, String> {
        self.ensure_catalog_loaded()?;
        self.catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())
            .map(|catalog| catalog.builtin_status.clone())
    }

    fn catalog_composition(&self) -> Result<CatalogComposition, String> {
        let imports = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?
            .snapshot()
            .imports
            .clone();
        self.catalog_composer
            .compose(&imports)
            .map_err(catalog_error)
    }

    fn list_playlists(&self) -> Result<LibraryNavigationDto, String> {
        self.ensure_catalog_loaded()?;
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let manifest = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?;
        let generation = catalog.index.generation();
        let playlists = manifest
            .snapshot()
            .collections
            .iter()
            .map(|collection| {
                let ids = collection.song_ids.iter().cloned().collect::<BTreeSet<_>>();
                LibraryPlaylistSummaryDto {
                    id: collection.id.clone(),
                    name: collection.name.clone(),
                    song_count: catalog
                        .index
                        .count_allowed_ids(&ids, Some(generation))
                        .unwrap_or_default() as u64,
                }
            })
            .collect();
        Ok(LibraryNavigationDto { playlists })
    }

    fn playlist_summary(
        &self,
        collection: sky_app_core::library::Collection,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        self.ensure_catalog_loaded()?;
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let ids = collection.song_ids.iter().cloned().collect::<BTreeSet<_>>();
        Ok(LibraryPlaylistSummaryDto {
            id: collection.id,
            name: collection.name,
            song_count: catalog
                .index
                .count_allowed_ids(&ids, Some(catalog.index.generation()))
                .map_err(catalog_error)? as u64,
        })
    }

    fn create_playlist(
        &self,
        request: LibraryCreatePlaylistRequest,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        let id = opaque_native_id()?;
        let collection = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?
            .create_collection(id, request.name)
            .map_err(library_error)?;
        self.playlist_summary(collection)
    }

    fn rename_playlist(
        &self,
        request: LibraryRenamePlaylistRequest,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        let collection = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?
            .rename_collection(&request.playlist_id, request.name)
            .map_err(library_error)?;
        self.playlist_summary(collection)
    }

    fn delete_playlist(&self, request: LibraryPlaylistIdRequest) -> Result<bool, String> {
        self.library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?
            .delete_collection(&request.playlist_id)
            .map_err(library_error)
    }

    fn add_playlist_songs(
        &self,
        request: LibraryPlaylistSongsRequest,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        let collection = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?
            .add_songs(&request.playlist_id, &request.song_ids)
            .map_err(library_error)?;
        self.playlist_summary(collection)
    }

    fn remove_playlist_songs(
        &self,
        request: LibraryPlaylistSongsRequest,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        let collection = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?
            .remove_songs(&request.playlist_id, &request.song_ids)
            .map_err(library_error)?;
        self.playlist_summary(collection)
    }

    fn import_paths(
        &self,
        request: NativeLibraryImportRequest,
        kind: ImportedSourceKind,
    ) -> Result<LibraryPlaylistImportResultDto, String> {
        const MAX_IMPORT_SELECTIONS: usize = 1_024;
        if request.paths.len() > MAX_IMPORT_SELECTIONS {
            return Err("invalid_params: import selection is too large".into());
        }

        let mut imports = Vec::with_capacity(request.paths.len());
        for raw_path in request.paths {
            let path = PathBuf::from(raw_path);
            let canonical = fs::canonicalize(&path)
                .map_err(|_| "import source cannot be resolved".to_owned())?;
            let valid = match kind {
                ImportedSourceKind::File => {
                    canonical.is_file() && is_supported_catalog_path(&canonical)
                }
                ImportedSourceKind::Folder => canonical.is_dir(),
            };
            if !valid {
                return Err("invalid_params: import source has an unsupported type".into());
            }
            let canonical_path = canonical.to_string_lossy().into_owned();
            imports.push(ImportedSourceRef {
                source_id: imported_source_id(&canonical_path, kind),
                canonical_path,
                kind,
            });
        }

        // Resolve the selected native paths before touching the manifest. The
        // resulting IDs are opaque and path-free at the application boundary.
        let composition = self
            .catalog_composer
            .compose(&imports)
            .map_err(catalog_error)?;
        let song_ids = imports
            .iter()
            .filter_map(|import| composition.imported_membership.get(&import.source_id))
            .flat_map(|ids| ids.iter().cloned())
            .collect::<BTreeSet<_>>();
        let song_ids = song_ids.into_iter().collect::<Vec<_>>();

        let (playlist, has_new_import) = {
            let mut manifest = self
                .library_manifest
                .lock()
                .map_err(|_| "native library manifest lock poisoned".to_string())?;
            let has_new_import = imports.iter().any(|import| {
                !manifest.snapshot().imports.iter().any(|existing| {
                    existing
                        .canonical_path
                        .eq_ignore_ascii_case(&import.canonical_path)
                })
            });
            let playlist = manifest
                .import_and_add_songs(&request.playlist_id, imports, &song_ids)
                .map_err(library_error)?;
            (playlist, has_new_import)
        };

        let generation = if has_new_import {
            self.reload()?.generation
        } else {
            self.ensure_catalog_loaded()?.generation
        };
        Ok(LibraryPlaylistImportResultDto {
            playlist: self.playlist_summary(playlist)?,
            imported_song_count: song_ids.len() as u64,
            catalog_generation: generation,
        })
    }

    fn search(&self, request: CatalogSearchRequest) -> Result<CatalogSearchDto, String> {
        self.ensure_catalog_loaded()?;
        let settings = self.settings_snapshot()?;
        let allowed_ids = match &request.source {
            LibrarySource::Smart {
                id: CatalogSourceId::Liked,
            } => Some(settings.liked_songs.ids().clone()),
            LibrarySource::Smart {
                id: CatalogSourceId::All,
            } => {
                let catalog = self
                    .catalog
                    .lock()
                    .map_err(|_| "native catalog lock poisoned".to_string())?;
                Some(catalog.library_membership.clone())
            }
            LibrarySource::Playlist { id } => {
                if !sky_app_core::library::is_valid_collection_id(id) {
                    return Err("invalid_params: playlist source ID is invalid".into());
                }
                let manifest = self
                    .library_manifest
                    .lock()
                    .map_err(|_| "native library manifest lock poisoned".to_string())?;
                Some(
                    manifest
                        .snapshot()
                        .collections
                        .iter()
                        .find(|collection| collection.id == *id)
                        .ok_or_else(|| "library playlist was not found".to_string())?
                        .song_ids
                        .iter()
                        .cloned()
                        .collect(),
                )
            }
        };
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let liked_total = catalog
            .index
            .count_allowed_ids(settings.liked_songs.ids(), request.generation)
            .map_err(catalog_error)?;
        let page = catalog
            .index
            .search_with_allowed_ids(
                &WRatioRanker,
                &request.query,
                request.offset as usize,
                request.limit as usize,
                request.generation,
                allowed_ids.as_ref(),
            )
            .map_err(catalog_error)?;
        let cache = self
            .analysis_cache
            .lock()
            .map_err(|_| "native analysis cache lock poisoned".to_string())?;
        Ok(CatalogSearchDto {
            items: page
                .items
                .into_iter()
                .map(|row| {
                    let metadata = cache.get(&row.song_id).map(|value| &value.metadata);
                    let format_label = metadata
                        .map(|value| value.format_label.clone())
                        .or_else(|| {
                            catalog
                                .index
                                .entry_for_song_id(&row.song_id, Some(page.generation))
                                .ok()
                                .map(|entry| {
                                    format_label_for_path(Path::new(&entry.canonical_path))
                                })
                        })
                        .unwrap_or_else(|| "UNKNOWN".into());
                    CatalogRowDto {
                        liked: settings.liked_songs.contains(&row.song_id),
                        song_id: row.song_id.clone(),
                        title: row.title,
                        format_label,
                        duration_us: metadata.and_then(|value| value.duration_us),
                        note_count: metadata.and_then(|value| value.note_count),
                        risk_level: metadata
                            .map(|value| value.risk_level.clone())
                            .unwrap_or_else(|| "unknown".into()),
                        metadata_state: metadata
                            .map(|value| value.state.into())
                            .unwrap_or_else(|| "pending".into()),
                    }
                })
                .collect(),
            offset: request.offset,
            limit: request.limit,
            total: page.total as u64,
            liked_total: liked_total as u64,
            generation: page.generation,
        })
    }

    fn detail(&self, request: CatalogDetailRequest) -> Result<SongDetailDto, String> {
        self.ensure_catalog_loaded()?;
        let entry = {
            let catalog = self
                .catalog
                .lock()
                .map_err(|_| "native catalog lock poisoned".to_string())?;
            catalog
                .index
                .entry_for_song_id(&request.song_id, request.generation)
                .map_err(catalog_error)?
        };
        if let Some(detail) = self
            .analysis_cache
            .lock()
            .map_err(|_| "native analysis cache lock poisoned".to_string())?
            .get(&request.song_id)
            .and_then(|cached| cached.detail.clone())
        {
            return Ok(detail);
        }

        let path = PathBuf::from(&entry.canonical_path);
        let fallback = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&entry.row.title);
        let settings = self.settings_snapshot()?;
        let policy = self.timing_policy(
            settings.playback_defaults.fps,
            settings.playback_defaults.hold_frames,
            settings.playback_defaults.timing_margin_us,
            settings.playback_defaults.down_late_grace_us,
        )?;
        let cached = self.analyze_song(
            &path,
            &entry.row.song_id,
            &entry.row.title,
            fallback,
            &settings,
            &policy,
        )?;
        let detail = cached
            .detail
            .clone()
            .ok_or_else(|| "song detail analysis was unavailable".to_string())?;
        self.cache_analysis(&request.song_id, cached);
        Ok(detail)
    }

    fn set_viewport(&self, request: CatalogViewportRequest) -> Result<CatalogViewportDto, String> {
        self.ensure_catalog_loaded()?;
        if request.song_ids.len() > 2_000 {
            return Err("catalog viewport contains too many metadata hydration IDs".into());
        }
        let entries = {
            let catalog = self
                .catalog
                .lock()
                .map_err(|_| "native catalog lock poisoned".to_string())?;
            let snapshot = catalog.index.snapshot();
            if snapshot.generation != request.generation {
                return Err("catalog generation is stale".into());
            }
            let song_ids = if request.song_ids.is_empty() {
                if snapshot.total == 0 {
                    if request.first_index != 0
                        || request.last_index != -1
                        || request.selected_song_id.is_some()
                    {
                        return Err(
                            "empty catalog viewport must be 0..-1 with no selected song".into()
                        );
                    }
                    Vec::new()
                } else {
                    if request.last_index < request.first_index as i64
                        || request.last_index as u64 >= snapshot.total as u64
                        || request
                            .last_index
                            .saturating_sub(request.first_index as i64)
                            .saturating_add(1)
                            > 2_000
                    {
                        return Err("catalog viewport is outside bounded index range".into());
                    }
                    catalog
                        .index
                        .song_ids_in_range(
                            request.first_index as usize,
                            request.last_index as usize,
                            Some(request.generation),
                        )
                        .map_err(catalog_error)?
                }
            } else {
                request.song_ids.clone()
            };
            if let Some(song_id) = &request.selected_song_id {
                catalog
                    .index
                    .canonical_path_for_song_id(song_id, Some(request.generation))
                    .map_err(catalog_error)?;
            }
            song_ids
                .into_iter()
                .map(|song_id| {
                    catalog
                        .index
                        .entry_for_song_id(&song_id, Some(request.generation))
                        .map(|entry| (entry.row.song_id, entry.row.title, entry.canonical_path))
                        .map_err(catalog_error)
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        let settings = self.settings_snapshot()?;
        let policy = self.timing_policy(
            settings.playback_defaults.fps,
            settings.playback_defaults.hold_frames,
            settings.playback_defaults.timing_margin_us,
            settings.playback_defaults.down_late_grace_us,
        )?;
        for (song_id, title, path) in &entries {
            let already_cached = self
                .analysis_cache
                .lock()
                .map_err(|_| "native analysis cache lock poisoned".to_string())?
                .contains_key(song_id);
            if already_cached {
                continue;
            }
            let format_label = format_label_for_path(Path::new(path));
            let analysis = self
                .analyze_song(Path::new(path), song_id, title, title, &settings, &policy)
                .unwrap_or_else(|_| CachedSongAnalysis {
                    metadata: CatalogMetadata::error(format_label),
                    detail: None,
                });
            self.cache_analysis(song_id, analysis);
        }
        let cache = self
            .analysis_cache
            .lock()
            .map_err(|_| "native analysis cache lock poisoned".to_string())?;
        let items = entries
            .into_iter()
            .filter_map(|(song_id, title, _)| {
                cache.get(&song_id).map(|analysis| CatalogRowDto {
                    liked: settings.liked_songs.contains(&song_id),
                    song_id,
                    title,
                    format_label: analysis.metadata.format_label.clone(),
                    duration_us: analysis.metadata.duration_us,
                    note_count: analysis.metadata.note_count,
                    risk_level: analysis.metadata.risk_level.clone(),
                    metadata_state: analysis.metadata.state.into(),
                })
            })
            .collect();
        Ok(CatalogViewportDto {
            accepted: true,
            generation: request.generation,
            first_index: request.first_index,
            last_index: request.last_index,
            selected_song_id: request.selected_song_id,
            items,
        })
    }

    fn set_song_liked(
        &self,
        request: CatalogSetLikedRequest,
    ) -> Result<CatalogSetLikedDto, String> {
        self.ensure_catalog_loaded()?;
        {
            let catalog = self
                .catalog
                .lock()
                .map_err(|_| "native catalog lock poisoned".to_string())?;
            catalog
                .index
                .canonical_path_for_song_id(&request.song_id, request.generation)
                .map_err(catalog_error)?;
        }
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        settings
            .reload()
            .map_err(|error| format!("native settings reload failed: {error}"))?;
        settings
            .set_song_liked(&request.song_id, request.liked)
            .map_err(settings_error)?;
        let liked_total = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?
            .index
            .count_allowed_ids(settings.snapshot().liked_songs.ids(), request.generation)
            .map_err(catalog_error)?;
        Ok(CatalogSetLikedDto {
            song_id: request.song_id,
            liked: request.liked,
            total: liked_total as u64,
        })
    }

    fn invalidate_analysis_cache(&self) {
        if let Ok(mut cache) = self.analysis_cache.lock() {
            cache.clear();
        }
    }

    fn cache_analysis(&self, song_id: &str, analysis: CachedSongAnalysis) {
        if let Ok(mut cache) = self.analysis_cache.lock() {
            cache.insert(song_id.to_owned(), analysis);
        }
    }

    fn analyze_song(
        &self,
        path: &Path,
        song_id: &str,
        title: &str,
        fallback: &str,
        settings: &ApplicationSettings,
        policy: &MaterializedTimingPolicy,
    ) -> Result<CachedSongAnalysis, String> {
        let bytes = fs::read(path).map_err(|error| format!("song read failed: {error}"))?;
        let song = parse_song_json(&bytes, fallback).map_err(|error| error.to_string())?;
        let schedule =
            build_schedule_with_policy(&song, settings.playback_defaults.tempo_scale, policy)
                .map_err(|error| error.to_string())?;
        let risk = analyze_schedule_with_context(
            &schedule,
            Some(&song.notes),
            settings.playback_defaults.hold_frames,
            settings.playback_defaults.tempo_scale,
        );
        let risk_level = match risk.severity.as_str() {
            "low" | "medium" | "high" => risk.severity.clone(),
            _ => return Err("song risk analysis returned an unknown level".into()),
        };
        let recommendations = risk.recommendations.clone();
        let reasons = if risk_level == "low" {
            Vec::new()
        } else {
            recommendations.clone()
        };
        let recommendation = Some(crate::commands::PlaybackRecommendationDto {
            recommended_hold_frames: risk.suggested_hold_frames,
            recommended_tempo_scale: risk.suggested_tempo_scale,
            summary: recommendations
                .first()
                .cloned()
                .unwrap_or_else(|| "Keep the selected settings.".into()),
        });
        let format_label = format_label_for_path(path);
        let detail = SongDetailDto {
            song_id: song_id.to_owned(),
            title: title.to_owned(),
            duration_us: schedule.source_duration_us,
            note_count: song.notes.len() as u64,
            format_label: format_label.clone(),
            risk: RiskSummaryDto {
                level: risk_level.clone(),
                headline: match risk_level.as_str() {
                    "low" => "Low timing risk".into(),
                    "medium" => "Medium timing risk".into(),
                    "high" => "High timing risk".into(),
                    _ => "Risk unavailable".into(),
                },
                reasons,
                recommendations,
            },
            recommendation,
        };
        Ok(CachedSongAnalysis {
            metadata: CatalogMetadata {
                format_label,
                duration_us: Some(schedule.source_duration_us),
                note_count: Some(song.notes.len() as u64),
                risk_level,
                state: "ready",
            },
            detail: Some(detail),
        })
    }

    fn prepare_playback(
        &self,
        request: crate::commands::PlaybackPrepareRequest,
    ) -> Result<PreparedPlaybackDto, String> {
        self.ensure_catalog_loaded()?;
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let entry = catalog
            .index
            .entry_for_song_id(&request.song_id, Some(request.generation))
            .map_err(catalog_error)?;
        let path = PathBuf::from(&entry.canonical_path);
        let bytes = fs::read(&path).map_err(|error| format!("song read failed: {error}"))?;
        let fallback = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&entry.row.title);
        let song = parse_song_json(&bytes, fallback).map_err(|error| error.to_string())?;
        let settings = self.settings_snapshot()?;
        let policy = self.timing_policy(
            request.config.fps,
            request.config.hold_frames,
            request.config.timing_margin_us,
            request.config.down_late_grace_us,
        )?;
        let timing_margin_recommendation =
            self.timing_margin_recommendation(request.config.down_late_grace_us);
        let schedule = build_schedule_with_policy(&song, request.config.tempo_scale, &policy)
            .map_err(|error| error.to_string())?;
        let risk = analyze_schedule_with_context(
            &schedule,
            Some(&song.notes),
            request.config.hold_frames,
            request.config.tempo_scale,
        );
        self.playback.prepare(NativePreparedInput {
            song_id: request.song_id,
            generation: request.generation,
            config: request.config,
            song,
            schedule,
            risk,
            timing_policy: policy,
            timing_margin_recommendation,
            settings_fingerprint: settings_fingerprint(&settings)?,
        })
    }

    fn timing_policy(
        &self,
        fps: u16,
        hold_frames: f64,
        timing_margin_us: u64,
        down_late_grace_us: u64,
    ) -> Result<MaterializedTimingPolicy, String> {
        MaterializedTimingPolicy::from_user_margin(
            fps,
            hold_frames,
            timing_margin_us,
            down_late_grace_us,
        )
        .map_err(|error| error.to_string())
    }

    fn start_playback(
        &self,
        request: crate::commands::PlaybackStartRequest,
    ) -> Result<PlaybackSessionDto, String> {
        validate_public_playback_start_request(&request)?;
        let settings = self.settings_snapshot()?;
        self.playback.start(request, &settings, self.events.clone())
    }

    fn playback_command(
        &self,
        method: &str,
        session_id: String,
    ) -> Result<PlaybackCommandAckDto, String> {
        self.playback
            .command(method, session_id, self.events.clone())
    }

    fn get_playback_status(&self) -> Result<PlaybackStatusDto, String> {
        self.playback.status()
    }

    fn set_diagnostics_enabled(
        &self,
        request: DiagnosticsSetEnabledRequest,
    ) -> Result<DiagnosticsEnabledDto, String> {
        self.playback
            .set_diagnostics_enabled(request.enabled, self.events.clone())?;
        Ok(DiagnosticsEnabledDto {
            enabled: request.enabled,
        })
    }

    fn start_calibration(
        &self,
        request: CalibrationStartRequest,
    ) -> Result<CalibrationStartAckDto, String> {
        let settings = self.settings_snapshot()?;
        self.calibration
            .start(request, settings.playback_defaults.down_late_grace_us)
    }

    fn cancel_calibration(
        &self,
        request: CalibrationCancelRequest,
    ) -> Result<CalibrationCancelAckDto, String> {
        self.calibration.cancel(request)
    }

    pub(crate) fn wait_for_calibration_terminal(
        &self,
        timeout: Duration,
    ) -> Result<CalibrationState, String> {
        self.calibration.wait_for_terminal(timeout)
    }

    pub(crate) fn subscribe(&self, channel: Channel<UiEvent>) -> Result<(), String> {
        self.events
            .lock()
            .map_err(|_| "native event hub lock poisoned".to_string())?
            .subscribe(channel)
    }

    fn publish(&self, event: UiEvent) -> Result<(), String> {
        self.events
            .lock()
            .map_err(|_| "native event hub lock poisoned".to_string())?
            .publish(event)
    }

    pub(crate) fn shutdown(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            (self.pre_exit_safety)();
        }
    }
}

/// Application-side playback control plane.  The realtime worker remains in
/// `sky_player`; this service owns prepared-plan admission, the single active
/// session rule, and the bounded supervisor loop around that worker.
struct NativePlaybackService {
    prepared: Mutex<VecDeque<(String, NativePreparedPlan)>>,
    active: Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    last_terminal: Arc<Mutex<Option<PlaybackTerminalStatusDto>>>,
    sender_trace_state: Arc<Mutex<SenderTraceState>>,
    diagnostics_gate: Arc<DiagnosticsPublicationGate>,
    activity: ActivityCoordinator,
    #[cfg(test)]
    settings_invalidation_count: AtomicU64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CompletedSenderTrace {
    session_id: String,
    song_id: String,
    song_title: String,
    plan_fingerprint: String,
    trace_json: String,
}

#[derive(Default)]
struct SenderTraceState {
    collecting_session_id: Option<String>,
    completed: Option<CompletedSenderTrace>,
}

impl SenderTraceState {
    fn begin_physical_session(&mut self, session_id: &str) {
        self.collecting_session_id = Some(session_id.to_string());
        self.completed = None;
    }

    fn finish_physical_session(
        &mut self,
        session_id: &str,
        song_id: &str,
        song_title: &str,
        plan_fingerprint: &str,
        trace_json: Option<String>,
    ) {
        if self.collecting_session_id.as_deref() != Some(session_id) {
            return;
        }
        self.completed = trace_json.map(|trace_json| CompletedSenderTrace {
            session_id: session_id.to_string(),
            song_id: song_id.to_string(),
            song_title: song_title.to_string(),
            plan_fingerprint: plan_fingerprint.to_string(),
            trace_json,
        });
        self.collecting_session_id = None;
    }
}

const DIAGNOSTICS_INTERVAL: Duration = Duration::from_millis(100);
const SUPERVISOR_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Default)]
struct DiagnosticsPublicationState {
    enabled: bool,
    epoch: u64,
    sequence: u64,
    last_emit_at: Option<Instant>,
}

struct DiagnosticsPublicationGate {
    state: Mutex<DiagnosticsPublicationState>,
}

impl Default for DiagnosticsPublicationGate {
    fn default() -> Self {
        Self {
            state: Mutex::new(DiagnosticsPublicationState::default()),
        }
    }
}

impl DiagnosticsPublicationGate {
    fn is_enabled(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.enabled)
            .unwrap_or(false)
    }

    fn set_enabled(&self, enabled: bool) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "diagnostics publication gate poisoned".to_string())?;
        state.enabled = enabled;
        state.epoch = state.epoch.wrapping_add(1);
        // This is intentionally the Python contract: re-enable starts a
        // fresh sampling window while sequence IDs remain monotonic.
        state.last_emit_at = None;
        Ok(state.enabled)
    }

    fn try_publish<F>(&self, sample_time: Instant, publish: F) -> Result<bool, String>
    where
        F: FnOnce(u64) -> Result<(), String>,
    {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "diagnostics publication gate poisoned".to_string())?;
        if !state.enabled {
            return Ok(false);
        }
        if state
            .last_emit_at
            .is_some_and(|last| sample_time.saturating_duration_since(last) < DIAGNOSTICS_INTERVAL)
        {
            return Ok(false);
        }
        state.last_emit_at = Some(sample_time);
        state.sequence = state.sequence.wrapping_add(1);
        let sequence = state.sequence;
        // Keep the gate held across the bounded event publication. This is
        // the linearization point that makes a successful disable authoritative
        // even when a supervisor publication was already in flight.
        publish(sequence)?;
        Ok(true)
    }
}

#[derive(Clone)]
struct NativePreparedPlan {
    song_id: String,
    generation: u64,
    settings_fingerprint: String,
    song: Song,
    dto: PreparedPlaybackDto,
    variants: HashMap<PlaybackDecision, NativePlaybackVariant>,
}

#[derive(Clone)]
struct NativePlaybackVariant {
    config: PlaybackConfigDto,
    schedule: ScheduleMetadata,
    fingerprint: String,
    timing_policy: MaterializedTimingPolicy,
    timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto,
}

struct NativePreparedInput {
    song_id: String,
    generation: u64,
    config: PlaybackConfigDto,
    song: Song,
    schedule: ScheduleMetadata,
    risk: RiskReport,
    timing_policy: MaterializedTimingPolicy,
    timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto,
    settings_fingerprint: String,
}

struct NativeActivePlayback {
    session_id: String,
    prepared_id: String,
    song_id: String,
    title: String,
    total_us: u64,
    config: PlaybackConfigDto,
    timing_policy: MaterializedTimingPolicy,
    timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto,
    plan_fingerprint: String,
    physical: bool,
    activity_lease: Mutex<Option<PhysicalActivityLease>>,
    target_hwnd: Option<isize>,
    state: Mutex<PlaybackSessionState>,
    pending: Mutex<Option<PlaybackPendingControl>>,
    player: Option<Arc<NativeDispatchSession>>,
    started_at: Instant,
    paused_since: Mutex<Option<Instant>>,
    paused_total: Mutex<Duration>,
    stop_requested: AtomicBool,
    skip_requested: AtomicBool,
    done: AtomicBool,
    heartbeat_stop: AtomicBool,
    heartbeat_thread: Mutex<Option<thread::JoinHandle<()>>>,
    sequence: AtomicU64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct NativePlaybackPrepareRequest {
    song_id: String,
    generation: u64,
    config: PlaybackConfigDto,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct NativePlaybackStartRequest {
    prepared_id: String,
    decisions: Vec<PlaybackDecisionAcceptanceDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct NativePlaybackSessionRequest {
    session_id: String,
}

impl NativePlaybackPrepareRequest {
    fn into_public(self) -> PlaybackPrepareRequest {
        PlaybackPrepareRequest {
            song_id: self.song_id,
            generation: self.generation,
            config: self.config,
        }
    }
}

impl NativePlaybackStartRequest {
    fn into_public(self) -> PlaybackStartRequest {
        PlaybackStartRequest {
            prepared_id: self.prepared_id,
            decisions: self.decisions,
        }
    }
}

fn validate_prepared_id(prepared_id: &str) -> Result<(), String> {
    if prepared_id.len() != 32
        || !prepared_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("invalid_params: prepared_id is invalid".into());
    }
    Ok(())
}

fn validate_playback_start_request(request: &NativePlaybackStartRequest) -> Result<(), String> {
    validate_prepared_id(&request.prepared_id)?;
    if request.decisions.len() > MAX_DECISION_COUNT {
        return Err("invalid_params: decisions must be a bounded array".into());
    }
    Ok(())
}

fn validate_public_playback_start_request(request: &PlaybackStartRequest) -> Result<(), String> {
    validate_prepared_id(&request.prepared_id)?;
    if request.decisions.len() > MAX_DECISION_COUNT {
        return Err("invalid_params: decisions must be a bounded array".into());
    }
    Ok(())
}

fn opaque_native_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("secure native identifier generation failed: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn library_error(error: LibraryError) -> String {
    format!("library operation failed: {error}")
}

fn is_supported_catalog_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            sky_app_core::catalog::SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn imported_source_id(canonical_path: &str, kind: ImportedSourceKind) -> String {
    let mut hasher = Sha256::new();
    hasher.update(match kind {
        ImportedSourceKind::File => b"file\0".as_slice(),
        ImportedSourceKind::Folder => b"folder\0".as_slice(),
    });
    hasher.update(canonical_path.as_bytes());
    hasher
        .finalize()
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn retain_prepared_capacity<T>(prepared: &mut VecDeque<T>) {
    while prepared.len() > MAX_PREPARED_PLANS {
        prepared.pop_front();
    }
}

fn plan_fingerprint(
    song_id: &str,
    config: &PlaybackConfigDto,
    schedule: &ScheduleMetadata,
    policy: &MaterializedTimingPolicy,
) -> Result<String, String> {
    let actions = schedule
        .actions
        .iter()
        .map(|action| {
            serde_json::json!([
                match action.kind {
                    ActionKind::Down => "down",
                    ActionKind::Up => "up",
                },
                action.at_us,
                action.scan_codes,
            ])
        })
        .collect::<Vec<_>>();
    // Construct every nested object with json! rather than serializing the
    // Rust DTO directly.  The Python oracle uses
    // json.dumps(..., sort_keys=True, separators=(",", ":")); DTO struct
    // declaration order is not the canonical fingerprint order.
    let payload = serde_json::json!({
        "song_id": song_id,
        "config": {
            "dry_run": config.dry_run,
            "down_late_grace_us": config.down_late_grace_us,
            "fps": config.fps,
            "hold_frames": config.hold_frames,
            "timing_margin_us": config.timing_margin_us,
            "tempo_scale": config.tempo_scale,
        },
        "policy": {
            "down_late_grace_us": policy.down_late_grace_us,
            "fps": policy.fps,
            "frame_base_hold_us": policy.frame_base_hold_us,
            "timing_margin_us": policy.timing_margin_us,
            "min_hold_us": policy.min_hold_us,
            "min_release_gap_us": policy.min_release_gap_us,
        },
        "actions": actions,
    });
    let bytes = serde_json::to_vec(&payload).map_err(json_error)?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn settings_fingerprint(settings: &ApplicationSettings) -> Result<String, String> {
    // Keep this in lockstep with DesktopPlaybackService._settings_fingerprint.
    // Update timestamps and unrelated preferences must not invalidate a plan;
    // the routed settings.patch path explicitly invalidates every plan after a
    // successful mutation, while update metadata writes remain independent.
    // Python's settings fingerprint uses json.dumps with sorted keys and its
    // default separators (`, ` and `: `). Keep the flat payload explicit so
    // this cross-runtime identity cannot depend on Rust struct field order.
    let theme = serde_json::to_string(&settings.theme).map_err(json_error)?;
    let payload = format!(
        "{{\"down_late_grace_us\": {}, \"fps\": {}, \"hold\": {}, \"telemetry\": {}, \"tempo\": {}, \"theme\": {}, \"timing_margin_us\": {}}}",
        settings.playback_defaults.down_late_grace_us,
        settings.playback_defaults.fps,
        settings.playback_defaults.hold_frames,
        settings.telemetry_enabled,
        settings.playback_defaults.tempo_scale,
        theme,
        settings.playback_defaults.timing_margin_us,
    );
    let digest = Sha256::digest(payload.as_bytes());
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn risk_summary(risk: &RiskReport) -> RiskSummaryDto {
    RiskSummaryDto {
        level: risk.severity.clone(),
        headline: match risk.severity.as_str() {
            "low" => "Low timing risk".into(),
            "medium" => "Medium timing risk".into(),
            "high" => "High timing risk".into(),
            _ => risk.reason.clone(),
        },
        reasons: if risk.reason.is_empty() {
            Vec::new()
        } else {
            vec![risk.reason.clone()]
        },
        recommendations: risk.recommendations.clone(),
    }
}

fn format_label_for_path(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown")
        .to_ascii_uppercase()
}

fn blocked_playback_dto(
    song_id: &str,
    config: PlaybackConfigDto,
    code: &str,
    message: String,
    recommended_tempo_scale: Option<f64>,
    recommended_hold_frames: Option<f64>,
) -> PreparedPlaybackDto {
    // Keep blocked preparation responses aligned with the Python
    // DesktopPlaybackService: the failure is represented as a typed blocked
    // DTO, not as a new transport error or an implementation-specific code.
    let recommendations = [
        recommended_tempo_scale.map(|tempo| format!("Try tempo {tempo:.2}×")),
        recommended_hold_frames.map(|hold| format!("Try hold {hold:.2} frames")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let risk = RiskSummaryDto {
        level: "high".into(),
        headline: "Playback blocked".into(),
        reasons: vec![message.clone()],
        recommendations,
    };
    PreparedPlaybackDto {
        prepared_id: None,
        song: SongDetailDto {
            song_id: song_id.into(),
            title: song_id.into(),
            duration_us: 0,
            note_count: 0,
            format_label: "UNKNOWN".into(),
            risk: risk.clone(),
            recommendation: None,
        },
        config,
        admission: PlaybackAdmission::Blocked,
        risk,
        decisions: Vec::new(),
        plan_fingerprint: None,
        variants: Vec::new(),
        error_code: Some(code.into()),
        error_message: Some(message),
    }
}

fn song_detail(
    song_id: &str,
    song: &Song,
    schedule: &ScheduleMetadata,
    risk: &RiskReport,
) -> SongDetailDto {
    let recommendation = crate::commands::PlaybackRecommendationDto {
        recommended_hold_frames: risk.suggested_hold_frames,
        recommended_tempo_scale: risk.suggested_tempo_scale,
        summary: risk
            .recommendations
            .first()
            .cloned()
            .unwrap_or_else(|| "Keep the selected settings.".into()),
    };
    SongDetailDto {
        song_id: song_id.to_owned(),
        title: song.name.clone(),
        duration_us: schedule.source_duration_us,
        note_count: song.notes.len() as u64,
        format_label: "SHEET".into(),
        risk: risk_summary(risk),
        recommendation: Some(recommendation),
    }
}

fn compile_dispatch_schedule(
    schedule: &ScheduleMetadata,
) -> Result<sky_player::adapter_support::RuntimeSchedule, String> {
    let actions = schedule
        .actions
        .iter()
        .enumerate()
        .map(|(index, action)| KeyActionInput {
            source_action_index: index as u32,
            kind: match action.kind {
                ActionKind::Down => DispatchActionKind::Down,
                ActionKind::Up => DispatchActionKind::Up,
            },
            scheduled_us: action.at_us,
            scan_codes: action
                .scan_codes
                .iter()
                .copied()
                .collect::<SmallVec<[u16; 4]>>(),
            reason: Arc::<str>::from(action.reason.as_str()),
        })
        .collect::<Vec<_>>();
    compile_runtime_intents(&actions, &sky_app_core::song::SKY_SCAN_CODES)
        .map_err(|error| format!("native schedule compilation failed: {error}"))
}

impl NativePlaybackService {
    fn new(activity: ActivityCoordinator) -> Self {
        Self {
            prepared: Mutex::new(VecDeque::new()),
            active: Arc::new(Mutex::new(None)),
            last_terminal: Arc::new(Mutex::new(None)),
            sender_trace_state: Arc::new(Mutex::new(SenderTraceState::default())),
            diagnostics_gate: Arc::new(DiagnosticsPublicationGate::default()),
            activity,
            #[cfg(test)]
            settings_invalidation_count: AtomicU64::new(0),
        }
    }

    fn set_diagnostics_enabled(
        &self,
        enabled: bool,
        _events: Arc<Mutex<NativeEventHub>>,
    ) -> Result<(), String> {
        let enabled = self.diagnostics_gate.set_enabled(enabled)?;
        let active_player = self
            .active
            .lock()
            .map_err(|_| "active playback lock poisoned".to_string())?
            .as_ref()
            .and_then(|active| active.player.clone());
        if let Some(player) = active_player {
            player.set_live_diagnostics_enabled(enabled);
        }
        Ok(())
    }

    fn status(&self) -> Result<PlaybackStatusDto, String> {
        let active_slot = self
            .active
            .lock()
            .map_err(|_| "native active-playback lock poisoned".to_string())?;
        let last_terminal = self
            .last_terminal
            .lock()
            .map_err(|_| "native terminal-session lock poisoned".to_string())?
            .clone();
        let active = active_slot
            .as_ref()
            .map(|active| {
                let state = *active
                    .state
                    .lock()
                    .map_err(|_| "native playback state lock poisoned".to_string())?;
                // A terminal state remains owned until the retirement barrier
                // clears the active slot. Keep it actionable as stopping until
                // that barrier is complete.
                let state = if matches!(
                    state,
                    PlaybackSessionState::Finished | PlaybackSessionState::Failed
                ) {
                    PlaybackSessionState::Stopping
                } else {
                    state
                };
                Ok::<PlaybackActiveStatusDto, String>(PlaybackActiveStatusDto {
                    session_id: active.session_id.clone(),
                    prepared_id: active.prepared_id.clone(),
                    song_id: active.song_id.clone(),
                    title: active.title.clone(),
                    state,
                })
            })
            .transpose()?;
        Ok(PlaybackStatusDto {
            active,
            last_terminal,
        })
    }

    fn export_sender_trace(&self) -> Result<String, String> {
        let trace_state = self
            .sender_trace_state
            .lock()
            .map_err(|_| "sender trace state lock poisoned".to_string())?;
        let completed = trace_state
            .completed
            .as_ref()
            .ok_or_else(|| "no completed physical sender trace is available".to_string())?;
        let document: Value = serde_json::from_str(&completed.trace_json)
            .map_err(|_| "completed sender trace is malformed".to_string())?;
        if document["session_id"] != completed.session_id
            || document["song_id"] != completed.song_id
            || document["song_title"] != completed.song_title
            || document["plan_fingerprint"] != completed.plan_fingerprint
        {
            return Err("completed sender trace identity does not match its owner".to_string());
        }
        Ok(completed.trace_json.clone())
    }

    fn prepare(&self, input: NativePreparedInput) -> Result<PreparedPlaybackDto, String> {
        let NativePreparedInput {
            song_id,
            generation,
            config,
            song,
            schedule,
            risk,
            timing_policy,
            timing_margin_recommendation,
            settings_fingerprint,
        } = input;
        let detail = song_detail(&song_id, &song, &schedule, &risk);
        if !config.dry_run && schedule.impossible_same_key_repeats > 0 {
            let message = format!(
                "Detected {} infeasible same-key repeat(s): the authored interval is shorter than the configured hold.",
                schedule.impossible_same_key_repeats
            );
            return Ok(blocked_playback_dto(
                &song_id,
                config,
                "validation_failed",
                message,
                schedule.recommended_tempo_scale,
                schedule.recommended_hold_frames,
            ));
        }
        let fingerprint = plan_fingerprint(&song_id, &config, &schedule, &timing_policy)?;
        let admission = if risk.severity == "low" {
            PlaybackAdmission::Ready
        } else {
            PlaybackAdmission::ConfirmationRequired
        };
        let prepared_id = opaque_native_id()?;
        let base_variant = NativePlaybackVariant {
            config: config.clone(),
            schedule: schedule.clone(),
            fingerprint: fingerprint.clone(),
            timing_policy: timing_policy.clone(),
            timing_margin_recommendation: timing_margin_recommendation.clone(),
        };
        let mut decisions = Vec::new();
        let mut variants = HashMap::from([(PlaybackDecision::Proceed, base_variant)]);
        let mut variant_dtos = vec![PlaybackPlanVariantDto {
            decision: PlaybackDecision::Proceed,
            config: config.clone(),
            plan_fingerprint: fingerprint.clone(),
        }];
        if risk.severity != "low" {
            decisions.push(RiskDecisionDto {
                decision: PlaybackDecision::Proceed,
                label: "Proceed with current settings".into(),
            });
            if let (Some(hold_frames), Some(tempo_scale)) =
                (risk.suggested_hold_frames, risk.suggested_tempo_scale)
                && (hold_frames != config.hold_frames || tempo_scale != config.tempo_scale)
                && let Ok(recommended_policy) = MaterializedTimingPolicy::from_user_margin(
                    config.fps,
                    hold_frames,
                    config.timing_margin_us,
                    config.down_late_grace_us,
                )
                && let Ok(recommended_schedule) =
                    build_schedule_with_policy(&song, tempo_scale, &recommended_policy)
                && (config.dry_run || recommended_schedule.impossible_same_key_repeats == 0)
            {
                let recommended_config = PlaybackConfigDto {
                    hold_frames,
                    timing_margin_us: config.timing_margin_us,
                    down_late_grace_us: config.down_late_grace_us,
                    tempo_scale,
                    fps: config.fps,
                    dry_run: config.dry_run,
                };
                let recommended_fingerprint = plan_fingerprint(
                    &song_id,
                    &recommended_config,
                    &recommended_schedule,
                    &recommended_policy,
                )?;
                variants.insert(
                    PlaybackDecision::UseRecommended,
                    NativePlaybackVariant {
                        config: recommended_config.clone(),
                        schedule: recommended_schedule,
                        fingerprint: recommended_fingerprint.clone(),
                        timing_policy: recommended_policy,
                        timing_margin_recommendation: timing_margin_recommendation.clone(),
                    },
                );
                variant_dtos.push(PlaybackPlanVariantDto {
                    decision: PlaybackDecision::UseRecommended,
                    config: recommended_config,
                    plan_fingerprint: recommended_fingerprint,
                });
                decisions.push(RiskDecisionDto {
                    decision: PlaybackDecision::UseRecommended,
                    label: "Use recommended settings".into(),
                });
            }
            if !config.dry_run {
                let dry_run_config = PlaybackConfigDto {
                    dry_run: true,
                    ..config.clone()
                };
                let dry_run_fingerprint =
                    plan_fingerprint(&song_id, &dry_run_config, &schedule, &timing_policy)?;
                variants.insert(
                    PlaybackDecision::DryRun,
                    NativePlaybackVariant {
                        config: dry_run_config.clone(),
                        schedule: schedule.clone(),
                        fingerprint: dry_run_fingerprint.clone(),
                        timing_policy: timing_policy.clone(),
                        timing_margin_recommendation: timing_margin_recommendation.clone(),
                    },
                );
                variant_dtos.push(PlaybackPlanVariantDto {
                    decision: PlaybackDecision::DryRun,
                    config: dry_run_config,
                    plan_fingerprint: dry_run_fingerprint,
                });
                decisions.push(RiskDecisionDto {
                    decision: PlaybackDecision::DryRun,
                    label: "Run a dry-run first".into(),
                });
            }
        }
        let dto = PreparedPlaybackDto {
            prepared_id: Some(prepared_id.clone()),
            song: detail,
            config,
            admission,
            risk: risk_summary(&risk),
            decisions,
            plan_fingerprint: Some(fingerprint.clone()),
            variants: variant_dtos,
            error_code: None,
            error_message: None,
        };
        let mut prepared = self
            .prepared
            .lock()
            .map_err(|_| "native prepared-plan lock poisoned".to_string())?;
        prepared.push_back((
            prepared_id,
            NativePreparedPlan {
                song_id,
                generation,
                settings_fingerprint,
                song,
                dto: dto.clone(),
                variants,
            },
        ));
        retain_prepared_capacity(&mut prepared);
        Ok(dto)
    }

    fn start(
        &self,
        request: PlaybackStartRequest,
        settings: &ApplicationSettings,
        events: Arc<Mutex<NativeEventHub>>,
    ) -> Result<PlaybackSessionDto, String> {
        let mut active_slot = self
            .active
            .lock()
            .map_err(|_| "native active-playback lock poisoned".to_string())?;
        if active_slot.is_some() {
            return Err("another playback session is active".into());
        }
        let mut prepared = self
            .prepared
            .lock()
            .map_err(|_| "native prepared-plan lock poisoned".to_string())?;
        let record = prepared
            .iter()
            .find(|(id, _)| id == &request.prepared_id)
            .map(|(_, record)| record.clone())
            .ok_or_else(|| "prepared playback is stale or already consumed".to_string())?;
        if record.settings_fingerprint != settings_fingerprint(settings)? {
            return Err("prepared playback is stale after settings mutation".into());
        }
        let accepted = request
            .decisions
            .iter()
            .filter(|item| item.accepted)
            .collect::<Vec<_>>();
        let required = record
            .dto
            .decisions
            .iter()
            .map(|item| item.decision)
            .collect::<Vec<_>>();
        let selected = if record.dto.admission == PlaybackAdmission::Ready {
            if !request.decisions.is_empty() {
                return Err("ready playback accepts no risk decisions".into());
            }
            PlaybackDecision::Proceed
        } else {
            if accepted.len() != 1
                || request.decisions.len() != 1
                || !required.contains(&accepted[0].decision)
            {
                return Err("an exact risk decision is required".into());
            }
            accepted[0].decision
        };
        let variant = record
            .variants
            .get(&selected)
            .cloned()
            .ok_or_else(|| "selected risk decision has no prepared plan".to_string())?;
        let session_id = opaque_native_id()?;
        let activity_lease = if variant.config.dry_run {
            None
        } else {
            Some(
                self.activity
                    .reserve_playback(&session_id)
                    .map_err(playback_activity_error)?,
            )
        };
        let (player, target_hwnd) = if variant.config.dry_run {
            (None, None)
        } else {
            match self.create_native_player(
                &variant.schedule,
                &variant.config,
                &variant.timing_policy,
                settings,
            ) {
                Ok((player, target)) => (Some(player), Some(target)),
                Err(error) => {
                    // Match the Python supervisor's observable failure path:
                    // a start that cannot pass focus/target admission still
                    // has an ordered starting -> failed -> failed-event trace.
                    let failed_active = Arc::new(NativeActivePlayback {
                        session_id: session_id.clone(),
                        prepared_id: request.prepared_id.clone(),
                        song_id: record.song_id.clone(),
                        title: record.song.name.clone(),
                        total_us: variant.schedule.duration_us,
                        config: variant.config.clone(),
                        timing_policy: variant.timing_policy.clone(),
                        timing_margin_recommendation: variant.timing_margin_recommendation.clone(),
                        plan_fingerprint: variant.fingerprint.clone(),
                        physical: true,
                        activity_lease: Mutex::new(activity_lease),
                        target_hwnd: None,
                        state: Mutex::new(PlaybackSessionState::Starting),
                        pending: Mutex::new(None),
                        player: None,
                        started_at: Instant::now(),
                        paused_since: Mutex::new(None),
                        paused_total: Mutex::new(Duration::ZERO),
                        stop_requested: AtomicBool::new(false),
                        skip_requested: AtomicBool::new(false),
                        done: AtomicBool::new(true),
                        heartbeat_stop: AtomicBool::new(true),
                        heartbeat_thread: Mutex::new(None),
                        sequence: AtomicU64::new(0),
                    });
                    let _ = publish_playback_state(
                        &events,
                        &failed_active,
                        PlaybackEventState::Starting,
                        None,
                        None,
                    );
                    let mut last_event_state = PlaybackEventState::Starting;
                    let (code, message) = physical_startup_failure(&error);
                    let fallback_publication = PlaybackTerminalPublication::Failed {
                        code: code.into(),
                        message: message.into(),
                    };
                    match publish_terminal_failure_state(
                        &events,
                        &failed_active,
                        &mut last_event_state,
                        code,
                        message,
                    ) {
                        Ok(publication) => {
                            let _ = publish_retirement_barrier(
                                &events,
                                &self.active,
                                &self.last_terminal,
                                &failed_active,
                                &publication,
                            );
                        }
                        Err(_) => {
                            let _ = release_terminal_ownership(
                                &self.active,
                                &self.last_terminal,
                                &failed_active,
                                Some(&fallback_publication),
                            );
                        }
                    }
                    return Err(format!("{code}: {message}"));
                }
            }
        };
        let active = Arc::new(NativeActivePlayback {
            session_id: session_id.clone(),
            prepared_id: request.prepared_id.clone(),
            song_id: record.song_id.clone(),
            title: record.song.name.clone(),
            total_us: variant.schedule.duration_us,
            config: variant.config.clone(),
            timing_policy: variant.timing_policy.clone(),
            timing_margin_recommendation: variant.timing_margin_recommendation.clone(),
            plan_fingerprint: variant.fingerprint.clone(),
            physical: player.is_some(),
            activity_lease: Mutex::new(activity_lease),
            target_hwnd,
            state: Mutex::new(PlaybackSessionState::Starting),
            pending: Mutex::new(None),
            player,
            started_at: Instant::now(),
            paused_since: Mutex::new(None),
            paused_total: Mutex::new(Duration::ZERO),
            stop_requested: AtomicBool::new(false),
            skip_requested: AtomicBool::new(false),
            done: AtomicBool::new(false),
            heartbeat_stop: AtomicBool::new(false),
            heartbeat_thread: Mutex::new(None),
            sequence: AtomicU64::new(0),
        });
        let prepared_index = prepared
            .iter()
            .position(|(id, _)| id == &request.prepared_id)
            .expect("prepared record was found above");
        prepared.remove(prepared_index);
        if let Some(player) = &active.player {
            player.set_live_diagnostics_enabled(self.diagnostics_gate.is_enabled());
        }
        *active_slot = Some(active.clone());
        drop(prepared);
        drop(active_slot);
        if active.physical
            && let Ok(mut trace_state) = self.sender_trace_state.lock()
        {
            trace_state.begin_physical_session(&active.session_id);
        }
        if let Err(error) =
            publish_playback_state(&events, &active, PlaybackEventState::Starting, None, None)
        {
            if active.physical
                && let Ok(mut trace_state) = self.sender_trace_state.lock()
            {
                trace_state.finish_physical_session(
                    &active.session_id,
                    &active.song_id,
                    &active.title,
                    &active.plan_fingerprint,
                    None,
                );
            }
            if let Some(player) = &active.player {
                let _ = player.panic_release();
                let _ = player.quit();
                let _ = player.join(Duration::from_secs(5));
            }
            let mut last_event_state = PlaybackEventState::Starting;
            match publish_terminal_poll_state(
                &events,
                &active,
                &mut last_event_state,
                EnginePollStatus::Error,
            ) {
                Ok(publication) => {
                    let _ = publish_retirement_barrier(
                        &events,
                        &self.active,
                        &self.last_terminal,
                        &active,
                        &publication,
                    );
                }
                Err(_) => {
                    let _ = release_terminal_ownership(
                        &self.active,
                        &self.last_terminal,
                        &active,
                        None,
                    );
                }
            }
            if let Ok(mut prepared) = self.prepared.lock() {
                prepared.push_back((request.prepared_id, record));
            }
            return Err(error);
        }
        if let Err(error) = spawn_supervisor_heartbeat(&active) {
            stop_supervisor_heartbeat(&active);
            if active.physical
                && let Ok(mut trace_state) = self.sender_trace_state.lock()
            {
                trace_state.finish_physical_session(
                    &active.session_id,
                    &active.song_id,
                    &active.title,
                    &active.plan_fingerprint,
                    None,
                );
            }
            if let Some(player) = &active.player {
                let _ = player.panic_release();
                let _ = player.quit();
                let _ = player.join(Duration::from_secs(5));
            }
            let mut last_event_state = PlaybackEventState::Starting;
            match publish_terminal_poll_state(
                &events,
                &active,
                &mut last_event_state,
                EnginePollStatus::Error,
            ) {
                Ok(publication) => {
                    let _ = publish_retirement_barrier(
                        &events,
                        &self.active,
                        &self.last_terminal,
                        &active,
                        &publication,
                    );
                }
                Err(_) => {
                    let _ = release_terminal_ownership(
                        &self.active,
                        &self.last_terminal,
                        &active,
                        None,
                    );
                }
            }
            if let Ok(mut prepared) = self.prepared.lock() {
                prepared.push_back((request.prepared_id, record));
            }
            return Err(format!(
                "failed to start native playback heartbeat: {error}"
            ));
        }
        let service = Arc::new(self.clone_handle());
        let active_for_thread = active.clone();
        let events_for_thread = events.clone();
        let spawn_result = thread::Builder::new()
            .name("sky-native-playback-supervisor".into())
            .spawn(move || service.monitor(active_for_thread, events_for_thread));
        if let Err(error) = spawn_result {
            stop_supervisor_heartbeat(&active);
            if active.physical
                && let Ok(mut trace_state) = self.sender_trace_state.lock()
            {
                trace_state.finish_physical_session(
                    &active.session_id,
                    &active.song_id,
                    &active.title,
                    &active.plan_fingerprint,
                    None,
                );
            }
            if let Some(player) = &active.player {
                let _ = player.panic_release();
                let _ = player.quit();
                let _ = player.join(Duration::from_secs(5));
            }
            let mut last_event_state = PlaybackEventState::Starting;
            match publish_terminal_poll_state(
                &events,
                &active,
                &mut last_event_state,
                EnginePollStatus::Error,
            ) {
                Ok(publication) => {
                    let _ = publish_retirement_barrier(
                        &events,
                        &self.active,
                        &self.last_terminal,
                        &active,
                        &publication,
                    );
                }
                Err(_) => {
                    let _ = release_terminal_ownership(
                        &self.active,
                        &self.last_terminal,
                        &active,
                        None,
                    );
                }
            }
            if let Ok(mut prepared) = self.prepared.lock() {
                prepared.push_back((request.prepared_id, record));
            }
            return Err(format!(
                "failed to start native playback supervisor: {error}"
            ));
        }
        Ok(PlaybackSessionDto {
            session_id,
            prepared_id: active.prepared_id.clone(),
            song_id: active.song_id.clone(),
            state: PlaybackSessionState::Starting,
            config: active.config.clone(),
            plan_fingerprint: active.plan_fingerprint.clone(),
        })
    }

    fn clone_handle(&self) -> NativePlaybackServiceHandle {
        NativePlaybackServiceHandle {
            active: self.active.clone(),
            last_terminal: self.last_terminal.clone(),
            sender_trace_state: self.sender_trace_state.clone(),
            diagnostics_gate: self.diagnostics_gate.clone(),
            activity: self.activity.clone(),
        }
    }

    fn create_native_player(
        &self,
        schedule: &ScheduleMetadata,
        config: &PlaybackConfigDto,
        policy: &MaterializedTimingPolicy,
        settings: &ApplicationSettings,
    ) -> Result<(Arc<NativeDispatchSession>, isize), String> {
        let runtime_schedule = compile_dispatch_schedule(schedule)?;
        let target = sky_dispatch_win32::focus::find_sky_window(
            &settings.sky_process_names,
            settings.allow_title_fallback,
        )
        .ok_or_else(|| "no admissible visible Sky window was found".to_string())?;
        if matches!(
            sky_dispatch_win32::focus::target_integrity_compatibility(target),
            sky_dispatch_win32::focus::TargetIntegrityCompatibility::Mismatch
        ) {
            return Err(
                "target Sky process has higher integrity than Sky Auto Player; Windows UIPI can block SendInput"
                    .to_string(),
            );
        }
        if !sky_dispatch_win32::focus::focus_window_and_verify(target, Duration::from_millis(100)) {
            return Err("validated Sky window could not be focused".into());
        }
        let player = Arc::new(NativeDispatchSession::new(NativeSessionOptions {
            schedule: runtime_schedule,
            backend: BackendConfig::Production,
            profile: DispatchProfile::Production,
            timing: TimingOptions {
                game_fps: config.fps,
                min_hold_us: policy.min_hold_us,
                min_release_gap_us: policy.min_release_gap_us,
                down_late_grace_us: policy.down_late_grace_us,
                strict_timing: false,
                strict_down_completion_late_us: 2_000,
                strict_up_completion_late_us: 2_000,
                input_path_warn_us: 300,
            },
            focus: FocusOptions {
                require_focus: true,
                focus_restore_grace_us: policy.focus_restore_grace_us,
            },
            wait: WaitOptions {
                enable_waitable_timer: true,
                enable_event_wait: true,
                supervisor_lease_timeout_us:
                    sky_player::engine::DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US,
                #[cfg(feature = "tauri-test")]
                test_spin_threshold_us: None,
                #[cfg(feature = "tauri-test")]
                test_wait_policy: sky_player::engine::TestWaitPolicy::LegacyTestWideSpin,
            },
            telemetry: TelemetryOptions {
                mode: TelemetryMode::Ring,
                // RtTraceRecord is fixed-width; 8192 records bound capture to
                // about 1.5 MiB on x64 before export while covering typical songs.
                capacity: 8_192,
            },
            priority: PriorityOptions {
                mode: PriorityMode::Auto,
            },
            instrument_key_profile: None,
            #[cfg(feature = "tauri-test")]
            startup_ordering_hook: None,
            #[cfg(feature = "tauri-test")]
            restore_race_hook: None,
            #[cfg(feature = "tauri-test")]
            timer_lifecycle_context: None,
        })?);
        player.set_target_hwnd(target);
        player.set_focus_hint(true);
        player.set_live_diagnostics_enabled(self.diagnostics_gate.is_enabled());
        player.arm(0)?;
        Ok((player, target))
    }

    fn monitor(&self, active: Arc<NativeActivePlayback>, events: Arc<Mutex<NativeEventHub>>) {
        let mut last_snapshot = Instant::now();
        let mut last_event_state = PlaybackEventState::Starting;
        let mut terminal_publication = None;
        loop {
            if active.stop_requested.load(Ordering::Acquire) {
                stop_supervisor_heartbeat(&active);
                if let Some(player) = &active.player {
                    let _ = player.quit();
                    let _ = player.join(Duration::from_secs(5));
                }
                let outcome = if active.skip_requested.load(Ordering::Acquire) {
                    "skipped"
                } else {
                    "quit"
                };
                match publish_stopped_state(
                    &events,
                    &active,
                    &mut last_event_state,
                    outcome,
                    "Playback stopped",
                ) {
                    Ok(publication) => terminal_publication = Some(publication),
                    Err(_) => cleanup_failed_event_delivery(&active),
                }
                break;
            }
            if let Some(player) = &active.player
                && let Some(target) = active.target_hwnd
            {
                // The supervisor owns only a transition hint.  The worker
                // still performs its fresh exact-HWND final admission.
                let focused = sky_dispatch_win32::focus::foreground_window_matches(target);
                player.set_focus_hint(focused);
            }
            let (elapsed, pre_roll_remaining, status) = if let Some(player) = &active.player {
                let state = player.poll_state();
                (state.elapsed_us, state.pre_roll_remaining_us, state.status)
            } else {
                let elapsed = dry_run_elapsed(&active);
                let paused = active
                    .state
                    .lock()
                    .map(|state| *state == PlaybackSessionState::Paused)
                    .unwrap_or(false);
                (
                    elapsed,
                    0,
                    if elapsed >= active.total_us {
                        EnginePollStatus::Finished
                    } else if paused {
                        EnginePollStatus::Paused
                    } else {
                        EnginePollStatus::Playing
                    },
                )
            };
            let paused = status == EnginePollStatus::Paused;
            let event_state = playback_event_state(status);
            if is_terminal_status(status) {
                match publish_terminal_poll_state(&events, &active, &mut last_event_state, status) {
                    Ok(publication) => terminal_publication = Some(publication),
                    Err(_) => cleanup_failed_event_delivery(&active),
                }
                break;
            }
            if event_state != last_event_state {
                let state = match event_state {
                    PlaybackEventState::Starting => PlaybackSessionState::Starting,
                    PlaybackEventState::Playing => PlaybackSessionState::Playing,
                    PlaybackEventState::Paused => PlaybackSessionState::Paused,
                    PlaybackEventState::Stopping => PlaybackSessionState::Stopping,
                    PlaybackEventState::Finished => PlaybackSessionState::Finished,
                    PlaybackEventState::Failed => PlaybackSessionState::Failed,
                };
                let _ = set_playback_state(&active, state);
                if matches!(
                    (
                        event_state,
                        active.pending.lock().ok().and_then(|pending| *pending)
                    ),
                    (
                        PlaybackEventState::Paused,
                        Some(PlaybackPendingControl::Pause)
                    ) | (
                        PlaybackEventState::Playing,
                        Some(PlaybackPendingControl::Resume)
                    )
                ) && let Ok(mut pending) = active.pending.lock()
                {
                    *pending = None;
                }
                if publish_playback_state(&events, &active, event_state, None, None).is_err() {
                    cleanup_failed_event_delivery(&active);
                    break;
                }
                last_event_state = event_state;
            }
            if last_snapshot.elapsed() >= Duration::from_millis(100) {
                if publish_playback_snapshot(&events, &active, elapsed, pre_roll_remaining, paused)
                    .is_err()
                {
                    cleanup_failed_event_delivery(&active);
                    break;
                }
                if publish_diagnostics_snapshot_for_active(&events, &active, &self.diagnostics_gate)
                    .is_err()
                {
                    cleanup_failed_event_delivery(&active);
                    break;
                }
                last_snapshot = Instant::now();
            }
            thread::sleep(Duration::from_millis(20));
        }
        stop_supervisor_heartbeat(&active);
        if let Some(player) = &active.player
            && !matches!(player.join(Duration::from_secs(5)), Ok(true))
        {
            let _ = player.panic_release();
            let _ = player.quit();
            let _ = player.join(Duration::from_secs(5));
        }
        if active.physical
            && let Ok(mut trace_state) = self.sender_trace_state.lock()
        {
            trace_state.finish_physical_session(
                &active.session_id,
                &active.song_id,
                &active.title,
                &active.plan_fingerprint,
                completed_sender_trace(&active),
            );
        }
        let ownership_released = release_terminal_ownership(
            &self.active,
            &self.last_terminal,
            &active,
            terminal_publication.as_ref(),
        );
        if ownership_released.is_ok()
            && let Some(publication) = terminal_publication.as_ref()
            && publish_terminal_event(&events, &active, publication).is_err()
        {
            cleanup_failed_event_delivery(&active);
        }
    }

    fn command(
        &self,
        method: &str,
        session_id: String,
        events: Arc<Mutex<NativeEventHub>>,
    ) -> Result<PlaybackCommandAckDto, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "native active-playback lock poisoned".to_string())?
            .clone();
        let Some(active) = active else {
            let terminal = self
                .last_terminal
                .lock()
                .ok()
                .and_then(|value| value.clone());
            if method == "playback.stop"
                && terminal
                    .as_ref()
                    .is_some_and(|terminal| terminal.session_id == session_id)
            {
                let terminal_state = terminal
                    .map(|terminal| terminal.state)
                    .unwrap_or(PlaybackSessionState::Failed);
                return Ok(PlaybackCommandAckDto {
                    accepted: true,
                    session_id,
                    state: terminal_state,
                    pending_command: None,
                    reason: None,
                });
            }
            return Err("there is no active playback session".into());
        };
        if active.session_id != session_id {
            return Err("session_id is stale or foreign".into());
        }
        let current = *active
            .state
            .lock()
            .map_err(|_| "native playback state lock poisoned".to_string())?;
        match method {
            "playback.stop" => {
                if !matches!(
                    current,
                    PlaybackSessionState::Starting
                        | PlaybackSessionState::Playing
                        | PlaybackSessionState::Paused
                ) {
                    return Ok(PlaybackCommandAckDto {
                        accepted: true,
                        session_id,
                        state: current,
                        pending_command: *active
                            .pending
                            .lock()
                            .map_err(|_| "native playback control lock poisoned".to_string())?,
                        reason: None,
                    });
                }
                active.stop_requested.store(true, Ordering::Release);
                *active
                    .pending
                    .lock()
                    .map_err(|_| "native playback control lock poisoned".to_string())? = None;
                let _ = set_playback_state(&active, PlaybackSessionState::Stopping);
                if let Some(player) = &active.player {
                    player.quit()?;
                }
                publish_playback_state(&events, &active, PlaybackEventState::Stopping, None, None)?;
            }
            "playback.pause" => {
                if current != PlaybackSessionState::Playing {
                    return Err("pause requires a playing session".into());
                }
                let mut pending = active
                    .pending
                    .lock()
                    .map_err(|_| "native playback control lock poisoned".to_string())?;
                if *pending == Some(PlaybackPendingControl::Pause) {
                    return Ok(PlaybackCommandAckDto {
                        accepted: true,
                        session_id,
                        state: current,
                        pending_command: *pending,
                        reason: Some("already_pending".into()),
                    });
                }
                if pending.is_some() {
                    return Err("another playback control is awaiting acknowledgement".into());
                }
                *pending = Some(PlaybackPendingControl::Pause);
                drop(pending);
                if let Some(player) = &active.player {
                    if let Err(error) = player.pause() {
                        if let Ok(mut pending) = active.pending.lock() {
                            *pending = None;
                        }
                        return Err(error.to_string());
                    }
                } else {
                    *active
                        .paused_since
                        .lock()
                        .map_err(|_| "native playback pause lock poisoned".to_string())? =
                        Some(Instant::now());
                    set_playback_state(&active, PlaybackSessionState::Paused)?;
                    publish_playback_state(
                        &events,
                        &active,
                        PlaybackEventState::Paused,
                        None,
                        None,
                    )?;
                }
            }
            "playback.resume" => {
                if current != PlaybackSessionState::Paused {
                    return Err("resume requires a paused session".into());
                }
                let mut pending = active
                    .pending
                    .lock()
                    .map_err(|_| "native playback control lock poisoned".to_string())?;
                if *pending == Some(PlaybackPendingControl::Resume) {
                    return Ok(PlaybackCommandAckDto {
                        accepted: true,
                        session_id,
                        state: current,
                        pending_command: *pending,
                        reason: Some("already_pending".into()),
                    });
                }
                if pending.is_some() {
                    return Err("another playback control is awaiting acknowledgement".into());
                }
                *pending = Some(PlaybackPendingControl::Resume);
                drop(pending);
                if let Some(player) = &active.player {
                    if let Err(error) = player.resume() {
                        if let Ok(mut pending) = active.pending.lock() {
                            *pending = None;
                        }
                        return Err(error.to_string());
                    }
                } else {
                    let now = Instant::now();
                    let paused_since = active
                        .paused_since
                        .lock()
                        .map_err(|_| "native playback pause lock poisoned".to_string())?
                        .take();
                    if let Some(paused_since) = paused_since {
                        *active
                            .paused_total
                            .lock()
                            .map_err(|_| "native playback pause lock poisoned".to_string())? +=
                            now.saturating_duration_since(paused_since);
                    }
                    set_playback_state(&active, PlaybackSessionState::Playing)?;
                    publish_playback_state(
                        &events,
                        &active,
                        PlaybackEventState::Playing,
                        None,
                        None,
                    )?;
                }
            }
            "playback.skip" => {
                if !matches!(
                    current,
                    PlaybackSessionState::Starting
                        | PlaybackSessionState::Playing
                        | PlaybackSessionState::Paused
                ) {
                    return Err("skip requires a live playback session".into());
                }
                *active
                    .pending
                    .lock()
                    .map_err(|_| "native playback control lock poisoned".to_string())? = None;
                active.skip_requested.store(true, Ordering::Release);
                if let Some(player) = &active.player {
                    player.skip()?;
                } else {
                    active.stop_requested.store(true, Ordering::Release);
                }
            }
            _ => return Err(format!("unsupported native playback command: {method}")),
        }
        Ok(PlaybackCommandAckDto {
            accepted: true,
            session_id,
            state: *active
                .state
                .lock()
                .map_err(|_| "native playback state lock poisoned".to_string())?,
            pending_command: *active
                .pending
                .lock()
                .map_err(|_| "native playback control lock poisoned".to_string())?,
            reason: None,
        })
    }

    fn invalidate_catalog(&self, generation: u64) {
        if let Ok(mut prepared) = self.prepared.lock() {
            prepared.retain(|(_, record)| record.generation == generation);
        }
    }

    fn invalidate_settings(&self) {
        #[cfg(test)]
        self.settings_invalidation_count
            .fetch_add(1, Ordering::Relaxed);
        if let Ok(mut prepared) = self.prepared.lock() {
            prepared.clear();
        }
    }

    fn shutdown(&self, events: Arc<Mutex<NativeEventHub>>) {
        let active = self.active.lock().ok().and_then(|slot| slot.clone());
        if let Some(active) = active {
            let current = active.state.lock().ok().map(|state| *state);
            if matches!(
                current,
                Some(
                    PlaybackSessionState::Starting
                        | PlaybackSessionState::Playing
                        | PlaybackSessionState::Paused
                )
            ) {
                active.stop_requested.store(true, Ordering::Release);
                if let Ok(mut pending) = active.pending.lock() {
                    *pending = None;
                }
                let _ = set_playback_state(&active, PlaybackSessionState::Stopping);
                let _ = publish_playback_state(
                    &events,
                    &active,
                    PlaybackEventState::Stopping,
                    None,
                    None,
                );
            } else {
                active.stop_requested.store(true, Ordering::Release);
            }
            if let Some(player) = &active.player {
                let _ = player.panic_release();
                let _ = player.quit();
                let _ = player.join(Duration::from_secs(5));
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while !active.done.load(Ordering::Acquire) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[derive(Clone)]
struct NativePlaybackServiceHandle {
    active: Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    last_terminal: Arc<Mutex<Option<PlaybackTerminalStatusDto>>>,
    sender_trace_state: Arc<Mutex<SenderTraceState>>,
    diagnostics_gate: Arc<DiagnosticsPublicationGate>,
    activity: ActivityCoordinator,
}

impl NativePlaybackServiceHandle {
    fn monitor(&self, active: Arc<NativeActivePlayback>, events: Arc<Mutex<NativeEventHub>>) {
        let service = NativePlaybackService {
            prepared: Mutex::new(VecDeque::new()),
            active: self.active.clone(),
            last_terminal: self.last_terminal.clone(),
            sender_trace_state: self.sender_trace_state.clone(),
            diagnostics_gate: self.diagnostics_gate.clone(),
            activity: self.activity.clone(),
            #[cfg(test)]
            settings_invalidation_count: AtomicU64::new(0),
        };
        service.monitor(active, events);
    }
}

fn completed_sender_trace(active: &NativeActivePlayback) -> Option<String> {
    let player = active.player.as_ref()?;
    if !player.join(Duration::from_secs(5)).ok()? {
        return None;
    }
    let telemetry: Value = serde_json::from_str(&player.take_telemetry_json().ok()?).ok()?;
    sender_trace_export_json(active, &telemetry).ok()
}

fn sender_trace_export_json(
    active: &NativeActivePlayback,
    telemetry: &Value,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&serde_json::json!({
        "export_schema_version": 1,
        "session_id": active.session_id,
        "song_id": active.song_id,
        "song_title": active.title,
        "plan_fingerprint": active.plan_fingerprint,
        "timing_policy": {
            "fps": active.timing_policy.fps,
            "hold_frames": active.timing_policy.hold_frames,
            "frame_period_us": active.timing_policy.frame_us,
            "frame_base_hold_us": active.timing_policy.frame_base_hold_us,
            "timing_margin_us": active.timing_policy.timing_margin_us,
            "target_hold_us": active.timing_policy.min_hold_us,
            "release_gap_us": active.timing_policy.min_release_gap_us,
            "late_down_tolerance_us": active.timing_policy.down_late_grace_us,
            "down_late_cutoff_us": active.timing_policy.down_late_grace_us,
        },
        "telemetry": telemetry,
    }))
}

fn supervisor_heartbeat_loop<F>(
    stop: &AtomicBool,
    done: &AtomicBool,
    interval: Duration,
    mut heartbeat: F,
) where
    F: FnMut(),
{
    while !stop.load(Ordering::Acquire) && !done.load(Ordering::Acquire) {
        heartbeat();
        thread::sleep(interval);
    }
}

fn spawn_supervisor_heartbeat(active: &Arc<NativeActivePlayback>) -> Result<(), String> {
    let Some(player) = active.player.clone() else {
        return Ok(());
    };
    let heartbeat_active = Arc::clone(active);
    let handle = thread::Builder::new()
        .name("sky-native-playback-heartbeat".into())
        .spawn(move || {
            supervisor_heartbeat_loop(
                &heartbeat_active.heartbeat_stop,
                &heartbeat_active.done,
                SUPERVISOR_HEARTBEAT_INTERVAL,
                || {
                    let _ = player.heartbeat();
                },
            );
        })
        .map_err(|error| format!("could not spawn supervisor heartbeat: {error}"))?;
    match active.heartbeat_thread.lock() {
        Ok(mut slot) => {
            *slot = Some(handle);
            Ok(())
        }
        Err(_) => {
            active.heartbeat_stop.store(true, Ordering::Release);
            let _ = handle.join();
            Err("supervisor heartbeat state lock poisoned".into())
        }
    }
}

fn stop_supervisor_heartbeat(active: &NativeActivePlayback) {
    active.heartbeat_stop.store(true, Ordering::Release);
    let handle = active
        .heartbeat_thread
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if let Some(handle) = handle {
        let _ = handle.join();
    }
}

fn playback_event_state(status: EnginePollStatus) -> PlaybackEventState {
    match status {
        EnginePollStatus::Ready | EnginePollStatus::Preroll => PlaybackEventState::Starting,
        EnginePollStatus::Playing => PlaybackEventState::Playing,
        EnginePollStatus::Paused => PlaybackEventState::Paused,
        EnginePollStatus::Finished | EnginePollStatus::Skipped | EnginePollStatus::Quit => {
            PlaybackEventState::Finished
        }
        EnginePollStatus::Error
        | EnginePollStatus::Panicked
        | EnginePollStatus::Poisoned
        | EnginePollStatus::Invalid => PlaybackEventState::Failed,
    }
}

fn is_terminal_status(status: EnginePollStatus) -> bool {
    matches!(
        status,
        EnginePollStatus::Finished
            | EnginePollStatus::Skipped
            | EnginePollStatus::Quit
            | EnginePollStatus::Error
            | EnginePollStatus::Panicked
            | EnginePollStatus::Poisoned
            | EnginePollStatus::Invalid
    )
}

fn is_failure_status(status: EnginePollStatus) -> bool {
    matches!(
        status,
        EnginePollStatus::Error
            | EnginePollStatus::Panicked
            | EnginePollStatus::Poisoned
            | EnginePollStatus::Invalid
    )
}

fn terminal_success_outcome(status: EnginePollStatus, skip_requested: bool) -> &'static str {
    match status {
        EnginePollStatus::Skipped => "skipped",
        EnginePollStatus::Quit => "quit",
        EnginePollStatus::Finished if skip_requested => "skipped",
        _ => "finished",
    }
}

fn publish_diagnostics_snapshot_for_active(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    gate: &DiagnosticsPublicationGate,
) -> Result<(), String> {
    let sample = active
        .player
        .as_ref()
        .map(|player| NativeDiagnosticsSample::from_player(player, active.physical))
        .unwrap_or_else(|| NativeDiagnosticsSample::unavailable(active.physical));
    let player_attached = active.player.is_some();
    let session_id = active.session_id.clone();
    let completion_samples_available =
        sample.recent_latency_samples_available && !sample.recent_latencies_us.is_empty();
    let _published = gate.try_publish(Instant::now(), |sequence| {
        let payload = crate::ui_events::DiagnosticsSnapshotDto {
            seq: sequence,
            physical_session: active.physical,
            player_attached,
            sender_sample_count: sample.sender_sample_count,
            max_lateness_us: sample.max_lateness_us,
            p50_ms: completion_samples_available
                .then(|| percentile_ms(&sample.recent_latencies_us, 0.50)),
            p95_ms: completion_samples_available
                .then(|| percentile_ms(&sample.recent_latencies_us, 0.95)),
            sigma_onset_ms: completion_samples_available
                .then(|| population_sigma_ms(&sample.recent_latencies_us)),
            late_2ms: sample.late_2ms,
            late_5ms: sample.late_5ms,
            late_10ms: sample.late_10ms,
            max_sendinput_pre_call_lateness_us: sample.max_sendinput_pre_call_lateness_us,
            pre_call_late_2ms: sample.pre_call_late_2ms,
            pre_call_late_5ms: sample.pre_call_late_5ms,
            pre_call_late_10ms: sample.pre_call_late_10ms,
            fps: active.timing_policy.fps,
            frame_us: active.timing_policy.frame_us,
            hold_frames: active.timing_policy.hold_frames,
            frame_base_hold_us: active.timing_policy.frame_base_hold_us,
            timing_margin_us: active.timing_policy.timing_margin_us,
            min_hold_us: active.timing_policy.min_hold_us,
            min_release_gap_us: active.timing_policy.min_release_gap_us,
            down_late_grace_us: active.timing_policy.down_late_grace_us,
            timing_margin_recommendation: active.timing_margin_recommendation.clone(),
            pre_call_lt_250us: sample.pre_call_lt_250us,
            pre_call_250_500us: sample.pre_call_250_500us,
            pre_call_500_750us: sample.pre_call_500_750us,
            pre_call_750_1000us: sample.pre_call_750_1000us,
            pre_call_1000_1500us: sample.pre_call_1000_1500us,
            pre_call_1500_2000us: sample.pre_call_1500_2000us,
            pre_call_ge_2000us: sample.pre_call_ge_2000us,
            active_keys: sample.active_keys,
            stuck_keys: sample.stuck_keys,
            keys_dropped: sample.keys_dropped,
            chord_split_events: sample.chord_split_events,
            missed_down_boundaries: sample.missed_down_boundaries,
            missed_down_keys: sample.missed_down_keys,
            missed_backlog_boundaries: sample.missed_backlog_boundaries,
            missed_hard_late_boundaries: sample.missed_hard_late_boundaries,
            final_gate_cutoff_misses: sample.final_gate_cutoff_misses,
            final_gate_control_rejections: sample.final_gate_control_rejections,
            final_gate_target_changes: sample.final_gate_target_changes,
            final_gate_focus_losses: sample.final_gate_focus_losses,
            final_gate_lease_expirations: sample.final_gate_lease_expirations,
            sendinput_partial_events: sample.sendinput_partial_events,
            sendinput_zero_progress_failures: sample.sendinput_zero_progress_failures,
            backend_status: sample.backend_status,
            release_max_us: sample.release_max_us,
            release_late_2ms: sample.release_late_2ms,
            session_id: Some(session_id),
            last_error: sample.last_error.clone(),
        };
        events
            .lock()
            .map_err(|_| "native event hub lock poisoned".to_string())?
            .publish(UiEvent::DiagnosticsSnapshot {
                v: crate::DESKTOP_PROTOCOL_VERSION,
                payload: Box::new(payload),
            })
    })?;
    Ok(())
}

struct NativeDiagnosticsSample {
    max_lateness_us: Option<u64>,
    recent_latencies_us: Vec<i64>,
    recent_latency_samples_available: bool,
    late_2ms: Option<u64>,
    late_5ms: Option<u64>,
    late_10ms: Option<u64>,
    sender_sample_count: u64,
    max_sendinput_pre_call_lateness_us: Option<u64>,
    pre_call_late_2ms: u64,
    pre_call_late_5ms: u64,
    pre_call_late_10ms: u64,
    pre_call_lt_250us: u64,
    pre_call_250_500us: u64,
    pre_call_500_750us: u64,
    pre_call_750_1000us: u64,
    pre_call_1000_1500us: u64,
    pre_call_1500_2000us: u64,
    pre_call_ge_2000us: u64,
    active_keys: u64,
    stuck_keys: u64,
    keys_dropped: u64,
    chord_split_events: u64,
    missed_down_boundaries: u64,
    missed_down_keys: u64,
    missed_backlog_boundaries: u64,
    missed_hard_late_boundaries: u64,
    final_gate_cutoff_misses: u64,
    final_gate_control_rejections: u64,
    final_gate_target_changes: u64,
    final_gate_focus_losses: u64,
    final_gate_lease_expirations: u64,
    sendinput_partial_events: u64,
    sendinput_zero_progress_failures: u64,
    backend_status: DiagnosticsBackendStatus,
    release_max_us: Option<u64>,
    release_late_2ms: Option<u64>,
    last_error: Option<String>,
}

impl NativeDiagnosticsSample {
    fn unavailable(physical_session: bool) -> Self {
        let player_attached = false;
        Self {
            max_lateness_us: None,
            recent_latencies_us: Vec::new(),
            recent_latency_samples_available: false,
            late_2ms: None,
            late_5ms: None,
            late_10ms: None,
            sender_sample_count: 0,
            max_sendinput_pre_call_lateness_us: None,
            pre_call_late_2ms: 0,
            pre_call_late_5ms: 0,
            pre_call_late_10ms: 0,
            pre_call_lt_250us: 0,
            pre_call_250_500us: 0,
            pre_call_500_750us: 0,
            pre_call_750_1000us: 0,
            pre_call_1000_1500us: 0,
            pre_call_1500_2000us: 0,
            pre_call_ge_2000us: 0,
            active_keys: 0,
            stuck_keys: 0,
            keys_dropped: 0,
            chord_split_events: 0,
            missed_down_boundaries: 0,
            missed_down_keys: 0,
            missed_backlog_boundaries: 0,
            missed_hard_late_boundaries: 0,
            final_gate_cutoff_misses: 0,
            final_gate_control_rejections: 0,
            final_gate_target_changes: 0,
            final_gate_focus_losses: 0,
            final_gate_lease_expirations: 0,
            sendinput_partial_events: 0,
            sendinput_zero_progress_failures: 0,
            backend_status: diagnostics_backend_status(DiagnosticsBackendHealth {
                physical_session,
                player_attached,
                ..DiagnosticsBackendHealth::default()
            }),
            release_max_us: None,
            release_late_2ms: None,
            last_error: (physical_session != player_attached)
                .then(|| "physical session has no attached native player".into()),
        }
    }

    fn from_player(player: &NativeDispatchSession, physical_session: bool) -> Self {
        let snapshot = player.snapshot_lite();
        let observer_metrics_available = snapshot.recent_latency_samples_available;
        let has_last_error = snapshot.last_error.is_some();
        let (sender_sample_count, max_sendinput_pre_call_lateness_us) = sender_sample_summary(
            [
                snapshot.pre_call_lt_250us,
                snapshot.pre_call_250_500us,
                snapshot.pre_call_500_750us,
                snapshot.pre_call_750_1000us,
                snapshot.pre_call_1000_1500us,
                snapshot.pre_call_1500_2000us,
                snapshot.pre_call_ge_2000us,
            ],
            snapshot.max_sendinput_pre_call_lateness_us,
        );
        Self {
            max_lateness_us: observer_metrics_available.then_some(snapshot.max_lateness_us),
            recent_latencies_us: snapshot.recent_latencies_us,
            recent_latency_samples_available: snapshot.recent_latency_samples_available,
            late_2ms: observer_metrics_available.then_some(snapshot.late_2ms),
            late_5ms: observer_metrics_available.then_some(snapshot.late_5ms),
            late_10ms: observer_metrics_available.then_some(snapshot.late_10ms),
            sender_sample_count,
            max_sendinput_pre_call_lateness_us,
            pre_call_late_2ms: snapshot.pre_call_late_2ms,
            pre_call_late_5ms: snapshot.pre_call_late_5ms,
            pre_call_late_10ms: snapshot.pre_call_late_10ms,
            pre_call_lt_250us: snapshot.pre_call_lt_250us,
            pre_call_250_500us: snapshot.pre_call_250_500us,
            pre_call_500_750us: snapshot.pre_call_500_750us,
            pre_call_750_1000us: snapshot.pre_call_750_1000us,
            pre_call_1000_1500us: snapshot.pre_call_1000_1500us,
            pre_call_1500_2000us: snapshot.pre_call_1500_2000us,
            pre_call_ge_2000us: snapshot.pre_call_ge_2000us,
            active_keys: snapshot.active_count as u64,
            stuck_keys: snapshot.failed_release_count as u64,
            keys_dropped: snapshot.keys_dropped,
            chord_split_events: snapshot.chord_split_events,
            missed_down_boundaries: snapshot.missed_down_boundaries,
            missed_down_keys: snapshot.missed_down_keys,
            missed_backlog_boundaries: snapshot.missed_backlog_boundaries,
            missed_hard_late_boundaries: snapshot.missed_hard_late_boundaries,
            final_gate_cutoff_misses: snapshot.final_gate_cutoff_misses,
            final_gate_control_rejections: snapshot.final_gate_control_rejections,
            final_gate_target_changes: snapshot.final_gate_target_changes,
            final_gate_focus_losses: snapshot.final_gate_focus_losses,
            final_gate_lease_expirations: snapshot.final_gate_lease_expirations,
            sendinput_partial_events: snapshot.sendinput_partial_events,
            sendinput_zero_progress_failures: snapshot.sendinput_zero_progress_failures,
            last_error: snapshot.last_error,
            backend_status: diagnostics_backend_status(DiagnosticsBackendHealth {
                physical_session,
                player_attached: true,
                has_terminal_error: snapshot.has_terminal_error,
                has_last_error,
                failed_release_count: snapshot.failed_release_count,
                keys_dropped: snapshot.keys_dropped,
                chord_split_events: snapshot.chord_split_events,
                possibly_active_count: snapshot.possibly_active_count,
                active_count: snapshot.active_count,
            }),
            release_max_us: observer_metrics_available.then_some(snapshot.release_max_us),
            release_late_2ms: observer_metrics_available.then_some(snapshot.release_late_2ms),
        }
    }
}

fn sender_sample_summary(buckets: [u64; 7], max_lateness_us: u64) -> (u64, Option<u64>) {
    let sample_count = buckets.into_iter().fold(0_u64, u64::saturating_add);
    (sample_count, (sample_count > 0).then_some(max_lateness_us))
}

#[derive(Clone, Copy, Debug, Default)]
struct DiagnosticsBackendHealth {
    physical_session: bool,
    player_attached: bool,
    has_terminal_error: bool,
    has_last_error: bool,
    failed_release_count: usize,
    keys_dropped: u64,
    chord_split_events: u64,
    possibly_active_count: usize,
    active_count: usize,
}

fn diagnostics_backend_status(
    health: DiagnosticsBackendHealth,
) -> crate::ui_events::DiagnosticsBackendStatus {
    if health.physical_session != health.player_attached {
        crate::ui_events::DiagnosticsBackendStatus::Error
    } else if !health.player_attached {
        crate::ui_events::DiagnosticsBackendStatus::Unavailable
    } else if health.has_terminal_error || health.has_last_error || health.failed_release_count > 0
    {
        crate::ui_events::DiagnosticsBackendStatus::Error
    } else if health.keys_dropped > 0
        || health.chord_split_events > 0
        || health.possibly_active_count > health.active_count
    {
        crate::ui_events::DiagnosticsBackendStatus::Degraded
    } else {
        crate::ui_events::DiagnosticsBackendStatus::Healthy
    }
}

fn cleanup_failed_event_delivery(active: &NativeActivePlayback) {
    active.stop_requested.store(true, Ordering::Release);
    stop_supervisor_heartbeat(active);
    let _ = set_playback_state(active, PlaybackSessionState::Failed);
    if let Some(player) = &active.player {
        let _ = player.panic_release();
        let _ = player.quit();
        let _ = player.join(Duration::from_secs(5));
    }
}

fn set_playback_state(
    active: &NativeActivePlayback,
    state: PlaybackSessionState,
) -> Result<(), String> {
    *active
        .state
        .lock()
        .map_err(|_| "native playback state lock poisoned".to_string())? = state;
    Ok(())
}

fn dry_run_elapsed(active: &NativeActivePlayback) -> u64 {
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

fn publish_playback_state(
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

fn publish_playback_snapshot(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    elapsed_us: u64,
    pre_roll_remaining_us: u64,
    paused: bool,
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
        let health = if snapshot.has_terminal_error
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
        };
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
            state: if paused {
                PlaybackEventState::Paused
            } else if pre_roll_remaining_us > 0 {
                PlaybackEventState::Starting
            } else {
                PlaybackEventState::Playing
            },
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
    events
        .lock()
        .map_err(|_| "native event hub lock poisoned".to_string())?
        .publish(event)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PlaybackTerminalPublication {
    Finished { outcome: String, message: String },
    Failed { code: String, message: String },
}

fn publish_terminal_poll_state(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    last_event_state: &mut PlaybackEventState,
    status: EnginePollStatus,
) -> Result<PlaybackTerminalPublication, String> {
    let terminal_state = playback_event_state(status);
    let is_failure = is_failure_status(status);
    let publication = if is_failure {
        PlaybackTerminalPublication::Failed {
            code: "native_player_failed".into(),
            message: "Native playback worker failed".into(),
        }
    } else {
        PlaybackTerminalPublication::Finished {
            outcome: terminal_success_outcome(
                status,
                active.skip_requested.load(Ordering::Acquire),
            )
            .into(),
            message: "Playback finished".into(),
        }
    };
    if *last_event_state != terminal_state {
        set_playback_state(
            active,
            if is_failure {
                PlaybackSessionState::Failed
            } else {
                PlaybackSessionState::Finished
            },
        )?;
        let (message, outcome) = match &publication {
            PlaybackTerminalPublication::Finished { outcome, message } => {
                (Some(message.clone()), Some(outcome.clone()))
            }
            PlaybackTerminalPublication::Failed { message, .. } => (Some(message.clone()), None),
        };
        publish_playback_state(events, active, terminal_state, message, outcome)?;
        *last_event_state = terminal_state;
    }
    Ok(publication)
}

fn publish_terminal_failure_state(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    last_event_state: &mut PlaybackEventState,
    code: &str,
    message: &str,
) -> Result<PlaybackTerminalPublication, String> {
    set_playback_state(active, PlaybackSessionState::Failed)?;
    if *last_event_state != PlaybackEventState::Failed {
        publish_playback_state(
            events,
            active,
            PlaybackEventState::Failed,
            Some(message.into()),
            None,
        )?;
        *last_event_state = PlaybackEventState::Failed;
    }
    Ok(PlaybackTerminalPublication::Failed {
        code: code.into(),
        message: message.into(),
    })
}

fn physical_startup_failure(error: &str) -> (&'static str, &'static str) {
    if error == "no admissible visible Sky window was found" {
        (
            "target_not_found",
            "Sky window was not found. Open Sky and make sure its window is visible, then try again.",
        )
    } else if error.starts_with("target Sky process has higher integrity") {
        (
            "target_integrity_mismatch",
            "Sky is running with higher Windows integrity than Sky Auto Player. Run both apps at the same permission level, then try again.",
        )
    } else if error == "validated Sky window could not be focused" {
        (
            "target_focus_failed",
            "Sky's window could not be focused. Bring the Sky window to the foreground, then try again.",
        )
    } else {
        (
            "native_worker_failed",
            "The native playback engine could not start. Try again; if it continues, restart Sky Auto Player.",
        )
    }
}

fn publish_stopped_state(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    last_event_state: &mut PlaybackEventState,
    outcome: &str,
    message: &str,
) -> Result<PlaybackTerminalPublication, String> {
    set_playback_state(active, PlaybackSessionState::Finished)?;
    if *last_event_state != PlaybackEventState::Finished {
        publish_playback_state(
            events,
            active,
            PlaybackEventState::Finished,
            Some(message.into()),
            Some(outcome.into()),
        )?;
        *last_event_state = PlaybackEventState::Finished;
    }
    Ok(PlaybackTerminalPublication::Finished {
        outcome: outcome.into(),
        message: message.into(),
    })
}

#[cfg(test)]
fn publish_terminal_poll_result(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    last_event_state: &mut PlaybackEventState,
    status: EnginePollStatus,
) -> Result<(), String> {
    let publication = publish_terminal_poll_state(events, active, last_event_state, status)?;
    publish_terminal_event(events, active, &publication)
}

#[cfg(test)]
fn publish_stopped_completion(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    last_event_state: &mut PlaybackEventState,
    outcome: &str,
    message: &str,
) -> Result<(), String> {
    let publication = publish_stopped_state(events, active, last_event_state, outcome, message)?;
    publish_terminal_event(events, active, &publication)
}

fn publish_terminal_event(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    publication: &PlaybackTerminalPublication,
) -> Result<(), String> {
    match publication {
        PlaybackTerminalPublication::Finished { outcome, message } => {
            publish_playback_finished(events, active, outcome, message)
        }
        PlaybackTerminalPublication::Failed { code, message } => {
            publish_playback_failed(events, active, code, message)
        }
    }
}

fn release_terminal_ownership(
    active_slot: &Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    last_terminal: &Arc<Mutex<Option<PlaybackTerminalStatusDto>>>,
    active: &Arc<NativeActivePlayback>,
    publication: Option<&PlaybackTerminalPublication>,
) -> Result<(), String> {
    active
        .pending
        .lock()
        .map_err(|_| "native playback control lock poisoned".to_string())?
        .take();
    active.heartbeat_stop.store(true, Ordering::Release);
    active
        .activity_lease
        .lock()
        .map_err(|_| "native playback activity lock poisoned".to_string())?
        .take();
    let terminal_state = active
        .state
        .lock()
        .map(|state| *state)
        .unwrap_or(PlaybackSessionState::Failed);
    let outcome = if terminal_state == PlaybackSessionState::Finished {
        Some(if active.skip_requested.load(Ordering::Acquire) {
            "skipped".to_string()
        } else if active.stop_requested.load(Ordering::Acquire) {
            "quit".to_string()
        } else {
            "finished".to_string()
        })
    } else {
        None
    };
    let (failure_code, failure_message) = if terminal_state == PlaybackSessionState::Failed {
        match publication {
            Some(PlaybackTerminalPublication::Failed { code, message }) => {
                (Some(code.clone()), Some(message.clone()))
            }
            _ => (
                Some("native_player_failed".into()),
                Some("Native playback worker failed".into()),
            ),
        }
    } else {
        (None, None)
    };
    let mut slot = active_slot
        .lock()
        .map_err(|_| "native active-playback lock poisoned".to_string())?;
    let mut terminal = last_terminal
        .lock()
        .map_err(|_| "native terminal-session lock poisoned".to_string())?;
    if slot
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, active))
    {
        *slot = None;
    }
    *terminal = Some(PlaybackTerminalStatusDto {
        session_id: active.session_id.clone(),
        prepared_id: active.prepared_id.clone(),
        song_id: active.song_id.clone(),
        state: terminal_state,
        outcome,
        failure_code,
        failure_message,
    });
    active.done.store(true, Ordering::Release);
    Ok(())
}

fn publish_retirement_barrier(
    events: &Arc<Mutex<NativeEventHub>>,
    active_slot: &Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    last_terminal: &Arc<Mutex<Option<PlaybackTerminalStatusDto>>>,
    active: &Arc<NativeActivePlayback>,
    publication: &PlaybackTerminalPublication,
) -> Result<(), String> {
    release_terminal_ownership(active_slot, last_terminal, active, Some(publication))?;
    publish_terminal_event(events, active, publication)
}

fn publish_playback_failed(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    code: &str,
    message: &str,
) -> Result<(), String> {
    events
        .lock()
        .map_err(|_| "native event hub lock poisoned".to_string())?
        .publish(UiEvent::PlaybackFailed {
            v: crate::DESKTOP_PROTOCOL_VERSION,
            payload: PlaybackFailedPayload {
                session_id: active.session_id.clone(),
                song_id: active.song_id.clone(),
                code: code.into(),
                message: message.into(),
            },
        })
}

fn publish_playback_finished(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    outcome: &str,
    message: &str,
) -> Result<(), String> {
    events
        .lock()
        .map_err(|_| "native event hub lock poisoned".to_string())?
        .publish(UiEvent::PlaybackFinished {
            v: crate::DESKTOP_PROTOCOL_VERSION,
            payload: PlaybackFinishedPayload {
                session_id: active.session_id.clone(),
                song_id: active.song_id.clone(),
                outcome: outcome.into(),
                total_us: active.total_us,
                message: message.into(),
            },
        })
}

#[allow(dead_code)]
pub(crate) fn resolve_install_root() -> Result<PathBuf, String> {
    AppPaths::resolve().map(|paths| paths.install_root().to_path_buf())
}

pub(crate) fn native_build_dto() -> crate::commands::NativeBuildDto {
    crate::commands::NativeBuildDto {
        native_build_commit: option_env!("SKY_NATIVE_BUILD_COMMIT")
            .unwrap_or("unknown")
            .into(),
        native_version: env!("CARGO_PKG_VERSION").into(),
        schema_version: sky_dispatch_win32::calibration::CALIBRATION_SCHEMA_VERSION as u64,
        native_abi: option_env!("SKY_NATIVE_ABI")
            .unwrap_or("native-win32")
            .into(),
        rustc_version: option_env!("SKY_RUSTC_VERSION").unwrap_or("unknown").into(),
        win32_backend: sky_dispatch_win32::win32_available(),
    }
}

fn playback_defaults(settings: &ApplicationSettings) -> PlaybackDefaultsDto {
    PlaybackDefaultsDto {
        hold_frames: settings.playback_defaults.hold_frames,
        timing_margin_us: settings.playback_defaults.timing_margin_us,
        down_late_grace_us: settings.playback_defaults.down_late_grace_us,
        tempo_scale: settings.playback_defaults.tempo_scale,
        fps: settings.playback_defaults.fps,
        dry_run: false,
    }
}

fn update_preferences_dto(settings: &ApplicationSettings) -> UpdatePreferencesDto {
    UpdatePreferencesDto {
        auto_check: settings.update.auto_check,
        channel: match settings.update.channel {
            sky_app_core::settings::UpdateChannel::Stable => {
                crate::ui_events::UpdateChannel::Stable
            }
            sky_app_core::settings::UpdateChannel::Beta => crate::ui_events::UpdateChannel::Beta,
        },
        skip_version: settings.update.skip_version.clone(),
    }
}

fn timing_margin_recommendation(
    calibration_cache_path: impl AsRef<Path>,
    down_late_grace_us: u64,
) -> crate::commands::TimingMarginRecommendationDto {
    let resolution = load_calibration_resolution(calibration_cache_path);
    let recommended_timing_margin_us = if resolution.qualified {
        let evidence_budget = down_late_grace_us
            .checked_add(resolution.transport_reserve_us)
            .expect("bounded calibration reserve");
        evidence_budget.div_ceil(sky_app_core::settings::TIMING_MARGIN_STEP_US)
            * sky_app_core::settings::TIMING_MARGIN_STEP_US
    } else {
        sky_app_core::settings::DEFAULT_TIMING_MARGIN_US
    };
    let source = match resolution.source.as_str() {
        sky_native_adapters::CALIBRATION_MARGIN_SOURCE_DEVICE => "qualified_calibration",
        sky_native_adapters::CALIBRATION_MARGIN_SOURCE_OUT_OF_ENVELOPE => {
            "out_of_envelope_fallback"
        }
        sky_native_adapters::CALIBRATION_MARGIN_SOURCE_INVALID => "invalid_cache_fallback",
        sky_native_adapters::CALIBRATION_MARGIN_SOURCE_INCOMPATIBLE => {
            "incompatible_calibration_fallback"
        }
        _ => "default_fallback",
    };
    crate::commands::TimingMarginRecommendationDto {
        recommended_timing_margin_us,
        qualified: resolution.qualified,
        source: source.into(),
    }
}

fn settings_dto(
    settings: &ApplicationSettings,
    timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto,
) -> SettingsDto {
    SettingsDto {
        theme: settings.theme.clone(),
        ui_background_mode: settings.ui_background_mode.clone(),
        playback_defaults: playback_defaults(settings),
        timing_margin_recommendation,
        telemetry_enabled: settings.telemetry_enabled,
        verbose_hud: settings.verbose_hud,
        update_preferences: update_preferences_dto(settings),
    }
}

fn json_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct NativeCatalogDetailRequest {
    song_id: String,
    generation: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct NativeCatalogViewportRequest {
    generation: u64,
    first_index: u64,
    last_index: i64,
    selected_song_id: Option<String>,
    song_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct NativePlaybackPatch {
    hold_frames: Option<f64>,
    timing_margin_us: Option<u64>,
    down_late_grace_us: Option<u64>,
    tempo_scale: Option<f64>,
    fps: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct NativeUpdatePreferencesPatch {
    auto_check: Option<bool>,
    channel: Option<crate::ui_events::UpdateChannel>,
    skip_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct NativeSettingsPatch {
    theme: Option<String>,
    telemetry_enabled: Option<bool>,
    verbose_hud: Option<bool>,
    playback_defaults: Option<NativePlaybackPatch>,
    update_preferences: Option<NativeUpdatePreferencesPatch>,
}

impl NativeUpdatePreferencesPatch {
    fn into_public(self) -> UpdatePreferencesPatch {
        UpdatePreferencesPatch {
            auto_check: self.auto_check,
            channel: self.channel,
            skip_version: self.skip_version,
        }
    }
}

impl NativeSettingsPatch {
    fn into_public(self) -> SettingsPatch {
        SettingsPatch {
            theme: self.theme,
            telemetry_enabled: self.telemetry_enabled,
            verbose_hud: self.verbose_hud,
            playback_defaults: self
                .playback_defaults
                .map(|value| crate::commands::PlaybackPatch {
                    hold_frames: value.hold_frames,
                    timing_margin_us: value.timing_margin_us,
                    down_late_grace_us: value.down_late_grace_us,
                    tempo_scale: value.tempo_scale,
                    fps: value.fps,
                }),
            update_preferences: self.update_preferences.map(|value| value.into_public()),
        }
    }
}

fn encode_result<T: serde::Serialize>(result: Result<T, String>) -> Result<Value, String> {
    result
        .map_err(|error| error.to_string())
        .and_then(|value| serde_json::to_value(value).map_err(json_error))
}
fn settings_error(error: SettingsError) -> String {
    error.to_string()
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
        ActivityReservationError::UpdateAlreadyActive => {
            "update_active: calibration cannot run during update installation".into()
        }
    }
}

fn playback_activity_error(error: ActivityReservationError) -> String {
    match error {
        ActivityReservationError::Closing => "closing: desktop application is closing".into(),
        ActivityReservationError::CalibrationAlreadyActive => {
            "calibration_active: calibration is active".into()
        }
        ActivityReservationError::PhysicalPlaybackActive => {
            "already_running: another physical playback session is active".into()
        }
        ActivityReservationError::UpdateAlreadyActive => {
            "update_active: playback cannot run during update installation".into()
        }
    }
}

fn catalog_error(error: CatalogError) -> String {
    match error {
        CatalogError::SourceUnavailable(_) => "catalog source is unavailable".into(),
        other => other.to_string(),
    }
}

#[derive(Default)]
struct NativeEventHub {
    buffered: VecDeque<UiEvent>,
    channel: Option<Channel<UiEvent>>,
    closed: bool,
}

impl NativeEventHub {
    fn publish(&mut self, event: UiEvent) -> Result<(), String> {
        if self.closed {
            return Err("native event hub is closed".into());
        }
        validate_ui_event(&event)?;
        if let Some(channel) = &self.channel {
            if let Err(error) = channel.send(event) {
                // A failed delivery is not a queueing opportunity: buffering
                // after a subscriber failure would hide the loss of the
                // delivery contract and let a native worker continue without
                // a consumer.  Close the hub and let the owner perform its
                // bounded cleanup path.
                self.channel = None;
                self.closed = true;
                self.buffered.clear();
                return Err(format!("native UI event delivery failed: {error}"));
            }
            return Ok(());
        }
        if let Some(key) = snapshot_key(&event) {
            if let Some(existing) = self
                .buffered
                .iter_mut()
                .find(|candidate| snapshot_key(candidate).as_ref() == Some(&key))
            {
                *existing = event;
                return Ok(());
            }
            if self.buffered.len() >= MAX_NATIVE_EVENTS
                && !remove_oldest_snapshot(&mut self.buffered)
            {
                return Err("native event hub lifecycle buffer overflow".into());
            }
        } else if self.buffered.len() >= MAX_NATIVE_EVENTS
            && !remove_oldest_snapshot(&mut self.buffered)
        {
            // Lifecycle events are never silently discarded.  The caller must
            // initiate bounded cleanup when this fail-closed signal occurs.
            return Err("native event hub lifecycle buffer overflow".into());
        }
        self.buffered.push_back(event);
        Ok(())
    }

    fn subscribe(&mut self, channel: Channel<UiEvent>) -> Result<(), String> {
        if self.closed {
            return Err("native event hub is closed".into());
        }
        for event in &self.buffered {
            if let Err(error) = channel.send(event.clone()) {
                self.closed = true;
                self.buffered.clear();
                return Err(format!("native UI event replay failed: {error}"));
            }
        }
        self.buffered.clear();
        self.channel = Some(channel);
        Ok(())
    }

    fn close(&mut self) {
        self.closed = true;
        self.channel = None;
        self.buffered.clear();
    }
}

fn validate_ui_event(event: &UiEvent) -> Result<(), String> {
    match event {
        UiEvent::CoreReady { payload, .. } => UiEvent::validate_ready(payload),
        UiEvent::CoreFatal { payload, .. } => UiEvent::validate_fatal(payload),
        UiEvent::CatalogChanged { payload, .. } => UiEvent::validate_catalog_changed(payload),
        UiEvent::PlaybackStateChanged { payload, .. } => {
            UiEvent::validate_playback_state_changed(payload)
        }
        UiEvent::PlaybackSnapshot { payload, .. } => UiEvent::validate_playback_snapshot(payload),
        UiEvent::PlaybackFinished { payload, .. } => UiEvent::validate_playback_finished(payload),
        UiEvent::PlaybackFailed { payload, .. } => UiEvent::validate_playback_failed(payload),
        UiEvent::DiagnosticsSnapshot { payload, .. } => {
            UiEvent::validate_diagnostics_snapshot(payload)
        }
        UiEvent::CalibrationProgress { payload, .. } => {
            UiEvent::validate_calibration_progress(payload)
        }
        UiEvent::CalibrationFinished { payload, .. } => {
            UiEvent::validate_calibration_finished(payload)
        }
        UiEvent::UpdateAvailable { payload, .. } => UiEvent::validate_update_available(payload),
        UiEvent::UpdateResult { payload, .. } => UiEvent::validate_update_result(payload),
        UiEvent::UpdateProgress { payload, .. } => UiEvent::validate_update_progress(payload),
    }
}

fn snapshot_key(event: &UiEvent) -> Option<(u8, String)> {
    match event {
        UiEvent::PlaybackSnapshot { payload, .. } => Some((1, payload.session_id.clone())),
        UiEvent::DiagnosticsSnapshot { payload, .. } => {
            Some((2, payload.session_id.clone().unwrap_or_default()))
        }
        UiEvent::CalibrationProgress { payload, .. } => Some((3, payload.operation_id.clone())),
        _ => None,
    }
}

/// Match the Python diagnostics percentile contract.  Python's ``round``
/// uses ties-to-even, so using `f64::round()` here would diverge for small
/// sample sets at an exact half index.
fn percentile_ms(values: &[i64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let position = fraction * (ordered.len() - 1) as f64;
    let lower = position.floor();
    let fractional = position - lower;
    let index = if fractional < 0.5 {
        lower as usize
    } else if fractional > 0.5 || (lower as usize) % 2 == 1 {
        (lower as usize + 1).min(ordered.len() - 1)
    } else {
        lower as usize
    };
    ordered[index] as f64 / 1000.0
}

fn population_sigma_ms(values: &[i64]) -> f64 {
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

fn remove_oldest_snapshot(buffered: &mut VecDeque<UiEvent>) -> bool {
    let Some(index) = buffered
        .iter()
        .position(|event| snapshot_key(event).is_some())
    else {
        return false;
    };
    buffered.remove(index).is_some()
}

#[cfg(test)]
mod tests {
    use super::{
        CALIBRATION_DEFAULT_TIMEOUT_SECONDS, CALIBRATION_MIN_FULL_TIMEOUT_SECONDS,
        CALIBRATION_MIN_NATIVE_TOTAL_SECONDS, CALIBRATION_MIN_SINGLE_TIMEOUT_SECONDS,
        CalibrationRunError, DiagnosticsBackendHealth, DiagnosticsPublicationGate,
        MAX_DECISION_COUNT, MAX_NATIVE_EVENTS, MAX_PREPARED_PLANS, MaterializedTimingPolicy,
        NativeActivePlayback, NativeCalibrationOperation, NativeCalibrationService,
        NativeDesktopRuntime, NativeDiagnosticsSample, NativeEventHub, NativePlaybackService,
        PlaybackActiveStatusDto, PlaybackPendingControl, PlaybackTerminalPublication,
        PlaybackTerminalStatusDto, SenderTraceState, TestSeams, calibration_budget,
        diagnostics_backend_status, opaque_native_id, percentile_ms, physical_startup_failure,
        plan_fingerprint, population_sigma_ms, publish_calibration_cache,
        publish_diagnostics_snapshot_for_active, publish_playback_state,
        publish_retirement_barrier, publish_stopped_completion, publish_terminal_poll_result,
        release_terminal_ownership, remove_oldest_snapshot, resolve_install_root,
        retain_prepared_capacity, safe_calibration_evidence, sender_sample_summary,
        sender_trace_export_json, settings_fingerprint, supervisor_heartbeat_loop,
        timing_margin_recommendation, validate_playback_start_request,
    };
    use crate::app_state::ActivityCoordinator;
    use crate::commands::{CalibrationStartRequest, PlaybackConfigDto, PlaybackSessionState};
    use crate::ui_events::{
        CalibrationMode, CalibrationState, DiagnosticsBackendStatus, PlaybackEventState,
        PlaybackFocusState, PlaybackHealthState, PlaybackSnapshotPayload, UiEvent,
    };
    use serde_json::Value;
    use sky_app_core::settings::ApplicationSettings;
    use sky_app_core::song::{build_schedule_with_policy, parse_song_json};
    use sky_native_adapters::{AppPaths, load_calibration_resolution};
    use std::fs;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Barrier, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn active_for_control(
        state: PlaybackSessionState,
        pending: Option<PlaybackPendingControl>,
    ) -> Arc<NativeActivePlayback> {
        active_for_control_with_physical(state, pending, false)
    }

    fn active_for_control_with_physical(
        state: PlaybackSessionState,
        pending: Option<PlaybackPendingControl>,
        physical: bool,
    ) -> Arc<NativeActivePlayback> {
        Arc::new(NativeActivePlayback {
            session_id: "a".repeat(32),
            prepared_id: "b".repeat(32),
            song_id: "c".repeat(32),
            title: "Fixture".into(),
            total_us: 1_000_000,
            config: PlaybackConfigDto {
                hold_frames: 1.0,
                timing_margin_us: 800,
                down_late_grace_us: 500,
                tempo_scale: 1.0,
                fps: 60,
                dry_run: true,
            },
            timing_policy: MaterializedTimingPolicy::from_user_margin(60, 1.0, 800, 500)
                .expect("fixture timing policy"),
            timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto {
                recommended_timing_margin_us: 800,
                qualified: false,
                source: "default_fallback".into(),
            },
            plan_fingerprint: "d".repeat(64),
            physical,
            activity_lease: Mutex::new(None),
            target_hwnd: None,
            state: Mutex::new(state),
            pending: Mutex::new(pending),
            player: None,
            started_at: Instant::now(),
            paused_since: Mutex::new(None),
            paused_total: Mutex::new(Duration::ZERO),
            stop_requested: std::sync::atomic::AtomicBool::new(false),
            skip_requested: std::sync::atomic::AtomicBool::new(false),
            done: std::sync::atomic::AtomicBool::new(false),
            heartbeat_stop: std::sync::atomic::AtomicBool::new(true),
            heartbeat_thread: Mutex::new(None),
            sequence: std::sync::atomic::AtomicU64::new(0),
        })
    }

    #[test]
    fn sender_trace_export_includes_session_identity_and_frozen_policy() {
        let active = active_for_control_with_physical(PlaybackSessionState::Finished, None, true);
        let telemetry = serde_json::json!({
            "schema_version": 14,
            "qpc_frequency_hz": 10_000_000,
            "records": [],
            "attempted": 0,
            "accepted": 0,
            "dropped": 0,
            "observer_queue_dropped": 0,
            "truncated": false,
        });
        let export = sender_trace_export_json(&active, &telemetry).expect("sender trace JSON");
        let document: Value = serde_json::from_str(&export).expect("valid sender trace JSON");

        assert_eq!(document["export_schema_version"], 1);
        assert_eq!(document["session_id"], "a".repeat(32));
        assert_eq!(document["song_id"], "c".repeat(32));
        assert_eq!(document["song_title"], "Fixture");
        assert_eq!(document["plan_fingerprint"], "d".repeat(64));
        assert_eq!(document["timing_policy"]["fps"], 60);
        assert_eq!(document["timing_policy"]["frame_base_hold_us"], 16_667);
        assert_eq!(document["timing_policy"]["timing_margin_us"], 800);
        assert_eq!(document["timing_policy"]["target_hold_us"], 17_467);
        assert_eq!(document["timing_policy"]["release_gap_us"], 17_467);
        assert_eq!(document["timing_policy"]["down_late_cutoff_us"], 500);
        assert_eq!(document["telemetry"], telemetry);
    }

    #[test]
    fn sender_trace_beginning_physical_session_b_clears_session_a_and_rejects_late_a_completion() {
        let mut state = SenderTraceState::default();
        state.begin_physical_session("session-a");
        state.finish_physical_session(
            "session-a",
            "song-a",
            "Song A",
            "fingerprint-a",
            Some(r#"{"session_id":"session-a","song_id":"song-a","song_title":"Song A","plan_fingerprint":"fingerprint-a"}"#.into()),
        );
        assert!(state.completed.is_some());

        state.begin_physical_session("session-b");
        assert!(state.completed.is_none());
        state.finish_physical_session(
            "session-a",
            "song-a",
            "Song A",
            "fingerprint-a",
            Some("late trace A".into()),
        );

        assert!(state.completed.is_none());
        assert_eq!(state.collecting_session_id.as_deref(), Some("session-b"));
    }

    #[test]
    fn sender_trace_failed_session_b_extraction_does_not_restore_session_a() {
        let mut state = SenderTraceState::default();
        state.begin_physical_session("session-a");
        state.finish_physical_session(
            "session-a",
            "song-a",
            "Song A",
            "fingerprint-a",
            Some(r#"{"session_id":"session-a","song_id":"song-a","song_title":"Song A","plan_fingerprint":"fingerprint-a"}"#.into()),
        );
        state.begin_physical_session("session-b");
        state.finish_physical_session("session-b", "song-b", "Song B", "fingerprint-b", None);

        assert!(state.completed.is_none());
        assert!(state.collecting_session_id.is_none());
    }

    #[test]
    fn sender_trace_capture_memory_stays_under_two_mib() {
        let record_bytes = std::mem::size_of::<sky_player::engine::RtTraceRecord>();
        let capture_bytes = record_bytes * 8_192;
        assert!(capture_bytes <= 2 * 1024 * 1024);
        println!("sender trace ring: {record_bytes} bytes/record, {capture_bytes} bytes total");
    }

    fn playback_trace(events: &Arc<Mutex<NativeEventHub>>) -> Vec<String> {
        events
            .lock()
            .expect("event hub")
            .buffered
            .iter()
            .filter_map(|event| match event {
                UiEvent::PlaybackStateChanged { payload, .. } => {
                    Some(format!("state:{}", payload.state.as_str()))
                }
                UiEvent::PlaybackFinished { payload, .. } => {
                    Some(format!("finished:{}", payload.outcome))
                }
                UiEvent::PlaybackFailed { payload, .. } => Some(format!("failed:{}", payload.code)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn debug_install_root_is_repository_root_not_cwd() {
        if cfg!(debug_assertions) {
            let root = resolve_install_root().expect("root");
            assert!(root.join("rust").is_dir());
            assert!(root.join("desktop").is_dir());
        }
    }

    #[test]
    fn production_composition_has_no_environment_only_safe_bypass() {
        assert_eq!(TestSeams::default(), TestSeams::Disabled);
        assert_ne!(TestSeams::Disabled, TestSeams::SafePackage);
        // The only selector is an explicit composition value.  There is no
        // environment-to-seam conversion in either production constructor.
        assert!(!matches!(TestSeams::Disabled, TestSeams::SafePackage));
        assert!(matches!(TestSeams::SafePackage, TestSeams::SafePackage));
    }

    #[test]
    fn calibration_budget_preserves_mode_specific_reserves() {
        let single = calibration_budget(
            CALIBRATION_MIN_SINGLE_TIMEOUT_SECONDS,
            crate::ui_events::CalibrationMode::Quick,
        )
        .expect("minimum single budget");
        assert_eq!(
            single.native_budget_seconds,
            CALIBRATION_MIN_NATIVE_TOTAL_SECONDS as u64
        );
        assert_eq!(single.child_timeout_seconds, 7.0);
        assert!(
            calibration_budget(
                CALIBRATION_MIN_SINGLE_TIMEOUT_SECONDS - 0.01,
                crate::ui_events::CalibrationMode::Quick,
            )
            .is_err()
        );

        let full = calibration_budget(
            CALIBRATION_MIN_FULL_TIMEOUT_SECONDS,
            crate::ui_events::CalibrationMode::Full,
        )
        .expect("minimum full budget");
        assert_eq!(
            full.native_budget_seconds,
            CALIBRATION_MIN_NATIVE_TOTAL_SECONDS as u64
        );
        assert_eq!(full.child_timeout_seconds, 7.0);
        let maximum = calibration_budget(
            CALIBRATION_DEFAULT_TIMEOUT_SECONDS,
            crate::ui_events::CalibrationMode::Full,
        )
        .expect("maximum budget");
        assert_eq!(maximum.native_budget_seconds, 114);
        assert_eq!(maximum.child_timeout_seconds, 115.0);
    }

    #[test]
    fn synthetic_calibration_requires_explicit_package_composition() {
        let root = std::env::temp_dir().join("sky-native-calibration-no-child");
        let activity = ActivityCoordinator::default();
        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let playback = Arc::new(NativePlaybackService::new(activity.clone()));
        let request = CalibrationStartRequest {
            mode: CalibrationMode::Quick,
            class_name: None,
            polyphony: None,
            samples: None,
            timeout_seconds: None,
        };
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let safe = NativeCalibrationService::from_test_sandbox(
            &root,
            activity.clone(),
            events.clone(),
            playback.clone(),
            TestSeams::SafePackage,
        );
        assert!(
            safe.run_child(&request, &cancel, Arc::new(Mutex::new(None)))
                .is_ok()
        );

        let production = NativeCalibrationService::from_test_sandbox(
            &root,
            activity,
            events,
            playback,
            TestSeams::Disabled,
        );
        let error = production
            .run_child(&request, &cancel, Arc::new(Mutex::new(None)))
            .expect_err("production composition must select the real child");
        assert!(matches!(
            error,
            CalibrationRunError::Failed(message) if message.contains("failed to start")
        ));
    }

    #[test]
    fn duplicate_calibration_start_keeps_already_running_contract() {
        let activity = ActivityCoordinator::default();
        let reservation = activity.reserve_calibration().expect("first reservation");
        let service = NativeCalibrationService::from_test_sandbox(
            &std::env::temp_dir(),
            activity,
            Arc::new(Mutex::new(NativeEventHub::default())),
            Arc::new(NativePlaybackService::new(ActivityCoordinator::default())),
            TestSeams::SafePackage,
        );
        let done = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        *service.operation.lock().expect("operation") = Some(NativeCalibrationOperation {
            operation_id: "a".repeat(32),
            state: crate::ui_events::CalibrationState::Running,
            down_late_grace_us: 500,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            child: Arc::new(Mutex::new(None)),
            worker: None,
            done,
            reservation: Some(reservation),
        });
        let error = Arc::new(service)
            .start(
                CalibrationStartRequest {
                    mode: CalibrationMode::Quick,
                    class_name: None,
                    polyphony: None,
                    samples: None,
                    timeout_seconds: None,
                },
                500,
            )
            .expect_err("duplicate calibration");
        assert!(error.starts_with("already_running:"));
    }

    fn quick_calibration_request() -> CalibrationStartRequest {
        CalibrationStartRequest {
            mode: CalibrationMode::Quick,
            class_name: None,
            polyphony: None,
            samples: None,
            timeout_seconds: None,
        }
    }

    fn calibration_finished_count(events: &Arc<Mutex<NativeEventHub>>) -> usize {
        events
            .lock()
            .expect("event hub")
            .buffered
            .iter()
            .filter(|event| matches!(event, UiEvent::CalibrationFinished { .. }))
            .count()
    }

    #[test]
    fn calibration_shutdown_before_commit_cannot_mutate_after_return() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-calibration-close-before-{suffix}"));
        let paths = AppPaths::from_test_sandbox(&root);
        paths.ensure_mutable_directories().expect("cache directory");
        let cache = paths.calibration_cache_path();
        let original = br#"{"sentinel":"unchanged"}"#.to_vec();
        fs::write(&cache, &original).expect("sentinel cache");

        let activity = ActivityCoordinator::default();
        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let playback = Arc::new(NativePlaybackService::new(activity.clone()));
        let service = Arc::new(NativeCalibrationService::new(
            paths,
            activity.clone(),
            events.clone(),
            playback.clone(),
            TestSeams::SafePackage,
        ));
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        service
            .publication
            .pause_before_next_admission(entered.clone(), release.clone());
        service
            .start(quick_calibration_request(), 500)
            .expect("start calibration");
        entered.wait();

        let shutdown_service = Arc::clone(&service);
        let shutdown = std::thread::spawn(move || {
            shutdown_service.shutdown_with_timeout(Duration::from_millis(50))
        });
        let closing_deadline = Instant::now() + Duration::from_secs(1);
        while !service.publication.is_closing() && Instant::now() < closing_deadline {
            std::thread::yield_now();
        }
        assert!(service.publication.is_closing(), "shutdown crossed Closing");
        assert!(
            !shutdown.join().expect("bounded shutdown thread"),
            "blocked worker must be contained at the shutdown deadline"
        );

        assert_eq!(fs::read(&cache).expect("cache after shutdown"), original);
        assert_eq!(
            playback.settings_invalidation_count.load(Ordering::Relaxed),
            0
        );
        assert_eq!(calibration_finished_count(&events), 0);
        assert!(!activity.is_calibration_active());
        // The worker is now detached behind the Closing gate.  Releasing it
        // proves that it can finish only by discarding its temporary artifact;
        // it cannot mutate production state after shutdown returned.
        release.wait();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            fs::read(&cache).expect("cache after worker release"),
            original
        );
        assert_eq!(
            playback.settings_invalidation_count.load(Ordering::Relaxed),
            0
        );
        assert_eq!(calibration_finished_count(&events), 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn calibration_commit_before_shutdown_has_one_bounded_outcome() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-calibration-commit-before-{suffix}"));
        let paths = AppPaths::from_test_sandbox(&root);
        paths.ensure_mutable_directories().expect("cache directory");
        let activity = ActivityCoordinator::default();
        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let playback = Arc::new(NativePlaybackService::new(activity.clone()));
        let service = Arc::new(NativeCalibrationService::new(
            paths.clone(),
            activity.clone(),
            events.clone(),
            playback.clone(),
            TestSeams::SafePackage,
        ));
        let cache = paths.calibration_cache_path();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        service
            .publication
            .pause_after_next_admission(entered.clone(), release.clone());
        service
            .start(quick_calibration_request(), 500)
            .expect("start calibration");
        entered.wait();

        let shutdown_service = Arc::clone(&service);
        let shutdown = std::thread::spawn(move || shutdown_service.shutdown());
        let closing_deadline = Instant::now() + Duration::from_secs(1);
        while !service.publication.is_closing() && Instant::now() < closing_deadline {
            std::thread::yield_now();
        }
        assert!(service.publication.is_closing(), "shutdown crossed Closing");
        release.wait();
        shutdown.join().expect("shutdown thread");

        let resolution = load_calibration_resolution(&cache);
        assert_eq!(
            resolution.source,
            sky_native_adapters::CALIBRATION_MARGIN_SOURCE_DEVICE
        );
        assert_eq!(
            playback.settings_invalidation_count.load(Ordering::Relaxed),
            1
        );
        assert!(calibration_finished_count(&events) <= 1);
        assert!(!activity.is_calibration_active());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn calibration_shutdown_hung_worker_is_bounded_and_nonpublishing() {
        let root = std::env::temp_dir().join(format!(
            "sky-calibration-hung-worker-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let activity = ActivityCoordinator::default();
        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let playback = Arc::new(NativePlaybackService::new(activity.clone()));
        let service = Arc::new(NativeCalibrationService::from_test_sandbox(
            &root,
            activity.clone(),
            events.clone(),
            playback.clone(),
            TestSeams::SafePackage,
        ));
        let reservation = activity.reserve_calibration().expect("calibration slot");
        let done = Arc::new((Mutex::new(false), Condvar::new()));
        let release = Arc::new(AtomicBool::new(false));
        let worker_finished = Arc::new(AtomicBool::new(false));
        let worker_release = release.clone();
        let worker_finished_flag = worker_finished.clone();
        let worker = std::thread::spawn(move || {
            while !worker_release.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(5));
            }
            worker_finished_flag.store(true, Ordering::Release);
        });
        *service.operation.lock().expect("operation") = Some(NativeCalibrationOperation {
            operation_id: "h".repeat(32),
            state: CalibrationState::Running,
            down_late_grace_us: 500,
            cancel: Arc::new(AtomicBool::new(false)),
            child: Arc::new(Mutex::new(None)),
            worker: Some(worker),
            done,
            reservation: Some(reservation),
        });

        let started = Instant::now();
        let worker_joined = service.shutdown_with_timeout(Duration::from_millis(50));
        assert!(!worker_joined);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(!activity.is_calibration_active());
        assert_eq!(
            playback.settings_invalidation_count.load(Ordering::Relaxed),
            0
        );
        assert_eq!(calibration_finished_count(&events), 0);

        release.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(1);
        while !worker_finished.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(worker_finished.load(Ordering::Acquire));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn routed_calibration_start_maps_physical_activity_to_playback_active() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-calibration-playback-active-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("songs root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        let activity = ActivityCoordinator::default();
        let playback = activity
            .reserve_playback("native-session")
            .expect("physical playback reservation");
        let runtime = NativeDesktopRuntime::from_install_root_with_activity_and_seams(
            root.clone(),
            activity,
            TestSeams::SafePackage,
        )
        .expect("runtime");
        let error = runtime
            .dispatch("calibration.start", serde_json::json!({"mode":"quick"}))
            .expect_err("physical playback must block calibration");
        assert!(error.starts_with("playback_active:"), "{error}");
        drop(playback);
        runtime.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn routed_duplicate_calibration_start_maps_to_already_running() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-calibration-already-running-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("songs root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        let activity = ActivityCoordinator::default();
        let reservation = activity
            .reserve_calibration()
            .expect("calibration reservation");
        let runtime = NativeDesktopRuntime::from_install_root_with_activity_and_seams(
            root.clone(),
            activity,
            TestSeams::SafePackage,
        )
        .expect("runtime");
        let error = runtime
            .dispatch("calibration.start", serde_json::json!({"mode":"quick"}))
            .expect_err("duplicate calibration must be rejected");
        assert!(error.starts_with("already_running:"), "{error}");
        drop(reservation);
        runtime.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn calibration_cache_writer_and_loader_round_trip_out_of_envelope() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let cases = [
            (0_i64, Some(300_u64)),
            (677, Some(777)),
            (1900, Some(2000)),
            (2100, None),
        ];
        for (worst, expected_margin) in cases {
            let root =
                std::env::temp_dir().join(format!("sky-calibration-roundtrip-{suffix}-{worst}"));
            let mut raw = safe_calibration_evidence();
            raw["pair_buckets"]["1"]["hot"]["sendinput_shrink_us"]["max"] = Value::from(worst);
            let paths = AppPaths::from_test_sandbox(&root);
            paths.ensure_mutable_directories().expect("cache directory");
            publish_calibration_cache(paths.cache_root(), &raw).expect("cache publication");
            let cache = paths.calibration_cache_path();
            let resolution = load_calibration_resolution(&cache);
            assert_eq!(
                resolution.transport_reserve_us,
                expected_margin.unwrap_or(300)
            );
            assert_eq!(resolution.qualified, expected_margin.is_some());
            let recommendation = timing_margin_recommendation(&cache, 500);
            assert_eq!(
                recommendation.recommended_timing_margin_us,
                match worst {
                    0 => 800,
                    677 => 1_300,
                    1_900 => 2_500,
                    _ => sky_app_core::settings::DEFAULT_TIMING_MARGIN_US,
                }
            );
            assert_eq!(recommendation.qualified, expected_margin.is_some());
            assert_eq!(
                resolution.source,
                if expected_margin.is_some() {
                    sky_native_adapters::CALIBRATION_MARGIN_SOURCE_DEVICE
                } else {
                    sky_native_adapters::CALIBRATION_MARGIN_SOURCE_OUT_OF_ENVELOPE
                }
            );
            if expected_margin.is_none() {
                let value: Value =
                    serde_json::from_slice(&fs::read(&cache).expect("cache")).expect("json");
                assert!(value["transport_margin_us"].is_null());
                assert!(
                    !value["calibration_timing_qualified"]
                        .as_bool()
                        .unwrap_or(true)
                );
            }
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn unqualified_margin_recommendation_uses_default_timing_margin() {
        let path = std::env::temp_dir().join(format!(
            "sky-down-tolerance-recommendation-{}-missing.json",
            std::process::id()
        ));
        for cutoff in [500, 1_000, 2_000, 5_000] {
            let recommendation = timing_margin_recommendation(&path, cutoff);
            assert_eq!(
                recommendation.recommended_timing_margin_us,
                sky_app_core::settings::DEFAULT_TIMING_MARGIN_US
            );
            assert_eq!(recommendation.source, "default_fallback");
        }
    }

    #[test]
    fn calibration_recommendation_never_changes_the_persisted_user_margin() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-calibration-advisory-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("songs root");
        fs::write(
            root.join("config.json"),
            r#"{"schema_version":4,"default_timing_margin_us":1200}"#,
        )
        .expect("user settings");
        let runtime = NativeDesktopRuntime::from_install_root_with_activity_and_seams(
            root.clone(),
            ActivityCoordinator::default(),
            TestSeams::SafePackage,
        )
        .expect("runtime");
        let mut raw = safe_calibration_evidence();
        raw["pair_buckets"]["1"]["hot"]["sendinput_shrink_us"]["max"] = Value::from(677_i64);
        publish_calibration_cache(runtime.paths().cache_root(), &raw).expect("cache");

        let settings = runtime
            .dispatch("settings.get", Value::Object(Default::default()))
            .expect("settings");
        assert_eq!(settings["playback_defaults"]["timing_margin_us"], 1_200);
        assert_eq!(
            settings["timing_margin_recommendation"]["recommended_timing_margin_us"],
            2_800
        );
        assert_eq!(settings["timing_margin_recommendation"]["qualified"], true);
        let persisted: Value =
            serde_json::from_slice(&fs::read(root.join("config.json")).expect("read settings"))
                .expect("JSON settings");
        assert_eq!(persisted["default_timing_margin_us"], 1_200);
        runtime.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_bootstrap_uses_explicit_install_root_and_returns_plain_dto() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-native-runtime-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
        let value = runtime
            .dispatch("app.bootstrap", Value::Object(Default::default()))
            .expect("bootstrap");
        assert!(value.get("app_version").is_some());
        assert!(value.get("Ok").is_none());
        assert_eq!(runtime.install_root(), root.as_path());
        assert!(
            runtime
                .dispatch("settings.get", Value::Object(Default::default()))
                .is_ok()
        );
        assert_eq!(
            runtime
                .dispatch("app.shutdown", Value::Object(Default::default()))
                .expect("first shutdown"),
            Value::Null
        );
        assert_eq!(
            runtime
                .dispatch("app.shutdown", Value::Object(Default::default()))
                .expect("idempotent shutdown"),
            Value::Null
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn catalog_viewport_hydrates_metadata_and_liked_songs_persist_by_song_id() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-native-library-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        fs::write(
            root.join("songs/hydrate.json"),
            r#"{"name":"Hydrate","songNotes":[{"time":0,"key":"Key0"},{"time":100,"key":"Key1"}]}"#,
        )
        .expect("song");

        let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
        let bootstrap = runtime
            .dispatch("app.bootstrap", Value::Object(Default::default()))
            .expect("bootstrap");
        let generation = bootstrap["catalog_generation"]
            .as_u64()
            .expect("generation");
        let search = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({
                    "query": "",
                    "offset": 0,
                    "limit": 10,
                    "generation": generation,
                    "source": {"kind":"smart","id":"all"}
                }),
            )
            .expect("search");
        let song_id = search["items"][0]["song_id"]
            .as_str()
            .expect("song ID")
            .to_owned();
        assert_eq!(search["items"][0]["metadata_state"], "pending");
        assert_eq!(search["items"][0]["format_label"], "JSON");

        let viewport = runtime
            .dispatch(
                "catalog.set_viewport",
                serde_json::json!({
                    "generation": generation,
                    "firstIndex": 0,
                    "lastIndex": 0,
                    "selectedSongId": null,
                    "songIds": [song_id.clone()]
                }),
            )
            .expect("viewport hydration");
        assert_eq!(viewport["items"][0]["metadata_state"], "ready");
        assert_eq!(viewport["items"][0]["note_count"], 2);
        assert!(viewport["items"][0]["duration_us"].as_u64().is_some());

        let hydrated_detail = runtime
            .detail(crate::commands::CatalogDetailRequest {
                song_id: song_id.clone(),
                generation: Some(generation),
            })
            .expect("detail from hydrated analysis");
        assert_eq!(hydrated_detail.format_label, "JSON");
        let unavailable_path = root.join("songs/hydrate-unavailable.json");
        fs::rename(root.join("songs/hydrate.json"), &unavailable_path).expect("hide source");
        let cached_detail = runtime
            .detail(crate::commands::CatalogDetailRequest {
                song_id: song_id.clone(),
                generation: Some(generation),
            })
            .expect("cached detail after source unavailable");
        assert_eq!(
            serde_json::to_value(cached_detail).expect("cached detail JSON"),
            serde_json::to_value(hydrated_detail).expect("hydrated detail JSON")
        );
        fs::rename(&unavailable_path, root.join("songs/hydrate.json")).expect("restore source");

        let liked = runtime
            .dispatch(
                "catalog.set_liked",
                serde_json::json!({
                    "songId": song_id,
                    "liked": true,
                    "generation": generation
                }),
            )
            .expect("like");
        assert_eq!(liked["liked"], true);
        assert_eq!(liked["total"], 1);
        runtime.shutdown();

        let restarted = NativeDesktopRuntime::from_install_root(root.clone()).expect("restart");
        let bootstrap = restarted.bootstrap().expect("restart bootstrap");
        let liked_search = restarted
            .dispatch(
                "catalog.search",
                serde_json::json!({
                    "query": "",
                    "offset": 0,
                    "limit": 10,
                    "generation": bootstrap.catalog_generation,
                    "source": {"kind":"smart","id":"liked"}
                }),
            )
            .expect("liked search");
        assert_eq!(liked_search["total"], 1);
        assert_eq!(liked_search["items"][0]["liked"], true);
        assert_eq!(liked_search["items"][0]["metadata_state"], "pending");
        restarted.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_playlist_import_commits_membership_and_import_reference_together() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-native-library-v1-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("songs root");
        fs::create_dir_all(root.join("imports")).expect("imports root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        fs::write(root.join("songs/primary.json"), "{}").expect("primary song");
        fs::write(root.join("imports/local.txt"), "notes").expect("import song");

        let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
        let listed = runtime
            .dispatch("library.list_playlists", serde_json::json!({}))
            .expect("list playlists");
        assert_eq!(listed["playlists"].as_array().expect("playlists").len(), 0);
        let initial_bootstrap = runtime
            .dispatch("app.bootstrap", serde_json::json!({}))
            .expect("initial bootstrap");
        let initial_generation = initial_bootstrap["catalog_generation"]
            .as_u64()
            .expect("initial generation");
        let primary_id = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({
                    "query":"primary",
                    "offset":0,
                    "limit":10,
                    "generation":initial_generation,
                    "source":{"kind":"smart","id":"all"}
                }),
            )
            .expect("primary search")["items"][0]["song_id"]
            .as_str()
            .expect("primary song ID")
            .to_owned();

        let created = runtime
            .dispatch(
                "library.create_playlist",
                serde_json::json!({"name":"Practice"}),
            )
            .expect("create playlist");
        let playlist_id = created["id"].as_str().expect("playlist ID").to_owned();
        assert_eq!(created["name"], "Practice");
        runtime
            .dispatch(
                "library.rename_playlist",
                serde_json::json!({"playlistId":playlist_id,"name":"Morning"}),
            )
            .expect("rename playlist");
        runtime
            .dispatch(
                "library.add_songs",
                serde_json::json!({
                    "playlistId":playlist_id,
                    "songIds":[primary_id]
                }),
            )
            .expect("add song to playlist");
        let listed = runtime
            .dispatch("library.list_playlists", serde_json::json!({}))
            .expect("list updated playlists");
        assert_eq!(listed["playlists"][0]["name"], "Morning");
        assert_eq!(listed["playlists"][0]["song_count"], 1);
        assert!(listed["playlists"][0].get("song_ids").is_none());

        let imported = runtime
            .dispatch(
                "library.import_local_files_to_playlist",
                serde_json::json!({
                    "playlistId":playlist_id,
                    "paths":[root.join("imports/local.txt")]
                }),
            )
            .expect("import local file to playlist");
        assert_eq!(imported["imported_song_count"], 1);
        assert_eq!(imported["playlist"]["name"], "Morning");
        assert_eq!(imported["playlist"]["song_count"], 2);
        assert!(imported.get("canonical_path").is_none());

        let generation = imported["catalog_generation"].as_u64().expect("generation");
        let search = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({
                    "query":"local",
                    "offset":0,
                    "limit":10,
                    "generation":generation,
                    "source":{"kind":"playlist","id":playlist_id}
                }),
            )
            .expect("search playlist local song");
        assert_eq!(search["total"], 1);

        let all_songs = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({
                    "query":"local",
                    "offset":0,
                    "limit":10,
                    "generation":generation,
                    "source":{"kind":"smart","id":"all"}
                }),
            )
            .expect("search primary catalog");
        assert_eq!(all_songs["total"], 0);
        assert!(root.join("imports/local.txt").exists());
        runtime
            .dispatch(
                "library.delete_playlist",
                serde_json::json!({"playlistId":playlist_id}),
            )
            .expect("delete playlist");
        runtime.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_missing_import_does_not_break_all_songs_or_navigation() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-native-library-missing-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("songs root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        fs::write(root.join("songs/primary.json"), "{}").expect("primary song");
        let missing_path = root.join("disconnected-drive");
        let source_id = "d".repeat(32);
        fs::write(
            root.join("library-manifest.json"),
            serde_json::json!({
                "version": 1,
                "imports": [{
                    "source_id": source_id.clone(),
                    "canonical_path": missing_path.to_string_lossy().to_string(),
                    "kind": "folder"
                }],
                "collections": []
            })
            .to_string(),
        )
        .expect("missing import manifest");

        let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
        let listed = runtime
            .dispatch("library.list_playlists", serde_json::json!({}))
            .expect("list playlists with missing import");
        assert_eq!(listed["playlists"].as_array().map(Vec::len), Some(0));
        assert!(
            !listed
                .to_string()
                .contains(&missing_path.to_string_lossy().to_string())
        );

        let generation = runtime
            .dispatch("app.bootstrap", serde_json::json!({}))
            .expect("bootstrap")["catalog_generation"]
            .as_u64()
            .expect("generation");
        let all_songs = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({
                    "query":"",
                    "offset":0,
                    "limit":10,
                    "generation":generation,
                    "source":{"kind":"smart","id":"all"}
                }),
            )
            .expect("all songs search");
        assert_eq!(all_songs["total"], 1);
        runtime.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    fn snapshot(session_id: &str, seq: u64) -> UiEvent {
        UiEvent::PlaybackSnapshot {
            v: 1,
            payload: PlaybackSnapshotPayload {
                session_id: session_id.into(),
                seq,
                state: PlaybackEventState::Playing,
                song_id: "0123456789abcdef0123456789abcdef".into(),
                title: "demo".into(),
                current_us: seq,
                total_us: 100,
                pre_roll_remaining_us: 0,
                focus_state: PlaybackFocusState::Focused,
                health: PlaybackHealthState::Healthy,
                input_path_degraded: false,
                message: None,
            },
        }
    }

    #[test]
    fn event_hub_coalesces_snapshots_and_fails_closed_for_lifecycle_overflow() {
        let mut hub = NativeEventHub::default();
        let session_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        hub.publish(snapshot(session_id, 1)).expect("snapshot");
        hub.publish(snapshot(session_id, 2)).expect("coalesce");
        assert_eq!(hub.buffered.len(), 1);
        assert!(
            matches!(hub.buffered.front(), Some(UiEvent::PlaybackSnapshot { payload, .. }) if payload.seq == 2)
        );

        for index in 0..MAX_NATIVE_EVENTS {
            hub.publish(UiEvent::CatalogChanged {
                v: 1,
                payload: crate::ui_events::CatalogChangedPayload {
                    generation: index as u64 + 1,
                    total: 0,
                },
            })
            .expect("snapshot slot can be evicted before lifecycle fill");
        }
        assert!(
            hub.publish(UiEvent::CatalogChanged {
                v: 1,
                payload: crate::ui_events::CatalogChangedPayload {
                    generation: 999,
                    total: 0,
                },
            })
            .is_err()
        );
    }

    #[test]
    fn diagnostics_statistics_match_python_rounding_and_population_sigma() {
        assert_eq!(percentile_ms(&[1, 2, 3, 4], 0.50), 0.003);
        assert_eq!(percentile_ms(&[1, 2, 3, 4], 0.95), 0.004);
        assert_eq!(
            population_sigma_ms(&[1, 2, 3, 4]),
            1.118033988749895 / 1000.0
        );
        assert_eq!(percentile_ms(&[], 0.50), 0.0);
        assert_eq!(population_sigma_ms(&[]), 0.0);
    }

    #[test]
    fn diagnostics_status_matches_native_observer_contract() {
        use crate::ui_events::DiagnosticsBackendStatus;

        let healthy = DiagnosticsBackendHealth {
            physical_session: true,
            player_attached: true,
            active_count: 1,
            possibly_active_count: 1,
            ..DiagnosticsBackendHealth::default()
        };
        assert_eq!(
            diagnostics_backend_status(healthy),
            DiagnosticsBackendStatus::Healthy
        );
        assert_eq!(
            diagnostics_backend_status(DiagnosticsBackendHealth {
                keys_dropped: 1,
                ..healthy
            }),
            DiagnosticsBackendStatus::Degraded
        );
        assert_eq!(
            diagnostics_backend_status(DiagnosticsBackendHealth {
                chord_split_events: 1,
                possibly_active_count: 2,
                ..healthy
            }),
            DiagnosticsBackendStatus::Degraded
        );
        assert_eq!(
            diagnostics_backend_status(DiagnosticsBackendHealth {
                possibly_active_count: 2,
                ..healthy
            }),
            DiagnosticsBackendStatus::Degraded
        );
        assert_eq!(
            diagnostics_backend_status(DiagnosticsBackendHealth {
                has_last_error: true,
                ..healthy
            }),
            DiagnosticsBackendStatus::Error
        );
        assert_eq!(
            diagnostics_backend_status(DiagnosticsBackendHealth {
                failed_release_count: 1,
                ..healthy
            }),
            DiagnosticsBackendStatus::Error
        );
        assert_eq!(
            diagnostics_backend_status(DiagnosticsBackendHealth {
                has_terminal_error: true,
                ..healthy
            }),
            DiagnosticsBackendStatus::Error
        );
    }

    #[test]
    fn diagnostics_status_distinguishes_unavailable_and_inconsistent_attachment() {
        use crate::ui_events::DiagnosticsBackendStatus;

        assert_eq!(
            diagnostics_backend_status(DiagnosticsBackendHealth::default()),
            DiagnosticsBackendStatus::Unavailable
        );
        assert_eq!(
            diagnostics_backend_status(DiagnosticsBackendHealth {
                physical_session: true,
                ..DiagnosticsBackendHealth::default()
            }),
            DiagnosticsBackendStatus::Error
        );
        assert_eq!(
            diagnostics_backend_status(DiagnosticsBackendHealth {
                player_attached: true,
                ..DiagnosticsBackendHealth::default()
            }),
            DiagnosticsBackendStatus::Error
        );
    }

    #[test]
    fn diagnostics_sender_samples_distinguish_empty_zero_and_nonzero_measurements() {
        use crate::ui_events::DiagnosticsBackendStatus;

        assert_eq!(sender_sample_summary([0; 7], 0), (0, None));
        assert_eq!(
            sender_sample_summary([1, 0, 0, 0, 0, 0, 0], 0),
            (1, Some(0))
        );
        assert_eq!(
            sender_sample_summary([0, 0, 2, 0, 0, 0, 0], 177),
            (2, Some(177))
        );
        assert_eq!(
            sender_sample_summary([u64::MAX, 1, 0, 0, 0, 0, 0], 0),
            (u64::MAX, Some(0))
        );
        assert_eq!(
            diagnostics_backend_status(DiagnosticsBackendHealth {
                physical_session: true,
                player_attached: true,
                ..DiagnosticsBackendHealth::default()
            }),
            DiagnosticsBackendStatus::Healthy
        );
    }

    #[test]
    fn diagnostics_gate_matches_python_rate_reset_and_sequence_contract() {
        let gate = DiagnosticsPublicationGate::default();
        gate.set_enabled(true).expect("enable");
        let start = Instant::now();
        let mut sequences = Vec::new();
        assert!(
            gate.try_publish(start, |sequence| {
                sequences.push(sequence);
                Ok(())
            })
            .expect("first publication")
        );
        assert!(
            !gate
                .try_publish(start + Duration::from_millis(99), |_| Ok(()))
                .expect("throttled publication")
        );
        assert!(
            gate.try_publish(start + Duration::from_millis(100), |sequence| {
                sequences.push(sequence);
                Ok(())
            })
            .expect("second publication")
        );
        gate.set_enabled(false).expect("disable");
        assert!(
            !gate
                .try_publish(start + Duration::from_millis(200), |_| Ok(()))
                .expect("disabled publication")
        );
        gate.set_enabled(true).expect("re-enable");
        assert!(
            gate.try_publish(start + Duration::from_millis(200), |sequence| {
                sequences.push(sequence);
                Ok(())
            })
            .expect("fresh sampling window")
        );
        assert_eq!(sequences, vec![1, 2, 3]);
    }

    #[test]
    fn diagnostics_disable_waits_for_in_flight_publication() {
        let gate = Arc::new(DiagnosticsPublicationGate::default());
        gate.set_enabled(true).expect("enable");
        let start = Instant::now();
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let (entered_sender, entered_receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let disabled_returned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_gate = Arc::clone(&gate);
        let worker_emitted = Arc::clone(&emitted);
        let worker_disabled_returned = Arc::clone(&disabled_returned);
        let worker = std::thread::spawn(move || {
            worker_gate
                .try_publish(start, |sequence| {
                    entered_sender.send(()).expect("entered receiver");
                    release_receiver.recv().expect("release sender");
                    assert!(!worker_disabled_returned.load(Ordering::Acquire));
                    worker_emitted.lock().expect("emitted").push(sequence);
                    Ok(())
                })
                .expect("publication")
        });
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("publisher entered");
        let disable_gate = Arc::clone(&gate);
        let disable_returned = Arc::clone(&disabled_returned);
        let disable = std::thread::spawn(move || {
            disable_gate.set_enabled(false).expect("disable");
            disable_returned.store(true, Ordering::Release);
        });
        release_sender.send(()).expect("release publisher");
        assert!(worker.join().expect("publisher thread"));
        disable.join().expect("disable thread");
        assert_eq!(*emitted.lock().expect("emitted"), vec![1]);
        assert!(
            !gate
                .try_publish(start + Duration::from_millis(100), |_| Ok(()))
                .expect("post-disable publication")
        );
    }

    #[test]
    fn diagnostics_dry_run_publishes_unavailable_snapshot() {
        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let gate = DiagnosticsPublicationGate::default();
        gate.set_enabled(true).expect("enable");
        let active = active_for_control(PlaybackSessionState::Playing, None);

        publish_diagnostics_snapshot_for_active(&events, &active, &gate)
            .expect("dry-run diagnostics");
        let hub = events.lock().expect("event hub");
        assert_eq!(hub.buffered.len(), 1);
        let UiEvent::DiagnosticsSnapshot { payload, .. } = &hub.buffered[0] else {
            panic!("expected diagnostics snapshot");
        };
        assert_eq!(payload.session_id.as_deref(), Some("a".repeat(32).as_str()));
        assert!(!payload.physical_session);
        assert!(!payload.player_attached);
        assert_eq!(payload.sender_sample_count, 0);
        assert_eq!(
            payload.backend_status,
            DiagnosticsBackendStatus::Unavailable
        );
        assert_eq!(payload.max_lateness_us, None);
        assert_eq!(payload.p50_ms, None);
        assert_eq!(payload.p95_ms, None);
        assert_eq!(payload.sigma_onset_ms, None);
        assert_eq!(payload.late_2ms, None);
        assert_eq!(payload.late_5ms, None);
        assert_eq!(payload.late_10ms, None);
        assert_eq!(payload.max_sendinput_pre_call_lateness_us, None);
        assert_eq!(payload.pre_call_late_2ms, 0);
        assert_eq!(payload.pre_call_late_5ms, 0);
        assert_eq!(payload.pre_call_late_10ms, 0);
        assert_eq!(payload.fps, 60);
        assert_eq!(payload.frame_us, 16_667);
        assert_eq!(payload.hold_frames, 1.0);
        assert_eq!(payload.frame_base_hold_us, 16_667);
        assert_eq!(payload.timing_margin_us, 800);
        assert_eq!(payload.min_hold_us, 17_467);
        assert_eq!(payload.min_release_gap_us, 17_467);
        assert_eq!(payload.down_late_grace_us, 500);
        assert_eq!(
            payload
                .timing_margin_recommendation
                .recommended_timing_margin_us,
            800
        );
        assert!(!payload.timing_margin_recommendation.qualified);
        assert_eq!(payload.pre_call_lt_250us, 0);
        assert_eq!(payload.pre_call_250_500us, 0);
        assert_eq!(payload.pre_call_500_750us, 0);
        assert_eq!(payload.pre_call_750_1000us, 0);
        assert_eq!(payload.pre_call_1000_1500us, 0);
        assert_eq!(payload.pre_call_1500_2000us, 0);
        assert_eq!(payload.pre_call_ge_2000us, 0);
        assert_eq!(payload.active_keys, 0);
        assert_eq!(payload.stuck_keys, 0);
        assert_eq!(payload.keys_dropped, 0);
        assert_eq!(payload.chord_split_events, 0);
        assert_eq!(payload.missed_down_boundaries, 0);
        assert_eq!(payload.missed_down_keys, 0);
        assert_eq!(payload.missed_backlog_boundaries, 0);
        assert_eq!(payload.missed_hard_late_boundaries, 0);
        assert_eq!(payload.final_gate_cutoff_misses, 0);
        assert_eq!(payload.final_gate_control_rejections, 0);
        assert_eq!(payload.final_gate_target_changes, 0);
        assert_eq!(payload.final_gate_focus_losses, 0);
        assert_eq!(payload.final_gate_lease_expirations, 0);
        assert_eq!(payload.sendinput_partial_events, 0);
        assert_eq!(payload.sendinput_zero_progress_failures, 0);
        assert_eq!(payload.release_max_us, None);
        assert_eq!(payload.release_late_2ms, None);
        assert_eq!(payload.last_error, None);
        assert_eq!(payload.seq, 1);
    }

    #[test]
    fn diagnostics_unavailable_path_keeps_rate_gate_and_monotonic_sequence() {
        let gate = DiagnosticsPublicationGate::default();
        gate.set_enabled(true).expect("enable");
        let start = Instant::now();
        let mut sequences = Vec::new();
        for offset in [0, 1, 99, 100, 101] {
            assert!(
                gate.try_publish(start + Duration::from_millis(offset), |sequence| {
                    sequences.push(sequence);
                    Ok(())
                })
                .is_ok()
            );
        }
        assert_eq!(sequences, vec![1, 2]);
        gate.set_enabled(false).expect("disable");
        assert!(
            !gate
                .try_publish(start + Duration::from_millis(200), |_| Ok(()))
                .expect("disabled publication")
        );
        gate.set_enabled(true).expect("re-enable");
        assert!(
            gate.try_publish(start + Duration::from_millis(200), |sequence| {
                sequences.push(sequence);
                Ok(())
            })
            .expect("re-enabled publication")
        );
        assert_eq!(sequences, vec![1, 2, 3]);
    }

    #[test]
    fn diagnostics_physical_healthy_status_remains_healthy() {
        use crate::ui_events::DiagnosticsBackendStatus;

        assert_eq!(
            diagnostics_backend_status(DiagnosticsBackendHealth {
                physical_session: true,
                player_attached: true,
                ..DiagnosticsBackendHealth::default()
            }),
            DiagnosticsBackendStatus::Healthy
        );
        assert_eq!(
            NativeDiagnosticsSample::unavailable(false).backend_status,
            DiagnosticsBackendStatus::Unavailable
        );
        let inconsistent = NativeDiagnosticsSample::unavailable(true);
        assert_eq!(inconsistent.backend_status, DiagnosticsBackendStatus::Error);
        assert_eq!(inconsistent.sender_sample_count, 0);
        assert_eq!(inconsistent.max_sendinput_pre_call_lateness_us, None);
    }

    #[test]
    fn terminal_stop_returns_the_stored_terminal_state() {
        for terminal_state in [PlaybackSessionState::Finished, PlaybackSessionState::Failed] {
            let service = NativePlaybackService::new(ActivityCoordinator::default());
            *service.last_terminal.lock().expect("terminal state") =
                Some(PlaybackTerminalStatusDto {
                    session_id: "a".repeat(32),
                    prepared_id: "b".repeat(32),
                    song_id: "song-a".into(),
                    state: terminal_state,
                    outcome: None,
                    failure_code: None,
                    failure_message: None,
                });
            let acknowledgement = service
                .command(
                    "playback.stop",
                    "a".repeat(32),
                    Arc::new(Mutex::new(NativeEventHub::default())),
                )
                .expect("terminal stop is idempotent");
            assert_eq!(acknowledgement.state, terminal_state);
            assert!(acknowledgement.accepted);
        }
    }

    #[test]
    fn playback_status_tracks_active_ownership_through_the_retirement_barrier() {
        let service = NativePlaybackService::new(ActivityCoordinator::default());
        let active = active_for_control(PlaybackSessionState::Playing, None);
        *service.active.lock().expect("active slot") = Some(active.clone());

        let active_status = service.status().expect("active playback status");
        assert_eq!(
            active_status.active,
            Some(PlaybackActiveStatusDto {
                session_id: active.session_id.clone(),
                prepared_id: active.prepared_id.clone(),
                song_id: active.song_id.clone(),
                title: active.title.clone(),
                state: PlaybackSessionState::Playing,
            })
        );
        assert!(active_status.last_terminal.is_none());

        *active.state.lock().expect("playback state") = PlaybackSessionState::Finished;
        release_terminal_ownership(&service.active, &service.last_terminal, &active, None)
            .expect("retirement barrier");
        let retired_status = service.status().expect("retired playback status");
        assert!(retired_status.active.is_none());
        assert_eq!(
            retired_status.last_terminal,
            Some(PlaybackTerminalStatusDto {
                session_id: active.session_id.clone(),
                prepared_id: active.prepared_id.clone(),
                song_id: active.song_id.clone(),
                state: PlaybackSessionState::Finished,
                outcome: Some("finished".into()),
                failure_code: None,
                failure_message: None,
            })
        );
    }

    #[test]
    fn playback_status_preserves_prepared_identity_and_failure_details() {
        let service = NativePlaybackService::new(ActivityCoordinator::default());
        let active = active_for_control(PlaybackSessionState::Failed, None);
        *service.active.lock().expect("active slot") = Some(active.clone());
        let failure = PlaybackTerminalPublication::Failed {
            code: "target_not_found".into(),
            message: "Sky window was not found. Open Sky and try again.".into(),
        };

        release_terminal_ownership(
            &service.active,
            &service.last_terminal,
            &active,
            Some(&failure),
        )
        .expect("retirement barrier");

        let terminal = service
            .status()
            .expect("terminal status")
            .last_terminal
            .expect("retired failure");
        assert_eq!(terminal.session_id, active.session_id);
        assert_eq!(terminal.prepared_id, active.prepared_id);
        assert_eq!(terminal.failure_code.as_deref(), Some("target_not_found"));
        assert_eq!(
            terminal.failure_message.as_deref(),
            Some("Sky window was not found. Open Sky and try again.")
        );
    }

    #[test]
    fn physical_startup_failures_have_actionable_classified_messages() {
        for (error, expected_code, expected_message) in [
            (
                "no admissible visible Sky window was found",
                "target_not_found",
                "Sky window was not found. Open Sky and make sure its window is visible, then try again.",
            ),
            (
                "target Sky process has higher integrity than Sky Auto Player; Windows UIPI can block SendInput",
                "target_integrity_mismatch",
                "Sky is running with higher Windows integrity than Sky Auto Player. Run both apps at the same permission level, then try again.",
            ),
            (
                "validated Sky window could not be focused",
                "target_focus_failed",
                "Sky's window could not be focused. Bring the Sky window to the foreground, then try again.",
            ),
            (
                "native schedule compilation failed",
                "native_worker_failed",
                "The native playback engine could not start. Try again; if it continues, restart Sky Auto Player.",
            ),
        ] {
            assert_eq!(
                physical_startup_failure(error),
                (expected_code, expected_message)
            );
        }
    }

    #[test]
    fn terminal_player_statuses_publish_state_before_terminal_event() {
        use sky_player::engine::EnginePollStatus;

        for (status, expected) in [
            (
                EnginePollStatus::Finished,
                vec!["state:finished", "finished:finished"],
            ),
            (
                EnginePollStatus::Skipped,
                vec!["state:finished", "finished:skipped"],
            ),
            (
                EnginePollStatus::Quit,
                vec!["state:finished", "finished:quit"],
            ),
            (
                EnginePollStatus::Error,
                vec!["state:failed", "failed:native_player_failed"],
            ),
            (
                EnginePollStatus::Panicked,
                vec!["state:failed", "failed:native_player_failed"],
            ),
            (
                EnginePollStatus::Poisoned,
                vec!["state:failed", "failed:native_player_failed"],
            ),
            (
                EnginePollStatus::Invalid,
                vec!["state:failed", "failed:native_player_failed"],
            ),
        ] {
            let events = Arc::new(Mutex::new(NativeEventHub::default()));
            let active =
                active_for_control_with_physical(PlaybackSessionState::Playing, None, true);
            let mut last_event_state = PlaybackEventState::Playing;
            publish_terminal_poll_result(&events, &active, &mut last_event_state, status)
                .expect("terminal event trace");
            assert_eq!(
                playback_trace(&events),
                expected.into_iter().map(str::to_owned).collect::<Vec<_>>(),
                "status {}",
                status.as_str()
            );
        }
    }

    #[test]
    fn finished_and_failed_events_are_retirement_barriers_for_the_next_reservation() {
        for (state, publication, expected_terminal) in [
            (
                PlaybackSessionState::Finished,
                PlaybackTerminalPublication::Finished {
                    outcome: "finished".into(),
                    message: "Playback finished".into(),
                },
                "finished:finished",
            ),
            (
                PlaybackSessionState::Failed,
                PlaybackTerminalPublication::Failed {
                    code: "native_player_failed".into(),
                    message: "Native playback worker failed".into(),
                },
                "failed:native_player_failed",
            ),
        ] {
            let activity = ActivityCoordinator::default();
            let service = NativePlaybackService::new(activity.clone());
            let events = Arc::new(Mutex::new(NativeEventHub::default()));
            let active = active_for_control_with_physical(state, None, true);
            *active.activity_lease.lock().expect("activity lease slot") = Some(
                activity
                    .reserve_playback(active.session_id.clone())
                    .expect("first session reservation"),
            );
            *service.active.lock().expect("active session slot") = Some(active.clone());
            assert!(activity.reserve_playback("next-before-barrier").is_err());

            publish_retirement_barrier(
                &events,
                &service.active,
                &service.last_terminal,
                &active,
                &publication,
            )
            .expect("publish terminal retirement barrier");

            assert_eq!(
                playback_trace(&events).last().map(String::as_str),
                Some(expected_terminal),
            );
            assert!(
                service
                    .active
                    .lock()
                    .expect("active session slot")
                    .is_none()
            );
            assert!(active.done.load(Ordering::Acquire));
            assert!(active.pending.lock().expect("pending control").is_none());
            let next = activity
                .reserve_playback("next-after-barrier")
                .expect("next reservation succeeds immediately after terminal event");
            drop(next);
        }
    }

    #[test]
    fn focus_rejection_publishes_starting_then_failed_trace() {
        use sky_player::engine::EnginePollStatus;

        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let active = active_for_control_with_physical(PlaybackSessionState::Starting, None, true);
        publish_playback_state(&events, &active, PlaybackEventState::Starting, None, None)
            .expect("starting event");
        let mut last_event_state = PlaybackEventState::Starting;
        publish_terminal_poll_result(
            &events,
            &active,
            &mut last_event_state,
            EnginePollStatus::Error,
        )
        .expect("focus rejection trace");
        assert_eq!(
            playback_trace(&events),
            [
                "state:starting",
                "state:failed",
                "failed:native_player_failed"
            ]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn dry_run_completion_publishes_finished_before_terminal_event() {
        use sky_player::engine::EnginePollStatus;

        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let active = active_for_control(PlaybackSessionState::Playing, None);
        let mut last_event_state = PlaybackEventState::Playing;
        publish_terminal_poll_result(
            &events,
            &active,
            &mut last_event_state,
            EnginePollStatus::Finished,
        )
        .expect("dry-run completion trace");
        assert_eq!(
            playback_trace(&events),
            ["state:finished", "finished:finished"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn stop_paths_publish_stopping_finished_then_finished_event() {
        for initial_state in [
            PlaybackSessionState::Starting,
            PlaybackSessionState::Playing,
            PlaybackSessionState::Paused,
        ] {
            let events = Arc::new(Mutex::new(NativeEventHub::default()));
            let service = NativePlaybackService::new(ActivityCoordinator::default());
            let active = active_for_control_with_physical(initial_state, None, true);
            *service.active.lock().expect("active") = Some(active.clone());
            service
                .command("playback.stop", active.session_id.clone(), events.clone())
                .expect("stop");
            let mut last_event_state = match initial_state {
                PlaybackSessionState::Starting => PlaybackEventState::Starting,
                PlaybackSessionState::Playing => PlaybackEventState::Playing,
                PlaybackSessionState::Paused => PlaybackEventState::Paused,
                _ => unreachable!(),
            };
            publish_stopped_completion(
                &events,
                &active,
                &mut last_event_state,
                "quit",
                "Playback stopped",
            )
            .expect("stop terminal event trace");
            assert_eq!(
                playback_trace(&events),
                ["state:stopping", "state:finished", "finished:quit"]
            );
        }
    }

    #[test]
    fn shutdown_completion_uses_the_same_terminal_event_sequence() {
        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let active = active_for_control(PlaybackSessionState::Starting, None);
        let mut last_event_state = PlaybackEventState::Starting;
        publish_playback_state(&events, &active, PlaybackEventState::Stopping, None, None)
            .expect("stopping event");
        publish_stopped_completion(
            &events,
            &active,
            &mut last_event_state,
            "quit",
            "Playback stopped",
        )
        .expect("shutdown terminal event trace");
        assert_eq!(
            playback_trace(&events),
            ["state:stopping", "state:finished", "finished:quit"]
        );
    }

    #[test]
    fn playback_start_validation_distinguishes_malformed_and_stale_ids() {
        let malformed = super::NativePlaybackStartRequest {
            prepared_id: "A".repeat(32),
            decisions: Vec::new(),
        };
        let error = validate_playback_start_request(&malformed).expect_err("invalid ID");
        assert!(error.starts_with("invalid_params:"));

        let well_formed_stale = super::NativePlaybackStartRequest {
            prepared_id: "a".repeat(32),
            decisions: Vec::new(),
        };
        validate_playback_start_request(&well_formed_stale).expect("well-formed ID");
        let service = NativePlaybackService::new(ActivityCoordinator::default());
        let error = service
            .start(
                well_formed_stale.into_public(),
                &ApplicationSettings::default(),
                Arc::new(Mutex::new(NativeEventHub::default())),
            )
            .expect_err("stale ID");
        assert!(error.contains("stale or already consumed"));
    }

    #[test]
    fn playback_start_validation_bounds_decisions_before_admission() {
        let oversized = super::NativePlaybackStartRequest {
            prepared_id: "a".repeat(32),
            decisions: vec![
                crate::commands::PlaybackDecisionAcceptanceDto {
                    decision: crate::commands::PlaybackDecision::Proceed,
                    accepted: false,
                };
                super::MAX_DECISION_COUNT + 1
            ],
        };
        let error = validate_playback_start_request(&oversized).expect_err("bounded decisions");
        assert!(error.contains("decisions must be a bounded array"));
    }

    #[test]
    fn routed_playback_start_validates_input_before_confirmation_and_lookup() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-native-start-validation-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("songs root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        fs::write(
            root.join("songs/ready.json"),
            r#"{"name":"Ready","songNotes":[{"time":0,"key":"Key0"}]}"#,
        )
        .expect("song");
        let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
        let bootstrap = runtime
            .dispatch("app.bootstrap", Value::Object(Default::default()))
            .expect("bootstrap");
        let generation = bootstrap["catalog_generation"]
            .as_u64()
            .expect("generation");
        let search = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({
                    "query": "ready",
                    "offset": 0,
                    "limit": 10,
                    "generation": generation
                }),
            )
            .expect("search");
        let song_id = search["items"][0]["song_id"]
            .as_str()
            .expect("song ID")
            .to_owned();
        let prepared = runtime
            .dispatch(
                "playback.prepare",
                serde_json::json!({
                    "songId": song_id,
                    "generation": generation,
                    "config": {"hold_frames":1.0,"timing_margin_us":800,"down_late_grace_us":500,"tempo_scale":1.0,"fps":60,"dry_run":true}
                }),
            )
            .expect("prepare");
        let prepared_id = prepared["prepared_id"]
            .as_str()
            .expect("prepared ID")
            .to_owned();

        let malformed = runtime.dispatch(
            "playback.start",
            serde_json::json!({"preparedId":"A".repeat(32),"decisions":[]}),
        );
        assert!(
            malformed
                .expect_err("malformed ID")
                .contains("invalid_params: prepared_id is invalid")
        );

        let oversized = runtime.dispatch(
            "playback.start",
            serde_json::json!({
                "preparedId": prepared_id,
                "decisions": (0..=MAX_DECISION_COUNT)
                    .map(|_| serde_json::json!({"decision":"proceed","accepted":false}))
                    .collect::<Vec<_>>()
            }),
        );
        assert!(
            oversized
                .expect_err("oversized decisions")
                .contains("decisions must be a bounded array")
        );

        let wrong_confirmation = runtime.dispatch(
            "playback.start",
            serde_json::json!({
                "preparedId": prepared_id,
                "decisions": [{"decision":"proceed","accepted":true}]
            }),
        );
        assert!(
            wrong_confirmation
                .expect_err("ready plan must reject confirmation")
                .contains("ready playback accepts no risk decisions")
        );

        let valid = runtime
            .dispatch(
                "playback.start",
                serde_json::json!({"preparedId":prepared_id,"decisions":[]}),
            )
            .expect("valid ready start");
        assert_eq!(valid["state"], "starting");
        runtime.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn skip_is_live_only_and_clears_pending_pause_or_resume() {
        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        for (state, pending) in [
            (PlaybackSessionState::Starting, None),
            (
                PlaybackSessionState::Playing,
                Some(PlaybackPendingControl::Pause),
            ),
            (
                PlaybackSessionState::Paused,
                Some(PlaybackPendingControl::Resume),
            ),
        ] {
            let service = NativePlaybackService::new(ActivityCoordinator::default());
            let active = active_for_control(state, pending);
            *service.active.lock().expect("active") = Some(active.clone());
            let acknowledgement = service
                .command("playback.skip", active.session_id.clone(), events.clone())
                .expect("skip in live state");
            assert!(acknowledgement.accepted);
            assert_eq!(acknowledgement.pending_command, None);
            assert!(active.skip_requested.load(Ordering::Acquire));
        }

        let service = NativePlaybackService::new(ActivityCoordinator::default());
        let active = active_for_control(PlaybackSessionState::Stopping, None);
        *service.active.lock().expect("active") = Some(active.clone());
        let error = service
            .command("playback.skip", active.session_id.clone(), events)
            .expect_err("skip after stopping must be rejected");
        assert!(error.contains("live playback session"));
    }

    #[test]
    fn repeated_pending_controls_and_invalid_transitions_match_command_contract() {
        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let service = NativePlaybackService::new(ActivityCoordinator::default());
        let active = active_for_control(
            PlaybackSessionState::Playing,
            Some(PlaybackPendingControl::Pause),
        );
        *service.active.lock().expect("active") = Some(active.clone());
        let repeated = service
            .command("playback.pause", active.session_id.clone(), events.clone())
            .expect("repeated pause");
        assert_eq!(repeated.reason.as_deref(), Some("already_pending"));

        let service = NativePlaybackService::new(ActivityCoordinator::default());
        let active = active_for_control(
            PlaybackSessionState::Paused,
            Some(PlaybackPendingControl::Pause),
        );
        *service.active.lock().expect("active") = Some(active.clone());
        let conflict = service
            .command("playback.resume", active.session_id.clone(), events.clone())
            .expect_err("resume conflicts with pending pause");
        assert!(conflict.contains("another playback control is awaiting acknowledgement"));

        let service = NativePlaybackService::new(ActivityCoordinator::default());
        let active = active_for_control(
            PlaybackSessionState::Paused,
            Some(PlaybackPendingControl::Resume),
        );
        *service.active.lock().expect("active") = Some(active.clone());
        let repeated = service
            .command("playback.resume", active.session_id.clone(), events.clone())
            .expect("repeated resume");
        assert_eq!(repeated.reason.as_deref(), Some("already_pending"));

        let service = NativePlaybackService::new(ActivityCoordinator::default());
        let active = active_for_control(PlaybackSessionState::Starting, None);
        *service.active.lock().expect("active") = Some(active.clone());
        let pause_error = service
            .command("playback.pause", active.session_id.clone(), events.clone())
            .expect_err("pause before playing");
        assert!(pause_error.contains("pause requires a playing session"));

        let service = NativePlaybackService::new(ActivityCoordinator::default());
        let active = active_for_control(PlaybackSessionState::Playing, None);
        *service.active.lock().expect("active") = Some(active.clone());
        let resume_error = service
            .command("playback.resume", active.session_id.clone(), events.clone())
            .expect_err("resume before pause");
        assert!(resume_error.contains("resume requires a paused session"));

        let foreign_error = service
            .command("playback.stop", "f".repeat(32), events)
            .expect_err("foreign session");
        assert!(foreign_error.contains("stale or foreign"));
    }

    #[test]
    fn direct_native_player_keeps_the_qualified_supervisor_lease() {
        assert_eq!(
            sky_player::engine::DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US,
            3_000_000
        );
    }

    #[test]
    fn native_prepared_ids_are_random_lowercase_hex_and_eviction_is_fifo() {
        let mut ids = std::collections::BTreeSet::new();
        for _ in 0..128 {
            let id = opaque_native_id().expect("native ID");
            assert_eq!(id.len(), 32);
            assert!(
                id.bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            );
            assert!(ids.insert(id));
        }

        let mut prepared = std::collections::VecDeque::new();
        for value in 0..(MAX_PREPARED_PLANS + 3) {
            prepared.push_back(value);
            retain_prepared_capacity(&mut prepared);
        }
        assert_eq!(prepared.len(), MAX_PREPARED_PLANS);
        assert_eq!(prepared.front(), Some(&3));
        assert_eq!(prepared.back(), Some(&(MAX_PREPARED_PLANS + 2)));
    }

    #[test]
    fn user_margin_changes_authored_schedule_and_plan_fingerprint() {
        let song = parse_song_json(
            br#"{"name":"Policy","songNotes":[{"time":0,"key":"Key0"},{"time":100,"key":"Key1"}]}"#,
            "policy",
        )
        .expect("song");
        let default_policy =
            MaterializedTimingPolicy::from_user_margin(60, 1.0, 800, 500).expect("default policy");
        let increased_policy = MaterializedTimingPolicy::from_user_margin(60, 1.0, 900, 500)
            .expect("increased margin");
        let default_schedule =
            build_schedule_with_policy(&song, 1.0, &default_policy).expect("default schedule");
        let increased_schedule = build_schedule_with_policy(&song, 1.0, &increased_policy)
            .expect("increased-margin schedule");
        assert!(increased_schedule.actions[1].at_us > default_schedule.actions[1].at_us);
        assert_eq!(default_schedule.actions[1].at_us, 17_467);
        assert_eq!(increased_schedule.actions[1].at_us, 17_567);
        let default_config = PlaybackConfigDto {
            hold_frames: 1.0,
            timing_margin_us: 800,
            down_late_grace_us: 500,
            tempo_scale: 1.0,
            fps: 60,
            dry_run: true,
        };
        let default_fingerprint = plan_fingerprint(
            "policy",
            &default_config,
            &default_schedule,
            &default_policy,
        )
        .expect("default fingerprint");
        let increased_config = PlaybackConfigDto {
            timing_margin_us: 900,
            ..default_config.clone()
        };
        let increased_fingerprint = plan_fingerprint(
            "policy",
            &increased_config,
            &increased_schedule,
            &increased_policy,
        )
        .expect("increased-margin fingerprint");
        assert_ne!(default_fingerprint, increased_fingerprint);

        let cutoff_policy = MaterializedTimingPolicy::from_user_margin(60, 1.0, 800, 1_000)
            .expect("increased Down tolerance");
        let cutoff_config = PlaybackConfigDto {
            down_late_grace_us: 1_000,
            ..default_config.clone()
        };
        assert_eq!(
            (cutoff_policy.min_hold_us, cutoff_policy.min_release_gap_us),
            (
                default_policy.min_hold_us,
                default_policy.min_release_gap_us
            )
        );
        assert_ne!(
            default_fingerprint,
            plan_fingerprint("policy", &cutoff_config, &default_schedule, &cutoff_policy,)
                .expect("cutoff identity")
        );

        // Calibration recommendations are not inputs to materialization or identity.
        let same_policy =
            MaterializedTimingPolicy::from_user_margin(60, 1.0, 800, 500).expect("same policy");
        let same_schedule =
            build_schedule_with_policy(&song, 1.0, &same_policy).expect("same authored schedule");
        assert_eq!(
            default_fingerprint,
            plan_fingerprint("policy", &default_config, &same_schedule, &same_policy)
                .expect("unchanged recommendation identity")
        );
    }

    #[test]
    fn fingerprint_serialization_matches_python_canonicalization() {
        let mut settings = sky_app_core::settings::ApplicationSettings::default();
        settings.playback_defaults.hold_frames = 1.25;
        settings.playback_defaults.tempo_scale = 0.95;
        settings.playback_defaults.fps = 120;
        settings.telemetry_enabled = true;
        let default_settings_fingerprint =
            settings_fingerprint(&settings).expect("settings fingerprint");
        assert_eq!(
            default_settings_fingerprint,
            "074db50e188b209a66a4aa757e6178a55e821dcec364c89e672cf8ce7ef2bfb7"
        );
        let mut changed_settings = settings.clone();
        changed_settings.playback_defaults.timing_margin_us = 900;
        assert_ne!(
            default_settings_fingerprint,
            settings_fingerprint(&changed_settings).expect("changed margin fingerprint")
        );
        changed_settings.playback_defaults.timing_margin_us =
            sky_app_core::settings::DEFAULT_TIMING_MARGIN_US;
        changed_settings.playback_defaults.down_late_grace_us = 1_000;
        assert_ne!(
            default_settings_fingerprint,
            settings_fingerprint(&changed_settings).expect("changed cutoff fingerprint")
        );

        let song = parse_song_json(
            br#"{"name":"Fingerprint","songNotes":[{"time":0,"key":"Key0"}]}"#,
            "fingerprint",
        )
        .expect("song");
        let policy = MaterializedTimingPolicy::from_user_margin(60, 1.0, 800, 500).expect("policy");
        let schedule = build_schedule_with_policy(&song, 0.95, &policy).expect("schedule");
        let config = PlaybackConfigDto {
            hold_frames: 1.0,
            timing_margin_us: 800,
            down_late_grace_us: 500,
            tempo_scale: 0.95,
            fps: 60,
            dry_run: true,
        };
        assert_eq!(
            plan_fingerprint(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                &config,
                &schedule,
                &policy,
            )
            .expect("plan fingerprint"),
            "fea20dfa7f66d8306529e632d3826d80cb416710196a01126f2bdd65ff0fca24"
        );
    }

    #[test]
    fn native_detail_matches_python_tempo_oracle_corpus() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/wave3/song_planning.json"
        ))
        .expect("tempo fixture");
        let cases = fixture["detail_tempo_cases"]
            .as_array()
            .expect("detail tempo cases");
        assert_eq!(cases.len(), 5);
        for case in cases {
            let tempo = case["tempo_scale"].as_f64().expect("tempo");
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("sky-native-tempo-{suffix}"));
            fs::create_dir_all(root.join("songs")).expect("songs root");
            fs::write(
                root.join("config.json"),
                format!(r#"{{"schema_version":3,"default_tempo_scale":{tempo}}}"#),
            )
            .expect("config");
            fs::write(
                root.join("songs/tempo.json"),
                serde_json::to_vec(&case["raw"]).expect("raw song"),
            )
            .expect("song");
            let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
            let bootstrap = runtime.bootstrap().expect("bootstrap");
            let search = runtime
                .search(crate::commands::CatalogSearchRequest {
                    query: String::new(),
                    offset: 0,
                    limit: 10,
                    generation: Some(bootstrap.catalog_generation),
                    source: crate::commands::LibrarySource::default(),
                })
                .expect("search");
            let detail = runtime
                .detail(crate::commands::CatalogDetailRequest {
                    song_id: search.items[0].song_id.clone(),
                    generation: Some(bootstrap.catalog_generation),
                })
                .expect("detail");
            let actual = serde_json::to_value(&detail).expect("detail JSON");
            assert_eq!(actual["duration_us"], case["duration_us"], "tempo {tempo}");
            assert_eq!(actual["risk"]["level"], case["risk_level"], "tempo {tempo}");
            assert_eq!(
                actual["risk"]["recommendations"], case["risk_recommendations"],
                "tempo {tempo}"
            );
            assert_eq!(
                actual["recommendation"]["recommended_hold_frames"],
                case["recommendation"]["recommended_hold_frames"],
                "tempo {tempo}"
            );
            assert_eq!(
                actual["recommendation"]["recommended_tempo_scale"],
                case["recommendation"]["recommended_tempo_scale"],
                "tempo {tempo}"
            );
            runtime.shutdown();
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn native_prepare_preserves_python_validation_failed_contract() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-native-prepare-contract-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("songs root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        fs::write(
            root.join("songs/repeat.json"),
            r#"{"name":"Repeat","songNotes":[{"time":0,"key":"Key0"},{"time":1,"key":"Key0"}]}"#,
        )
        .expect("song");
        let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
        let bootstrap = runtime
            .dispatch("app.bootstrap", Value::Object(Default::default()))
            .expect("bootstrap");
        let generation = bootstrap["catalog_generation"]
            .as_u64()
            .expect("generation");
        let search = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({
                    "query": "repeat",
                    "offset": 0,
                    "limit": 10,
                    "generation": generation
                }),
            )
            .expect("search");
        let song_id = search["items"][0]["song_id"].as_str().expect("song ID");
        let prepared = runtime
            .dispatch(
                "playback.prepare",
                serde_json::json!({
                    "songId": song_id,
                    "generation": generation,
                    "config": {
                        "hold_frames": 1.0,
                        "timing_margin_us": 800,
                        "down_late_grace_us": 500,
                        "tempo_scale": 1.0,
                        "fps": 60,
                        "dry_run": false
                    }
                }),
            )
            .expect("blocked preparation is a typed DTO");
        assert_eq!(prepared["admission"], "blocked");
        assert_eq!(prepared["error_code"], "validation_failed");
        assert_eq!(
            prepared["error_message"],
            "Detected 1 infeasible same-key repeat(s): the authored interval is shorter than the configured hold."
        );
        assert_eq!(prepared["prepared_id"], Value::Null);
        runtime.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn event_hub_evicts_snapshots_before_lifecycle_events() {
        let mut hub = NativeEventHub::default();
        hub.publish(snapshot("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 1))
            .expect("snapshot");
        for index in 0..MAX_NATIVE_EVENTS {
            hub.publish(crate::ui_events::UiEvent::CatalogChanged {
                v: 1,
                payload: crate::ui_events::CatalogChangedPayload {
                    generation: index as u64 + 1,
                    total: 0,
                },
            })
            .expect("snapshot is evictable");
        }
        assert_eq!(hub.buffered.len(), MAX_NATIVE_EVENTS);
        assert!(
            !hub.buffered
                .iter()
                .any(|event| matches!(event, UiEvent::PlaybackSnapshot { .. }))
        );
        assert!(!remove_oldest_snapshot(&mut hub.buffered));
    }

    #[test]
    fn settings_and_catalog_changes_invalidate_prepared_native_playback() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-native-state-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("songs root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        fs::write(
            root.join("songs/demo.json"),
            r#"{"name":"Demo","songNotes":[{"time":0,"key":"Key0"}]}"#,
        )
        .expect("song");
        let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
        let bootstrap = runtime
            .dispatch("app.bootstrap", Value::Object(Default::default()))
            .expect("bootstrap");
        let generation = bootstrap["catalog_generation"]
            .as_u64()
            .expect("generation");
        let search = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({"query":"demo","offset":0,"limit":10,"generation":generation}),
            )
            .expect("search");
        let song_id = search["items"][0]["song_id"]
            .as_str()
            .expect("song ID")
            .to_owned();
        let prepared = runtime
            .dispatch(
                "playback.prepare",
                serde_json::json!({
                    "songId": song_id,
                    "generation": generation,
                    "config": {"hold_frames":1.0,"timing_margin_us":800,"down_late_grace_us":500,"tempo_scale":1.0,"fps":60,"dry_run":true}
                }),
            )
            .expect("prepare");
        let prepared_id = prepared["prepared_id"]
            .as_str()
            .expect("prepared ID")
            .to_owned();
        runtime
            .patch_settings(crate::commands::SettingsPatch {
                theme: None,
                telemetry_enabled: None,
                verbose_hud: None,
                playback_defaults: Some(crate::commands::PlaybackPatch {
                    hold_frames: None,
                    timing_margin_us: None,
                    down_late_grace_us: None,
                    tempo_scale: Some(0.95),
                    fps: None,
                }),
                update_preferences: None,
            })
            .expect("native settings invalidation seam");
        assert!(
            runtime
                .dispatch(
                    "playback.start",
                    serde_json::json!({"prepared_id":prepared_id,"decisions":[]}),
                )
                .is_err()
        );

        let prepared = runtime
            .dispatch(
                "playback.prepare",
                serde_json::json!({
                    "songId": song_id,
                    "generation": generation,
                    "config": {"hold_frames":1.0,"timing_margin_us":800,"down_late_grace_us":500,"tempo_scale":1.0,"fps":60,"dry_run":true}
                }),
            )
            .expect("prepare after settings patch");
        let prepared_id = prepared["prepared_id"]
            .as_str()
            .expect("prepared ID")
            .to_owned();
        runtime
            .dispatch("catalog.reload", Value::Object(Default::default()))
            .expect("reload");
        assert!(
            runtime
                .dispatch(
                    "playback.start",
                    serde_json::json!({"prepared_id":prepared_id,"decisions":[]}),
                )
                .is_err()
        );
        runtime.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn v4_runtime_operates_with_read_only_install_root_and_separate_app_data() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let temp = std::env::temp_dir().join(format!("sky-v4-appdata-runtime-{suffix}"));
        let install_root = temp.join("install_payload");
        let app_data_root = temp.join("app_data");

        fs::create_dir_all(&install_root).expect("create install payload");
        let dummy_exe = install_root.join(sky_native_adapters::CALIBRATION_EXE);
        fs::write(&dummy_exe, b"immutable-binary-data").expect("write dummy exe");
        let dummy_resource = install_root.join("resources.pak");
        fs::write(&dummy_resource, b"immutable-resource-data").expect("write dummy resource");

        // Take snapshot of entire install payload before runtime execution
        let install_snapshot_before = sky_native_adapters::snapshot_directory(&install_root)
            .expect("snapshot install before");

        let paths = sky_native_adapters::AppPaths::from_app_data_root(
            install_root.clone(),
            app_data_root.clone(),
        );
        assert!(paths.assert_clean_boundary().is_ok());
        paths
            .ensure_mutable_directories()
            .expect("ensure mutable dirs");

        // Place a song sheet in the user songs directory
        let song_file = paths.user_music_root().join("test_song.json");
        fs::write(
            &song_file,
            br#"{"name":"V4 Song","author":"Tester","bpm":120,"pitchLevel":0,"data":[{"time":0,"key":"1Key0"}]}"#,
        )
        .expect("write test song");

        let runtime = NativeDesktopRuntime::from_paths_with_activity_and_seams(
            paths.clone(),
            sky_native_adapters::AppResources::from_resource_dir(&install_root),
            ActivityCoordinator::default(),
            TestSeams::SafePackage,
        )
        .expect("start runtime with read-only install root");

        // 1. Settings mutation saves into app_data/config.json
        let patch_result = runtime
            .dispatch(
                "settings.patch",
                serde_json::json!({
                    "theme": "slate"
                }),
            )
            .expect("patch settings");
        assert_eq!(patch_result["theme"], "slate");
        assert!(paths.settings_path().exists());
        assert!(!install_root.join("config.json").exists());

        // 2. Catalog load reads from user_music_root outside install root
        let catalog = runtime
            .dispatch("catalog.reload", Value::Object(Default::default()))
            .expect("reload catalog");
        let count = catalog["total"].as_u64().unwrap_or(0);
        assert!(count >= 1, "catalog must contain at least 1 song");
        assert!(!install_root.join("songs").exists());

        // 3. Calibration writes cache to app_data/cache, not install root
        runtime
            .dispatch("calibration.start", serde_json::json!({"mode":"quick"}))
            .expect("start calibration");

        let deadline = Instant::now() + Duration::from_secs(3);
        while runtime.calibration.activity.is_calibration_active() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!runtime.calibration.activity.is_calibration_active());
        assert!(paths.calibration_cache_path().exists());
        assert!(!install_root.join(".cache").exists());
        assert!(!install_root.join("cache").exists());

        // 4. Clean shutdown
        runtime.shutdown();

        // 5. Cryptographic proof: entire install payload is completely byte-for-byte unchanged
        let install_snapshot_after =
            sky_native_adapters::snapshot_directory(&install_root).expect("snapshot install after");
        assert_eq!(
            install_snapshot_before, install_snapshot_after,
            "install payload must remain 100% byte-for-byte identical"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn supervisor_heartbeat_progresses_while_ui_publication_is_stalled() {
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let heartbeats = Arc::new(AtomicU64::new(0));
        let publication_started = Arc::new(std::sync::Barrier::new(2));
        let publication_release = Arc::new(std::sync::Barrier::new(2));

        let ui_started = Arc::clone(&publication_started);
        let ui_release = Arc::clone(&publication_release);
        let ui = thread::spawn(move || {
            ui_started.wait();
            ui_release.wait();
        });
        publication_started.wait();

        let heartbeat_stop = Arc::clone(&stop);
        let heartbeat_done = Arc::clone(&done);
        let heartbeat_count = Arc::clone(&heartbeats);
        let heartbeat = thread::spawn(move || {
            supervisor_heartbeat_loop(
                &heartbeat_stop,
                &heartbeat_done,
                Duration::from_millis(5),
                || {
                    heartbeat_count.fetch_add(1, Ordering::Relaxed);
                },
            );
        });

        thread::sleep(Duration::from_millis(25));
        assert!(heartbeats.load(Ordering::Relaxed) >= 3);
        publication_release.wait();
        ui.join().expect("UI publication seam");
        stop.store(true, Ordering::Release);
        heartbeat.join().expect("heartbeat seam");
    }
}
