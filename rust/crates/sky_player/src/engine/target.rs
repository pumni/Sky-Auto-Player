use parking_lot::{Mutex, MutexGuard};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TargetProofOperationCounts {
    pub(super) authority_arc_clones: u64,
    pub(super) authority_mutex_attempts: u64,
    pub(super) writer_permit_attempts: u64,
    pub(super) generation_revalidations: u64,
}

#[cfg(any(test, feature = "test-support"))]
std::thread_local! {
    static TARGET_PROOF_OPERATION_COUNTS: std::cell::Cell<TargetProofOperationCounts> =
        const { std::cell::Cell::new(TargetProofOperationCounts {
            authority_arc_clones: 0,
            authority_mutex_attempts: 0,
            writer_permit_attempts: 0,
            generation_revalidations: 0,
        }) };
}

#[cfg(any(test, feature = "test-support"))]
fn record_target_proof_operation(update: impl FnOnce(&mut TargetProofOperationCounts)) {
    TARGET_PROOF_OPERATION_COUNTS.with(|counts| {
        let mut current = counts.get();
        update(&mut current);
        counts.set(current);
    });
}

/// Session-owned target publication and its retained process authority.
///
/// The worker and watchdog share this object. Publishing a new target epoch
/// invalidates the old owner atomics and drops the stored authority; any
/// worker query already holding an `Arc` can finish, but its final epoch check
/// cannot authorize that stale binding.
pub struct SessionTarget {
    target_hwnd: AtomicIsize,
    target_generation: AtomicU64,
    publication_epoch: AtomicU64,
    owner_identity_required: AtomicBool,
    owner_pid: AtomicU32,
    owner_generation: AtomicU64,
    owner_live: AtomicBool,
    owner_failure: AtomicU32,
    owner_process_identity_required: AtomicBool,
    /// Keeps the query-only process handle alive for the whole session
    /// control-plane lifetime. The precision path uses `try_lock`, clones the
    /// immutable authority, and performs OS queries without holding this lock.
    owner_authority: Mutex<Option<(Arc<sky_dispatch_win32::focus::WindowIdentityAuthority>, u64)>>,
    writer: Mutex<()>,
    #[cfg(test)]
    owner_poll_result_barrier_for_test:
        Mutex<Option<(Arc<std::sync::Barrier>, Arc<std::sync::Barrier>)>>,
    #[cfg(test)]
    owner_close_snapshot_barrier_for_test:
        Mutex<Option<(Arc<std::sync::Barrier>, Arc<std::sync::Barrier>)>>,
    #[cfg(test)]
    safety_release_validation_barrier_for_test:
        Mutex<Option<(Arc<std::sync::Barrier>, Arc<std::sync::Barrier>)>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OwnerIdentityStatus {
    NotRequired,
    Missing,
    Stale,
    ProcessTerminated,
    OwnerMismatch,
    WindowUnavailable,
    ProcessIdentityMismatch,
    QueryUnavailable,
    Bound(u32),
}

#[repr(u8)]
#[derive(Clone, Copy)]
pub(super) enum OwnerIdentityFailure {
    None = 0,
    ProcessTerminated = 1,
    OwnerMismatch = 2,
    WindowUnavailable = 3,
    ProcessIdentityMismatch = 4,
    QueryUnavailable = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OwnerAuthorityReadError {
    Busy,
    Status(OwnerIdentityStatus),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReleaseOwnerPermitError {
    WriterBusy,
    OwnerAuthorityBusy,
    NotCurrent,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TargetSnapshotReadPoint {
    EpochStart,
    Hwnd,
    Generation,
    EpochEnd,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TargetPublicationWritePoint {
    BeforeOwnerAuthorityReset,
    InProgress,
    HwndPublished,
    GenerationPublished,
    Stable,
}

impl SessionTarget {
    pub(super) fn new(hwnd: isize, generation: u64) -> Self {
        Self {
            target_hwnd: AtomicIsize::new(hwnd),
            target_generation: AtomicU64::new(generation),
            publication_epoch: AtomicU64::new(0),
            owner_identity_required: AtomicBool::new(false),
            owner_pid: AtomicU32::new(0),
            owner_generation: AtomicU64::new(0),
            owner_live: AtomicBool::new(false),
            owner_failure: AtomicU32::new(OwnerIdentityFailure::None as u32),
            owner_process_identity_required: AtomicBool::new(false),
            owner_authority: Mutex::new(None),
            writer: Mutex::new(()),
            #[cfg(test)]
            owner_poll_result_barrier_for_test: Mutex::new(None),
            #[cfg(test)]
            owner_close_snapshot_barrier_for_test: Mutex::new(None),
            #[cfg(test)]
            safety_release_validation_barrier_for_test: Mutex::new(None),
        }
    }

    pub(super) fn require_owner_identity(&self) {
        self.owner_identity_required.store(true, Ordering::Release);
        self.owner_process_identity_required
            .store(true, Ordering::Release);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn bind_owner_identity(&self, hwnd: isize, generation: u64, owner_pid: u32) -> bool {
        self.bind_owner_identity_inner(hwnd, generation, owner_pid, None)
    }

    pub(super) fn bind_owner_identity_authority(
        &self,
        hwnd: isize,
        generation: u64,
        owner_pid: u32,
        authority: Arc<sky_dispatch_win32::focus::WindowIdentityAuthority>,
    ) -> bool {
        if authority.identity().hwnd != hwnd {
            return false;
        }
        self.bind_owner_identity_inner(hwnd, generation, owner_pid, Some(authority))
    }

    fn bind_owner_identity_inner(
        &self,
        hwnd: isize,
        generation: u64,
        owner_pid: u32,
        authority: Option<Arc<sky_dispatch_win32::focus::WindowIdentityAuthority>>,
    ) -> bool {
        if hwnd == 0 || owner_pid == 0 {
            return false;
        }
        let _writer = self.writer.lock();
        if self.publication_epoch.load(Ordering::SeqCst) & 1 != 0
            || self.target_hwnd.load(Ordering::SeqCst) != hwnd
            || self.target_generation.load(Ordering::SeqCst) != generation
        {
            return false;
        }
        if self.owner_pid.load(Ordering::SeqCst) != 0
            && self.owner_generation.load(Ordering::SeqCst) == generation
        {
            let authority_upgrade = authority.is_some()
                && !self.owner_process_identity_required.load(Ordering::SeqCst)
                && self.owner_authority.lock().is_none();
            if !authority_upgrade {
                return false;
            }
        }

        self.owner_identity_required.store(true, Ordering::SeqCst);
        self.owner_failure
            .store(OwnerIdentityFailure::None as u32, Ordering::SeqCst);
        self.owner_live.store(false, Ordering::SeqCst);
        let has_authority = authority.is_some();
        *self.owner_authority.lock() = authority.map(|authority| (authority, generation));
        if has_authority {
            self.owner_process_identity_required
                .store(true, Ordering::SeqCst);
        }
        self.owner_pid.store(owner_pid, Ordering::SeqCst);
        self.owner_generation.store(generation, Ordering::SeqCst);
        self.owner_live.store(true, Ordering::SeqCst);
        true
    }

    pub(super) fn owner_process_identity_required(&self) -> bool {
        self.owner_process_identity_required.load(Ordering::Acquire)
    }

    /// Obtain an authority snapshot for a final worker gate without waiting
    /// behind control-plane binding or polling. Contention rejects this one
    /// packet before the syscall and lets the worker replan.
    pub(super) fn try_owner_authority_for_generation(
        &self,
        expected_generation: u64,
    ) -> Result<Arc<sky_dispatch_win32::focus::WindowIdentityAuthority>, OwnerAuthorityReadError>
    {
        #[cfg(any(test, feature = "test-support"))]
        record_target_proof_operation(|counts| {
            counts.authority_mutex_attempts = counts.authority_mutex_attempts.saturating_add(1)
        });
        let Some(authority) = self.owner_authority.try_lock() else {
            return Err(OwnerAuthorityReadError::Busy);
        };
        let owner_pid = match self.owner_identity_status(expected_generation) {
            OwnerIdentityStatus::Bound(owner_pid) if owner_pid != 0 => owner_pid,
            status => return Err(OwnerAuthorityReadError::Status(status)),
        };
        let Some((hwnd, generation)) = self.load_stable() else {
            return Err(OwnerAuthorityReadError::Status(OwnerIdentityStatus::Stale));
        };
        if generation != expected_generation {
            return Err(OwnerAuthorityReadError::Status(OwnerIdentityStatus::Stale));
        }
        let Some((authority, bound_generation)) = authority.as_ref() else {
            return Err(OwnerAuthorityReadError::Status(
                OwnerIdentityStatus::Missing,
            ));
        };
        if *bound_generation != expected_generation || authority.identity().hwnd != hwnd {
            return Err(OwnerAuthorityReadError::Status(OwnerIdentityStatus::Stale));
        }
        if authority.identity().owner_pid != owner_pid {
            return Err(OwnerAuthorityReadError::Status(
                OwnerIdentityStatus::OwnerMismatch,
            ));
        }
        #[cfg(any(test, feature = "test-support"))]
        record_target_proof_operation(|counts| {
            counts.authority_arc_clones = counts.authority_arc_clones.saturating_add(1)
        });
        Ok(Arc::clone(authority))
    }

    pub(super) fn try_release_owner_proof_for_generation(
        &self,
        expected_hwnd: isize,
        expected_generation: u64,
    ) -> Result<sky_dispatch_win32::input::ReleaseOwnerProof, OwnerAuthorityReadError> {
        if self.load_stable() != Some((expected_hwnd, expected_generation)) {
            return Err(OwnerAuthorityReadError::Status(OwnerIdentityStatus::Stale));
        }
        let mut owner_pid = None;
        let mut authority = None;
        if self.owner_identity_required.load(Ordering::Acquire) {
            match self.owner_identity_status(expected_generation) {
                OwnerIdentityStatus::Bound(pid) if pid != 0 => owner_pid = Some(pid),
                status => return Err(OwnerAuthorityReadError::Status(status)),
            }
            if self.owner_process_identity_required() {
                let retained = self.try_owner_authority_for_generation(expected_generation)?;
                if retained.identity().hwnd != expected_hwnd
                    || Some(retained.identity().owner_pid) != owner_pid
                {
                    return Err(OwnerAuthorityReadError::Status(
                        OwnerIdentityStatus::OwnerMismatch,
                    ));
                }
                authority = Some(retained);
            }
        }
        sky_dispatch_win32::input::ReleaseOwnerProof::new(
            expected_hwnd,
            expected_generation,
            owner_pid,
            authority,
        )
        .ok_or(OwnerAuthorityReadError::Status(
            OwnerIdentityStatus::OwnerMismatch,
        ))
    }

    pub(super) fn owner_authority_snapshot(
        &self,
    ) -> Option<(Arc<sky_dispatch_win32::focus::WindowIdentityAuthority>, u64)> {
        self.owner_authority
            .lock()
            .as_ref()
            .map(|(authority, generation)| (Arc::clone(authority), *generation))
    }

    /// Apply a supervisor poll failure only while both the published target
    /// and retained authority still identify the snapshot that was queried.
    /// The writer lock serializes this comparison and invalidation with target
    /// publication, owner binding, and close.
    pub(super) fn invalidate_polled_owner_if_current(
        &self,
        authority: &Arc<sky_dispatch_win32::focus::WindowIdentityAuthority>,
        generation: u64,
        failure: OwnerIdentityFailure,
    ) -> bool {
        let _writer = self.writer.lock();
        if !self.owner_authority_matches_locked(authority, generation)
            || !self.owner_live.load(Ordering::SeqCst)
        {
            return false;
        }
        self.owner_failure.store(failure as u32, Ordering::SeqCst);
        self.owner_live.store(false, Ordering::SeqCst);
        true
    }

    /// A completed successful poll is useful only when it still describes
    /// the current retained authority. It never mutates a newer binding.
    pub(super) fn polled_owner_is_current(
        &self,
        authority: &Arc<sky_dispatch_win32::focus::WindowIdentityAuthority>,
        generation: u64,
    ) -> bool {
        let _writer = self.writer.lock();
        self.owner_authority_matches_locked(authority, generation)
            && self.owner_live.load(Ordering::SeqCst)
    }

    /// Close a previously snapshotted authority without allowing a late
    /// teardown callback to clear a replacement binding.
    pub(super) fn close_owner_authority_if_current(
        &self,
        authority: &Arc<sky_dispatch_win32::focus::WindowIdentityAuthority>,
        generation: u64,
    ) -> bool {
        let _writer = self.writer.lock();
        if !self.owner_authority_matches_locked(authority, generation) {
            return false;
        }
        self.owner_authority.lock().take();
        self.owner_failure.store(
            OwnerIdentityFailure::ProcessTerminated as u32,
            Ordering::SeqCst,
        );
        self.owner_live.store(false, Ordering::SeqCst);
        true
    }

    /// Acquire a short-lived release permit which serializes the final origin
    /// proof check with target publication, binding, close, and the ensuing
    /// safety release syscall. Callers must hold the returned guard through
    /// that syscall. The potentially slow process continuity query happens
    /// before this method, so a generation change during that query rejects
    /// the permit rather than being ordered after a stale release.
    pub(super) fn lock_current_release_owner_proof(
        &self,
        proof: &sky_dispatch_win32::input::ReleaseOwnerProof,
    ) -> Option<MutexGuard<'_, ()>> {
        let writer = self.writer.lock();
        if self.release_owner_proof_is_current_serialized(proof) {
            Some(writer)
        } else {
            None
        }
    }

    /// Try to hold the current owner binding stable through one musical
    /// SendInput call. Unlike cleanup, the realtime path must not wait behind
    /// target publication: contention rejects this candidate before syscall.
    pub(super) fn try_lock_current_release_owner_proof(
        &self,
        proof: &sky_dispatch_win32::input::ReleaseOwnerProof,
    ) -> Result<MutexGuard<'_, ()>, ReleaseOwnerPermitError> {
        #[cfg(any(test, feature = "test-support"))]
        record_target_proof_operation(|counts| {
            counts.writer_permit_attempts = counts.writer_permit_attempts.saturating_add(1)
        });
        let writer = self
            .writer
            .try_lock()
            .ok_or(ReleaseOwnerPermitError::WriterBusy)?;
        let current = self
            .try_release_owner_proof_is_current(proof)
            .ok_or(ReleaseOwnerPermitError::OwnerAuthorityBusy)?;
        if current {
            Ok(writer)
        } else {
            Err(ReleaseOwnerPermitError::NotCurrent)
        }
    }

    #[cfg(test)]
    pub(super) fn lock_owner_authority_for_test(
        &self,
    ) -> parking_lot::MutexGuard<
        '_,
        Option<(Arc<sky_dispatch_win32::focus::WindowIdentityAuthority>, u64)>,
    > {
        self.owner_authority.lock()
    }

    #[cfg(test)]
    pub(super) fn lock_writer_for_test(&self) -> MutexGuard<'_, ()> {
        self.writer.lock()
    }

    /// The musical final suffix must never wait for the authority mutex. A
    /// busy snapshot/publication drops this candidate before its sender call.
    fn try_release_owner_proof_is_current(
        &self,
        proof: &sky_dispatch_win32::input::ReleaseOwnerProof,
    ) -> Option<bool> {
        #[cfg(any(test, feature = "test-support"))]
        record_target_proof_operation(|counts| {
            counts.authority_mutex_attempts = counts.authority_mutex_attempts.saturating_add(1)
        });
        let retained = self.owner_authority.try_lock()?;
        Some(self.release_owner_proof_matches_current(proof, &retained))
    }

    /// Safety cleanup may use serialized synchronization after its retained
    /// proof validation; unlike musical admission this path is allowed to wait.
    fn release_owner_proof_is_current_serialized(
        &self,
        proof: &sky_dispatch_win32::input::ReleaseOwnerProof,
    ) -> bool {
        #[cfg(any(test, feature = "test-support"))]
        record_target_proof_operation(|counts| {
            counts.authority_mutex_attempts = counts.authority_mutex_attempts.saturating_add(1)
        });
        let retained = self.owner_authority.lock();
        self.release_owner_proof_matches_current(proof, &retained)
    }

    fn release_owner_proof_matches_current(
        &self,
        proof: &sky_dispatch_win32::input::ReleaseOwnerProof,
        retained: &Option<(Arc<sky_dispatch_win32::focus::WindowIdentityAuthority>, u64)>,
    ) -> bool {
        if self.load_stable() != Some((proof.target_hwnd(), proof.target_generation())) {
            return false;
        }
        if proof.owner_pid().is_none() && self.owner_identity_required.load(Ordering::Acquire) {
            return false;
        }
        if let Some(expected_pid) = proof.owner_pid()
            && self.owner_identity_status(proof.target_generation())
                != OwnerIdentityStatus::Bound(expected_pid)
        {
            return false;
        }
        match (proof.authority(), retained.as_ref()) {
            (Some(expected), Some((current, generation))) => {
                *generation == proof.target_generation() && Arc::ptr_eq(expected, current)
            }
            (None, None) => true,
            _ => false,
        }
    }

    fn owner_authority_matches_locked(
        &self,
        authority: &Arc<sky_dispatch_win32::focus::WindowIdentityAuthority>,
        generation: u64,
    ) -> bool {
        if self.load_stable() != Some((authority.identity().hwnd, generation))
            || self.owner_generation.load(Ordering::SeqCst) != generation
            || self.owner_pid.load(Ordering::SeqCst) != authority.identity().owner_pid
        {
            return false;
        }
        self.owner_authority
            .lock()
            .as_ref()
            .is_some_and(|(retained, retained_generation)| {
                *retained_generation == generation && Arc::ptr_eq(retained, authority)
            })
    }

    #[cfg(test)]
    pub(super) fn set_owner_poll_result_barrier_for_test(
        &self,
        reached: Arc<std::sync::Barrier>,
        resume: Arc<std::sync::Barrier>,
    ) {
        *self.owner_poll_result_barrier_for_test.lock() = Some((reached, resume));
    }

    #[cfg(test)]
    pub(super) fn run_owner_poll_result_hook_for_test(&self) {
        if let Some((reached, resume)) = self.owner_poll_result_barrier_for_test.lock().take() {
            reached.wait();
            resume.wait();
        }
    }

    #[cfg(test)]
    pub(super) fn set_owner_close_snapshot_barrier_for_test(
        &self,
        reached: Arc<std::sync::Barrier>,
        resume: Arc<std::sync::Barrier>,
    ) {
        *self.owner_close_snapshot_barrier_for_test.lock() = Some((reached, resume));
    }

    #[cfg(test)]
    pub(super) fn run_owner_close_snapshot_hook_for_test(&self) {
        if let Some((reached, resume)) = self.owner_close_snapshot_barrier_for_test.lock().take() {
            reached.wait();
            resume.wait();
        }
    }

    #[cfg(test)]
    pub(super) fn set_safety_release_validation_barrier_for_test(
        &self,
        reached: Arc<std::sync::Barrier>,
        resume: Arc<std::sync::Barrier>,
    ) {
        *self.safety_release_validation_barrier_for_test.lock() = Some((reached, resume));
    }

    #[cfg(test)]
    pub(super) fn run_safety_release_validation_hook_for_test(&self) {
        if let Some((reached, resume)) = self
            .safety_release_validation_barrier_for_test
            .lock()
            .take()
        {
            reached.wait();
            resume.wait();
        }
    }

    #[inline(always)]
    pub(super) fn owner_identity_status(&self, expected_generation: u64) -> OwnerIdentityStatus {
        if !self.owner_identity_required.load(Ordering::Acquire) {
            return OwnerIdentityStatus::NotRequired;
        }
        let first_generation = self.owner_generation.load(Ordering::SeqCst);
        let owner_pid = self.owner_pid.load(Ordering::SeqCst);
        let owner_live = self.owner_live.load(Ordering::SeqCst);
        let final_generation = self.owner_generation.load(Ordering::SeqCst);
        if first_generation != final_generation {
            return OwnerIdentityStatus::Stale;
        }
        if owner_pid == 0 {
            return OwnerIdentityStatus::Missing;
        }
        if first_generation != expected_generation {
            return OwnerIdentityStatus::Stale;
        }
        if !owner_live {
            return match self.owner_failure.load(Ordering::SeqCst) {
                value if value == OwnerIdentityFailure::ProcessTerminated as u32 => {
                    OwnerIdentityStatus::ProcessTerminated
                }
                value if value == OwnerIdentityFailure::OwnerMismatch as u32 => {
                    OwnerIdentityStatus::OwnerMismatch
                }
                value if value == OwnerIdentityFailure::WindowUnavailable as u32 => {
                    OwnerIdentityStatus::WindowUnavailable
                }
                value if value == OwnerIdentityFailure::ProcessIdentityMismatch as u32 => {
                    OwnerIdentityStatus::ProcessIdentityMismatch
                }
                value if value == OwnerIdentityFailure::QueryUnavailable as u32 => {
                    OwnerIdentityStatus::QueryUnavailable
                }
                _ => OwnerIdentityStatus::Missing,
            };
        }
        OwnerIdentityStatus::Bound(owner_pid)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn invalidate_owner_identity(&self) {
        self.invalidate_owner_identity_with(OwnerIdentityFailure::ProcessTerminated);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn invalidate_owner_identity_with(&self, failure: OwnerIdentityFailure) {
        let _writer = self.writer.lock();
        self.owner_failure.store(failure as u32, Ordering::SeqCst);
        self.owner_live.store(false, Ordering::Release);
    }

    /// Read one coherent target snapshot. This is one bounded attempt: a
    /// writer in progress or any publication during the read fails closed.
    #[inline(always)]
    pub(super) fn load_stable(&self) -> Option<(isize, u64)> {
        #[cfg(any(test, feature = "test-support"))]
        record_target_proof_operation(|counts| {
            counts.generation_revalidations = counts.generation_revalidations.saturating_add(1)
        });
        self.load_stable_inner(
            #[cfg(test)]
            None,
        )
    }

    #[inline(always)]
    fn load_stable_inner(
        &self,
        #[cfg(test)] mut hook: Option<&mut dyn FnMut(TargetSnapshotReadPoint)>,
    ) -> Option<(isize, u64)> {
        // SeqCst puts both epoch reads and both payload reads in one total
        // order. Equal even epochs therefore prove no writer publication
        // overlapped the HWND/generation sample.
        let first_epoch = self.publication_epoch.load(Ordering::SeqCst);
        #[cfg(test)]
        if let Some(hook) = hook.as_mut() {
            (**hook)(TargetSnapshotReadPoint::EpochStart);
        }
        if first_epoch & 1 != 0 {
            return None;
        }

        let hwnd = self.target_hwnd.load(Ordering::SeqCst);
        #[cfg(test)]
        if let Some(hook) = hook.as_mut() {
            (**hook)(TargetSnapshotReadPoint::Hwnd);
        }
        let generation = self.target_generation.load(Ordering::SeqCst);
        #[cfg(test)]
        if let Some(hook) = hook.as_mut() {
            (**hook)(TargetSnapshotReadPoint::Generation);
        }
        let final_epoch = self.publication_epoch.load(Ordering::SeqCst);
        #[cfg(test)]
        if let Some(hook) = hook.as_mut() {
            (**hook)(TargetSnapshotReadPoint::EpochEnd);
        }

        (first_epoch == final_epoch && final_epoch & 1 == 0).then_some((hwnd, generation))
    }

    #[cfg(test)]
    pub(super) fn load_stable_with_hook_for_test<F>(&self, mut hook: F) -> Option<(isize, u64)>
    where
        F: FnMut(TargetSnapshotReadPoint),
    {
        self.load_stable_inner(Some(&mut hook))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn reset_proof_operation_counts_for_test(&self) {
        TARGET_PROOF_OPERATION_COUNTS
            .with(|counts| counts.set(TargetProofOperationCounts::default()));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn proof_operation_counts_for_test(&self) -> TargetProofOperationCounts {
        TARGET_PROOF_OPERATION_COUNTS.with(std::cell::Cell::get)
    }

    #[inline(always)]
    pub(super) fn is_current(&self, hwnd: isize, generation: u64) -> bool {
        self.load_stable() == Some((hwnd, generation))
    }

    /// Latest HWND is used only by release/cleanup paths that do not authorize
    /// a musical Down and remain independent of target proof.
    #[inline(always)]
    pub(super) fn hwnd_for_safety_release(&self) -> isize {
        self.target_hwnd.load(Ordering::Acquire)
    }

    pub(super) fn publish(&self, hwnd: isize) -> bool {
        self.publish_inner(
            hwnd,
            false,
            #[cfg(test)]
            None,
        )
    }

    /// Advance the target generation even if the numeric HWND is unchanged.
    /// This is required before rebinding owner identity after a full
    /// control-plane revalidation of a target window lifetime.
    pub(super) fn republish_same_hwnd(&self, hwnd: isize) -> bool {
        self.publish_inner(
            hwnd,
            true,
            #[cfg(test)]
            None,
        )
    }

    #[cfg(feature = "test-support")]
    pub fn publish_for_test(&self, hwnd: isize) -> bool {
        self.publish(hwnd)
    }

    fn publish_inner(
        &self,
        hwnd: isize,
        force_generation: bool,
        #[cfg(test)] mut hook: Option<&mut dyn FnMut(TargetPublicationWritePoint)>,
    ) -> bool {
        // `set_target_hwnd(&self, ..)` admits concurrent callers. Serialize
        // them before inspecting or changing either payload field so the
        // epoch has exactly one writer at a time.
        let _writer = self.writer.lock();
        if !force_generation && self.target_hwnd.load(Ordering::SeqCst) == hwnd {
            return false;
        }

        let stable_epoch = self.publication_epoch.load(Ordering::SeqCst);
        // The sequence advances by two per transition. Poison permanently
        // before either the epoch or generation can wrap and resurrect proof.
        if stable_epoch & 1 != 0 || stable_epoch > u64::MAX - 3 {
            self.publication_epoch.store(u64::MAX, Ordering::SeqCst);
            return false;
        }

        self.publication_epoch
            .store(stable_epoch + 1, Ordering::SeqCst);
        self.owner_failure
            .store(OwnerIdentityFailure::None as u32, Ordering::SeqCst);
        self.owner_live.store(false, Ordering::SeqCst);
        #[cfg(test)]
        if let Some(hook) = hook.as_mut() {
            (**hook)(TargetPublicationWritePoint::BeforeOwnerAuthorityReset);
        }
        self.owner_authority.lock().take();
        #[cfg(test)]
        if let Some(hook) = hook.as_mut() {
            (**hook)(TargetPublicationWritePoint::InProgress);
        }
        self.target_hwnd.store(hwnd, Ordering::SeqCst);
        #[cfg(test)]
        if let Some(hook) = hook.as_mut() {
            (**hook)(TargetPublicationWritePoint::HwndPublished);
        }
        self.target_generation.fetch_add(1, Ordering::SeqCst);
        #[cfg(test)]
        if let Some(hook) = hook.as_mut() {
            (**hook)(TargetPublicationWritePoint::GenerationPublished);
        }
        self.publication_epoch
            .store(stable_epoch + 2, Ordering::SeqCst);
        #[cfg(test)]
        if let Some(hook) = hook.as_mut() {
            (**hook)(TargetPublicationWritePoint::Stable);
        }
        true
    }

    #[cfg(test)]
    pub(super) fn publish_with_hook_for_test<F>(&self, hwnd: isize, mut hook: F) -> bool
    where
        F: FnMut(TargetPublicationWritePoint),
    {
        self.publish_inner(hwnd, false, Some(&mut hook))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_snapshot_fails_closed_when_writer_runs_between_each_load() {
        for interleave in [
            TargetSnapshotReadPoint::EpochStart,
            TargetSnapshotReadPoint::Hwnd,
            TargetSnapshotReadPoint::Generation,
        ] {
            let target = SessionTarget::new(100, 0);
            let mut published = false;
            let snapshot = target.load_stable_with_hook_for_test(|point| {
                if point == interleave && !published {
                    published = true;
                    target.publish(200);
                }
            });

            assert!(published, "writer interleave {interleave:?} ran");
            assert_eq!(snapshot, None, "interleave {interleave:?} must reject");
            assert_eq!(target.load_stable(), Some((200, 1)));
        }
    }

    #[test]
    fn target_snapshot_rejects_the_published_hwnd_before_generation_seam() {
        let target = SessionTarget::new(100, 0);
        let mut observed_in_progress = false;
        assert!(target.publish_with_hook_for_test(200, |point| {
            if point == TargetPublicationWritePoint::HwndPublished {
                observed_in_progress = true;
                assert_eq!(target.load_stable(), None);
                assert!(!target.is_current(100, 0));
            }
        }));

        assert!(observed_in_progress);
        assert_eq!(target.load_stable(), Some((200, 1)));
    }

    #[test]
    fn target_aba_and_repeated_transitions_invalidate_frozen_proof() {
        let target = SessionTarget::new(100, 0);
        let frozen = target.load_stable().unwrap();
        assert!(target.is_current(frozen.0, frozen.1));
        assert!(!target.publish(100));
        assert_eq!(target.load_stable(), Some((100, 0)));

        target.publish(200);
        target.publish(100);
        assert_eq!(target.load_stable(), Some((100, 2)));
        assert!(!target.is_current(frozen.0, frozen.1));

        target.publish(200);
        target.publish(100);
        target.publish(300);
        assert_eq!(target.load_stable(), Some((300, 5)));
        assert!(!target.is_current(frozen.0, frozen.1));
    }

    #[test]
    fn concurrent_target_writers_publish_all_serialized_transitions() {
        let target = SessionTarget::new(100, 0);
        let start = std::sync::Barrier::new(4);

        std::thread::scope(|scope| {
            for hwnd in [200, 300, 400] {
                let start = &start;
                let target = &target;
                scope.spawn(move || {
                    start.wait();
                    assert!(target.publish(hwnd));
                });
            }
            start.wait();
        });

        let (hwnd, generation) = target.load_stable().expect("stable final publication");
        assert!([200, 300, 400].contains(&hwnd));
        assert_eq!(generation, 3);
        assert!(!target.is_current(100, 0));
    }

    #[test]
    fn target_publication_poison_prevents_epoch_wraparound() {
        let target = SessionTarget::new(100, 0);
        target
            .publication_epoch
            .store(u64::MAX - 1, Ordering::SeqCst);

        assert!(!target.publish(200));
        assert_eq!(target.load_stable(), None);
        assert!(!target.publish(300));
    }

    #[test]
    fn owner_identity_is_bound_to_one_generation_and_invalidated_on_transition() {
        let target = SessionTarget::new(123, 1);
        target.require_owner_identity();
        assert_eq!(
            target.owner_identity_status(1),
            OwnerIdentityStatus::Missing
        );
        assert!(target.bind_owner_identity(123, 1, 77));
        assert_eq!(
            target.owner_identity_status(1),
            OwnerIdentityStatus::Bound(77)
        );
        assert!(!target.bind_owner_identity(123, 1, 88));

        assert!(target.publish(456));
        assert_eq!(target.owner_identity_status(2), OwnerIdentityStatus::Stale);
        assert!(target.bind_owner_identity(456, 2, 88));
        assert_eq!(
            target.owner_identity_status(2),
            OwnerIdentityStatus::Bound(88)
        );

        assert!(target.republish_same_hwnd(456));
        assert_eq!(target.owner_identity_status(3), OwnerIdentityStatus::Stale);
        assert!(target.bind_owner_identity(456, 3, 99));
        target.invalidate_owner_identity();
        assert_eq!(
            target.owner_identity_status(3),
            OwnerIdentityStatus::ProcessTerminated
        );
    }

    #[test]
    fn pid_only_release_proof_matches_current_pid_binding_without_process_authority() {
        let target = SessionTarget::new(123, 1);
        assert!(target.bind_owner_identity(123, 1, 77));

        let proof = target
            .try_release_owner_proof_for_generation(123, 1)
            .expect("PID-bound target release proof");
        assert_eq!(proof.owner_pid(), Some(77));
        assert!(proof.authority().is_none());
        assert!(target.lock_current_release_owner_proof(&proof).is_some());
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn release_proof_without_owner_cannot_be_upgraded_by_late_same_generation_bind() {
        let target = SessionTarget::new(123, 1);
        let proof = target
            .try_release_owner_proof_for_generation(123, 1)
            .expect("unbound target proof");
        assert_eq!(proof.owner_pid(), None);
        assert!(target.bind_owner_identity_authority(
            123,
            1,
            77,
            Arc::new(
                sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 77)
            ),
        ));

        assert!(
            target.lock_current_release_owner_proof(&proof).is_none(),
            "a later owner binding cannot upgrade a release obligation created without identity"
        );
    }

    #[test]
    fn owner_bind_racing_target_writer_never_authorizes_the_new_generation() {
        let target = SessionTarget::new(123, 1);
        target.require_owner_identity();
        let start = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            let start_bind = &start;
            let target_bind = &target;
            scope.spawn(move || {
                start_bind.wait();
                let _ = target_bind.bind_owner_identity(123, 1, 77);
            });
            let start_publish = &start;
            let target_publish = &target;
            scope.spawn(move || {
                start_publish.wait();
                target_publish.publish(456);
            });
            start.wait();
        });

        let (hwnd, generation) = target.load_stable().expect("stable target after race");
        assert_eq!((hwnd, generation), (456, 2));
        assert_ne!(
            target.owner_identity_status(generation),
            OwnerIdentityStatus::Bound(77)
        );
        assert!(target.bind_owner_identity(456, generation, 88));
        assert_eq!(
            target.owner_identity_status(generation),
            OwnerIdentityStatus::Bound(88)
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn retained_owner_authority_tracks_generation_and_worker_clone_lifetime() {
        let target = SessionTarget::new(123, 1);
        target.require_owner_identity();
        let authority = Arc::new(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 77),
        );
        let weak_authority = Arc::downgrade(&authority);
        assert!(target.bind_owner_identity_authority(123, 1, 77, Arc::clone(&authority)));

        let worker_binding = target
            .try_owner_authority_for_generation(1)
            .expect("worker acquires current binding without waiting");
        assert!(Arc::ptr_eq(&authority, &worker_binding));
        drop(authority);

        assert!(target.publish(456));
        assert_eq!(target.load_stable(), Some((456, 2)));
        assert_eq!(
            target.owner_identity_status(2),
            OwnerIdentityStatus::Stale,
            "a target generation change invalidates the prior owner proof"
        );
        assert!(matches!(
            target.try_owner_authority_for_generation(2),
            Err(OwnerAuthorityReadError::Status(OwnerIdentityStatus::Stale))
        ));
        assert!(weak_authority.upgrade().is_some());

        drop(worker_binding);
        assert!(
            weak_authority.upgrade().is_none(),
            "the old process authority is released only after the worker's in-flight clone"
        );
    }
}
