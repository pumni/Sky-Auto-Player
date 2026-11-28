use super::*;

#[cfg(test)]
pub(in crate::native_runtime::playback) fn compile_dispatch_schedule(
    schedule: &ScheduleMetadata,
    cancelled: &AtomicBool,
) -> Result<sky_player::adapter_support::OnsetPlan, String> {
    compile_dispatch_schedule_with_checkpoint(schedule, 0, cancelled, |_| {})
}

pub(super) fn compile_dispatch_schedule_with_checkpoint(
    schedule: &ScheduleMetadata,
    caller_retained_capacity_bytes: usize,
    cancelled: &AtomicBool,
    mut checkpoint: impl FnMut(usize),
) -> Result<sky_player::adapter_support::OnsetPlan, String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("native schedule compilation cancelled".into());
    }
    if schedule.chords.len() > sky_player::adapter_support::MAX_CHORDS {
        return Err("native schedule exceeds onset chord limit".into());
    }
    let caller_retained_capacity_bytes = caller_retained_capacity_bytes
        .checked_add(schedule_metadata_heap_capacity_bytes(schedule)?)
        .ok_or_else(|| "native caller-retained startup capacity overflow".to_string())?;
    let estimated_spilled_scan_bytes: usize = schedule
        .chords
        .iter()
        .filter(|chord| chord.scan_codes.len() > 4)
        .try_fold(0usize, |total, chord| {
            checked_capacity_bytes(chord.scan_codes.len(), std::mem::size_of::<u16>())
                .and_then(|bytes| checked_capacity_sum([total, bytes]))
        })?;
    let estimated_input_bytes = checked_capacity_sum([
        checked_capacity_bytes(
            schedule.chords.len(),
            std::mem::size_of::<OnsetChordInput>(),
        )?,
        estimated_spilled_scan_bytes,
    ])?;
    let preflight_bytes =
        checked_capacity_sum([caller_retained_capacity_bytes, estimated_input_bytes])?;
    if preflight_bytes > sky_player::adapter_support::MAX_RETAINED_DISPATCH_BYTES {
        return Err(format!(
            "native onset chord input preflight ({preflight_bytes} bytes) exceeds the {} byte retained-dispatch budget",
            sky_player::adapter_support::MAX_RETAINED_DISPATCH_BYTES
        ));
    }
    checkpoint(0);
    let mut chords = Vec::new();
    chords
        .try_reserve(schedule.chords.len())
        .map_err(|error| format!("native onset chord allocation failed: {error}"))?;
    for (index, chord) in schedule.chords.iter().enumerate() {
        if index.is_multiple_of(256) && cancelled.load(Ordering::Acquire) {
            return Err("native schedule compilation cancelled".into());
        }
        if chord.scan_codes.is_empty()
            || chord.scan_codes.len() > sky_app_core::song::SKY_SCAN_CODES.len()
        {
            return Err(format!("native chord {index} has invalid scan code count"));
        }
        let mut scan_codes = SmallVec::<[u16; 4]>::new();
        scan_codes
            .try_reserve(chord.scan_codes.len())
            .map_err(|error| format!("native scan code allocation failed: {error}"))?;
        scan_codes.extend_from_slice(&chord.scan_codes);
        chords.push(OnsetChordInput {
            source_action_index: u32::try_from(index).map_err(|_| "native chord index overflow")?,
            scheduled_us: chord.at_us,
            scan_codes,
        });
        if (index + 1).is_multiple_of(256) {
            checkpoint(index + 1);
        }
    }
    // `chords` (and any spilled SmallVec scan-code storage) is temporary to this
    // compile call and is counted once by `build_onset_plan_with_memory_accounting`
    // in `input_capacity_bytes`. `caller_retained_capacity_bytes` excludes those
    // temporary inputs so they are neither double-counted during build nor carried
    // into subsequent QPC materialization after `chords` is dropped.
    sky_player::adapter_support::build_onset_plan_with_memory_accounting(
        &chords,
        &sky_app_core::song::SKY_SCAN_CODES,
        schedule.hold_us,
        chords.capacity(),
        caller_retained_capacity_bytes,
        || cancelled.load(Ordering::Acquire),
    )
    .map_err(|error| format!("native onset plan construction failed: {error}"))
}

pub(super) fn compile_dispatch_schedule_with_prepared_capacity(
    schedule: &ScheduleMetadata,
    caller_retained_capacity_bytes: usize,
    cancelled: &AtomicBool,
) -> Result<sky_player::adapter_support::OnsetPlan, String> {
    compile_dispatch_schedule_with_checkpoint(
        schedule,
        caller_retained_capacity_bytes,
        cancelled,
        |_| {},
    )
}

fn checked_capacity_bytes(count: usize, element_size: usize) -> Result<usize, String> {
    count
        .checked_mul(element_size)
        .ok_or_else(|| "native startup capacity size overflow".to_string())
}

pub(super) fn checked_capacity_sum(
    values: impl IntoIterator<Item = usize>,
) -> Result<usize, String> {
    values.into_iter().try_fold(0usize, |total, value| {
        total
            .checked_add(value)
            .ok_or_else(|| "native startup capacity sum overflow".to_string())
    })
}

