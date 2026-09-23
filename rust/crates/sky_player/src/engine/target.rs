use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};

pub struct SessionTarget {
    target_hwnd: AtomicIsize,
    target_generation: AtomicU64,
    publication_epoch: AtomicU64,
    owner_identity_required: AtomicBool,
    owner_pid: AtomicU32,
    owner_generation: AtomicU64,
    owner_live: AtomicBool,
    owner_failure: AtomicU32,
    writer: Mutex<()>,
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
            writer: Mutex::new(()),
        }
    }

    pub(super) fn require_owner_identity(&self) {
        self.owner_identity_required.store(true, Ordering::Release);
    }

    pub(super) fn bind_owner_identity(&self, hwnd: isize, generation: u64, owner_pid: u32) -> bool {
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
            return false;
        }

        self.owner_identity_required.store(true, Ordering::SeqCst);
        self.owner_failure
            .store(OwnerIdentityFailure::None as u32, Ordering::SeqCst);
        self.owner_live.store(false, Ordering::SeqCst);
        self.owner_pid.store(owner_pid, Ordering::SeqCst);
        self.owner_generation.store(generation, Ordering::SeqCst);
        self.owner_live.store(true, Ordering::SeqCst);
        true
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

    pub(super) fn invalidate_owner_identity(&self) {
        self.invalidate_owner_identity_with(OwnerIdentityFailure::ProcessTerminated);
    }

    pub(super) fn invalidate_owner_identity_with(&self, failure: OwnerIdentityFailure) {
        self.owner_failure.store(failure as u32, Ordering::SeqCst);
        self.owner_live.store(false, Ordering::Release);
    }

    /// Read one coherent target snapshot. This is one bounded attempt: a
    /// writer in progress or any publication during the read fails closed.
    #[inline(always)]
    pub(super) fn load_stable(&self) -> Option<(isize, u64)> {
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
}
