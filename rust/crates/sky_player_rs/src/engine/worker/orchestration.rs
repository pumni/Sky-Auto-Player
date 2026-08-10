#[cfg(any(test, feature = "test-support"))]
use super::super::BackendConfig;
#[cfg(any(test, feature = "test-support"))]
use super::super::CommandTimingCleanup;
use super::{
    FinalizeInput, FinalizePublication, FinalizeResources, FinalizeSignals, FinalizeState,
    FinalizeTiming, Worker, boot, dispatch_loop, finalize_worker,
};
use std::any::Any;

pub(super) fn run(worker: &mut Worker<'_>) -> u8 {
    let _shared = worker.shared;
    #[cfg(any(test, feature = "test-support"))]
    let _command_timing_cleanup = CommandTimingCleanup(&_shared.commands.command_timing);
    #[cfg(any(test, feature = "test-support"))]
    let (focus_loss_fault, wait_fault) = match &worker.config.backend {
        BackendConfig::Production => (false, false),
        BackendConfig::Mock { fault_script, .. } => (
            fault_script.focus_loss_after_due_before_send,
            fault_script.wait_failure,
        ),
    };
    #[cfg(not(any(test, feature = "test-support")))]
    let (focus_loss_fault, wait_fault) = (false, false);

    let exit_code = boot::initialize(worker, wait_fault);
    if exit_code != 0 {
        return exit_code;
    }
    let worker_result = dispatch_loop::dispatch(worker, focus_loss_fault);
    worker.finalize(worker_result)
}

impl Worker<'_> {
    fn finalize(&mut self, worker_result: Result<(), Box<dyn Any + Send>>) -> u8 {
        let shared = self.shared;
        let core = &mut self.core;
        let resources = core
            .resources
            .take()
            .expect("worker resources available for finalization");
        let timing = core
            .timing
            .as_ref()
            .expect("worker timing available for finalization");

        finalize_worker(FinalizeInput {
            resources: FinalizeResources {
                backend: resources.backend,
                coordinator: resources.coordinator,
                telemetry: resources.telemetry,
                estimator: resources.estimator,
                playback: resources.playback,
                qpc_clock: resources.clock,
                scheduling: resources.scheduling,
            },
            state: FinalizeState {
                worker_result,
                local_metrics: std::mem::take(&mut core.metrics),
                abort_counts: std::mem::take(&mut core.errors.abort_counts),
                force_full_cleanup: core.runtime.force_full_cleanup,
                terminal_error: std::mem::take(&mut core.runtime.terminal_error),
                secondary_errors: std::mem::take(&mut core.errors.secondary),
                last_published_error: std::mem::take(&mut core.errors.last_published),
            },
            signals: FinalizeSignals {
                target_hwnd: &shared.target.target_hwnd,
                skip_requested: &shared.commands.skip_requested,
                quit_requested: &shared.commands.quit_requested,
            },
            publication: FinalizePublication {
                metrics: &shared.publication.metrics,
                telemetry_output: &shared.publication.telemetry_output,
                estimator_output: &shared.publication.estimator_output,
                priority_acquired: &shared.publication.priority_acquired,
                progress_clock: &shared.publication.progress_clock,
            },
            timing: FinalizeTiming {
                start_wall_time_us: timing.start_wall_time_us,
                start_thread_cpu_us: timing.start_thread_cpu_us,
                start_process_cpu_us: timing.start_process_cpu_us,
            },
        })
    }
}
