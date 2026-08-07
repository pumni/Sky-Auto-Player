use smallvec::SmallVec;

pub const SKY_PLAYER_SIGNATURE: usize = 0x5C1B9111;

pub const PHYSICAL_INSTRUMENT_SCAN_CODES: [u16; 15] = [
    0x15, 0x16, 0x17, 0x18, 0x19, // Y U I O P
    0x23, 0x24, 0x25, 0x26, 0x27, // H J K L ;
    0x31, 0x32, 0x33, 0x34, 0x35, // N M , . /
];
pub const FULL_INSTRUMENT_MASK: u16 = (1u16 << PHYSICAL_INSTRUMENT_SCAN_CODES.len()) - 1;

// The current instrument allowlist contains no E0/E1 extended scan codes.
pub(crate) const MAX_INSTRUMENT_SCAN_CODE: usize = 0x35;
pub(crate) const SCAN_CODE_TO_MASK: [u16; MAX_INSTRUMENT_SCAN_CODE + 1] = {
    let mut table = [0u16; MAX_INSTRUMENT_SCAN_CODE + 1];
    table[0x15] = 1 << 0;
    table[0x16] = 1 << 1;
    table[0x17] = 1 << 2;
    table[0x18] = 1 << 3;
    table[0x19] = 1 << 4;
    table[0x23] = 1 << 5;
    table[0x24] = 1 << 6;
    table[0x25] = 1 << 7;
    table[0x26] = 1 << 8;
    table[0x27] = 1 << 9;
    table[0x31] = 1 << 10;
    table[0x32] = 1 << 11;
    table[0x33] = 1 << 12;
    table[0x34] = 1 << 13;
    table[0x35] = 1 << 14;
    table
};

#[inline]
pub(crate) fn key_mask(scan_code: u16) -> Option<u16> {
    let mask = SCAN_CODE_TO_MASK
        .get(scan_code as usize)
        .copied()
        .unwrap_or(0);
    (mask != 0).then_some(mask)
}

#[inline]
pub(crate) fn valid_instrument_scan_code(scan_code: u16) -> bool {
    key_mask(scan_code).is_some()
}

pub(crate) fn scan_codes_from_mask(mask: u16) -> SmallVec<[u16; 15]> {
    PHYSICAL_INSTRUMENT_SCAN_CODES
        .iter()
        .enumerate()
        .filter_map(|(slot, &scan_code)| (mask & (1u16 << slot) != 0).then_some(scan_code))
        .collect()
}
