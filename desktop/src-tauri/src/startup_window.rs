use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Manager, Runtime};

const FAILSAFE_DELAY: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum StartupWindowPhase {
    #[default]
    Pending,
    Revealed,
    Closing,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FallbackOutcome {
    Shown,
    AlreadyVisible,
    AlreadyRevealed,
    Cancelled,
    ShowFailed,
    VisibilityCheckFailed,
    StateUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseOutcome {
    CancelledPendingStartup,
    ClosedAfterReveal,
    ClosedAfterFailure,
    AlreadyClosing,
}

#[derive(Clone, Default)]
pub(crate) struct StartupWindowFailsafe {
    phase: Arc<Mutex<StartupWindowPhase>>,
}

impl StartupWindowFailsafe {
    pub(crate) fn acknowledge_frontend_reveal(
        &self,
        is_visible: impl FnOnce() -> Result<bool, String>,
    ) -> Result<(), String> {
        let mut phase = match self.phase.lock() {
            Ok(phase) => phase,
            Err(_) => {
                crate::startup_telemetry::record("window.startup_failsafe.state_poisoned");
                return Err("startup window state is unavailable".into());
            }
        };

        match *phase {
            StartupWindowPhase::Pending => match is_visible() {
                Ok(true) => {
                    *phase = StartupWindowPhase::Revealed;
                    crate::startup_telemetry::record("window.startup_failsafe.normal_success");
                    Ok(())
                }
                Ok(false) => {
                    crate::startup_telemetry::record("window.startup_failsafe.ack_rejected_hidden");
                    Err("startup window is not visible".into())
                }
                Err(error) => {
                    crate::startup_telemetry::record(
                        "window.startup_failsafe.visibility_check_failed",
                    );
                    Err(error)
                }
            },
            StartupWindowPhase::Revealed => Ok(()),
            StartupWindowPhase::Closing => Err("startup window is closing".into()),
            StartupWindowPhase::Failed => Err("startup window fallback has failed".into()),
        }
    }

    pub(crate) fn cancel_for_close(&self) -> CloseOutcome {
        let mut phase = match self.phase.lock() {
            Ok(phase) => phase,
            Err(poisoned) => {
                crate::startup_telemetry::record("window.startup_failsafe.state_poisoned");
                poisoned.into_inner()
            }
        };

        let outcome = match *phase {
            StartupWindowPhase::Pending => {
                *phase = StartupWindowPhase::Closing;
                CloseOutcome::CancelledPendingStartup
            }
            StartupWindowPhase::Revealed => {
                *phase = StartupWindowPhase::Closing;
                CloseOutcome::ClosedAfterReveal
            }
            StartupWindowPhase::Failed => {
                *phase = StartupWindowPhase::Closing;
                CloseOutcome::ClosedAfterFailure
            }
            StartupWindowPhase::Closing => CloseOutcome::AlreadyClosing,
        };

        if outcome == CloseOutcome::CancelledPendingStartup {
            crate::startup_telemetry::record("window.startup_failsafe.cancelled");
        }
        outcome
    }

    fn is_pending(&self) -> bool {
        match self.phase.lock() {
            Ok(phase) => *phase == StartupWindowPhase::Pending,
            Err(_) => {
                crate::startup_telemetry::record("window.startup_failsafe.state_poisoned");
                false
            }
        }
    }

    fn reveal_on_expiry(
        &self,
        is_visible: impl FnOnce() -> Result<bool, String>,
        show: impl FnOnce() -> Result<(), String>,
    ) -> FallbackOutcome {
        // Keep the phase lock through show so accepted close and fallback reveal
        // have one ordering boundary.
        let mut phase = match self.phase.lock() {
            Ok(phase) => phase,
            Err(_) => {
                crate::startup_telemetry::record("window.startup_failsafe.state_poisoned");
                return FallbackOutcome::StateUnavailable;
            }
        };

        match *phase {
            StartupWindowPhase::Pending => match is_visible() {
                Ok(true) => {
                    *phase = StartupWindowPhase::Revealed;
                    crate::startup_telemetry::record("window.startup_failsafe.normal_success");
                    FallbackOutcome::AlreadyVisible
                }
                Ok(false) => {
                    crate::startup_telemetry::record("window.startup_failsafe.expired");
                    match show() {
                        Ok(()) => {
                            *phase = StartupWindowPhase::Revealed;
                            crate::startup_telemetry::record(
                                "window.startup_failsafe.fallback_shown",
                            );
                            FallbackOutcome::Shown
                        }
                        Err(error) => {
                            *phase = StartupWindowPhase::Failed;
                            crate::startup_telemetry::record("window.startup_failsafe.show_failed");
                            eprintln!(
                                "startup window fallback failed to show the main window: {error}"
                            );
                            FallbackOutcome::ShowFailed
                        }
                    }
                }
                Err(error) => {
                    crate::startup_telemetry::record(
                        "window.startup_failsafe.visibility_check_failed",
                    );
                    eprintln!(
                        "startup window fallback could not inspect main window visibility: {error}"
                    );
                    FallbackOutcome::VisibilityCheckFailed
                }
            },
            StartupWindowPhase::Revealed => FallbackOutcome::AlreadyRevealed,
            StartupWindowPhase::Closing => FallbackOutcome::Cancelled,
            StartupWindowPhase::Failed => FallbackOutcome::ShowFailed,
        }
    }
}

pub(crate) fn schedule_failsafe<R: Runtime + 'static>(
    app_handle: AppHandle<R>,
    failsafe: StartupWindowFailsafe,
) {
    let spawn_result = thread::Builder::new()
        .name("startup-window-failsafe".into())
        .spawn(move || {
            thread::sleep(FAILSAFE_DELAY);
            if !failsafe.is_pending() {
                return;
            }

            let main_thread_app = app_handle.clone();
            let main_thread_failsafe = failsafe.clone();
            if let Err(error) = app_handle.run_on_main_thread(move || {
                let Some(window) = main_thread_app.get_webview_window("main") else {
                    crate::startup_telemetry::record("window.startup_failsafe.window_missing");
                    eprintln!("startup window fallback could not find the main window");
                    return;
                };

                let outcome = main_thread_failsafe.reveal_on_expiry(
                    || {
                        window
                            .is_visible()
                            .map_err(|error| format!("could not inspect visibility: {error}"))
                    },
                    || window.show().map_err(|error| error.to_string()),
                );
                if outcome == FallbackOutcome::StateUnavailable {
                    eprintln!("startup window fallback stopped because its state is unavailable");
                }
            }) {
                crate::startup_telemetry::record("window.startup_failsafe.dispatch_failed");
                eprintln!("startup window fallback could not reach the UI thread: {error}");
            }
        });

    if let Err(error) = spawn_result {
        crate::startup_telemetry::record("window.startup_failsafe.schedule_failed");
        eprintln!("startup window fallback could not be scheduled: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::{CloseOutcome, FallbackOutcome, StartupWindowFailsafe, StartupWindowPhase};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, mpsc};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn successful_frontend_reveal_completes_startup_once() {
        let failsafe = StartupWindowFailsafe::default();
        let visibility_checks = AtomicUsize::new(0);

        assert!(
            failsafe
                .acknowledge_frontend_reveal(|| {
                    visibility_checks.fetch_add(1, Ordering::SeqCst);
                    Ok(true)
                })
                .is_ok()
        );
        assert!(
            failsafe
                .acknowledge_frontend_reveal(|| {
                    visibility_checks.fetch_add(1, Ordering::SeqCst);
                    Ok(false)
                })
                .is_ok()
        );

        assert_eq!(visibility_checks.load(Ordering::SeqCst), 1);
        assert_eq!(
            *failsafe.phase.lock().unwrap(),
            StartupWindowPhase::Revealed
        );
    }

    #[test]
    fn hidden_window_cannot_acknowledge_startup_success() {
        let failsafe = StartupWindowFailsafe::default();
        let fallback_show_calls = AtomicUsize::new(0);

        assert!(failsafe.acknowledge_frontend_reveal(|| Ok(false)).is_err());
        assert_eq!(*failsafe.phase.lock().unwrap(), StartupWindowPhase::Pending);

        assert_eq!(
            failsafe.reveal_on_expiry(
                || Ok(false),
                || {
                    fallback_show_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            ),
            FallbackOutcome::Shown
        );
        assert_eq!(fallback_show_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn late_timer_expiry_does_not_reveal_a_successfully_hidden_window() {
        let failsafe = StartupWindowFailsafe::default();
        failsafe
            .acknowledge_frontend_reveal(|| Ok(true))
            .expect("frontend reveal should complete");
        let fallback_show_calls = AtomicUsize::new(0);

        let outcome = failsafe.reveal_on_expiry(
            || Ok(false),
            || {
                fallback_show_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        );

        assert_eq!(outcome, FallbackOutcome::AlreadyRevealed);
        assert_eq!(fallback_show_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn accepted_close_before_fallback_prevents_reveal() {
        let failsafe = StartupWindowFailsafe::default();
        assert_eq!(
            failsafe.cancel_for_close(),
            CloseOutcome::CancelledPendingStartup
        );
        assert!(
            failsafe
                .acknowledge_frontend_reveal(|| panic!(
                    "closed startup must not inspect visibility"
                ))
                .is_err()
        );
        let fallback_show_calls = AtomicUsize::new(0);

        let outcome = failsafe.reveal_on_expiry(
            || Ok(false),
            || {
                fallback_show_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        );

        assert_eq!(outcome, FallbackOutcome::Cancelled);
        assert_eq!(fallback_show_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn close_and_fallback_reveal_share_a_linearizable_transition() {
        let failsafe = StartupWindowFailsafe::default();
        let show_started = Arc::new(Barrier::new(2));
        let release_show = Arc::new(Barrier::new(2));
        let show_finished = Arc::new(AtomicBool::new(false));
        let fallback_failsafe = failsafe.clone();
        let fallback_started = show_started.clone();
        let fallback_release = release_show.clone();
        let fallback_finished = show_finished.clone();
        let fallback = thread::spawn(move || {
            fallback_failsafe.reveal_on_expiry(
                || Ok(false),
                || {
                    fallback_started.wait();
                    fallback_release.wait();
                    fallback_finished.store(true, Ordering::SeqCst);
                    Ok(())
                },
            )
        });

        show_started.wait();
        let close_failsafe = failsafe.clone();
        let (close_started_tx, close_started_rx) = mpsc::channel();
        let (close_done_tx, close_done_rx) = mpsc::channel();
        let close = thread::spawn(move || {
            close_started_tx.send(()).expect("close start receiver");
            let outcome = close_failsafe.cancel_for_close();
            close_done_tx
                .send((outcome, show_finished.load(Ordering::SeqCst)))
                .expect("close result receiver");
        });

        close_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("close should start while fallback owns the transition lock");
        let close_completed_before_show = close_done_rx.try_recv().ok();
        release_show.wait();

        assert_eq!(fallback.join().unwrap(), FallbackOutcome::Shown);
        let close_result = close_completed_before_show.unwrap_or_else(|| {
            close_done_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("accepted close should finish after fallback reveal")
        });
        assert!(close_completed_before_show.is_none());
        assert_eq!(close_result, (CloseOutcome::ClosedAfterReveal, true));
        close.join().unwrap();
    }

    #[test]
    fn fallback_show_failure_is_terminal_and_diagnostic() {
        let failsafe = StartupWindowFailsafe::default();
        let outcome = failsafe.reveal_on_expiry(|| Ok(false), || Err("show failed".into()));
        assert_eq!(outcome, FallbackOutcome::ShowFailed);
        assert_eq!(*failsafe.phase.lock().unwrap(), StartupWindowPhase::Failed);
        assert_eq!(
            failsafe.cancel_for_close(),
            CloseOutcome::ClosedAfterFailure
        );
    }
}
