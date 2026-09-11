use sky_dispatch_core::model::MAX_KEYS;
use smallvec::SmallVec;

/// A physical keyboard identity at the Win32 input boundary.
///
/// W4 accepts only non-extended Set-1 make codes, but the flag is retained in
/// the value so a future extended-key work order cannot infer semantics from a
/// scan-code number or keyboard layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalKey {
    pub scan_code: u16,
    pub extended: bool,
}

/// The finite W4 domain: ordinary non-extended main-key Set-1 make codes.
/// Control, modifier, lock, function, keypad, navigation, and E0/E1 keys are
/// intentionally outside this first profile contract.
pub const SUPPORTED_NON_EXTENDED_SCAN_CODES: &[u16] = &[
    0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x10, 0x11, 0x12, 0x13,
    0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25,
    0x26, 0x27, 0x28, 0x29, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x39,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstrumentKeyProfileSpec {
    pub keys: [PhysicalKey; MAX_KEYS],
}

impl InstrumentKeyProfileSpec {
    pub fn canonical() -> Self {
        Self {
            keys: std::array::from_fn(|slot| PhysicalKey {
                scan_code: super::scan_code::PHYSICAL_INSTRUMENT_SCAN_CODES[slot],
                extended: false,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstrumentKeyProfileError {
    #[error("instrument profile scan code is zero at logical slot {slot}")]
    ZeroScanCode { slot: usize },
    #[error("instrument profile uses an extended key at logical slot {slot}")]
    ExtendedKey { slot: usize, scan_code: u16 },
    #[error("instrument profile uses an E0/E1 prefix at logical slot {slot}: {scan_code:#x}")]
    ExtendedPrefix { slot: usize, scan_code: u16 },
    #[error(
        "instrument profile scan code is outside the W4 supported domain at logical slot {slot}: {scan_code:#x}"
    )]
    UnsupportedScanCode { slot: usize, scan_code: u16 },
    #[error(
        "instrument profile duplicates scan code {scan_code:#x} at logical slot {slot} (already used at slot {first_slot})"
    )]
    DuplicateScanCode {
        slot: usize,
        first_slot: usize,
        scan_code: u16,
    },
}

/// A validated, fixed-capacity profile. Its fields are private so callers
/// cannot construct a value that bypasses admission validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstrumentKeyProfile {
    keys: [PhysicalKey; MAX_KEYS],
}

impl InstrumentKeyProfile {
    pub fn try_from_spec(
        spec: InstrumentKeyProfileSpec,
    ) -> Result<Self, InstrumentKeyProfileError> {
        for (slot, key) in spec.keys.iter().copied().enumerate() {
            if key.scan_code == 0 {
                return Err(InstrumentKeyProfileError::ZeroScanCode { slot });
            }
            if key.scan_code == 0xe0 || key.scan_code == 0xe1 {
                return Err(InstrumentKeyProfileError::ExtendedPrefix {
                    slot,
                    scan_code: key.scan_code,
                });
            }
            if key.extended {
                return Err(InstrumentKeyProfileError::ExtendedKey {
                    slot,
                    scan_code: key.scan_code,
                });
            }
            if !SUPPORTED_NON_EXTENDED_SCAN_CODES.contains(&key.scan_code) {
                return Err(InstrumentKeyProfileError::UnsupportedScanCode {
                    slot,
                    scan_code: key.scan_code,
                });
            }
            if let Some(first_slot) = spec.keys[..slot]
                .iter()
                .position(|prior| prior.scan_code == key.scan_code)
            {
                return Err(InstrumentKeyProfileError::DuplicateScanCode {
                    slot,
                    first_slot,
                    scan_code: key.scan_code,
                });
            }
        }
        Ok(Self { keys: spec.keys })
    }

    pub fn canonical() -> Self {
        Self::try_from_spec(InstrumentKeyProfileSpec::canonical())
            .expect("the canonical instrument profile must be valid")
    }

    pub(crate) fn keys(&self) -> &[PhysicalKey; MAX_KEYS] {
        &self.keys
    }
}

/// The one session-owned Win32 profile. Template arrays are built once during
/// session admission and are never rebuilt on the precision path.
pub struct MaterializedInstrumentKeyProfile {
    keys: [PhysicalKey; MAX_KEYS],
    #[cfg(windows)]
    down_templates: [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT; MAX_KEYS],
    #[cfg(windows)]
    up_templates: [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT; MAX_KEYS],
}

impl std::fmt::Debug for MaterializedInstrumentKeyProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MaterializedInstrumentKeyProfile")
            .field("keys", &self.keys)
            .finish()
    }
}

