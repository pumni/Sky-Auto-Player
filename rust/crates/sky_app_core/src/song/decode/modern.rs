use super::{
    CanonicalEvent, CanonicalPitch, CanonicalSong, DiagnosticCounts, MAX_SOURCE_COLUMNS,
    MAX_SOURCE_NOTES, MAX_SOURCE_TRACKS, PITCH_NAMES, RawNoteIdentity, SongDecodeError, SongFormat,
    SourceGame, SourcePosition, SourceTrack, check_limit, composed_boundaries, decode_bpm,
    decode_source_game, decode_source_pitch, diagnostics, integer_i32, invalid,
    milliseconds_to_microseconds, nonnegative_index, optional_pitch, pitch_index, required_array,
    required_string, required_value, resource_limit, tables, tempo_id, track_pitch_offset,
    unsupported_instrument,
};
use serde_json::{Map, Value};

pub(super) fn decode_composed_tracks(
    object: &Map<String, Value>,
    format: SongFormat,
    name: String,
) -> Result<CanonicalSong, SongDecodeError> {
    let SongFormat::Composed { version } = format else {
        unreachable!("composed decoder requires composed format")
    };
    let (source_game, source_app_name) = decode_source_game(object, false)?;
    let source_pitch = decode_source_pitch(object, false)?;
    let song_pitch_offset =
        pitch_index(&source_pitch).expect("decoded source pitch is valid") as i32;
    let bpm = decode_bpm(object, true)?.expect("required BPM is present");
    let column_tempo_values = required_array(object, "columnTempos", "columnTempos")?;
    check_limit("columns", column_tempo_values.len(), MAX_SOURCE_COLUMNS)?;
    let mut tempo_ids = Vec::with_capacity(column_tempo_values.len());
    for (index, raw_tempo) in column_tempo_values.iter().enumerate() {
        tempo_ids.push(tempo_id(raw_tempo, &format!("columnTempos[{index}]"))?);
    }
    let boundaries = composed_boundaries(&tempo_ids, bpm)?;
    let raw_tracks = required_array(object, "tracks", "tracks")?;
    check_limit("source tracks", raw_tracks.len(), MAX_SOURCE_TRACKS)?;
    let mut tracks = Vec::with_capacity(raw_tracks.len());
    let mut note_lists = Vec::with_capacity(raw_tracks.len());
    let mut source_note_count = 0usize;
    for (track_index, raw_track) in raw_tracks.iter().enumerate() {
        let path = format!("tracks[{track_index}]");
        let track_object = raw_track
            .as_object()
            .ok_or_else(|| invalid(&path, "must be an object"))?;
        let instrument_value = required_value(track_object, "instrument", &path)?;
        let (instrument_name, track_pitch_offset) = parse_modern_instrument(
            instrument_value,
            source_game,
            song_pitch_offset,
            &format!("{path}.instrument"),
        )?;
        tracks.push(SourceTrack {
            index: track_index,
            instrument: Some(instrument_name),
            effective_pitch: Some(PITCH_NAMES[track_pitch_offset as usize].into()),
        });
        let notes = required_array(track_object, "notes", &format!("{path}.notes"))?;
        source_note_count = source_note_count
            .checked_add(notes.len())
            .ok_or_else(|| resource_limit("source notes", usize::MAX, MAX_SOURCE_NOTES))?;
        check_limit("source notes", source_note_count, MAX_SOURCE_NOTES)?;
        note_lists.push(notes);
    }

    let mut events = Vec::with_capacity(source_note_count);
    let mut source_index = 0usize;
    for (track_index, notes) in note_lists.iter().enumerate() {
        let track = &tracks[track_index];
        let instrument = track
            .instrument
            .as_deref()
            .unwrap_or_else(|| tables::default_instrument(source_game));
        let pitch_offset = track_pitch_offset(track, &source_pitch);
        for (note_index, raw_note) in notes.iter().enumerate() {
            let path = format!("tracks[{track_index}].notes[{note_index}]");
            let note = raw_note
                .as_array()
                .filter(|note| (2..=3).contains(&note.len()))
                .ok_or_else(|| invalid(&path, "must be [columnIndex, noteNumber, span?]"))?;
            let column_index = nonnegative_index(&note[0], &format!("{path}[0]"))?;
            let stored_number = integer_i32(&note[1], &format!("{path}[1]"))?;
            let span = if let Some(raw_span) = note.get(2) {
                let value = nonnegative_index(raw_span, &format!("{path}[2]"))?;
                if value == 0 {
                    return Err(invalid(
                        format!("{path}[2]"),
                        "span must be at least one column",
                    ));
                }
                value
            } else {
                1
            };
            if column_index >= tempo_ids.len() {
                return Err(invalid(
                    format!("{path}[0]"),
                    format!(
                        "column index {column_index} is outside {} columns",
                        tempo_ids.len()
                    ),
                ));
            }
            let end_column = column_index
                .checked_add(span)
                .ok_or_else(|| invalid(format!("{path}[2]"), "span end overflowed"))?;
            if end_column > tempo_ids.len() {
                return Err(invalid(
                    format!("{path}[2]"),
                    format!(
                        "span ends at column {end_column}, beyond {} columns",
                        tempo_ids.len()
                    ),
                ));
            }
            let onset_us = boundaries[column_index];
            let duration_us = if span == 1 {
                0
            } else {
                boundaries[end_column]
                    .checked_sub(onset_us)
                    .ok_or_else(|| SongDecodeError::TimeOverflow { path: path.clone() })?
            };
            let (number, source_pitchability, identity) = if version == 4 {
                let (number, pitchability) = tables::nominal_to_absolute(
                    source_game,
                    instrument,
                    stored_number,
                    pitch_offset,
                )
                .ok_or_else(|| unsupported_instrument(source_game, instrument))?;
                (
                    number,
                    pitchability,
                    RawNoteIdentity::NominalId(stored_number),
                )
            } else {
                let pitchability = tables::absolute_note_pitchability(
                    source_game,
                    instrument,
                    stored_number,
                    pitch_offset,
                )
                .ok_or_else(|| unsupported_instrument(source_game, instrument))?;
                (
                    stored_number,
                    pitchability,
                    RawNoteIdentity::AbsoluteNoteNumber(stored_number),
                )
            };
            events.push(CanonicalEvent {
                source_index,
                source_position: SourcePosition::TrackNote {
                    track_index,
                    note_index,
                    column_index: Some(column_index),
                },
                track_index,
                onset_us,
                duration_us,
                raw_identity: identity,
                pitch: CanonicalPitch::NoteNumber {
                    number,
                    source_pitchability,
                },
                layer_mask: None,
                span_columns: Some(span),
            });
            source_index += 1;
        }
    }
    events.sort_by_key(|event| event.onset_us);
    let diagnostics = diagnostics(
        DiagnosticCounts {
            source_note_count,
            source_track_count: tracks.len(),
            source_column_count: tempo_ids.len(),
            empty_layer_mask_count: 0,
            potential_physical_key_merge_count: 0,
        },
        &events,
        Vec::new(),
    );
    Ok(CanonicalSong {
        name,
        format,
        source_game,
        source_app_name,
        source_pitch,
        bpm: Some(bpm),
        tracks,
        events,
        diagnostics,
    })
}