pub(super) fn runtime_schedule_capacity_bytes(
    schedule: &sky_player::adapter_support::OnsetPlan,
    cancelled: &AtomicBool,
) -> Result<usize, String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("onset plan capacity accounting cancelled".to_string());
    }
    let memory = schedule.memory_metrics()?;
    memory
        .heap_payload_bytes
        .checked_add(memory.struct_bytes)
        .ok_or_else(|| "onset plan capacity overflow".to_string())
}

/// Heap payload retained by one source schedule. Struct headers are counted
/// by the containing prepared variant; allocator metadata is excluded.
pub(super) fn schedule_metadata_heap_capacity_bytes(
    schedule: &ScheduleMetadata,
) -> Result<usize, String> {
    let chord_headers = checked_capacity_bytes(
        schedule.chords.capacity(),
        std::mem::size_of::<sky_app_core::song::OnsetChord>(),
    )?;
    let chord_payloads = schedule.chords.iter().try_fold(0usize, |total, chord| {
        let scan_codes =
            checked_capacity_bytes(chord.scan_codes.capacity(), std::mem::size_of::<u16>())?;
        total
            .checked_add(scan_codes)
            .ok_or_else(|| "native chord source capacity overflow".to_string())
    })?;
    let warning_headers =
        checked_capacity_bytes(schedule.warnings.capacity(), std::mem::size_of::<String>())?;
    let warning_payloads = schedule
        .warnings
        .iter()
        .try_fold(0usize, |total, warning| {
            total
                .checked_add(warning.capacity())
                .ok_or_else(|| "native warning source capacity overflow".to_string())
        })?;
    checked_capacity_sum([
        chord_headers,
        chord_payloads,
        warning_headers,
        warning_payloads,
    ])
}

pub(super) struct NativePrepareCapacityInput<'a> {
    pub(super) song: &'a sky_app_core::song::Song,
    pub(super) song_id: &'a String,
    pub(super) settings_fingerprint: &'a String,
    pub(super) publication_fingerprint: &'a String,
    pub(super) source_path: &'a PathBuf,
    pub(super) source_hash: &'a String,
    pub(super) plan_identity: &'a String,
}

pub(super) fn native_prepare_caller_retained_capacity_bytes(
    input: NativePrepareCapacityInput<'_>,
    other_prepared_capacity_bytes: usize,
) -> Result<usize, String> {
    let note_headers = checked_capacity_bytes(
        input.song.notes.capacity(),
        std::mem::size_of::<sky_app_core::song::Note>(),
    )?;
    let note_keys = input.song.notes.iter().try_fold(0usize, |total, note| {
        total
            .checked_add(note.key.capacity())
            .ok_or_else(|| "prepared song-key capacity overflow".to_string())
    })?;
    checked_capacity_sum([
        input.song.name.capacity(),
        note_headers,
        note_keys,
        input.song_id.capacity(),
        input.settings_fingerprint.capacity(),
        input.publication_fingerprint.capacity(),
        input.source_path.capacity(),
        input.source_hash.capacity(),
        input.plan_identity.capacity(),
        other_prepared_capacity_bytes,
    ])
}

fn optional_string_capacity(value: Option<&String>) -> usize {
    value.map_or(0, String::capacity)
}

fn string_vec_capacity_bytes(values: &[String], capacity: usize) -> Result<usize, String> {
    let headers = checked_capacity_bytes(capacity, std::mem::size_of::<String>())?;
    let payloads = values.iter().try_fold(0usize, |total, value| {
        total
            .checked_add(value.capacity())
            .ok_or_else(|| "prepared string-vector capacity overflow".to_string())
    })?;
    checked_capacity_sum([headers, payloads])
}

fn risk_summary_capacity_bytes(risk: &crate::commands::RiskSummaryDto) -> Result<usize, String> {
    checked_capacity_sum([
        risk.level.capacity(),
        risk.headline.capacity(),
        string_vec_capacity_bytes(&risk.reasons, risk.reasons.capacity())?,
        string_vec_capacity_bytes(&risk.recommendations, risk.recommendations.capacity())?,
    ])
}

fn sheet_analysis_capacity_bytes(
    analysis: Option<&crate::commands::SheetAnalysisDto>,
) -> Result<usize, String> {
    let Some(analysis) = analysis else {
        return Ok(0);
    };
    let track_headers = checked_capacity_bytes(
        analysis.tracks.capacity(),
        std::mem::size_of::<crate::commands::SheetTrackDto>(),
    )?;
    let track_strings = analysis.tracks.iter().try_fold(0usize, |total, track| {
        total
            .checked_add(optional_string_capacity(track.instrument.as_ref()))
            .and_then(|total| {
                total.checked_add(optional_string_capacity(track.effective_pitch.as_ref()))
            })
            .ok_or_else(|| "prepared sheet-track capacity overflow".to_string())
    })?;
    checked_capacity_sum([
        analysis.format_label.capacity(),
        track_headers,
        track_strings,
        string_vec_capacity_bytes(&analysis.loss_reasons, analysis.loss_reasons.capacity())?,
        optional_string_capacity(analysis.error_message.as_ref()),
        optional_string_capacity(analysis.confirmation_fingerprint.as_ref()),
    ])
}

