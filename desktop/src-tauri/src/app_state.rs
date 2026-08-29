use crate::core::CoreSupervisor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};

#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    core: Mutex<CoreState>,
    settings_writes: Mutex<()>,
    closing: AtomicBool,
}

enum CoreState {
    Idle,
    Starting(Vec<mpsc::Sender<Result<Arc<CoreSupervisor>, String>>>),
    Ready(Arc<CoreSupervisor>),
    Failed(String),
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                core: Mutex::new(CoreState::Idle),
                settings_writes: Mutex::new(()),
                closing: AtomicBool::new(false),
            }),
        }
    }
}

impl AppState {
    /// Start the Core at most once and let concurrent callers share its result.
    /// The caller must invoke this from a blocking worker because Core startup
    /// waits for the child process readiness event.
    pub fn ensure_core_blocking(&self) -> Result<Arc<CoreSupervisor>, String> {
        self.ensure_core_blocking_with(|| {
            CoreSupervisor::spawn().map_err(|error| error.to_string())
        })
    }

    fn ensure_core_blocking_with<F>(&self, starter: F) -> Result<Arc<CoreSupervisor>, String>
    where
        F: FnOnce() -> Result<Arc<CoreSupervisor>, String>,
    {
        let receiver = {
            let mut state = self.inner.core.lock().expect("desktop state poisoned");
            match &mut *state {
                CoreState::Ready(supervisor) => return Ok(Arc::clone(supervisor)),
                CoreState::Failed(error) => return Err(error.clone()),
                CoreState::Starting(waiters) => {
                    let (sender, receiver) = mpsc::channel();
                    waiters.push(sender);
                    Some(receiver)
                }
                CoreState::Idle => {
                    *state = CoreState::Starting(Vec::new());
                    None
                }
            }
        };

        if let Some(receiver) = receiver {
            return receiver
                .recv()
                .unwrap_or_else(|_| Err("Core startup worker stopped unexpectedly".into()));
        }

        let result = if self.is_closing() {
            Err("Desktop application is closing".into())
        } else {
            starter()
        };
        let result = match result {
            Ok(supervisor) if self.is_closing() => {
                supervisor.shutdown();
                Err("Desktop application is closing".into())
            }
            result => result,
        };
        let mut state = self.inner.core.lock().expect("desktop state poisoned");
        let waiters = match std::mem::replace(&mut *state, CoreState::Idle) {
            CoreState::Starting(waiters) => waiters,
            other => {
                *state = other;
                return Err("Core startup state was corrupted".into());
            }
        };
        match &result {
            Ok(supervisor) => *state = CoreState::Ready(Arc::clone(supervisor)),
            Err(error) => *state = CoreState::Failed(error.clone()),
        }
        for waiter in waiters {
            let _ = waiter.send(result.clone());
        }
        result
    }

    pub fn supervisor(&self) -> Result<Arc<CoreSupervisor>, String> {
        match &*self.inner.core.lock().expect("desktop state poisoned") {
            CoreState::Ready(supervisor) => Ok(Arc::clone(supervisor)),
            CoreState::Starting(_) => Err("Desktop Core is still starting".into()),
            CoreState::Idle => Err("Desktop Core has not started".into()),
            CoreState::Failed(error) => Err(error.clone()),
        }
    }

    pub fn lock_settings_writes(&self) -> MutexGuard<'_, ()> {
        self.inner
            .settings_writes
            .lock()
            .expect("settings write queue poisoned")
    }

    pub fn begin_close(&self) -> bool {
        !self.inner.closing.swap(true, Ordering::AcqRel)
    }

    fn is_closing(&self) -> bool {
        self.inner.closing.load(Ordering::Acquire)
    }

    #[cfg(all(test, feature = "tauri-test", not(feature = "desktop-runtime")))]
    pub(crate) fn install_ready_for_test(&self, supervisor: Arc<CoreSupervisor>) {
        *self.inner.core.lock().expect("desktop state poisoned") = CoreState::Ready(supervisor);
    }
}

#[cfg(test)]
mod tests {
    use super::AppState;

    #[test]
    fn close_transition_is_idempotent() {
        let state = AppState::default();
        assert!(state.begin_close());
        assert!(!state.begin_close());
        assert!(state.is_closing());
    }

    #[test]
    fn startup_is_single_flight_and_waiters_share_failure() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let state = Arc::new(AppState::default());
        let barrier = Arc::new(Barrier::new(2));
        let first_state = Arc::clone(&state);
        let first_barrier = Arc::clone(&barrier);
        let first = thread::spawn(move || {
            first_state.ensure_core_blocking_with(|| {
                first_barrier.wait();
                Err("deterministic startup failure".into())
            })
        });
        barrier.wait();
        let second = state.ensure_core_blocking_with(|| {
            panic!("a concurrent caller must not launch a second Core")
        });
        let first_result = first.join().expect("startup worker panicked");
        let first_error = match first_result {
            Ok(_) => panic!("startup must fail"),
            Err(error) => error,
        };
        let second_error = match second {
            Ok(_) => panic!("waiter must observe startup failure"),
            Err(error) => error,
        };
        assert_eq!(first_error, "deterministic startup failure");
        assert_eq!(second_error, "deterministic startup failure");
    }

    #[test]
    fn startup_does_not_launch_when_close_wins_the_race() {
        let state = AppState::default();
        assert!(state.begin_close());
        let result =
            state.ensure_core_blocking_with(|| panic!("closing state must prevent Core launch"));
        let error = match result {
            Ok(_) => panic!("closing state must reject startup"),
            Err(error) => error,
        };
        assert_eq!(error, "Desktop application is closing");
    }
}
