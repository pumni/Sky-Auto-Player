use super::{
    CanonicalEvent, CanonicalPitch, CanonicalSong, DecodeWarning, DiagnosticCounts,
    MAX_SOURCE_COLUMNS, MAX_SOURCE_NOTES, MAX_SOURCE_TRACKS, RawNoteIdentity, SongDecodeError,
    SongFormat, SourcePosition, SourceTrack, binary_layer_mask, check_limit, composed_boundaries,
    decode_bpm, decode_flat_key, decode_source_game, decode_source_pitch, diagnostics,
    flat_key_collisions, hex_layer_mask, highest_track, invalid, layer_mask_integer,
    milliseconds_to_microseconds, nonnegative_index, optional_pitch, pitch_index,
    recorded_v1_layer_mask, required_array, required_string, required_value, resource_limit,
    source_track, tables, tempo_id, track_pitch_offset, unsupported_instrument,
};
use serde_json::{Map, Value};

pub(super) fn decode_legacy_flat(
    object: &Map<String, Value>,
    format: SongFormat,
    name: String,
) -> Result<CanonicalSong, SongDecodeError> {
    let (source_game, source_app_name) = decode_source_game(object, true)?;
    let source_pitch = decode_source_pitch(object, true)?;
    let bpm = decode_bpm(object, false)?;
    let notes = required_array(object, "songNotes", "songNotes")?;
    check_limit("source notes", notes.len(), MAX_SOURCE_NOTES)?;

    let mut events = Vec::with_capacity(notes.len());
    let mut tracks = Vec::new();
    let mut empty_layer_mask_count = 0;
    let mut warnings = Vec::new();
    let mut expanded_count = 0usize;
    for (source_index, raw) in notes.iter().enumerate() {
        let note = raw
            .as_object()
            .ok_or_else(|| invalid(format!("songNotes[{source_index}]"), "must be an object"))?;
        let key_path = format!("songNotes[{source_index}].key");
        let key = required_string(note, "key", &key_path)?;
        let (key_index, prefix_mask) = decode_flat_key(key)
            .ok_or_else(|| invalid(&key_path, "must be Key0..Key14 or 1KeyN/2KeyN/3KeyN"))?;
        let mask = match note.get("l") {
            Some(value) => layer_mask_integer(value, &format!("songNotes[{source_index}].l"))?,
            None => prefix_mask,
        };
        let time_path = format!("songNotes[{source_index}].time");
        let onset_us =
            milliseconds_to_microseconds(required_value(note, "time", &time_path)?, &time_path)?;
        if mask == 0 {
            empty_layer_mask_count += 1;
            warnings.push(DecodeWarning::EmptyLayerMask { source_index });
            continue;
        }
        let highest_track = highest_track(mask);
        check_limit("source tracks", highest_track, MAX_SOURCE_TRACKS)?;
        expanded_count = expanded_count
            .checked_add(mask.count_ones() as usize)
            .ok_or_else(|| resource_limit("expanded source notes", usize::MAX, MAX_SOURCE_NOTES))?;
        check_limit("expanded source notes", expanded_count, MAX_SOURCE_NOTES)?;
        while tracks.len() < highest_track {
            tracks.push(SourceTrack {
                index: tracks.len(),
                instrument: None,
                effective_pitch: None,
            });
        }
        for track_index in 0..highest_track {
            if mask & (1u16 << track_index) == 0 {
                continue;
            }
            events.push(CanonicalEvent {
                source_index,
                source_position: SourcePosition::FlatSongNote {
                    note_index: source_index,
                },
                track_index,
                onset_us,
                duration_us: 0,
                raw_identity: RawNoteIdentity::LegacyKeyIndex(key_index),
                pitch: CanonicalPitch::SkyKeyIndex(key_index),
                layer_mask: Some(mask),
                span_columns: None,
            });
        }
    }
    events.sort_by_key(|event| event.onset_us);
    let (potential_physical_key_merge_count, collision_warnings) = flat_key_collisions(&events);
    warnings.extend(collision_warnings);
    let diagnostics = diagnostics(
        DiagnosticCounts {
            source_note_count: notes.len(),
            source_track_count: tracks.len(),
            source_column_count: 0,
            empty_layer_mask_count,
            potential_physical_key_merge_count,
        },
        &events,
        warnings,
    );
    Ok(CanonicalSong {
        name,
        format,
        source_game,
        source_app_name,
        source_pitch,
        bpm,
        tracks,
        events,
        diagnostics,
    })
}

