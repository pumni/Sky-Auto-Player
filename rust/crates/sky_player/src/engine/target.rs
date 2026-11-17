use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TargetProofOperationCounts {
    pub(super) authority_arc_clones: u64,
    pub(super) generation_revalidations: u64,
}

#[cfg(any(test, feature = "test-support"))]
std::thread_local! {
    static TARGET_PROOF_OPERATION_COUNTS: std::cell::Cell<TargetProofOperationCounts> =
        const { std::cell::Cell::new(TargetProofOperationCounts {
            authority_arc_clones: 0,
            generation_revalidations: 0,
        }) };
}

#[cfg(any(test, feature = "test-support"))]
fn record_target_stamp_read() {
    TARGET_PROOF_OPERATION_COUNTS.with(|counts| {
        let mut current = counts.get();
        current.generation_revalidations = current.generation_revalidations.saturating_add(1);
        counts.set(current);
    });
}

/// The target and retained process identity are written once before worker
/// startup. After binding, the worker and supervisor only read this token;
/// owner failure is a one-way latch and never publishes a replacement.
pub struct SessionTarget {
    target: OnceLock<(isize, u64)>,
    owner: OnceLock<sky_dispatch_win32::input::ReleaseOwnerProof>,
    owner_identity_required: AtomicBool,
    owner_process_identity_required: AtomicBool,
    owner_failure: AtomicU32,
    target_invalidated: AtomicBool,
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
    Status(OwnerIdentityStatus),
}

impl SessionTarget {
    pub(super) fn new(hwnd: isize, generation: u64) -> Self {
        let target = OnceLock::new();
        if hwnd != 0 || generation != 0 {
            let _ = target.set((hwnd, generation));
        }
        Self {
            target,
            owner: OnceLock::new(),
            owner_identity_required: AtomicBool::new(false),
            owner_process_identity_required: AtomicBool::new(false),
            owner_failure: AtomicU32::new(OwnerIdentityFailure::None as u32),
            target_invalidated: AtomicBool::new(false),
        }
    }

    /// Publish the initial HWND only. A second, different HWND invalidates
    /// this session instead of changing its destination.
    pub(super) fn set_initial_target(&self, hwnd: isize) -> bool {
        if hwnd == 0 || self.target_invalidated.load(Ordering::Acquire) {
            self.invalidate_target();
            return false;
        }
        if let Some((current_hwnd, _)) = self.target.get() {
            if *current_hwnd == hwnd {
                return true;
            }
            self.invalidate_target();
            return false;
        }
        if self.target.set((hwnd, 1)).is_ok()
            || self
                .target
                .get()
                .is_some_and(|(current_hwnd, _)| *current_hwnd == hwnd)
        {
            true
        } else {
            self.invalidate_target();
            false
        }
    }

    pub(super) fn invalidate_target(&self) {
        self.target_invalidated.store(true, Ordering::Release);
    }

    /// The session identity cannot change after binding. Runtime gates only
    /// need this monotonic invalidation read between their initial stamp check
    /// and the syscall boundary.
    #[inline(always)]
    pub(super) fn is_valid(&self) -> bool {
        !self.target_invalidated.load(Ordering::Acquire)
    }

