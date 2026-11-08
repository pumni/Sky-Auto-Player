//! Offline, test-support-only contiguous Win32 INPUT arena. The normal
//! sender and its fixed-capacity prepared packets are not modified.
//!
//! Equality here checks initialized keyboard fields, never raw INPUT bytes:
//! C-compatible unions/structs may contain uninitialized padding.

use super::outcome::PhysicalPacket;
use super::packet::PreparedPhysicalPacket;
use std::mem::size_of;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT;

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

impl FrozenCompactInputArena {
    /// Structural identity only. Semantic keyboard-field equality is covered
    /// by dedicated Win32 unit tests; no union inspection on the live path.
    pub fn packet_identity(&self, index: usize) -> Option<PhysicalPacket> {
        Some(self.descriptors.get(index)?.packet)
    }

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
mod tests;