pub(super) fn decode_composed_legacy(
    object: &Map<String, Value>,
    version: u8,
    format: SongFormat,
    name: String,
) -> Result<CanonicalSong, SongDecodeError> {
    let (source_game, source_app_name) = decode_source_game(object, false)?;
    let source_pitch = decode_source_pitch(object, false)?;
    let pitch_offset = pitch_index(&source_pitch).expect("decoded source pitch is valid") as i32;
    let bpm = decode_bpm(object, true)?.expect("required BPM is present");
    let columns = required_array(object, "columns", "columns")?;
    check_limit("columns", columns.len(), MAX_SOURCE_COLUMNS)?;
    let instruments = required_array(object, "instruments", "instruments")?;
    check_limit("source tracks", instruments.len(), MAX_SOURCE_TRACKS)?;

    let mut roster = Vec::with_capacity(instruments.len());
    for (index, instrument) in instruments.iter().enumerate() {
        let path = format!("instruments[{index}]");
        let track = match version {
            1 | 2 => {
                let instrument_name = instrument.as_str().ok_or_else(|| {
                    invalid(&path, "must be a source instrument name for composed v1/v2")
                })?;
                source_track(source_game, instrument_name, None, pitch_offset, index)?
            }
            3 => {
                let metadata = instrument.as_object().ok_or_else(|| {
                    invalid(&path, "must be an instrument object for composed v3")
                })?;
                let instrument_name = required_string(metadata, "name", &path)?;
                let track_pitch = optional_pitch(metadata.get("pitch"), pitch_offset, &path)?;
                source_track(
                    source_game,
                    instrument_name,
                    Some(track_pitch.0),
                    track_pitch.1,
                    index,
                )?
            }
            _ => unreachable!("legacy composed version range is 1..=3"),
        };
        roster.push(track);
    }

    let mut raw_columns = Vec::with_capacity(columns.len());
    let mut source_note_count = 0usize;
    let mut source_index = 0usize;
    for (column_index, raw_column) in columns.iter().enumerate() {
        let column_path = format!("columns[{column_index}]");
        let tuple = raw_column
            .as_array()
            .filter(|tuple| tuple.len() == 2)
            .ok_or_else(|| invalid(&column_path, "must be [tempoId, notes]"))?;
        let tempo_path = format!("{column_path}[0]");
        let tempo_id = tempo_id(&tuple[0], &tempo_path)?;
        let notes = tuple[1]
            .as_array()
            .ok_or_else(|| invalid(format!("{column_path}[1]"), "must be an array"))?;
        source_note_count = source_note_count
            .checked_add(notes.len())
            .ok_or_else(|| resource_limit("source notes", usize::MAX, MAX_SOURCE_NOTES))?;
        check_limit("source notes", source_note_count, MAX_SOURCE_NOTES)?;
        let mut parsed_notes = Vec::with_capacity(notes.len());
        for (note_index, raw_note) in notes.iter().enumerate() {
            let note_path = format!("{column_path}[1][{note_index}]");
            let note = raw_note
                .as_array()
                .filter(|note| note.len() == 2)
                .ok_or_else(|| invalid(&note_path, "must be [legacyIndex, layerMask]"))?;
            let index = nonnegative_index(&note[0], &format!("{note_path}[0]"))?;
            let mask = if version == 1 {
                binary_layer_mask(&note[1], &format!("{note_path}[1]"))?
            } else {
                hex_layer_mask(&note[1], &format!("{note_path}[1]"))?
            };
            let selected_track_count = highest_track(mask);
            if selected_track_count > roster.len() {
                return Err(invalid(
                    format!("{note_path}[1]"),
                    format!(
                        "layer mask selects track {}, but only {} instrument tracks are declared",
                        selected_track_count - 1,
                        roster.len()
                    ),
                ));
            }
            parsed_notes.push((source_index, note_index, index, mask));
            source_index += 1;
        }
        raw_columns.push((tempo_id, parsed_notes));
    }
    let tempo_ids = raw_columns
        .iter()
        .map(|(tempo_id, _)| *tempo_id)
        .collect::<Vec<_>>();
    let boundaries = composed_boundaries(&tempo_ids, bpm)?;

    let mut events = Vec::with_capacity(source_note_count);
    let mut empty_layer_mask_count = 0usize;
    let mut warnings = Vec::new();
    let mut expanded_count = 0usize;
    for (column_index, (_, notes)) in raw_columns.iter().enumerate() {
        for (source_index, note_index, index, mask) in notes {
            if *mask == 0 {
                empty_layer_mask_count += 1;
                warnings.push(DecodeWarning::EmptyLayerMask {
                    source_index: *source_index,
                });
                continue;
            }
            expanded_count = expanded_count
                .checked_add(mask.count_ones() as usize)
                .ok_or_else(|| {
                    resource_limit("expanded source notes", usize::MAX, MAX_SOURCE_NOTES)
                })?;
            check_limit("expanded source notes", expanded_count, MAX_SOURCE_NOTES)?;
            let onset_us = boundaries[column_index];
            // Each layer has its own instrument table; resolve the legacy index per track below.
            for (track_index, track) in roster.iter().enumerate() {
                if mask & (1u16 << track_index) == 0 {
                    continue;
                }
                let instrument_name = track
                    .instrument
                    .as_deref()
                    .unwrap_or_else(|| tables::default_instrument(source_game));
                let nominal = tables::legacy_index_to_nominal(source_game, instrument_name, *index)
                    .ok_or_else(|| {
                        invalid(
                            format!("columns[{column_index}][1][{note_index}][0]"),
                            format!("legacy index {index} is outside the frozen {source_game:?}/{instrument_name} table"),
                        )
                    })?;
                let pitch_offset = track_pitch_offset(track, &source_pitch);
                let (number, source_pitchability) = tables::nominal_to_absolute(
                    source_game,
                    instrument_name,
                    nominal,
                    pitch_offset,
                )
                .ok_or_else(|| unsupported_instrument(source_game, instrument_name))?;
                events.push(CanonicalEvent {
                    source_index: *source_index,
                    source_position: SourcePosition::ComposedColumn {
                        column_index,
                        note_index: *note_index,
                    },
                    track_index,
                    onset_us,
                    duration_us: 0,
                    raw_identity: RawNoteIdentity::LegacyIndex(*index as u32),
                    pitch: CanonicalPitch::NoteNumber {
                        number,
                        source_pitchability,
                    },
                    layer_mask: Some(*mask),
                    span_columns: None,
                });
            }
        }
    }
    events.sort_by_key(|event| event.onset_us);
    let diagnostics = diagnostics(
        DiagnosticCounts {
            source_note_count,
            source_track_count: roster.len(),
            source_column_count: columns.len(),
            empty_layer_mask_count,
            potential_physical_key_merge_count: 0,
        },
        &events,
        warnings,
    );
    Ok(CanonicalSong {
        name,
        format,
        source_game,
        source_app_name,
        source_pitch,
        bpm: Some(bpm),
        tracks: roster,
        events,
        diagnostics,
    })
}

