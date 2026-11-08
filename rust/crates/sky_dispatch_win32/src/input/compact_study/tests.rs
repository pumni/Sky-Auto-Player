use super::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_KEYBOARD, KEYEVENTF_KEYUP};

fn same_keyboard_fields(left: &INPUT, right: &INPUT) -> bool {
    if left.r#type != INPUT_KEYBOARD || right.r#type != INPUT_KEYBOARD {
        return false;
    }
    // SAFETY: checked INPUT_KEYBOARD discriminants identify initialized
    // KEYBDINPUT union arms created by the prebuilt keyboard packet builder.
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

fn verify_against(
    arena: &FrozenCompactInputArena,
    index: usize,
    original: &PreparedPhysicalPacket,
) -> Result<(), String> {
    let descriptor = arena
        .descriptors
        .get(index)
        .ok_or_else(|| "compact packet index outside descriptors".to_string())?;
    if descriptor.packet != original.packet() || descriptor.length != original.event_count() {
        return Err("compact packet identity or length mismatch".into());
    }
    let packed = arena
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
    match (original.up_recovery_view(), arena.mixed_up_prefix(index)) {
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

#[test]
fn all_chord_widths_and_max_mixed_packet_preserve_inputs() {
    let mut originals = Vec::new();
    for width in 1..=15 {
        let mask = (1u16 << width) - 1;
        originals.push(PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, mask)).unwrap());
        originals.push(PreparedPhysicalPacket::try_new(PhysicalPacket::new(mask, 0)).unwrap());
    }
    originals.push(PreparedPhysicalPacket::try_new(PhysicalPacket::new(0x007f, 0x7f80)).unwrap());
    let events: usize = originals
        .iter()
        .map(|packet| usize::from(packet.event_count()))
        .sum();
    let mut builder = CompactInputArenaBuilder::try_with_capacity(originals.len(), events).unwrap();
    for packet in &originals {
        builder.append_prepared(packet).unwrap();
    }
    let frozen = builder.freeze().unwrap();
    for (index, packet) in originals.iter().enumerate() {
        verify_against(&frozen, index, packet).unwrap();
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
    verify_against(&frozen, 0, &original).unwrap();
    // SAFETY: canonical packets contain keyboard INPUT values.
    unsafe { frozen.inputs[0].Anonymous.ki.wScan ^= 1 };
    assert!(verify_against(&frozen, 0, &original).is_err());
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
