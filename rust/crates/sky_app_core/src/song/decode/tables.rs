//! Frozen source-game note tables used by the versioned sheet decoder.
//!
//! The numeric reference data is derived from Specy's Sky Music Nightly
//! `legacyNoteTables.ts`, `noteIds.ts`, `game.json`, `presets.json`, and
//! instrument metadata at commit `d41f8a703955ba4ed1e1c4450407080bb3ae364f`.
//! The upstream repository is AGPL-3.0. No upstream implementation code is
//! copied here; these are the small numeric tables needed to preserve source
//! note identity and its sounding pitch.

use super::{Pitchability, SourceGame};

#[derive(Debug, Clone, Copy)]
pub(crate) struct PitchEntry {
    pub(crate) nominal: i32,
    pub(crate) sounding: i32,
    pub(crate) pitched: bool,
}

const fn build_table<const N: usize>(
    nominal: [i32; N],
    sounding_offset: i32,
    unpitched_mask: u32,
) -> [PitchEntry; N] {
    let mut table = [PitchEntry {
        nominal: 0,
        sounding: 0,
        pitched: false,
    }; N];
    let mut index = 0;
    while index < N {
        table[index] = PitchEntry {
            nominal: nominal[index],
            sounding: nominal[index] + sounding_offset,
            pitched: (unpitched_mask & (1 << index)) == 0,
        };
        index += 1;
    }
    table
}

const fn build_ukulele_table() -> [PitchEntry; 21] {
    let mut table = build_table(GENSHIN_STANDARD_21, 0, 0x7f);
    let mut index = 0;
    while index < 7 {
        table[index].sounding += 12;
        index += 1;
    }
    table
}

const SKY_STANDARD_15: [i32; 15] = [60, 62, 64, 65, 67, 69, 71, 72, 74, 76, 77, 79, 81, 83, 84];
const SKY_DRUM_8: [i32; 8] = [60, 62, 64, 65, 67, 69, 71, 72];
const SKY_DRUM_4: [i32; 4] = [60, 62, 64, 65];
const SKY_SFX_DANCE_6: [i32; 6] = [60, 62, 64, 65, 67, 69];
const SKY_BELLS_8: [i32; 8] = [60, 62, 67, 69, 72, 74, 79, 81];
const SKY_HANDPAN_8: [i32; 8] = [62, 69, 72, 74, 77, 79, 81, 84];
const SKY_KRILL_HORN_8: [i32; 8] = [65, 67, 69, 71, 72, 74, 76, 77];
const GENSHIN_STANDARD_21: [i32; 21] = [
    72, 74, 76, 77, 79, 81, 83, 60, 62, 64, 65, 67, 69, 71, 48, 50, 52, 53, 55, 57, 59,
];
const GENSHIN_NIGHTWIND_14: [i32; 14] = [60, 62, 64, 65, 67, 69, 71, 48, 50, 52, 53, 55, 57, 59];
const GENSHIN_VODYANITSA_14: [i32; 14] = [72, 74, 76, 77, 79, 81, 83, 60, 62, 64, 65, 67, 69, 71];

