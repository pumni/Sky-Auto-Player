use crate::app_state::AppState;
use tauri::{Manager, Runtime, Window};

pub fn close_window<R: Runtime + 'static>(window: Window<R>) {
    let app_handle = window.app_handle().clone();
    let state = app_handle.state::<AppState>().inner().clone();
    let exit_after_close = state.should_exit_after_close();
    let exit_code = state.gui_smoke_exit_code();
    if exit_after_close {
        crate::record_gui_smoke_phase("lifecycle.close.enter");
    }
    let startup_failsafe = app_handle
        .try_state::<crate::startup_window::StartupWindowFailsafe>()
        .map(|failsafe| failsafe.inner().clone());
    if !accept_close(&state, startup_failsafe.as_ref()) {
        if exit_after_close {
            crate::record_gui_smoke_phase("lifecycle.close.duplicate");
        }
        return;
    }
    if exit_after_close {
        crate::record_gui_smoke_phase("lifecycle.close.accepted");
    }

    tauri::async_runtime::spawn_blocking(move || {
        if exit_after_close {
            crate::record_gui_smoke_phase("lifecycle.shutdown_native.enter");
        }
        // `app.shutdown` is a Native-owned command. Route lifecycle cleanup
        // through the same executable handler so ownership accounting and the
        // real close path cannot diverge.
        state.shutdown_native();
        if exit_after_close {
            crate::record_gui_smoke_phase("lifecycle.shutdown_native.return");
            crate::record_gui_smoke_phase("lifecycle.window_destroy.enter");
        }
        // `destroy` does not emit another CloseRequested event, so the
        // bounded shutdown path cannot recurse through this callback.
        let _ = window.destroy();
        if exit_after_close {
            crate::record_gui_smoke_phase("lifecycle.window_destroy.return");
        }
        if exit_after_close {
            crate::record_gui_smoke_phase("lifecycle.app_exit.enter");
            app_handle.exit(exit_code);
            crate::record_gui_smoke_phase("lifecycle.app_exit.return");
        }
    });
}

fn accept_close(
    state: &AppState,
    startup_failsafe: Option<&crate::startup_window::StartupWindowFailsafe>,
) -> bool {
    // Cancellation shares the fallback's lock and must precede close acceptance.
    // The fallback keeps that lock through show, so close either cancels it or
    // commits only after the reveal request has completed.
    if let Some(startup_failsafe) = startup_failsafe {
        startup_failsafe.cancel_for_close();
    }
    state.begin_close()
}

#[cfg(test)]
mod tests {
    use super::{AppState, accept_close};

    fn close_sequence_for_test(
        state: &AppState,
        order: &mut Vec<&'static str>,
        native_shutdown: impl FnOnce(&mut Vec<&'static str>),
        destroy: impl FnOnce(&mut Vec<&'static str>),
    ) -> bool {
        if !state.begin_close() {
            return false;
        }
        native_shutdown(order);
        destroy(order);
        true
    }

    #[test]
    fn startup_failsafe_cancellation_precedes_close_commit() {
        let state = AppState::default();
        let failsafe = crate::startup_window::StartupWindowFailsafe::default();

        assert!(accept_close(&state, Some(&failsafe)));
        assert_eq!(
            failsafe.cancel_for_close(),
            crate::startup_window::CloseOutcome::AlreadyClosing
        );
        assert!(!accept_close(&state, Some(&failsafe)));
        assert!(!state.begin_close());
    }

    #[test]
    fn controlled_close_cleans_native_before_destroy_and_is_idempotent() {
        let state = AppState::default();
        let mut order = Vec::new();
        assert!(close_sequence_for_test(
            &state,
            &mut order,
            |order| order.push("native_shutdown"),
            |order| order.push("destroy"),
        ));
        assert!(!close_sequence_for_test(
            &state,
            &mut order,
            |order| order.push("duplicate_shutdown"),
            |order| order.push("duplicate_destroy"),
        ));
        assert_eq!(order, ["native_shutdown", "destroy"]);
    }
}
