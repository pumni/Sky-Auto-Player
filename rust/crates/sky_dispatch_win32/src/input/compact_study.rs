//! Offline, test-support-only contiguous Win32 INPUT arena. The normal
//! sender and its fixed-capacity prepared packets are not modified.
//!
//! Equality here checks initialized keyboard fields, never raw INPUT bytes:
//! C-compatible unions/structs may contain uninitialized padding.

use super::outcome::PhysicalPacket;
use super::packet::PreparedPhysicalPacket;
use std::mem::size_of;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_KEYBOARD};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompactPacketDescriptor {
    pub packet: PhysicalPacket,
    pub offset: u32,
    pub length: u8,
}

pub struct CompactInputArenaBuilder {
    descriptors: Vec<CompactPacketDescriptor>,
    inputs: Vec<INPUT>,
}

pub struct FrozenCompactInputArena {
    descriptors: Vec<CompactPacketDescriptor>,
    inputs: Vec<INPUT>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompactArenaSizes {
    pub packet_count: usize,
    pub event_count: usize,
    pub inline_packet_size: usize,
    pub inline_packet_component_bytes: usize,
    pub descriptor_size: usize,
    pub input_size: usize,
    pub descriptor_reserved_bytes: usize,
    pub input_reserved_bytes: usize,
    pub total_arena_reserved_bytes: usize,
}

impl CompactInputArenaBuilder {
    pub fn try_with_capacity(packets: usize, events: usize) -> Result<Self, String> {
        let mut descriptors = Vec::new();
        descriptors
            .try_reserve_exact(packets)
            .map_err(|error| format!("reserve packet descriptors: {error}"))?;
        let mut inputs = Vec::new();
        inputs
            .try_reserve_exact(events)
            .map_err(|error| format!("reserve INPUT arena: {error}"))?;
        Ok(Self {
            descriptors,
            inputs,
        })
    }

    pub fn append_prepared(&mut self, original: &PreparedPhysicalPacket) -> Result<(), String> {
        let events = original.initialized_inputs();
        if events.is_empty()
            || events.len() > super::packet::MAX_PACKET_EVENTS
            || events.len() != usize::from(original.event_count())
            || events.len() != usize::from(original.packet().event_count())
        {
            return Err("invalid prepared packet length".into());
        }
        let start = self.inputs.len();
        let end = start
            .checked_add(events.len())
            .ok_or_else(|| "compact INPUT arena offset overflow".to_string())?;
        let offset =
            u32::try_from(start).map_err(|_| "compact INPUT offset exceeds u32".to_string())?;
        u32::try_from(end).map_err(|_| "compact INPUT arena end exceeds u32".to_string())?;
        self.descriptors
            .try_reserve(1)
            .map_err(|error| format!("grow packet descriptors: {error}"))?;
        self.inputs
            .try_reserve(events.len())
            .map_err(|error| format!("grow compact INPUT storage: {error}"))?;
        self.inputs.extend_from_slice(events);
        self.descriptors.push(CompactPacketDescriptor {
            packet: original.packet(),
            offset,
            length: original.event_count(),
        });
        Ok(())
    }

    pub fn freeze(self) -> Result<FrozenCompactInputArena, String> {
        let mut next = 0usize;
        for descriptor in &self.descriptors {
            if descriptor.offset as usize != next
                || descriptor.length == 0
                || usize::from(descriptor.length) != usize::from(descriptor.packet.event_count())
            {
                return Err("invalid compact packet descriptor".into());
            }
            next = next
                .checked_add(usize::from(descriptor.length))
                .ok_or_else(|| "descriptor span overflow".to_string())?;
        }
        if next != self.inputs.len() {
            return Err("compact INPUT arena does not cover all events".into());
        }
        // Vec ownership moves without retaining pointers while the builder
        // is mutable. This frozen owner exposes immutable slices only.
        Ok(FrozenCompactInputArena {
            descriptors: self.descriptors,
            inputs: self.inputs,
        })
    }
}

fn same_keyboard_fields(left: &INPUT, right: &INPUT) -> bool {
    if left.r#type != INPUT_KEYBOARD || right.r#type != INPUT_KEYBOARD {
        return false;
    }
    // SAFETY: the checked INPUT_KEYBOARD tag identifies the initialized
    // KEYBDINPUT union arm written by the canonical packet builder.
    unsafe {
        let a = left.Anonymous.ki;
        let b = right.Anonymous.ki;
        a.wVk == b.wVk
            && a.wScan == b.wScan
            && a.dwFlags == b.dwFlags
            && a.time == b.time
            && a.dwExtraInfo == b.dwExtraInfo
    }
}

impl FrozenCompactInputArena {
    pub fn packet_inputs(&self, index: usize) -> Option<&[INPUT]> {
        let descriptor = self.descriptors.get(index)?;
        let start = usize::try_from(descriptor.offset).ok()?;
        let end = start.checked_add(usize::from(descriptor.length))?;
        self.inputs.get(start..end)
    }

    pub fn mixed_up_prefix(&self, index: usize) -> Option<(PhysicalPacket, &[INPUT])> {
        let descriptor = self.descriptors.get(index)?;
        if descriptor.packet.up_mask == 0 || descriptor.packet.down_mask == 0 {
            return None;
        }
        let inputs = self.packet_inputs(index)?;
        if inputs.len() != usize::from(descriptor.packet.event_count()) {
            return None;
        }
        let up_len = descriptor.packet.up_mask.count_ones() as usize;
        Some((
            PhysicalPacket::new(descriptor.packet.up_mask, 0),
            inputs.get(..up_len)?,
        ))
    }