pub(super) fn decode_recorded_legacy(
    object: &Map<String, Value>,
    version: u8,
    format: SongFormat,
    name: String,
) -> Result<CanonicalSong, SongDecodeError> {
    let (source_game, source_app_name) = decode_source_game(object, false)?;
    let source_pitch = decode_source_pitch(object, false)?;
    let song_pitch_offset =
        pitch_index(&source_pitch).expect("decoded source pitch is valid") as i32;
    let bpm = decode_bpm(object, false)?;
    let tracks = match object.get("instruments") {
        None => vec![source_track(
            source_game,
            tables::default_instrument(source_game),
            Some(source_pitch.clone()),
            song_pitch_offset,
            0,
        )?],
        Some(value) => {
            let raw_tracks = value
                .as_array()
                .ok_or_else(|| invalid("instruments", "must be an array"))?;
            check_limit("source tracks", raw_tracks.len(), MAX_SOURCE_TRACKS)?;
            raw_tracks
                .iter()
                .enumerate()
                .map(|(index, raw)| {
                    let path = format!("instruments[{index}]");
                    let metadata = raw
                        .as_object()
                        .ok_or_else(|| invalid(&path, "must be an instrument object"))?;
                    let instrument = required_string(metadata, "name", &path)?;
                    let track_pitch =
                        optional_pitch(metadata.get("pitch"), song_pitch_offset, &path)?;
                    source_track(
                        source_game,
                        instrument,
                        Some(track_pitch.0),
                        track_pitch.1,
                        index,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    let notes = required_array(object, "notes", "notes")?;
    check_limit("source notes", notes.len(), MAX_SOURCE_NOTES)?;
    let mut events = Vec::with_capacity(notes.len());
    let mut empty_layer_mask_count = 0;
    let mut warnings = Vec::new();
    let mut expanded_count = 0usize;
    for (source_index, raw_note) in notes.iter().enumerate() {
        let path = format!("notes[{source_index}]");
        let note = raw_note
            .as_array()
            .filter(|note| (2..=3).contains(&note.len()))
            .ok_or_else(|| invalid(&path, "must be [legacyIndex, timeMs, layerMask?]"))?;
        let index = nonnegative_index(&note[0], &format!("{path}[0]"))?;
        let onset_us = milliseconds_to_microseconds(&note[1], &format!("{path}[1]"))?;
        let mask = if version == 1 {
            recorded_v1_layer_mask(note.get(2), &format!("{path}[2]"))?
        } else {
            hex_layer_mask(
                note.get(2).ok_or_else(|| SongDecodeError::MissingField {
                    path: format!("{path}[2]"),
                })?,
                &format!("{path}[2]"),
            )?
        };
        if mask == 0 {
            empty_layer_mask_count += 1;
            warnings.push(DecodeWarning::EmptyLayerMask { source_index });
            continue;
        }
        let highest_track = highest_track(mask);
        if highest_track > tracks.len() {
            return Err(invalid(
                format!("{path}[2]"),
                format!(
                    "layer mask selects track {}, but only {} instrument tracks exist",
                    highest_track - 1,
                    tracks.len()
                ),
            ));
        }
        expanded_count = expanded_count
            .checked_add(mask.count_ones() as usize)
            .ok_or_else(|| resource_limit("expanded source notes", usize::MAX, MAX_SOURCE_NOTES))?;
        check_limit("expanded source notes", expanded_count, MAX_SOURCE_NOTES)?;
        for (track_index, track) in tracks.iter().enumerate().take(highest_track) {
            if mask & (1u16 << track_index) == 0 {
                continue;
            }
            let instrument = track
                .instrument
                .as_deref()
                .unwrap_or_else(|| tables::default_instrument(source_game));
            let nominal = tables::legacy_index_to_nominal(source_game, instrument, index)
                .ok_or_else(|| {
                    invalid(
                        format!("{path}[0]"),
                        format!("legacy index {index} is outside the frozen {source_game:?}/{instrument} table"),
                    )
                })?;
            let pitch_offset = track_pitch_offset(track, &source_pitch);
            let (number, source_pitchability) =
                tables::nominal_to_absolute(source_game, instrument, nominal, pitch_offset)
                    .ok_or_else(|| unsupported_instrument(source_game, instrument))?;
            events.push(CanonicalEvent {
                source_index,
                source_position: SourcePosition::RecordedNote {
                    note_index: source_index,
                },
                track_index,
                onset_us,
                duration_us: 0,
                raw_identity: RawNoteIdentity::LegacyIndex(index as u32),
                pitch: CanonicalPitch::NoteNumber {
                    number,
                    source_pitchability,
                },
                layer_mask: Some(mask),
                span_columns: None,
            });
        }
    }
    events.sort_by_key(|event| event.onset_us);
    let diagnostics = diagnostics(
        DiagnosticCounts {
            source_note_count: notes.len(),
            source_track_count: tracks.len(),
            source_column_count: 0,
            empty_layer_mask_count,
            potential_physical_key_merge_count: 0,
        },
        &events,
        warnings,
    );
    Ok(CanonicalSong {
        name,
        format,
        source_game,
        source_app_name,
        source_pitch,
        bpm,
        tracks,
        events,
        diagnostics,
    })
}