    pub(super) fn require_owner_identity(&self) {
        self.owner_identity_required.store(true, Ordering::Release);
        self.owner_process_identity_required
            .store(true, Ordering::Release);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn bind_owner_identity(&self, hwnd: isize, generation: u64, owner_pid: u32) -> bool {
        if owner_pid == 0 || self.load_stable() != Some((hwnd, generation)) {
            return false;
        }
        self.owner_identity_required.store(true, Ordering::Release);
        let Some(proof) = sky_dispatch_win32::input::ReleaseOwnerProof::new(
            hwnd,
            generation,
            Some(owner_pid),
            None,
        ) else {
            return false;
        };
        self.owner.set(proof).is_ok()
    }

    pub(super) fn bind_owner_identity_authority(
        &self,
        hwnd: isize,
        generation: u64,
        owner_pid: u32,
        authority: std::sync::Arc<sky_dispatch_win32::focus::WindowIdentityAuthority>,
    ) -> bool {
        if owner_pid == 0
            || authority.identity().hwnd != hwnd
            || authority.identity().owner_pid != owner_pid
            || self.load_stable() != Some((hwnd, generation))
        {
            return false;
        }
        self.owner_identity_required.store(true, Ordering::Release);
        self.owner_process_identity_required
            .store(true, Ordering::Release);
        let Some(proof) = sky_dispatch_win32::input::ReleaseOwnerProof::new(
            hwnd,
            generation,
            Some(owner_pid),
            Some(authority),
        ) else {
            return false;
        };
        self.owner.set(proof).is_ok()
    }

    pub(super) fn owner_process_identity_required(&self) -> bool {
        self.owner_process_identity_required.load(Ordering::Acquire)
    }

    pub(super) fn owner_identity_status(&self, expected_generation: u64) -> OwnerIdentityStatus {
        if !self.owner_identity_required.load(Ordering::Acquire) {
            return OwnerIdentityStatus::NotRequired;
        }
        if let Some(status) = self.latched_owner_failure() {
            return status;
        }
        if !self.is_valid() {
            return OwnerIdentityStatus::Stale;
        }
        let Some(owner) = self.owner.get() else {
            return OwnerIdentityStatus::Missing;
        };
        if owner.target_generation() != expected_generation {
            return OwnerIdentityStatus::Stale;
        }
        match owner.owner_pid() {
            Some(owner_pid) if owner_pid != 0 => OwnerIdentityStatus::Bound(owner_pid),
            _ => OwnerIdentityStatus::Missing,
        }
    }

    pub(super) fn try_release_owner_proof_for_generation(
        &self,
        expected_hwnd: isize,
        expected_generation: u64,
    ) -> Result<sky_dispatch_win32::input::ReleaseOwnerProof, OwnerAuthorityReadError> {
        if self.load_stable() != Some((expected_hwnd, expected_generation)) {
            return Err(OwnerAuthorityReadError::Status(OwnerIdentityStatus::Stale));
        }
        if self.owner_identity_required.load(Ordering::Acquire) {
            match self.owner_identity_status(expected_generation) {
                OwnerIdentityStatus::Bound(owner_pid) if owner_pid != 0 => {
                    let Some(owner) = self.owner.get() else {
                        return Err(OwnerAuthorityReadError::Status(
                            OwnerIdentityStatus::Missing,
                        ));
                    };
                    return Ok(owner.clone());
                }
                status => return Err(OwnerAuthorityReadError::Status(status)),
            }
        }
        if let Some(owner) = self.owner.get() {
            return Ok(owner.clone());
        }
        sky_dispatch_win32::input::ReleaseOwnerProof::new(
            expected_hwnd,
            expected_generation,
            None,
            None,
        )
        .ok_or(OwnerAuthorityReadError::Status(OwnerIdentityStatus::Stale))
    }

    pub(super) fn owner_authority_for_generation(
        &self,
        expected_generation: u64,
    ) -> Option<&std::sync::Arc<sky_dispatch_win32::focus::WindowIdentityAuthority>> {
        let (_, generation) = self.load_stable()?;
        if generation != expected_generation {
            return None;
        }
        self.owner.get()?.authority()
    }

    pub(super) fn owner_authority_continuity(
        &self,
        expected_generation: u64,
    ) -> Option<sky_dispatch_win32::focus::WindowIdentityContinuity> {
        self.owner_authority_for_generation(expected_generation)
            .map(|authority| authority.continuity())
    }

    /// Latch a supervisor failure only for this session's immutable target.
    /// A late callback cannot publish a new owner or affect a later session.
    pub(super) fn invalidate_polled_owner_if_current(
        &self,
        generation: u64,
        failure: OwnerIdentityFailure,
    ) -> bool {
        if self
            .load_stable()
            .is_none_or(|(_, current)| current != generation)
            || self.owner.get().is_none()
            || self.latched_owner_failure().is_some()
        {
            return false;
        }
        self.invalidate_owner_identity_with(failure)
    }

    pub(super) fn polled_owner_is_current(&self, generation: u64) -> bool {
        self.load_stable()
            .is_some_and(|(_, current)| current == generation)
            && self.owner_identity_status(generation).is_bound()
    }

    pub(super) fn release_owner_proof_is_current(
        &self,
        proof: &sky_dispatch_win32::input::ReleaseOwnerProof,
    ) -> bool {
        if self.load_stable() != Some((proof.target_hwnd(), proof.target_generation())) {
            return false;
        }
        if self.owner_identity_required.load(Ordering::Acquire)
            && !self
                .owner_identity_status(proof.target_generation())
                .is_bound()
        {
            return false;
        }
        match self.owner.get() {
            Some(current) => current.same_owner(proof),
            None => proof.owner_pid().is_none() && proof.authority().is_none(),
        }
    }

    pub(super) fn invalidate_owner_identity(&self) {
        self.invalidate_owner_identity_with(OwnerIdentityFailure::ProcessTerminated);
    }

    pub(super) fn invalidate_owner_identity_with(&self, failure: OwnerIdentityFailure) -> bool {
        self.owner_failure
            .compare_exchange(
                OwnerIdentityFailure::None as u32,
                failure as u32,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    #[inline(always)]
    pub(super) fn load_stable(&self) -> Option<(isize, u64)> {
        #[cfg(any(test, feature = "test-support"))]
        record_target_stamp_read();
        if self.target_invalidated.load(Ordering::Acquire) {
            return None;
        }
        // A focus-free session uses the zero target as its stable sentinel.
        // Keep that snapshot readable until startup publishes a real HWND;
        // OnceLock remains available for the one permitted initial bind.
        Some(self.target.get().copied().unwrap_or((0, 0)))
    }

    #[inline(always)]
    pub(super) fn is_current(&self, hwnd: isize, generation: u64) -> bool {
        self.load_stable() == Some((hwnd, generation))
    }

    #[inline(always)]
    pub(super) fn hwnd_for_safety_release(&self) -> isize {
        self.target.get().map_or(0, |(hwnd, _)| *hwnd)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn invalidate_for_test(&self) {
        self.invalidate_target();
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

    fn latched_owner_failure(&self) -> Option<OwnerIdentityStatus> {
        match self.owner_failure.load(Ordering::Acquire) {
            value if value == OwnerIdentityFailure::ProcessTerminated as u32 => {
                Some(OwnerIdentityStatus::ProcessTerminated)
            }
            value if value == OwnerIdentityFailure::OwnerMismatch as u32 => {
                Some(OwnerIdentityStatus::OwnerMismatch)
            }
            value if value == OwnerIdentityFailure::WindowUnavailable as u32 => {
                Some(OwnerIdentityStatus::WindowUnavailable)
            }
            value if value == OwnerIdentityFailure::ProcessIdentityMismatch as u32 => {
                Some(OwnerIdentityStatus::ProcessIdentityMismatch)
            }
            value if value == OwnerIdentityFailure::QueryUnavailable as u32 => {
                Some(OwnerIdentityStatus::QueryUnavailable)
            }
            _ => None,
        }
    }
}

impl OwnerIdentityStatus {
    fn is_bound(self) -> bool {
        matches!(self, Self::Bound(pid) if pid != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn target_can_be_bound_once_and_changed_hwnd_invalidates_the_session() {
        let target = SessionTarget::new(0, 0);
        assert_eq!(target.load_stable(), Some((0, 0)));
        assert!(target.set_initial_target(123));
        assert_eq!(target.load_stable(), Some((123, 1)));
        assert!(target.set_initial_target(123), "same HWND is idempotent");
        assert!(!target.set_initial_target(456));
        assert_eq!(target.load_stable(), None);
        assert_eq!(target.hwnd_for_safety_release(), 123);
    }

    #[test]
    fn failed_owner_latch_is_terminal_for_the_bound_target() {
        let target = SessionTarget::new(123, 1);
        target.require_owner_identity();
        assert!(target.bind_owner_identity(123, 1, 77));
        assert_eq!(
            target.owner_identity_status(1),
            OwnerIdentityStatus::Bound(77)
        );
        assert!(target.invalidate_owner_identity_with(OwnerIdentityFailure::ProcessTerminated));
        assert!(!target.invalidate_owner_identity_with(OwnerIdentityFailure::OwnerMismatch));
        assert_eq!(
            target.owner_identity_status(1),
            OwnerIdentityStatus::ProcessTerminated
        );
        assert!(
            target
                .try_release_owner_proof_for_generation(123, 1)
                .is_err()
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn release_proof_clones_share_one_retained_process_authority() {
        let target = SessionTarget::new(123, 1);
        target.require_owner_identity();
        let authority = Arc::new(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 77),
        );
        let weak_authority = Arc::downgrade(&authority);
        assert!(target.bind_owner_identity_authority(123, 1, 77, Arc::clone(&authority)));
        drop(authority);

        let proof = target
            .try_release_owner_proof_for_generation(123, 1)
            .expect("target proof");
        let proof_clone = proof.clone();
        let authority = proof.authority().expect("retained authority");
        assert_eq!(Arc::strong_count(authority), 1);
        assert!(proof.same_owner(&proof_clone));
        assert!(weak_authority.upgrade().is_some());

        drop(proof);
        drop(proof_clone);
        assert!(
            weak_authority.upgrade().is_some(),
            "target retains authority"
        );
        drop(target);
        assert!(weak_authority.upgrade().is_none());
    }

    #[test]
    fn release_proof_for_another_target_is_rejected() {
        let target = SessionTarget::new(123, 1);
        let proof = sky_dispatch_win32::input::ReleaseOwnerProof::new(456, 2, None, None)
            .expect("valid detached proof");
        assert!(!target.release_owner_proof_is_current(&proof));
    }
}
