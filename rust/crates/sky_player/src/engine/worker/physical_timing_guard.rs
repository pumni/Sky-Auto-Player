//! Fixed-size physical QPC floors used by production dispatch.
//!
//! It consumes sender completion timestamps supplied by its caller and never
//! samples QPC, reads settings, or walks authored work.

use sky_dispatch_core::{
    model::MAX_KEYS,
    time::{DurationTicks, QpcTicks},
};

const VALID_KEY_MASK: u16 = (1_u16 << MAX_KEYS) - 1;

/// The immutable timing window derived for one prepared physical packet.
///
/// hold_floor_mask and release_floor_mask identify packet keys whose stored
/// physical floor is later than the authored target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PhysicalTimingWindow {
    pub(super) authored_target_qpc: QpcTicks,
    pub(super) musical_up_not_before_qpc: QpcTicks,
    pub(super) down_not_before_qpc: QpcTicks,
    pub(super) packet_not_before_qpc: QpcTicks,
    pub(super) latest_down_start_qpc: Option<QpcTicks>,
    pub(super) hold_floor_mask: u16,
    pub(super) release_floor_mask: u16,
}

/// Errors that make a physical timing query or observation unusable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PhysicalTimingGuardError {
    Invalidated,
    InvalidPacketMasks,
    ArithmeticOverflow,
}

/// Worker-owned sender evidence for the 15 instrument slots.
///
/// A successful Up consumes the prior Up hold floor for that slot and replaces
/// its Down release floor. A successful Down consumes the prior Down release
/// floor and replaces its Up hold floor. Different packet directions therefore
/// retain independent per-key state.
pub(crate) struct PhysicalTimingGuard {
    musical_up_not_before_qpc: [Option<QpcTicks>; MAX_KEYS],
    down_not_before_qpc: [Option<QpcTicks>; MAX_KEYS],
    frame_base_hold_ticks: DurationTicks,
    frame_ticks: DurationTicks,
    timing_margin_ticks: DurationTicks,
    valid: bool,
}

impl PhysicalTimingGuard {
    pub(super) const fn new(
        frame_base_hold_ticks: DurationTicks,
        frame_ticks: DurationTicks,
        timing_margin_ticks: DurationTicks,
    ) -> Self {
        Self {
            musical_up_not_before_qpc: [None; MAX_KEYS],
            down_not_before_qpc: [None; MAX_KEYS],
            frame_base_hold_ticks,
            frame_ticks,
            timing_margin_ticks,
            valid: true,
        }
    }

    /// Clear sender evidence and make the guard usable for a fresh lifecycle.
    pub(super) fn reset(&mut self) {
        self.musical_up_not_before_qpc = [None; MAX_KEYS];
        self.down_not_before_qpc = [None; MAX_KEYS];
        self.valid = true;
    }

    /// Clear sender evidence and make queries fail closed until reset.
    pub(super) fn invalidate(&mut self) {
        self.musical_up_not_before_qpc = [None; MAX_KEYS];
        self.down_not_before_qpc = [None; MAX_KEYS];
        self.valid = false;
    }

    /// Record the completion of an already-successful canonical physical
    /// packet. Masks must be disjoint and limited to the 15 instrument slots.
    ///
    /// Overflow invalidates all evidence so a caller cannot continue with
    /// floors that omit a successful physical transition.
    pub(super) fn observe_successful_packet(
        &mut self,
        completed_qpc: QpcTicks,
        up_mask: u16,
        down_mask: u16,
    ) -> Result<(), PhysicalTimingGuardError> {
        self.ensure_valid()?;
        Self::validate_packet_masks(up_mask, down_mask)?;

        let new_down_floor = if up_mask != 0 {
            match completed_qpc.checked_add_duration(self.frame_ticks) {
                Ok(floor) => Some(floor),
                Err(_) => {
                    self.invalidate();
                    return Err(PhysicalTimingGuardError::ArithmeticOverflow);
                }
            }
        } else {
            None
        };
        let new_up_floor = if down_mask != 0 {
            match completed_qpc.checked_add_duration(self.frame_base_hold_ticks) {
                Ok(floor) => Some(floor),
                Err(_) => {
                    self.invalidate();
                    return Err(PhysicalTimingGuardError::ArithmeticOverflow);
                }
            }
        } else {
            None
        };

        let mut remaining = up_mask;
        while remaining != 0 {
            let slot = remaining.trailing_zeros() as usize;
            let bit = 1_u16 << slot;
            self.musical_up_not_before_qpc[slot] = None;
            self.down_not_before_qpc[slot] = new_down_floor;
            remaining &= !bit;
        }

        let mut remaining = down_mask;
        while remaining != 0 {
            let slot = remaining.trailing_zeros() as usize;
            let bit = 1_u16 << slot;
            self.down_not_before_qpc[slot] = None;
            self.musical_up_not_before_qpc[slot] = new_up_floor;
            remaining &= !bit;
        }

        Ok(())
    }

