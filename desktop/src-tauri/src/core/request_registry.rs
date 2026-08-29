use super::protocol::CoreResponse;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};

const MAX_EXPIRED_REQUESTS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Completion {
    Delivered,
    /// The caller already received a timeout. Ignore this response so a slow
    /// but otherwise healthy Core cannot poison the whole session.
    LateAfterTimeout,
    Unknown,
}

pub(crate) struct PendingRegistry {
    pending: Mutex<HashMap<u64, Sender<Result<CoreResponse, String>>>>,
    expired: Mutex<ExpiredRequests>,
}

#[derive(Default)]
struct ExpiredRequests {
    ids: HashSet<u64>,
    order: VecDeque<u64>,
}

impl PendingRegistry {
    pub(crate) fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            expired: Mutex::new(ExpiredRequests::default()),
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

    pub(crate) fn complete(&self, id: u64, result: Result<CoreResponse, String>) -> Completion {
        if let Some(sender) = self
            .pending
            .lock()
            .expect("pending registry poisoned")
            .remove(&id)
        {
            let _ = sender.send(result);
            return Completion::Delivered;
        }

        let mut expired = self.expired.lock().expect("expired registry poisoned");
        if expired.ids.remove(&id) {
            expired.order.retain(|expired_id| *expired_id != id);
            Completion::LateAfterTimeout
        } else {
            Completion::Unknown
        }
    }

    pub(crate) fn remove(&self, id: u64) {
        self.pending
            .lock()
            .expect("pending registry poisoned")
            .remove(&id);
    }

    pub(crate) fn expire(&self, id: u64) {
        self.pending
            .lock()
            .expect("pending registry poisoned")
            .remove(&id);
        let mut expired = self.expired.lock().expect("expired registry poisoned");
        if expired.ids.insert(id) {
            expired.order.push_back(id);
        }
        while expired.order.len() > MAX_EXPIRED_REQUESTS {
            if let Some(oldest) = expired.order.pop_front() {
                expired.ids.remove(&oldest);
            }
        }
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
        assert_eq!(
            registry.complete(
                2,
                Ok(CoreResponse {
                    id: 2,
                    ok: true,
                    result: Some(json!({"n": 2})),
                    error: None
                })
            ),
            Completion::Delivered
        );
        assert_eq!(
            registry.complete(
                1,
                Ok(CoreResponse {
                    id: 1,
                    ok: true,
                    result: Some(json!({"n": 1})),
                    error: None
                })
            ),
            Completion::Delivered
        );
        assert_eq!(second.recv().unwrap().unwrap().id, 2);
        assert_eq!(first.recv().unwrap().unwrap().id, 1);
        assert_eq!(
            registry.complete(3, Err("unknown".into())),
            Completion::Unknown
        );
    }

    #[test]
    fn late_response_after_timeout_is_ignored_once() {
        let registry = PendingRegistry::new();
        let receiver = registry.register(7);
        registry.expire(7);
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            registry.complete(
                7,
                Ok(CoreResponse {
                    id: 7,
                    ok: true,
                    result: Some(json!({})),
                    error: None,
                }),
            ),
            Completion::LateAfterTimeout
        );
        assert_eq!(
            registry.complete(7, Err("unknown".into())),
            Completion::Unknown
        );
    }
}