const SKY_STANDARD: [PitchEntry; 15] = build_table(SKY_STANDARD_15, 0, 0);
const SKY_STANDARD_DOWN_12: [PitchEntry; 15] = build_table(SKY_STANDARD_15, -12, 0);
const SKY_STANDARD_DOWN_24: [PitchEntry; 15] = build_table(SKY_STANDARD_15, -24, 0);
const SKY_STANDARD_DOWN_36: [PitchEntry; 15] = build_table(SKY_STANDARD_15, -36, 0);
const SKY_STANDARD_UP_12: [PitchEntry; 15] = build_table(SKY_STANDARD_15, 12, 0);
const SKY_DRUMS: [PitchEntry; 8] = build_table(SKY_DRUM_8, 0, 0xff);
const SKY_DRUMS_DOWN_24: [PitchEntry; 8] = build_table(SKY_DRUM_8, -24, 0);
const SKY_DRUMS_UP_12: [PitchEntry; 8] = build_table(SKY_DRUM_8, 12, 0);
const SKY_DRUMS_UP_24: [PitchEntry; 8] = build_table(SKY_DRUM_8, 24, 0);
const SKY_DRUMS_4: [PitchEntry; 4] = build_table(SKY_DRUM_4, 0, 0xf);
const SKY_SFX_DANCE: [PitchEntry; 6] = build_table(SKY_SFX_DANCE_6, 0, 0x3f);
const SKY_BELLS: [PitchEntry; 8] = build_table(SKY_BELLS_8, 0, 0);
const SKY_HANDPAN: [PitchEntry; 8] = build_table(SKY_HANDPAN_8, -12, 0);
const SKY_KRILL_HORN: [PitchEntry; 8] = build_table(SKY_KRILL_HORN_8, -12, 0);
const GENSHIN_STANDARD: [PitchEntry; 21] = build_table(GENSHIN_STANDARD_21, 0, 0);
const GENSHIN_DRUMS: [PitchEntry; 8] = build_table(SKY_DRUM_8, 0, 0xff);
const GENSHIN_NIGHTWIND: [PitchEntry; 14] = build_table(GENSHIN_NIGHTWIND_14, 0, 0);
const GENSHIN_VODYANITSA: [PitchEntry; 14] = build_table(GENSHIN_VODYANITSA_14, 0, 0);
const GENSHIN_LINGERING_EUPHONIA: [PitchEntry; 21] = build_table(GENSHIN_STANDARD_21, 0, 0x7f);
const GENSHIN_UKULELE: [PitchEntry; 21] = build_ukulele_table();
const GENSHIN_VINTAGE_LYRE: [PitchEntry; 21] = [
    PitchEntry {
        nominal: 72,
        sounding: 72,
        pitched: true,
    },
    PitchEntry {
        nominal: 74,
        sounding: 73,
        pitched: true,
    },
    PitchEntry {
        nominal: 76,
        sounding: 75,
        pitched: true,
    },
    PitchEntry {
        nominal: 77,
        sounding: 77,
        pitched: true,
    },
    PitchEntry {
        nominal: 79,
        sounding: 79,
        pitched: true,
    },
    PitchEntry {
        nominal: 81,
        sounding: 80,
        pitched: true,
    },
    PitchEntry {
        nominal: 83,
        sounding: 82,
        pitched: true,
    },
    PitchEntry {
        nominal: 60,
        sounding: 60,
        pitched: true,
    },
    PitchEntry {
        nominal: 62,
        sounding: 62,
        pitched: true,
    },
    PitchEntry {
        nominal: 64,
        sounding: 63,
        pitched: true,
    },
    PitchEntry {
        nominal: 65,
        sounding: 65,
        pitched: true,
    },
    PitchEntry {
        nominal: 67,
        sounding: 67,
        pitched: true,
    },
    PitchEntry {
        nominal: 69,
        sounding: 69,
        pitched: true,
    },
    PitchEntry {
        nominal: 71,
        sounding: 70,
        pitched: true,
    },
    PitchEntry {
        nominal: 48,
        sounding: 48,
        pitched: true,
    },
    PitchEntry {
        nominal: 50,
        sounding: 50,
        pitched: true,
    },
    PitchEntry {
        nominal: 52,
        sounding: 51,
        pitched: true,
    },
    PitchEntry {
        nominal: 53,
        sounding: 53,
        pitched: true,
    },
    PitchEntry {
        nominal: 55,
        sounding: 55,
        pitched: true,
    },
    PitchEntry {
        nominal: 57,
        sounding: 57,
        pitched: true,
    },
    PitchEntry {
        nominal: 59,
        sounding: 58,
        pitched: true,
    },
];

const LEGACY_GENSHIN_DEFAULT: [i32; 21] = GENSHIN_STANDARD_21;
const LEGACY_GENSHIN_DRUMS: [i32; 8] = SKY_DRUM_8;
const LEGACY_SKY_DEFAULT: [i32; 15] = SKY_STANDARD_15;
const LEGACY_SKY_8: [i32; 8] = SKY_DRUM_8;
const LEGACY_SKY_BELLS: [i32; 8] = SKY_BELLS_8;
const LEGACY_SKY_HANDPAN: [i32; 8] = SKY_HANDPAN_8;
const LEGACY_SKY_SFX_6: [i32; 6] = SKY_SFX_DANCE_6;

pub(crate) fn legacy_index_to_nominal(
    game: SourceGame,
    instrument: &str,
    index: usize,
) -> Option<i32> {
    let table: &[i32] = match game {
        SourceGame::Genshin => match instrument {
            "DunDun" | "DjemDjemDrum" => &LEGACY_GENSHIN_DRUMS,
            "Lyre" | "Vintage-Lyre" | "Zither" | "Old-Zither" | "Ukulele" | "LingeringEuphonia"
            | "LeapingSpiritPiano" | "HarmonicKey" => &LEGACY_GENSHIN_DEFAULT,
            _ => return None,
        },
        SourceGame::Sky => match instrument {
            "Bells" => &LEGACY_SKY_BELLS,
            "HandPan" => &LEGACY_SKY_HANDPAN,
            "Drum" | "DunDun" | "SFX_SineSynth" | "SFX_BassSynth" | "SFX_ChimeSynth"
            | "SFX_TR-909" => &LEGACY_SKY_8,
            "SFX_Dance" => &LEGACY_SKY_SFX_6,
            "Piano"
            | "GrandPiano"
            | "WinterPiano"
            | "Cello"
            | "Harp"
            | "Guitar"
            | "LightGuitar"
            | "ToyUkulele"
            | "Pipa"
            | "Kalimba"
            | "Xylophone"
            | "Flute"
            | "Panflute"
            | "Ocarina"
            | "MantaOcarina"
            | "Trumpet"
            | "Horn"
            | "Contrabass"
            | "Aurora"
            | "Aurora_Short"
            | "SFX_BirdCall"
            | "SFX_CrabCall"
            | "SFX_FishCall"
            | "SFX_SpiritMantaCall"
            | "SFX_JellyCall"
            | "SFX_MantaCall"
            | "SFX_MothCall" => &LEGACY_SKY_DEFAULT,
            _ => return None,
        },
    };
    table.get(index).copied()
}