    /// Compute packet floors without reading the clock or mutating guard state.
    pub(super) fn query(
        &self,
        authored_target_qpc: QpcTicks,
        up_mask: u16,
        down_mask: u16,
    ) -> Result<PhysicalTimingWindow, PhysicalTimingGuardError> {
        self.ensure_valid()?;
        Self::validate_packet_masks(up_mask, down_mask)?;
        if up_mask == 0 && down_mask == 0 {
            return Err(PhysicalTimingGuardError::InvalidPacketMasks);
        }

        let (musical_up_not_before_qpc, hold_floor_mask) = Self::floor_for_mask(
            authored_target_qpc,
            up_mask,
            &self.musical_up_not_before_qpc,
        );
        let (down_not_before_qpc, release_floor_mask) =
            Self::floor_for_mask(authored_target_qpc, down_mask, &self.down_not_before_qpc);
        let packet_not_before_qpc = core::cmp::max(musical_up_not_before_qpc, down_not_before_qpc);
        let latest_down_start_qpc = if down_mask == 0 {
            None
        } else {
            Some(
                authored_target_qpc
                    .checked_add_duration(self.timing_margin_ticks)
                    .map_err(|_| PhysicalTimingGuardError::ArithmeticOverflow)?,
            )
        };

        Ok(PhysicalTimingWindow {
            authored_target_qpc,
            musical_up_not_before_qpc,
            down_not_before_qpc,
            packet_not_before_qpc,
            latest_down_start_qpc,
            hold_floor_mask,
            release_floor_mask,
        })
    }

    fn ensure_valid(&self) -> Result<(), PhysicalTimingGuardError> {
        if self.valid {
            Ok(())
        } else {
            Err(PhysicalTimingGuardError::Invalidated)
        }
    }

    fn validate_packet_masks(up_mask: u16, down_mask: u16) -> Result<(), PhysicalTimingGuardError> {
        if (up_mask | down_mask) & !VALID_KEY_MASK != 0 || up_mask & down_mask != 0 {
            return Err(PhysicalTimingGuardError::InvalidPacketMasks);
        }
        Ok(())
    }

    fn floor_for_mask(
        authored_target_qpc: QpcTicks,
        mask: u16,
        floors: &[Option<QpcTicks>; MAX_KEYS],
    ) -> (QpcTicks, u16) {
        let mut not_before_qpc = authored_target_qpc;
        let mut floor_mask = 0;
        let mut remaining = mask;
        while remaining != 0 {
            let slot = remaining.trailing_zeros() as usize;
            let bit = 1_u16 << slot;
            match floors[slot] {
                Some(floor) if floor > authored_target_qpc => {
                    floor_mask |= bit;
                    not_before_qpc = core::cmp::max(not_before_qpc, floor);
                }
                _ => {}
            }
            remaining &= !bit;
        }
        (not_before_qpc, floor_mask)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PhysicalTimingGuard, PhysicalTimingGuardError, PhysicalTimingWindow, VALID_KEY_MASK,
    };
    use sky_dispatch_core::time::{DurationTicks, QpcTicks};

    const HOLD_TICKS: DurationTicks = DurationTicks::from_raw(30);
    const FRAME_TICKS: DurationTicks = DurationTicks::from_raw(12);
    const MARGIN_TICKS: DurationTicks = DurationTicks::from_raw(5);

    fn guard() -> PhysicalTimingGuard {
        PhysicalTimingGuard::new(HOLD_TICKS, FRAME_TICKS, MARGIN_TICKS)
    }

    fn qpc(value: u64) -> QpcTicks {
        QpcTicks::from_raw(value)
    }

    fn bit(slot: usize) -> u16 {
        1_u16 << slot
    }

