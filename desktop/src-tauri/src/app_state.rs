use crate::core::CoreSupervisor;
use std::sync::{Arc, Mutex};

pub struct AppState {
    supervisor: Mutex<Option<Arc<CoreSupervisor>>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            supervisor: Mutex::new(None),
        }
    }
}

impl AppState {
    pub fn install(&self, supervisor: Arc<CoreSupervisor>) {
        *self.supervisor.lock().expect("desktop state poisoned") = Some(supervisor);
    }

    pub fn supervisor(&self) -> Result<Arc<CoreSupervisor>, String> {
        self.supervisor
            .lock()
            .expect("desktop state poisoned")
            .clone()
            .ok_or_else(|| "Desktop Core has not started".into())
    }
}