impl MaterializedInstrumentKeyProfile {
    pub fn from_validated(profile: InstrumentKeyProfile) -> Self {
        let keys = *profile.keys();
        #[cfg(windows)]
        {
            let mut down_templates = [super::packet::create_keyboard_input(0, false); MAX_KEYS];
            let mut up_templates = [super::packet::create_keyboard_input(0, true); MAX_KEYS];
            for slot in 0..MAX_KEYS {
                down_templates[slot] =
                    super::packet::create_keyboard_input(keys[slot].scan_code, false);
                up_templates[slot] =
                    super::packet::create_keyboard_input(keys[slot].scan_code, true);
            }
            Self {
                keys,
                down_templates,
                up_templates,
            }
        }
        #[cfg(not(windows))]
        Self { keys }
    }

    pub fn canonical() -> Self {
        Self::from_validated(InstrumentKeyProfile::canonical())
    }

    pub fn physical_key(&self, slot: usize) -> PhysicalKey {
        self.keys[slot]
    }

    pub(crate) fn logical_mask_for_scan_codes(&self, scan_codes: &[u16]) -> Option<u16> {
        scan_codes.iter().try_fold(0u16, |mask, scan_code| {
            self.keys
                .iter()
                .position(|key| key.scan_code == *scan_code && !key.extended)
                .map(|slot| mask | (1u16 << slot))
        })
    }

    pub(crate) fn scan_codes_from_mask(&self, mask: u16) -> SmallVec<[u16; MAX_KEYS]> {
        let mut scan_codes = SmallVec::new();
        for slot in 0..MAX_KEYS {
            if mask & (1u16 << slot) != 0 {
                scan_codes.push(self.keys[slot].scan_code);
            }
        }
        scan_codes
    }

    #[cfg(windows)]
    pub(crate) fn down_template(
        &self,
        slot: usize,
    ) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
        self.down_templates[slot]
    }

    #[cfg(windows)]
    pub(crate) fn up_template(
        &self,
        slot: usize,
    ) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
        self.up_templates[slot]
    }
}

impl Default for MaterializedInstrumentKeyProfile {
    fn default() -> Self {
        Self::canonical()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InstrumentKeyProfile, InstrumentKeyProfileError, InstrumentKeyProfileSpec, PhysicalKey,
        SUPPORTED_NON_EXTENDED_SCAN_CODES,
    };
    use sky_dispatch_core::model::MAX_KEYS;

    #[test]
    fn accepts_non_canonical_main_key() {
        let mut spec = InstrumentKeyProfileSpec::canonical();
        spec.keys[0] = PhysicalKey {
            scan_code: 0x02,
            extended: false,
        };
        let profile = InstrumentKeyProfile::try_from_spec(spec).expect("profile");
        assert_eq!(profile.keys()[0].scan_code, 0x02);
    }

    #[test]
    fn rejects_duplicate_extended_and_out_of_domain_keys() {
        let mut duplicate = InstrumentKeyProfileSpec::canonical();
        duplicate.keys[1] = duplicate.keys[0];
        assert!(matches!(
            InstrumentKeyProfile::try_from_spec(duplicate),
            Err(InstrumentKeyProfileError::DuplicateScanCode { .. })
        ));

        let mut extended = InstrumentKeyProfileSpec::canonical();
        extended.keys[0].extended = true;
        assert!(matches!(
            InstrumentKeyProfile::try_from_spec(extended),
            Err(InstrumentKeyProfileError::ExtendedKey { .. })
        ));

        let mut unsupported = InstrumentKeyProfileSpec::canonical();
        unsupported.keys[0].scan_code = 0x3a;
        assert!(matches!(
            InstrumentKeyProfile::try_from_spec(unsupported),
            Err(InstrumentKeyProfileError::UnsupportedScanCode { .. })
        ));
    }

    #[test]
    fn supported_domain_is_exact_unique_and_exhaustively_validated() {
        const EXPECTED: [u16; 48] = [
            0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x10, 0x11,
            0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1e, 0x1f, 0x20, 0x21,
            0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x30,
            0x31, 0x32, 0x33, 0x34, 0x35, 0x39,
        ];
        assert_eq!(SUPPORTED_NON_EXTENDED_SCAN_CODES, EXPECTED);
        let mut sorted = EXPECTED;
        sorted.sort_unstable();
        assert!(sorted.windows(2).all(|pair| pair[0] != pair[1]));
        assert!(EXPECTED.iter().all(|scan_code| *scan_code != 0));

        for &scan_code in &EXPECTED {
            let mut keys = [PhysicalKey {
                scan_code: EXPECTED[0],
                extended: false,
            }; MAX_KEYS];
            keys[0] = PhysicalKey {
                scan_code,
                extended: false,
            };
            let mut slot = 1;
            for &other in &EXPECTED {
                if other != scan_code && slot < MAX_KEYS {
                    keys[slot] = PhysicalKey {
                        scan_code: other,
                        extended: false,
                    };
                    slot += 1;
                }
            }
            assert_eq!(slot, MAX_KEYS);
            assert!(InstrumentKeyProfile::try_from_spec(InstrumentKeyProfileSpec { keys }).is_ok());
        }
    }
}