    #[test]
    fn fresh_guard_uses_authored_target_and_margin_only() {
        let window = guard().query(qpc(100), bit(0), bit(1)).unwrap();
        assert_eq!(
            window,
            PhysicalTimingWindow {
                authored_target_qpc: qpc(100),
                musical_up_not_before_qpc: qpc(100),
                down_not_before_qpc: qpc(100),
                packet_not_before_qpc: qpc(100),
                latest_down_start_qpc: Some(qpc(105)),
                hold_floor_mask: 0,
                release_floor_mask: 0,
            }
        );
    }

    #[test]
    fn successful_down_sets_the_authored_up_hold_floor() {
        let mut guard = guard();
        guard
            .observe_successful_packet(qpc(100), 0, bit(2))
            .unwrap();

        let window = guard.query(qpc(110), bit(2), 0).unwrap();
        assert_eq!(window.musical_up_not_before_qpc, qpc(130));
        assert_eq!(window.packet_not_before_qpc, qpc(130));
        assert_eq!(window.hold_floor_mask, bit(2));
        assert_eq!(window.latest_down_start_qpc, None);
    }

    #[test]
    fn successful_authored_up_sets_the_release_floor_for_down() {
        let mut guard = guard();
        guard
            .observe_successful_packet(qpc(100), bit(3), 0)
            .unwrap();

        let window = guard.query(qpc(105), 0, bit(3)).unwrap();
        assert_eq!(window.down_not_before_qpc, qpc(112));
        assert_eq!(window.packet_not_before_qpc, qpc(112));
        assert_eq!(window.release_floor_mask, bit(3));
        assert_eq!(window.latest_down_start_qpc, Some(qpc(110)));
    }

    #[test]
    fn mixed_packet_keeps_up_and_down_floors_on_their_own_keys() {
        let mut guard = guard();
        guard
            .observe_successful_packet(qpc(100), bit(0), bit(1))
            .unwrap();

        let window = guard.query(qpc(105), bit(1), bit(0)).unwrap();
        assert_eq!(window.musical_up_not_before_qpc, qpc(130));
        assert_eq!(window.down_not_before_qpc, qpc(112));
        assert_eq!(window.packet_not_before_qpc, qpc(130));
        assert_eq!(window.hold_floor_mask, bit(1));
        assert_eq!(window.release_floor_mask, bit(0));
        assert_eq!(window.latest_down_start_qpc, Some(qpc(110)));

        let up_recovery = guard.query(qpc(105), bit(1), 0).unwrap();
        assert_eq!(up_recovery.packet_not_before_qpc, qpc(130));
        assert_eq!(up_recovery.down_not_before_qpc, qpc(105));
        assert_eq!(up_recovery.latest_down_start_qpc, None);
    }

    #[test]
    fn packet_query_uses_the_latest_relevant_per_key_floor() {
        let mut guard = guard();
        guard
            .observe_successful_packet(qpc(100), 0, bit(0))
            .unwrap();
        guard
            .observe_successful_packet(qpc(200), 0, bit(1))
            .unwrap();

        let window = guard.query(qpc(110), bit(0) | bit(1), 0).unwrap();
        assert_eq!(window.musical_up_not_before_qpc, qpc(230));
        assert_eq!(window.hold_floor_mask, bit(0) | bit(1));
        assert_eq!(window.packet_not_before_qpc, qpc(230));
    }

    #[test]
    fn repeated_generations_replace_and_clear_opposite_direction_floors() {
        let mut guard = guard();
        let slot = bit(4);

        guard.observe_successful_packet(qpc(100), 0, slot).unwrap();
        guard.observe_successful_packet(qpc(150), slot, 0).unwrap();
        let after_up = guard.query(qpc(151), slot, 0).unwrap();
        assert_eq!(after_up.musical_up_not_before_qpc, qpc(151));
        assert_eq!(after_up.hold_floor_mask, 0);
        let release = guard.query(qpc(151), 0, slot).unwrap();
        assert_eq!(release.down_not_before_qpc, qpc(162));

        guard.observe_successful_packet(qpc(200), 0, slot).unwrap();
        let after_next_down = guard.query(qpc(201), slot, 0).unwrap();
        assert_eq!(after_next_down.musical_up_not_before_qpc, qpc(230));
        assert_eq!(after_next_down.hold_floor_mask, slot);
        assert_eq!(
            guard.query(qpc(201), 0, slot).unwrap().down_not_before_qpc,
            qpc(201)
        );
    }

