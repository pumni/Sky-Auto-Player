use super::protocol::CoreResponse;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};

pub(crate) struct PendingRegistry {
    pending: Mutex<HashMap<u64, Sender<Result<CoreResponse, String>>>>,
}

impl PendingRegistry {
    pub(crate) fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn register(&self, id: u64) -> Receiver<Result<CoreResponse, String>> {
        let (sender, receiver) = mpsc::channel();
        self.pending
            .lock()
            .expect("pending registry poisoned")
            .insert(id, sender);
        receiver
    }

    pub(crate) fn complete(&self, id: u64, result: Result<CoreResponse, String>) -> bool {
        self.pending
            .lock()
            .expect("pending registry poisoned")
            .remove(&id)
            .is_some_and(|sender| sender.send(result).is_ok())
    }

    pub(crate) fn remove(&self, id: u64) {
        self.pending
            .lock()
            .expect("pending registry poisoned")
            .remove(&id);
    }

    pub(crate) fn fail_all(&self, error: &str) {
        let senders = std::mem::take(&mut *self.pending.lock().expect("pending registry poisoned"));
        for sender in senders.into_values() {
            let _ = sender.send(Err(error.to_owned()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn responses_correlate_by_id_and_registry_cleans_up() {
        let registry = PendingRegistry::new();
        let first = registry.register(1);
        let second = registry.register(2);
        assert!(registry.complete(
            2,
            Ok(CoreResponse {
                id: 2,
                ok: true,
                result: Some(json!({"n": 2})),
                error: None
            })
        ));
        assert!(registry.complete(
            1,
            Ok(CoreResponse {
                id: 1,
                ok: true,
                result: Some(json!({"n": 1})),
                error: None
            })
        ));
        assert_eq!(second.recv().unwrap().unwrap().id, 2);
        assert_eq!(first.recv().unwrap().unwrap().id, 1);
        assert!(!registry.complete(3, Err("unknown".into())));
    }
}
