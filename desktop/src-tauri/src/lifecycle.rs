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
    if !state.begin_close() {
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
            crate::record_gui_smoke_phase("lifecycle.shutdown_core.enter");
        }
        if let Ok(supervisor) = state.supervisor() {
            supervisor.shutdown();
        }
        if exit_after_close {
            crate::record_gui_smoke_phase("lifecycle.shutdown_core.return");
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

#[cfg(test)]
mod tests {
    use super::AppState;

    fn close_sequence_for_test(
        state: &AppState,
        order: &mut Vec<&'static str>,
        core_shutdown: impl FnOnce(&mut Vec<&'static str>),
        destroy: impl FnOnce(&mut Vec<&'static str>),
    ) -> bool {
        if !state.begin_close() {
            return false;
        }
        core_shutdown(order);
        destroy(order);
        true
    }

    #[test]
    fn controlled_close_cleans_core_before_destroy_and_is_idempotent() {
        let state = AppState::default();
        let mut order = Vec::new();
        assert!(close_sequence_for_test(
            &state,
            &mut order,
            |order| order.push("core_shutdown"),
            |order| order.push("destroy"),
        ));
        assert!(!close_sequence_for_test(
            &state,
            &mut order,
            |order| order.push("duplicate_shutdown"),
            |order| order.push("duplicate_destroy"),
        ));
        assert_eq!(order, ["core_shutdown", "destroy"]);
    }
}