    #[test]
    fn all_fifteen_slots_are_tracked_and_high_mask_bits_are_rejected() {
        let mut guard = guard();
        guard
            .observe_successful_packet(qpc(1_000), 0, VALID_KEY_MASK)
            .unwrap();
        let up_window = guard.query(qpc(1_001), VALID_KEY_MASK, 0).unwrap();
        assert_eq!(up_window.musical_up_not_before_qpc, qpc(1_030));
        assert_eq!(up_window.hold_floor_mask, VALID_KEY_MASK);

        guard
            .observe_successful_packet(qpc(1_100), VALID_KEY_MASK, 0)
            .unwrap();
        let down_window = guard.query(qpc(1_101), 0, VALID_KEY_MASK).unwrap();
        assert_eq!(down_window.down_not_before_qpc, qpc(1_112));
        assert_eq!(down_window.release_floor_mask, VALID_KEY_MASK);
        assert_eq!(
            guard.query(qpc(1_101), 0, 1 << 15),
            Err(PhysicalTimingGuardError::InvalidPacketMasks)
        );
    }

    #[test]
    fn overlapping_or_empty_packet_masks_are_rejected() {
        let mut guard = guard();
        assert_eq!(
            guard.observe_successful_packet(qpc(10), bit(0), bit(0)),
            Err(PhysicalTimingGuardError::InvalidPacketMasks)
        );
        assert_eq!(
            guard.query(qpc(10), 0, 0),
            Err(PhysicalTimingGuardError::InvalidPacketMasks)
        );
        assert_eq!(
            guard.query(qpc(10), bit(0), bit(0)),
            Err(PhysicalTimingGuardError::InvalidPacketMasks)
        );
        assert_eq!(
            guard
                .query(qpc(10), bit(0), 0)
                .unwrap()
                .packet_not_before_qpc,
            qpc(10)
        );
    }

    #[test]
    fn zero_margin_keeps_the_down_start_window_at_the_authored_target() {
        let guard = PhysicalTimingGuard::new(HOLD_TICKS, FRAME_TICKS, DurationTicks::ZERO);
        let window = guard.query(qpc(42), 0, bit(0)).unwrap();
        assert_eq!(window.latest_down_start_qpc, Some(qpc(42)));
    }

    #[test]
    fn observation_overflow_invalidates_all_floors_until_reset() {
        let mut guard = guard();
        guard
            .observe_successful_packet(qpc(100), 0, bit(0))
            .unwrap();
        assert_eq!(
            guard.observe_successful_packet(qpc(u64::MAX - 10), 0, bit(1)),
            Err(PhysicalTimingGuardError::ArithmeticOverflow)
        );
        assert_eq!(
            guard.query(qpc(100), bit(0), 0),
            Err(PhysicalTimingGuardError::Invalidated)
        );
        assert_eq!(
            guard.observe_successful_packet(qpc(100), 0, bit(0)),
            Err(PhysicalTimingGuardError::Invalidated)
        );

        guard.reset();
        assert_eq!(
            guard
                .query(qpc(100), bit(0), 0)
                .unwrap()
                .packet_not_before_qpc,
            qpc(100)
        );
    }

    #[test]
    fn release_floor_overflow_invalidates_guard_and_lifecycle_invalidate_is_fail_closed() {
        let mut guard = guard();
        guard
            .observe_successful_packet(qpc(100), bit(0), 0)
            .unwrap();
        assert_eq!(
            guard.observe_successful_packet(qpc(u64::MAX - 5), bit(1), 0),
            Err(PhysicalTimingGuardError::ArithmeticOverflow)
        );
        assert_eq!(
            guard.query(qpc(100), 0, bit(0)),
            Err(PhysicalTimingGuardError::Invalidated)
        );

        guard.reset();
        guard
            .observe_successful_packet(qpc(100), 0, bit(0))
            .unwrap();
        guard.invalidate();
        assert_eq!(
            guard.query(qpc(100), bit(0), 0),
            Err(PhysicalTimingGuardError::Invalidated)
        );
        guard.reset();
        assert_eq!(
            guard
                .query(qpc(100), bit(0), 0)
                .unwrap()
                .packet_not_before_qpc,
            qpc(100)
        );
    }

    #[test]
    fn query_overflow_fails_closed_without_corrupting_sender_evidence() {
        let guard = guard();
        assert_eq!(
            guard.query(qpc(u64::MAX - 1), 0, bit(0)),
            Err(PhysicalTimingGuardError::ArithmeticOverflow)
        );
        assert_eq!(
            guard
                .query(qpc(10), 0, bit(0))
                .unwrap()
                .latest_down_start_qpc,
            Some(qpc(15))
        );
    }
}
