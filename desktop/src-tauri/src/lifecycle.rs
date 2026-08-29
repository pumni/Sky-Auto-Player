use crate::app_state::AppState;
use crate::core::CoreSupervisor;
use tauri::{App, AppHandle, Manager};

pub fn start_core(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let supervisor = CoreSupervisor::spawn()?;
    app.state::<AppState>().install(supervisor);
    Ok(())
}

pub fn shutdown_core(app: &AppHandle) {
    if let Ok(supervisor) = app.state::<AppState>().supervisor() {
        supervisor.shutdown();
    }
}