pub(crate) fn instrument_supported(game: SourceGame, instrument: &str) -> bool {
    instrument_pitch_table(game, instrument).is_some()
}

pub(crate) fn default_instrument(game: SourceGame) -> &'static str {
    match game {
        SourceGame::Sky => "Piano",
        SourceGame::Genshin => "Lyre",
    }
}

pub(crate) fn nominal_to_absolute(
    game: SourceGame,
    instrument: &str,
    nominal: i32,
    pitch_offset: i32,
) -> Option<(i32, Pitchability)> {
    let table = instrument_pitch_table(game, instrument)?;
    match table.iter().find(|entry| entry.nominal == nominal) {
        Some(entry) => Some((
            entry.sounding.checked_add(pitch_offset)?,
            entry.pitchability(),
        )),
        // Upstream preserves a stranded Nominal Id on the absolute axis. It has
        // no source button and is explicitly marked unresolved for later planning.
        None => Some((nominal.checked_add(pitch_offset)?, Pitchability::Unresolved)),
    }
}

pub(crate) fn absolute_note_pitchability(
    game: SourceGame,
    instrument: &str,
    number: i32,
    pitch_offset: i32,
) -> Option<Pitchability> {
    let table = instrument_pitch_table(game, instrument)?;
    let source_sounding = number.checked_sub(pitch_offset)?;
    Some(
        table
            .iter()
            .find(|entry| entry.sounding == source_sounding)
            .map(|entry| entry.pitchability())
            .unwrap_or(Pitchability::Unresolved),
    )
}

fn instrument_pitch_table(game: SourceGame, instrument: &str) -> Option<&'static [PitchEntry]> {
    let table: &'static [PitchEntry] = match game {
        SourceGame::Genshin => match instrument {
            "Lyre" | "Zither" | "Old-Zither" | "LeapingSpiritPiano" | "HarmonicKey" => {
                &GENSHIN_STANDARD
            }
            "Vintage-Lyre" => &GENSHIN_VINTAGE_LYRE,
            "Ukulele" => &GENSHIN_UKULELE,
            "LingeringEuphonia" => &GENSHIN_LINGERING_EUPHONIA,
            "DunDun" | "DjemDjemDrum" => &GENSHIN_DRUMS,
            "NightwindHorn" => &GENSHIN_NIGHTWIND,
            "Vodyanitsa" => &GENSHIN_VODYANITSA,
            _ => return None,
        },
        SourceGame::Sky => match instrument {
            "Contrabass" => &SKY_STANDARD_DOWN_36,
            "Cello" | "Horn" => &SKY_STANDARD_DOWN_24,
            "Guitar"
            | "LightGuitar"
            | "Harp"
            | "Saxophone"
            | "Pipa"
            | "ToyUkulele"
            | "SFX_FishCall"
            | "SFX_SpiritMantaCall"
            | "SFX_JellyCall"
            | "SFX_MantaCall"
            | "SFX_MothCall" => &SKY_STANDARD_DOWN_12,
            "WinterPiano" | "Xylophone" => &SKY_STANDARD_UP_12,
            "Piano" | "GrandPiano" | "Violin" | "Trumpet" | "Flute" | "TransverseFlute"
            | "Panflute" | "Ocarina" | "MantaOcarina" | "Harmonica" | "Aurora" | "Kalimba"
            | "SFX_BirdCall" | "SFX_CrabCall" | "Aurora_Short" => &SKY_STANDARD,
            "Drum" | "DunDun" | "SFX_TR-909" => &SKY_DRUMS,
            "FortuneDrum" | "Cymbals" => &SKY_DRUMS_4,
            "Bells" | "SmallBell" => &SKY_BELLS,
            "HandPan" | "TriumphHandPan" => &SKY_HANDPAN,
            "KrillHorn" => &SKY_KRILL_HORN,
            "SFX_Dance" => &SKY_SFX_DANCE,
            "SFX_BassSynth" => &SKY_DRUMS_DOWN_24,
            "SFX_SineSynth" => &SKY_DRUMS_UP_12,
            "SFX_ChimeSynth" => &SKY_DRUMS_UP_24,
            _ => return None,
        },
    };
    Some(table)
}

impl PitchEntry {
    fn pitchability(self) -> Pitchability {
        if self.pitched {
            Pitchability::Pitched
        } else {
            Pitchability::Unpitched
        }
    }
}