pub(super) fn decode_recorded_tracks(
    object: &Map<String, Value>,
    format: SongFormat,
    name: String,
) -> Result<CanonicalSong, SongDecodeError> {
    let SongFormat::Recorded { version } = format else {
        unreachable!("recorded decoder requires recorded format")
    };
    let (source_game, source_app_name) = decode_source_game(object, false)?;
    let source_pitch = decode_source_pitch(object, false)?;
    let song_pitch_offset =
        pitch_index(&source_pitch).expect("decoded source pitch is valid") as i32;
    let bpm = decode_bpm(object, false)?;
    let raw_tracks = required_array(object, "tracks", "tracks")?;
    check_limit("source tracks", raw_tracks.len(), MAX_SOURCE_TRACKS)?;
    let mut tracks = Vec::with_capacity(raw_tracks.len());
    let mut note_lists = Vec::with_capacity(raw_tracks.len());
    let mut source_note_count = 0usize;
    for (track_index, raw_track) in raw_tracks.iter().enumerate() {
        let path = format!("tracks[{track_index}]");
        let track_object = raw_track
            .as_object()
            .ok_or_else(|| invalid(&path, "must be an object"))?;
        let instrument_value = required_value(track_object, "instrument", &path)?;
        let (instrument_name, track_pitch_offset) = parse_modern_instrument(
            instrument_value,
            source_game,
            song_pitch_offset,
            &format!("{path}.instrument"),
        )?;
        tracks.push(SourceTrack {
            index: track_index,
            instrument: Some(instrument_name),
            effective_pitch: Some(PITCH_NAMES[track_pitch_offset as usize].into()),
        });
        let notes = required_array(track_object, "notes", &format!("{path}.notes"))?;
        source_note_count = source_note_count
            .checked_add(notes.len())
            .ok_or_else(|| resource_limit("source notes", usize::MAX, MAX_SOURCE_NOTES))?;
        check_limit("source notes", source_note_count, MAX_SOURCE_NOTES)?;
        note_lists.push(notes);
    }
    let mut events = Vec::with_capacity(source_note_count);
    let mut source_index = 0usize;
    for (track_index, notes) in note_lists.iter().enumerate() {
        let track = &tracks[track_index];
        let instrument = track
            .instrument
            .as_deref()
            .unwrap_or_else(|| tables::default_instrument(source_game));
        let pitch_offset = track_pitch_offset(track, &source_pitch);
        for (note_index, raw_note) in notes.iter().enumerate() {
            let path = format!("tracks[{track_index}].notes[{note_index}]");
            let note = raw_note
                .as_array()
                .filter(|note| (2..=3).contains(&note.len()))
                .ok_or_else(|| invalid(&path, "must be [noteNumber, timeMs, durationMs?]"))?;
            let stored_number = integer_i32(&note[0], &format!("{path}[0]"))?;
            let onset_us = milliseconds_to_microseconds(&note[1], &format!("{path}[1]"))?;
            let duration_us = if let Some(duration) = note.get(2) {
                milliseconds_to_microseconds(duration, &format!("{path}[2]"))?
            } else {
                0
            };
            onset_us
                .checked_add(duration_us)
                .ok_or_else(|| SongDecodeError::TimeOverflow { path: path.clone() })?;
            let (number, source_pitchability, identity) = if version == 3 {
                let (number, pitchability) = tables::nominal_to_absolute(
                    source_game,
                    instrument,
                    stored_number,
                    pitch_offset,
                )
                .ok_or_else(|| unsupported_instrument(source_game, instrument))?;
                (
                    number,
                    pitchability,
                    RawNoteIdentity::NominalId(stored_number),
                )
            } else {
                let pitchability = tables::absolute_note_pitchability(
                    source_game,
                    instrument,
                    stored_number,
                    pitch_offset,
                )
                .ok_or_else(|| unsupported_instrument(source_game, instrument))?;
                (
                    stored_number,
                    pitchability,
                    RawNoteIdentity::AbsoluteNoteNumber(stored_number),
                )
            };
            events.push(CanonicalEvent {
                source_index,
                source_position: SourcePosition::TrackNote {
                    track_index,
                    note_index,
                    column_index: None,
                },
                track_index,
                onset_us,
                duration_us,
                raw_identity: identity,
                pitch: CanonicalPitch::NoteNumber {
                    number,
                    source_pitchability,
                },
                layer_mask: None,
                span_columns: None,
            });
            source_index += 1;
        }
    }
    events.sort_by_key(|event| event.onset_us);
    let diagnostics = diagnostics(
        DiagnosticCounts {
            source_note_count,
            source_track_count: tracks.len(),
            source_column_count: 0,
            empty_layer_mask_count: 0,
            potential_physical_key_merge_count: 0,
        },
        &events,
        Vec::new(),
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

fn parse_modern_instrument(
    value: &Value,
    game: SourceGame,
    song_pitch_offset: i32,
    path: &str,
) -> Result<(String, i32), SongDecodeError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(path, "must be an instrument object"))?;
    let instrument = required_string(object, "name", path)?;
    if !tables::instrument_supported(game, instrument) {
        return Err(unsupported_instrument(game, instrument));
    }
    let (_, pitch_offset) = optional_pitch(object.get("pitch"), song_pitch_offset, path)?;
    Ok((instrument.into(), pitch_offset))
}