fn generation_timing_advisory_capacity_bytes(
    advisory: Option<&crate::commands::GenerationTimingAdvisoryDto>,
) -> Result<usize, String> {
    let Some(advisory) = advisory else {
        return Ok(0);
    };
    checked_capacity_sum([
        advisory.plan_fingerprint.capacity(),
        advisory.explanation.capacity(),
    ])
}

fn prepared_dto_capacity_bytes(
    dto: &crate::commands::PreparedPlaybackDto,
) -> Result<usize, String> {
    let song = &dto.song;
    let mut total = checked_capacity_sum([
        optional_string_capacity(dto.prepared_id.as_ref()),
        song.song_id.capacity(),
        song.title.capacity(),
        song.format_label.capacity(),
        risk_summary_capacity_bytes(&song.risk)?,
        sheet_analysis_capacity_bytes(song.sheet_analysis.as_ref())?,
        risk_summary_capacity_bytes(&dto.risk)?,
        checked_capacity_bytes(
            dto.decisions.capacity(),
            std::mem::size_of::<crate::commands::RiskDecisionDto>(),
        )?,
        optional_string_capacity(dto.plan_fingerprint.as_ref()),
        checked_capacity_bytes(
            dto.variants.capacity(),
            std::mem::size_of::<crate::commands::PlaybackPlanVariantDto>(),
        )?,
        optional_string_capacity(dto.error_code.as_ref()),
        optional_string_capacity(dto.error_message.as_ref()),
    ])?;
    if let Some(recommendation) = &song.recommendation {
        total = total
            .checked_add(recommendation.summary.capacity())
            .ok_or_else(|| "prepared recommendation capacity overflow".to_string())?;
    }
    for decision in &dto.decisions {
        total = total
            .checked_add(decision.label.capacity())
            .ok_or_else(|| "prepared decision capacity overflow".to_string())?;
    }
    for variant in &dto.variants {
        let advisory_bytes =
            generation_timing_advisory_capacity_bytes(variant.timing_advisory.as_ref())?;
        total = checked_capacity_sum([total, variant.plan_fingerprint.capacity(), advisory_bytes])?;
    }
    Ok(total)
}

pub(super) fn prepared_plan_capacity_bytes(
    id: &String,
    record: &NativePreparedPlan,
) -> Result<usize, String> {
    let variant_entry_size =
        std::mem::size_of::<(crate::commands::PlaybackDecision, NativePlaybackVariant)>();
    let variant_bucket_count = if record.variants.capacity() == 0 {
        0
    } else {
        record
            .variants
            .capacity()
            .checked_mul(8)
            .and_then(|value| value.checked_add(6))
            .map(|value| value / 7)
            .and_then(usize::checked_next_power_of_two)
            .ok_or_else(|| "prepared variant-map capacity overflow".to_string())?
    };
    let variant_map_bytes = if variant_bucket_count == 0 {
        0
    } else {
        checked_capacity_bytes(variant_bucket_count, variant_entry_size.saturating_add(1))?
            .checked_add(16 + std::mem::align_of::<usize>() - 1)
            .ok_or_else(|| "prepared variant-map bytes overflow".to_string())?
    };
    let mut variant_payloads = 0usize;
    for variant in record.variants.values() {
        variant_payloads = checked_capacity_sum([
            variant_payloads,
            variant.fingerprint.capacity(),
            variant
                .prepared_schedule
                .as_ref()
                .map_or(0, |schedule| schedule.memory_metrics().plan_capacity_bytes),
        ])?;
    }
    checked_capacity_sum([
        id.capacity(),
        record.song_id.capacity(),
        record.settings_fingerprint.capacity(),
        record.song_title.capacity(),
        record.source_path.capacity(),
        record.source_hash.capacity(),
        variant_map_bytes,
        variant_payloads,
        prepared_dto_capacity_bytes(&record.dto)?,
    ])
}

pub(super) fn prepared_cache_retained_capacity_bytes(
    prepared: &std::collections::VecDeque<(String, NativePreparedPlan)>,
    excluded_index: Option<usize>,
) -> Result<usize, String> {
    let queue_buffer_bytes = checked_capacity_bytes(
        prepared.capacity(),
        std::mem::size_of::<(String, NativePreparedPlan)>(),
    )?;
    prepared
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != excluded_index)
        .try_fold(0usize, |total, (_, (_id, record))| {
            let plan_bytes = record.retained_capacity_bytes;
            total
                .checked_add(plan_bytes)
                .ok_or_else(|| "prepared-cache retained capacity overflow".to_string())
        })
        .and_then(|record_bytes| {
            queue_buffer_bytes
                .checked_add(record_bytes)
                .ok_or_else(|| "prepared-cache queue capacity overflow".to_string())
        })
}

pub(in crate::native_runtime) fn retain_prepared_capacity<T>(prepared: &mut VecDeque<T>) {
    while prepared.len() > MAX_PREPARED_PLANS {
        prepared.pop_front();
    }
}
