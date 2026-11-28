use super::*;

#[derive(Clone, Debug)]
pub(crate) struct NativeUpdateCandidate {
    pub version: String,
    pub channel: UpdateChannel,
    pub release_notes: Option<String>,
    pub published_at: Option<String>,
}

pub(crate) struct NativeUpdateState {
    pub(crate) revision: u64,
    pub(crate) channel: UpdateChannel,
    pub(crate) candidate: Option<NativeUpdateCandidate>,
    pub(crate) updates: Vec<Update>,
    pub(crate) operation_id: Option<String>,
    pub(crate) state: UpdateState,
    pub(crate) error_code: Option<UpdateErrorCode>,
    pub(crate) error_detail: Option<String>,
    pub(crate) retry_action: UpdateRetryAction,
    pub(crate) progress: Option<UpdateProgressDto>,
}

impl Default for NativeUpdateState {
    fn default() -> Self {
        Self {
            revision: 0,
            channel: UpdateChannel::Stable,
            candidate: None,
            updates: Vec::new(),
            operation_id: None,
            state: UpdateState::Idle,
            error_code: None,
            error_detail: None,
            retry_action: UpdateRetryAction::None,
            progress: None,
        }
    }
}

impl NativeUpdateState {
    pub(crate) fn apply_transition(
        &mut self,
        transition: StateTransition,
    ) -> (u64, UpdateSnapshotPayload) {
        self.revision += 1;

        let error_detail = transition.error_detail.as_deref().map(bounded);

        self.state = transition.state;
        self.channel = transition.channel;
        self.candidate = transition.candidate.clone();
        self.updates = transition.updates;
        self.operation_id = transition.operation_id.clone();
        self.progress = transition.progress.clone();
        self.error_code = transition.error_code;
        self.error_detail = error_detail.clone();
        self.retry_action = transition.retry_action;

        let (available_version, release_notes, published_at) = match &transition.candidate {
            Some(c) => (
                Some(c.version.clone()),
                c.release_notes.clone(),
                c.published_at.clone(),
            ),
            None => (None, None, None),
        };

        let snapshot = UpdateSnapshotPayload {
            revision: self.revision,
            state: transition.state,
            current_version: env!("CARGO_PKG_VERSION").to_owned(),
            available_version,
            channel: transition.channel,
            release_notes,
            published_at,
            error_code: transition.error_code,
            error_detail,
            retry_action: transition.retry_action,
            operation_id: transition.operation_id,
            progress: transition.progress,
        };

        (self.revision, snapshot)
    }

    pub(crate) fn snapshot(&self) -> UpdateSnapshotPayload {
        let (available_version, release_notes, published_at) = match &self.candidate {
            Some(c) => (
                Some(c.version.clone()),
                c.release_notes.clone(),
                c.published_at.clone(),
            ),
            None => (None, None, None),
        };

        UpdateSnapshotPayload {
            revision: self.revision,
            state: self.state,
            current_version: env!("CARGO_PKG_VERSION").to_owned(),
            available_version,
            channel: self.channel,
            release_notes,
            published_at,
            error_code: self.error_code,
            error_detail: self.error_detail.clone(),
            retry_action: self.retry_action,
            operation_id: self.operation_id.clone(),
            progress: self.progress.clone(),
        }
    }
}

pub(crate) struct StateTransition {
    pub(super) state: UpdateState,
    pub(super) channel: UpdateChannel,
    pub(super) candidate: Option<NativeUpdateCandidate>,
    pub(super) updates: Vec<Update>,
    pub(super) operation_id: Option<String>,
    pub(super) progress: Option<UpdateProgressDto>,
    pub(super) error_code: Option<UpdateErrorCode>,
    pub(super) error_detail: Option<String>,
    pub(super) retry_action: UpdateRetryAction,
}

impl Default for StateTransition {
    fn default() -> Self {
        Self {
            state: UpdateState::Idle,
            channel: UpdateChannel::Stable,
            candidate: None,
            updates: Vec::new(),
            operation_id: None,
            progress: None,
            error_code: None,
            error_detail: None,
            retry_action: UpdateRetryAction::None,
        }
    }
}