    pub fn verify_against(
        &self,
        index: usize,
        original: &PreparedPhysicalPacket,
    ) -> Result<(), String> {
        let descriptor = self
            .descriptors
            .get(index)
            .ok_or_else(|| "compact packet index outside descriptors".to_string())?;
        if descriptor.packet != original.packet() || descriptor.length != original.event_count() {
            return Err("compact packet identity or length mismatch".into());
        }
        let packed = self
            .packet_inputs(index)
            .ok_or_else(|| "compact packet span outside INPUT arena".to_string())?;
        let source = original.initialized_inputs();
        if packed.len() != source.len()
            || !packed
                .iter()
                .zip(source)
                .all(|(a, b)| same_keyboard_fields(a, b))
        {
            return Err("materialized keyboard INPUT mismatch".into());
        }
        match (original.up_recovery_view(), self.mixed_up_prefix(index)) {
            (None, None) => Ok(()),
            (Some(original_up), Some((packed_mask, packed_up))) => {
                let count = usize::from(original_up.packet().event_count());
                if packed_mask != original_up.packet()
                    || packed_up.len() != count
                    || !packed_up
                        .iter()
                        .zip(&source[..count])
                        .all(|(a, b)| same_keyboard_fields(a, b))
                {
                    return Err("mixed Up-prefix recovery mismatch".into());
                }
                Ok(())
            }
            _ => Err("mixed Up-prefix presence mismatch".into()),
        }
    }

    pub fn sizes(&self) -> CompactArenaSizes {
        let descriptor_reserved_bytes =
            self.descriptors.capacity() * size_of::<CompactPacketDescriptor>();
        let input_reserved_bytes = self.inputs.capacity() * size_of::<INPUT>();
        CompactArenaSizes {
            packet_count: self.descriptors.len(),
            event_count: self.inputs.len(),
            inline_packet_size: size_of::<PreparedPhysicalPacket>(),
            inline_packet_component_bytes: self.descriptors.len()
                * size_of::<PreparedPhysicalPacket>(),
            descriptor_size: size_of::<CompactPacketDescriptor>(),
            input_size: size_of::<INPUT>(),
            descriptor_reserved_bytes,
            input_reserved_bytes,
            total_arena_reserved_bytes: descriptor_reserved_bytes + input_reserved_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_KEYUP;

    #[test]
    fn all_chord_widths_and_max_mixed_packet_preserve_inputs() {
        let mut originals = Vec::new();
        for width in 1..=15 {
            let mask = (1u16 << width) - 1;
            originals.push(PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, mask)).unwrap());
            originals.push(PreparedPhysicalPacket::try_new(PhysicalPacket::new(mask, 0)).unwrap());
        }
        originals
            .push(PreparedPhysicalPacket::try_new(PhysicalPacket::new(0x007f, 0x7f80)).unwrap());
        let events: usize = originals
            .iter()
            .map(|packet| usize::from(packet.event_count()))
            .sum();
        let mut builder =
            CompactInputArenaBuilder::try_with_capacity(originals.len(), events).unwrap();
        for packet in &originals {
            builder.append_prepared(packet).unwrap();
        }
        let frozen = builder.freeze().unwrap();
        for (index, packet) in originals.iter().enumerate() {
            frozen.verify_against(index, packet).unwrap();
        }
        let (mask, up) = frozen.mixed_up_prefix(originals.len() - 1).unwrap();
        assert_eq!(mask, PhysicalPacket::new(0x007f, 0));
        assert_eq!(up.len(), 7);
        assert!(
            up.iter()
                .all(|event| unsafe { event.Anonymous.ki.dwFlags & KEYEVENTF_KEYUP != 0 })
        );
        assert_eq!(frozen.sizes().event_count, events);
        assert_eq!(frozen.sizes().packet_count, 31);
    }

    #[test]
    fn freeze_rejects_corrupt_descriptor_and_verifier_rejects_corrupt_event() {
        let original = PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 1)).unwrap();
        let mut builder = CompactInputArenaBuilder::try_with_capacity(1, 1).unwrap();
        builder.append_prepared(&original).unwrap();
        builder.descriptors[0].offset = 2;
        assert!(builder.freeze().is_err());

        let mut builder = CompactInputArenaBuilder::try_with_capacity(1, 1).unwrap();
        builder.append_prepared(&original).unwrap();
        let mut frozen = builder.freeze().unwrap();
        frozen.verify_against(0, &original).unwrap();
        // SAFETY: canonical packets contain keyboard INPUT values.
        unsafe { frozen.inputs[0].Anonymous.ki.wScan ^= 1 };
        assert!(frozen.verify_against(0, &original).is_err());
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn windows_x64_layout_is_reported_not_assumed() {
        let builder = CompactInputArenaBuilder::try_with_capacity(0, 0).unwrap();
        let sizes = builder.freeze().unwrap().sizes();
        assert_eq!(sizes.input_size, 40);
        assert_eq!(sizes.inline_packet_size, 608);
        assert!(sizes.descriptor_size <= 16);
    }
}
