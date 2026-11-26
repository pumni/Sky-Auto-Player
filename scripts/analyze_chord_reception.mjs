#!/usr/bin/env node
/**
 * Strict Offline Chord & Receiver Timing Analyzer (R2-P / C1).
 *
 * Zero-dependency Node.js module + CLI for `chord-width-pilot-v1` qualification
 * evidence. Consumes the ACTUAL existing case evidence files:
 *   - `--report`           : `rt-native-acceptance.jsonl` (1 JSONL line from `evidence_report.rs::write_report`, `status: "PASS"`, flat `details`)
 *   - `--invocation`       : `invocation.json` (from `scripts/run_native_acceptance_case.ps1`)
 *   - `--sink-ready`       : `sink-ready.json` (schema 3, `SchedulerQueuedV3Compat3`)
 *   - `--sink-events`      : `sink-events.jsonl` (event schema 3 with `observed_qpc_ticks`)
 *   - `--observer-summary` : `sink-observer-summary.json` (finalized schema 1)
 *   - `--width`            : `1`, `5`, or `15`
 *   - Provenance binding (MANDATORY): either `--manifest <chord-width-pilot-v1-manifest.json>`
 *     or all three explicit `--expected-source-sha`, `--expected-source-tree`, `--expected-binary-sha256`
 *
 * Invalid runs ALWAYS return `valid: false`, `accepted_for_aggregation: false`, `metrics: null`
 * and exit with code 1.
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const CANONICAL_SCAN_CODES = Object.freeze([
  0x15, 0x16, 0x17, 0x18, 0x19, // Y U I O P (slots 0..4)
  0x23, 0x24, 0x25, 0x26, 0x27, // H J K L ; (slots 5..9)
  0x31, 0x32, 0x33, 0x34, 0x35, // N M , . / (slots 10..14)
]);

export const EXPECTED_SENDER_TELEMETRY_SCHEMA = 28;
export const EXPECTED_RECEIVER_OBSERVER_MODE = 'SchedulerQueuedV3Compat3';
export const EXPECTED_RECEIVER_OBSERVER_VERSION = 'scheduler-receiveonly-queue-v3-compat3';
export const EXPECTED_RECEIVER_EVENT_SCHEMA = 3;
export const EXPECTED_RECEIVER_READY_SCHEMA = 3;
export const EXPECTED_RECEIVER_SUMMARY_SCHEMA = 1;
export const EXPECTED_RECEIVER_QUEUE_CAPACITY = 32768;
export const TRACE_FLAG_RECOVERY = 1 << 1;

export const PILOT_ONSET_COUNT = 150;
export const PILOT_FIRST_OFFSET_US = 50_000;
export const PILOT_HOLD_US = 17_167;
export const PILOT_CADENCE_US = 100_000;
export const PILOT_GAME_FPS = 60;
export const PILOT_TIMING_MARGIN_US = 500;
export const PILOT_SAMPLE_SECONDS = 15;
export const PILOT_ALLOWED_WIDTHS = Object.freeze([1, 5, 15]);

export function pilotRotatingSlots(onsetIndex, width) {
  const start = (onsetIndex * width) % 15;
  const slots = [];
  for (let k = 0; k < width; k++) slots.push((start + k) % 15);
  slots.sort((a, b) => a - b);
  return slots;
}

export function slotsToMask(slots) {
  let mask = 0;
  for (const s of slots) mask |= 1 << s;
  return mask;
}

/**
 * Extracts an exact unsigned integer field from raw JSON text BEFORE JSON.parse
 * can lose precision on 64-bit integers (> Number.MAX_SAFE_INTEGER, e.g. Windows FILETIME).
 */
export function extractExactIntegerField(rawJson, fieldName) {
  if (typeof rawJson !== 'string') return null;
  const escaped = fieldName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const re = new RegExp(`"${escaped}"\\s*:\\s*(?:"(\\d+)"|(\\d+))(?=\\s*[,}\\]])`, 'g');
  const matches = [];
  let m;
  while ((m = re.exec(rawJson)) !== null) {
    matches.push(m[1] ?? m[2]);
  }
  if (matches.length === 0) return null;
  // Require all occurrences (if any) to be identical so duplicate keys cannot smuggle values.
  const first = matches[0];
  for (const v of matches) {
    if (v !== first) return null;
  }
  return first;
}

function toBigIntStrict(value, label) {
  if (typeof value === 'bigint') return value;
  if (typeof value === 'number' && Number.isSafeInteger(value)) return BigInt(value);
  if (typeof value === 'string' && /^-?\d+$/.test(value)) return BigInt(value);
  throw new Error(`malformed integer for ${label}: ${String(value)}`);
}

function toExactPositiveBigInt(value) {
  if (typeof value === 'bigint') return value > 0n ? value : null;
  if (typeof value === 'string' && /^\d+$/.test(value)) {
    const b = BigInt(value);
    return b > 0n ? b : null;
  }
  if (typeof value === 'number' && Number.isSafeInteger(value) && value > 0) {
    return BigInt(value);
  }
  return null;
}

function usToTicksExactOrCeil(us, freqHz) {
  const num = BigInt(us) * freqHz;
  return (num + 999_999n) / 1_000_000n;
}

function usToTicksFloor(us, freqHz) {
  return (BigInt(us) * freqHz) / 1_000_000n;
}

function ticksToUs(deltaTicks, freqHz) {
  return (Number(deltaTicks) * 1_000_000) / Number(freqHz);
}

/**
 * Nearest-rank quantile convention (matches native_acceptance_sink.ps1:311):
 * index = Math.max(0, Math.ceil(q * n) - 1) on ascending sorted values.
 */
export function summarizeTicks(samplesBigInt, freqHz, unavailableStatus = 'NOT_MEASURABLE') {
  const n = samplesBigInt.length;
  if (n === 0) {
    return {
      status: unavailableStatus,
      n: 0,
      quantile_convention: 'nearest_rank_ceil',
      ticks: null,
      us: null,
    };
  }
  const sorted = [...samplesBigInt].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  const at = (q) => sorted[Math.max(0, Math.ceil(q * n) - 1)];
  return {
    status: 'MEASURED',
    n,
    quantile_convention: 'nearest_rank_ceil',
    ticks: {
      min: sorted[0].toString(),
      p50: at(0.5).toString(),
      p95: at(0.95).toString(),
      p99: at(0.99).toString(),
      max: sorted[n - 1].toString(),
    },
    us: {
      min: Number(ticksToUs(sorted[0], freqHz).toFixed(3)),
      p50: Number(ticksToUs(at(0.5), freqHz).toFixed(3)),
      p95: Number(ticksToUs(at(0.95), freqHz).toFixed(3)),
      p99: Number(ticksToUs(at(0.99), freqHz).toFixed(3)),
      max: Number(ticksToUs(sorted[n - 1], freqHz).toFixed(3)),
    },
  };
}

function expandMaskAscending(mask, profile) {
  const slots = [];
  for (let slot = 0; slot < 15; slot++) {
    if ((mask & (1 << slot)) !== 0) {
      if (slot >= profile.length) {
        throw new Error(`slot ${slot} exceeds validated physical profile length ${profile.length}`);
      }
      slots.push({
        slot,
        scan_code: profile[slot].scan_code,
        extended: Boolean(profile[slot].extended),
      });
    }
  }
  return slots;
}

/**
 * Strict qualification analyzer for `chord-width-pilot-v1` consuming the actual
 * Rust native report (`rt-native-acceptance.jsonl`), case `invocation.json`,
 * `sink-ready.json`, `sink-events.jsonl`, `sink-observer-summary.json`, and
 * mandatory expected source/tree/binary provenance (from approved manifest or
 * explicit expected values).
 */
export function analyzePilotRun({
  acceptanceReport,
  invocation,
  sinkReady,
  receiverJsonlLines,
  observerSummary,
  expectedWidth,
  expectedSourceSha,
  expectedSourceTreeSha,
  expectedBinarySha256,
  sinkReadyStartFiletimeExact = null,
  invocationStartFiletimeExact = null,
  physicalProfile = null,
}) {
  const profile =
    physicalProfile ?? CANONICAL_SCAN_CODES.map((scan_code) => ({ scan_code, extended: false }));

  const invalidReasons = [];
  const invalidCounts = {
    report_unhealthy: 0,
    invocation_invalid: 0,
    identity_mismatch: 0,
    frequency_mismatch: 0,
    schema_or_protocol_mismatch: 0,
    legacy_no_qpc: 0,
    sender_trace_incomplete: 0,
    observer_health_invalid: 0,
    malformed_record: 0,
    header_anomaly: 0,
    duplicate_or_out_of_order_sequence: 0,
    receiver_non_monotonic_qpc: 0,
    receiver_precedes_sender: 0,
    foreground_mismatch: 0,
    wrong_scan_code_or_extended: 0,
    missing_receiver_event: 0,
    extra_receiver_event: 0,
    partial_or_failed_send: 0,
    cleanup_record: 0,
    sender_timestamp_invalid: 0,
    authored_timing_inconsistent: 0,
    hold_floor_violation: 0,
    duplicate_down_or_orphan_up: 0,
    unclosed_generation: 0,
    overlapping_packet_masks: 0,
    unexpected_mixed_or_split_packet: 0,
    pilot_geometry_mismatch: 0,
    up_slot_attribution_mismatch: 0,
  };
  const fail = (bucket, msg) => {
    invalidCounts[bucket]++;
    invalidReasons.push(msg);
  };

  if (!PILOT_ALLOWED_WIDTHS.includes(expectedWidth)) {
    fail('pilot_geometry_mismatch', `unsupported expectedWidth=${expectedWidth} (must be 1, 5, or 15)`);
  }

  // Mandatory provenance expectations (no optional missing-source success)
  if (typeof expectedSourceSha !== 'string' || !/^[0-9a-f]{40}$/.test(expectedSourceSha)) {
    fail('identity_mismatch', 'missing or malformed mandatory expectedSourceSha (40 lowercase hex)');
  }
  if (typeof expectedSourceTreeSha !== 'string' || !/^[0-9a-f]{40}$/.test(expectedSourceTreeSha)) {
    fail('identity_mismatch', 'missing or malformed mandatory expectedSourceTreeSha (40 lowercase hex)');
  }
  if (typeof expectedBinarySha256 !== 'string' || !/^[0-9a-f]{64}$/.test(expectedBinarySha256)) {
    fail('identity_mismatch', 'missing or malformed mandatory expectedBinarySha256 (64 lowercase hex)');
  }

  const expectedPacketCount = PILOT_ONSET_COUNT * 2; // 300 (150 Down + 150 Up)
  const expectedUpSlotCount = PILOT_ONSET_COUNT * (PILOT_ALLOWED_WIDTHS.includes(expectedWidth) ? expectedWidth : 0);
  const expectedReceiverEvents = expectedUpSlotCount * 2; // 300 / 1500 / 4500

  // 1. Validate actual Rust acceptance report (`evidence_report.rs::write_report` + flat `snapshot_json` details)
  const details = acceptanceReport?.details;
  const telemetry = details?.native_timing_telemetry;
  const runId = typeof acceptanceReport?.run_id === 'string' && acceptanceReport.run_id.length > 0
    ? acceptanceReport.run_id
    : null;

  if (!acceptanceReport || typeof acceptanceReport !== 'object' || !details || typeof details !== 'object') {
    fail('report_unhealthy', 'missing acceptance report or details object');
  } else {
    if (acceptanceReport.status !== 'PASS') {
      fail('report_unhealthy', `acceptance report status is '${acceptanceReport.status}', expected 'PASS'`);
    }
    if (!runId) {
      fail('identity_mismatch', 'acceptance report missing non-empty string run_id');
    }
    if (acceptanceReport.scenario !== 'chord-width-pilot-v1' ||
        acceptanceReport.admission_variant !== 'control-a' ||
        acceptanceReport.timing_margin_us !== PILOT_TIMING_MARGIN_US ||
        acceptanceReport.sample_seconds !== PILOT_SAMPLE_SECONDS ||
        acceptanceReport.game_fps !== PILOT_GAME_FPS ||
        acceptanceReport.min_hold_us !== PILOT_HOLD_US) {
      fail(
        'pilot_geometry_mismatch',
        `report top-level pilot settings mismatch: scenario=${acceptanceReport.scenario} variant=${acceptanceReport.admission_variant} margin=${acceptanceReport.timing_margin_us} sample=${acceptanceReport.sample_seconds} fps=${acceptanceReport.game_fps} min_hold=${acceptanceReport.min_hold_us}`,
      );
    }
    if (details.calibration_placement !== 'baseline' ||
        details.timing_observer_for_qualification !== true ||
        details.workload_definition_identity !== 'chord-width-pilot-v1' ||
        details.pilot_chord_width !== expectedWidth ||
        details.pilot_cadence_us !== PILOT_CADENCE_US ||
        details.pilot_onset_count !== PILOT_ONSET_COUNT ||
        details.scenario_hold_us !== PILOT_HOLD_US) {
      fail(
        'pilot_geometry_mismatch',
        `report details pilot settings mismatch: cal=${details.calibration_placement} obs=${details.timing_observer_for_qualification} workload=${details.workload_definition_identity} width=${details.pilot_chord_width}/${expectedWidth} cadence=${details.pilot_cadence_us} onsets=${details.pilot_onset_count} hold=${details.scenario_hold_us}`,
      );
    }
    if (details.native_timing_telemetry_available !== true) {
      fail('sender_trace_incomplete', 'details.native_timing_telemetry_available is not true');
    }
    // Flat snapshot cleanup/health fields emitted by snapshot_report.rs::snapshot_json
    const rel = details.release_outcome;
    if (details.active_count !== 0 ||
        details.possibly_active_count !== 0 ||
        details.failed_release_count !== 0 ||
        details.final_release_obligation_mask !== 0 ||
        details.terminal_error !== null ||
        details.sendinput_partial_events !== 0 ||
        details.sendinput_zero_progress_failures !== 0 ||
        !rel || typeof rel !== 'object' ||
        rel.released_successfully !== true ||
        rel.stuck_mask !== 0 ||
        rel.verification_inconclusive !== false ||
        rel.transport_anomaly !== false) {
      fail('report_unhealthy', 'flat worker snapshot/cleanup fields in report details are missing or unhealthy');
    }
    if (details.expected_sink_event_count !== expectedReceiverEvents ||
        details.observed_sink_event_count !== expectedReceiverEvents ||
        details.sink_event_count !== expectedReceiverEvents) {
      fail(
        'report_unhealthy',
        `report sink event counts mismatch: expected=${details.expected_sink_event_count} observed=${details.observed_sink_event_count} sink=${details.sink_event_count} (want ${expectedReceiverEvents})`,
      );
    }
  }

  // 2. Validate mandatory case invocation.json (`scripts/run_native_acceptance_case.ps1:976-1071`)
  if (!invocation || typeof invocation !== 'object') {
    fail('invocation_invalid', 'missing mandatory case invocation.json');
  } else {
    if (invocation.run_id !== runId ||
        invocation.qualification_profile !== 'ChordWidthPilotV1' ||
        invocation.workload !== 'chord-width-pilot-v1' ||
        invocation.workload_version !== 'chord-width-pilot-v1' ||
        invocation.scenario !== 'chord-width-pilot-v1' ||
        invocation.status !== 'PASS' ||
        invocation.exit_code !== 0 ||
        invocation.report_window_match !== true ||
        invocation.source_tree_clean !== true ||
        invocation.owner_approved_receive_only_run !== true ||
        invocation.calibration_placement !== 'baseline' ||
        invocation.cpu_contention !== false ||
        invocation.timing_margin_us !== PILOT_TIMING_MARGIN_US ||
        invocation.game_fps !== PILOT_GAME_FPS ||
        invocation.sample_seconds !== PILOT_SAMPLE_SECONDS ||
        invocation.configured_H_us !== PILOT_HOLD_US ||
        invocation.receiveonly_observer_mode !== EXPECTED_RECEIVER_OBSERVER_MODE ||
        invocation.receiveonly_observer_version !== EXPECTED_RECEIVER_OBSERVER_VERSION) {
      fail(
        'invocation_invalid',
        `invocation.json contract mismatch: profile=${invocation.qualification_profile} status=${invocation.status} exit=${invocation.exit_code} win=${invocation.report_window_match} clean=${invocation.source_tree_clean} owner=${invocation.owner_approved_receive_only_run}`,
      );
    }
    if (invocation.source_revision !== expectedSourceSha ||
        invocation.source_tree_sha !== expectedSourceTreeSha ||
        invocation.harness_source_revision !== expectedSourceSha ||
        invocation.harness_source_tree_sha !== expectedSourceTreeSha ||
        invocation.harness_source_role !== 'Candidate' ||
        invocation.harness_binary_sha256 !== expectedBinarySha256) {
      fail(
        'identity_mismatch',
        `invocation provenance mismatch: src=${invocation.source_revision}/${invocation.source_tree_sha} harness=${invocation.harness_source_revision}/${invocation.harness_source_tree_sha} role=${invocation.harness_source_role} bin=${invocation.harness_binary_sha256}`,
      );
    }
  }

  // 3. Validate sink ready record & exact lossless FILETIME / target identity across sinkReady + invocation + report
  if (!sinkReady || typeof sinkReady !== 'object') {
    fail('identity_mismatch', 'missing sinkReady record');
  } else {
    if (sinkReady.schema_version !== EXPECTED_RECEIVER_READY_SCHEMA ||
        sinkReady.event_schema_version !== EXPECTED_RECEIVER_EVENT_SCHEMA ||
        sinkReady.observer_mode !== EXPECTED_RECEIVER_OBSERVER_MODE ||
        sinkReady.observer_version !== EXPECTED_RECEIVER_OBSERVER_VERSION ||
        sinkReady.observer_queue_capacity !== EXPECTED_RECEIVER_QUEUE_CAPACITY) {
      fail(
        'schema_or_protocol_mismatch',
        `sinkReady protocol mismatch: schema=${sinkReady.schema_version}/${sinkReady.event_schema_version} mode=${sinkReady.observer_mode} ver=${sinkReady.observer_version} cap=${sinkReady.observer_queue_capacity}`,
      );
    }
    const readyStartFt = toExactPositiveBigInt(sinkReadyStartFiletimeExact ?? sinkReady.process_start_time_filetime);
    const invTarget = invocation?.target_identity;
    const invStartFt = toExactPositiveBigInt(invocationStartFiletimeExact ?? invTarget?.process_start_time_filetime);

    if (sinkReady.role !== 'ReceiveOnly' ||
        sinkReady.game_target !== false ||
        sinkReady.run_id !== runId ||
        typeof sinkReady.event_log_id !== 'string' || sinkReady.event_log_id.length === 0 ||
        !Number.isSafeInteger(sinkReady.pid) || sinkReady.pid <= 0 ||
        !Number.isSafeInteger(sinkReady.hwnd) || sinkReady.hwnd <= 0 ||
        readyStartFt === null) {
      fail('identity_mismatch', 'sinkReady role/game_target/run_id/pid/hwnd/exact_start_time identity invalid');
    }
    if (details && details.sink_event_log_id !== sinkReady.event_log_id) {
      fail('identity_mismatch', `report details.sink_event_log_id (${details.sink_event_log_id}) != sinkReady (${sinkReady.event_log_id})`);
    }
    if (invocation) {
      if (!invTarget || typeof invTarget !== 'object' ||
          invocation.event_log_id !== sinkReady.event_log_id ||
          invTarget.role !== 'ReceiveOnly' ||
          invTarget.game_target !== false ||
          invTarget.pid !== sinkReady.pid ||
          invTarget.hwnd !== sinkReady.hwnd ||
          invTarget.event_log_id !== sinkReady.event_log_id ||
          invStartFt === null ||
          readyStartFt === null ||
          invStartFt !== readyStartFt) {
        fail(
          'identity_mismatch',
          `invocation.target_identity mismatch vs sinkReady (start_ft inv=${invStartFt?.toString() ?? 'null'} ready=${readyStartFt?.toString() ?? 'null'})`,
        );
      }
    }
  }

  // 4. Validate finalized observer summary & clock frequency domain
  const senderFreq = telemetry?.qpc_frequency_hz;
  const readyFreq = sinkReady?.observer_qpc_frequency_hz;
  const summaryFreq = observerSummary?.qpc_frequency_hz;

  if (!Number.isSafeInteger(senderFreq) || senderFreq <= 0 ||
      !Number.isSafeInteger(readyFreq) || readyFreq <= 0 ||
      !Number.isSafeInteger(summaryFreq) || summaryFreq <= 0 ||
      senderFreq !== readyFreq || senderFreq !== summaryFreq) {
    fail('frequency_mismatch', `QPC frequency invalid/mismatch: sender=${senderFreq} ready=${readyFreq} summary=${summaryFreq}`);
  }

  if (!observerSummary || typeof observerSummary !== 'object') {
    fail('observer_health_invalid', 'missing finalized observerSummary');
  } else {
    if (observerSummary.schema_version !== EXPECTED_RECEIVER_SUMMARY_SCHEMA ||
        observerSummary.observer_mode !== EXPECTED_RECEIVER_OBSERVER_MODE ||
        observerSummary.observer_version !== EXPECTED_RECEIVER_OBSERVER_VERSION) {
      fail('schema_or_protocol_mismatch', `observerSummary schema/protocol mismatch: ${observerSummary.schema_version}/${observerSummary.observer_mode}`);
    }
    if (observerSummary.run_id !== runId || observerSummary.role !== 'ReceiveOnly' ||
        observerSummary.event_log_id !== sinkReady?.event_log_id ||
        observerSummary.target_hwnd !== sinkReady?.hwnd ||
        observerSummary.target_pid !== sinkReady?.pid) {
      fail('identity_mismatch', 'observerSummary identity does not match report/sinkReady');
    }
    if (observerSummary.stream_header_flushed !== true ||
        observerSummary.events_flushed !== true ||
        observerSummary.consumer_stopped !== true ||
        observerSummary.queue_drained !== true ||
        observerSummary.observer_startup_failures !== 0 ||
        observerSummary.writer_failures !== 0 ||
        observerSummary.observer_internal_failures !== 0 ||
        observerSummary.queue_dropped !== 0 ||
        observerSummary.queue_overflow !== false ||
        observerSummary.queue_capacity !== EXPECTED_RECEIVER_QUEUE_CAPACITY ||
        observerSummary.events_seen !== expectedReceiverEvents ||
        observerSummary.events_enqueued !== expectedReceiverEvents ||
        observerSummary.events_written !== expectedReceiverEvents) {
      fail(
        'observer_health_invalid',
        `observerSummary incomplete/unhealthy: seen=${observerSummary.events_seen} enq=${observerSummary.events_enqueued} wr=${observerSummary.events_written} expected=${expectedReceiverEvents} drop=${observerSummary.queue_dropped} ovf=${observerSummary.queue_overflow} drained=${observerSummary.queue_drained} hdr_fl=${observerSummary.stream_header_flushed} ev_fl=${observerSummary.events_flushed} stopped=${observerSummary.consumer_stopped}`,
      );
    }
  }

  // 5. Validate sender telemetry schema, completeness, and counts
  if (!telemetry || typeof telemetry !== 'object') {
    fail('sender_trace_incomplete', 'missing native_timing_telemetry');
  } else {
    if (telemetry.schema_version !== EXPECTED_SENDER_TELEMETRY_SCHEMA) {
      fail('schema_or_protocol_mismatch', `sender telemetry schema_version=${telemetry.schema_version} != ${EXPECTED_SENDER_TELEMETRY_SCHEMA}`);
    }
    if (telemetry.truncated !== false ||
        telemetry.dropped !== 0 ||
        telemetry.observer_queue_dropped !== 0 ||
        telemetry.attempted !== expectedPacketCount ||
        telemetry.accepted !== expectedPacketCount ||
        !Array.isArray(telemetry.records) ||
        telemetry.records.length !== expectedPacketCount ||
        telemetry.up_slot_records_expected !== expectedUpSlotCount ||
        telemetry.up_slot_records_dropped !== 0 ||
        telemetry.up_slot_accounting_status !== 'valid' ||
        !Array.isArray(telemetry.up_slot_records) ||
        telemetry.up_slot_records.length !== expectedUpSlotCount) {
      fail(
        'sender_trace_incomplete',
        `sender trace incomplete: truncated=${telemetry.truncated} att=${telemetry.attempted} acc=${telemetry.accepted} rec=${telemetry.records?.length}/${expectedPacketCount} up_exp=${telemetry.up_slot_records_expected} up_rec=${telemetry.up_slot_records?.length}/${expectedUpSlotCount} up_drop=${telemetry.up_slot_records_dropped} up_status=${telemetry.up_slot_accounting_status}`,
      );
    }
  }

  // 6. Parse receiver JSONL strictly (exact 1 first stream_start + exact N keyboard events)
  const nonEmptyLines = Array.isArray(receiverJsonlLines)
    ? receiverJsonlLines.filter((l) => typeof l === 'string' && l.trim() !== '')
    : [];
  const receiverEvents = [];
  let headerCount = 0;
  let expectedSeq = 1;
  let lastRxQpc = null;

  for (let i = 0; i < nonEmptyLines.length; i++) {
    let rec;
    try {
      rec = JSON.parse(nonEmptyLines[i]);
    } catch {
      fail('malformed_record', `malformed JSON at receiver line ${i + 1}`);
      continue;
    }
    if (!rec || typeof rec !== 'object') {
      fail('malformed_record', `non-object JSON at receiver line ${i + 1}`);
      continue;
    }
    if (rec.schema_version !== EXPECTED_RECEIVER_EVENT_SCHEMA ||
        rec.observer_mode !== EXPECTED_RECEIVER_OBSERVER_MODE ||
        rec.observer_version !== EXPECTED_RECEIVER_OBSERVER_VERSION) {
      fail('schema_or_protocol_mismatch', `receiver line ${i + 1} schema/protocol mismatch`);
    }
    if (rec.run_id !== runId || rec.role !== 'ReceiveOnly' ||
        rec.event_log_id !== sinkReady?.event_log_id ||
        rec.target_hwnd !== sinkReady?.hwnd || rec.target_pid !== sinkReady?.pid) {
      fail('identity_mismatch', `receiver line ${i + 1} identity mismatch`);
    }

    if (rec.kind === 'stream_start') {
      headerCount++;
      if (i !== 0 || rec.sequence !== 0 || headerCount > 1) {
        fail('header_anomaly', `stream_start misplaced or duplicated at line ${i + 1} (seq=${rec.sequence})`);
      }
      if (rec.observed_qpc_ticks === undefined || rec.observed_qpc_ticks === null) {
        fail('legacy_no_qpc', 'stream_start missing observed_qpc_ticks (NOT MEASURABLE)');
      }
      continue;
    }

    if (i === 0) {
      fail('header_anomaly', 'first receiver JSONL line is not stream_start');
    }
    if (rec.kind !== 'key_press' && rec.kind !== 'key_release') {
      fail('malformed_record', `unexpected receiver kind '${rec.kind}' at line ${i + 1}`);
      continue;
    }
    if (!Number.isSafeInteger(rec.sequence) || rec.sequence !== expectedSeq) {
      fail('duplicate_or_out_of_order_sequence', `receiver sequence anomaly at line ${i + 1}: got ${rec.sequence}, expected ${expectedSeq}`);
    }
    expectedSeq = Number.isSafeInteger(rec.sequence) ? rec.sequence + 1 : expectedSeq + 1;

    if (rec.foreground_hwnd !== sinkReady?.hwnd || rec.foreground_pid !== sinkReady?.pid) {
      fail('foreground_mismatch', `foreground HWND/PID mismatch at seq ${rec.sequence}`);
    }

    if (rec.observed_qpc_ticks === undefined || rec.observed_qpc_ticks === null) {
      fail('legacy_no_qpc', `missing observed_qpc_ticks at seq ${rec.sequence} (NOT MEASURABLE)`);
      continue;
    }
    let qpc;
    try {
      qpc = toBigIntStrict(rec.observed_qpc_ticks, `observed_qpc_ticks@${rec.sequence}`);
      if (qpc <= 0n) throw new Error('non-positive QPC');
    } catch (e) {
      fail('malformed_record', `malformed observed_qpc_ticks at seq ${rec.sequence}: ${e.message}`);
      continue;
    }
    if (lastRxQpc !== null && qpc < lastRxQpc) {
      fail('receiver_non_monotonic_qpc', `same-thread receiver QPC regressed at seq ${rec.sequence}: ${qpc} < ${lastRxQpc}`);
    }
    lastRxQpc = qpc;

    receiverEvents.push({
      sequence: rec.sequence,
      kind: rec.kind,
      scan_code: rec.scan_code,
      extended: Boolean(rec.extended),
      qpc,
    });
  }

  if (headerCount !== 1) {
    fail('header_anomaly', `expected exactly 1 stream_start header, saw ${headerCount}`);
  }
  if (receiverEvents.length !== expectedReceiverEvents) {
    if (receiverEvents.length < expectedReceiverEvents) {
      fail('missing_receiver_event', `receiver keyboard event count ${receiverEvents.length} < expected ${expectedReceiverEvents}`);
    } else {
      fail('extra_receiver_event', `receiver keyboard event count ${receiverEvents.length} > expected ${expectedReceiverEvents}`);
    }
  }

  // 7. Index and verify UpSlotTrace sidecar (zero orphan/duplicate sidecars)
  const upSlotByRecord = new Map();
  let totalSidecarIndexed = 0;
  for (const trace of telemetry?.up_slot_records ?? []) {
    if (!trace || typeof trace !== 'object' ||
        !Number.isSafeInteger(trace.trace_record_index) ||
        trace.confirmed !== true) {
      fail('up_slot_attribution_mismatch', `unconfirmed or malformed UpSlotTrace entry for record ${trace?.trace_record_index}`);
      continue;
    }
    const list = upSlotByRecord.get(trace.trace_record_index) ?? [];
    list.push(trace);
    upSlotByRecord.set(trace.trace_record_index, list);
    totalSidecarIndexed++;
  }

  const freqHz = BigInt(Number.isSafeInteger(senderFreq) && senderFreq > 0 ? senderFreq : 10_000_000);
  const exactFreqRoundingTol = freqHz % 1_000_000n === 0n ? 0n : 1n;
  const expectedHoldTicks = usToTicksExactOrCeil(PILOT_HOLD_US, freqHz);
  const expectedHoldTicksFloor = usToTicksFloor(PILOT_HOLD_US, freqHz);

  const records = Array.isArray(telemetry?.records) ? telemetry.records : [];
  const metrics = {
    s_minus_d: [],
    s_minus_e: [],
    w_minus_wait_target: [],
    s_minus_w: [],
    c_minus_s: [],
    r1_minus_d: [],
    rn_minus_d: [],
    r1_minus_c: [],
    down_skew: [],
    up_skew: [],
    per_key_receiver_hold: [],
    receiver_release_gap: [],
    common_chord_hold: [],
  };

  const activeSlotGen = new Map();
  const lastReleasedSlot = new Map();
  const chordGenerations = new Map();
  const downCompletionByOnset = new Map();
  const consumedSidecarRecords = new Set();

  let rxCursor = 0;
  let downOnsetIdx = 0;
  let upOnsetIdx = 0;
  let firstDownD = null;
  let firstDownAuthOffset = null;

  for (let i = 0; i < records.length; i++) {
    const pkt = records[i];
    if (!pkt || typeof pkt !== 'object') {
      fail('malformed_record', `non-object sender record at index ${i}`);
      continue;
    }
    if (pkt.trace_record_index !== i) {
      fail('sender_trace_incomplete', `non-contiguous trace_record_index=${pkt.trace_record_index} at position ${i}`);
    }
    if (((pkt.flags ?? 0) & TRACE_FLAG_RECOVERY) !== 0) {
      fail('cleanup_record', `unexpected cleanup/recovery record at trace_record_index=${pkt.trace_record_index}`);
      continue;
    }
    if (pkt.send_status !== 0 || pkt.transport_anomaly === true ||
        pkt.send_attempts !== 1 || pkt.sent_count !== pkt.requested_count ||
        pkt.pre_call_qpc_available !== true || pkt.sendinput_completion_qpc_available !== true) {
      fail('partial_or_failed_send', `unhealthy sender record at trace_record_index=${pkt.trace_record_index}`);
      continue;
    }

    let S;
    let C;
    try {
      S = toBigIntStrict(pkt.pre_call_qpc_ticks, `S@${i}`);
      C = toBigIntStrict(pkt.sendinput_completion_qpc_ticks, `C@${i}`);
      if (S <= 0n || C < S) {
        fail('sender_timestamp_invalid', `invalid sender S/C ordering at record ${i}: S=${S} C=${C}`);
        continue;
      }
    } catch (e) {
      fail('sender_timestamp_invalid', e.message);
      continue;
    }

    const upMask = pkt.up_mask ?? 0;
    const downMask = pkt.down_mask ?? 0;
    if (!Number.isSafeInteger(upMask) || !Number.isSafeInteger(downMask)) {
      fail('malformed_record', `non-integer packet mask at record ${i}`);
      continue;
    }
    if ((upMask & downMask) !== 0) {
      fail('overlapping_packet_masks', `overlapping up/down mask at record ${i}`);
      continue;
    }
    if (upMask !== 0 && downMask !== 0) {
      fail('unexpected_mixed_or_split_packet', `unexpected mixed packet in non-overlapping pilot at record ${i}`);
      continue;
    }
    if (upMask === 0 && downMask === 0) {
      fail('malformed_record', `empty packet mask at record ${i}`);
      continue;
    }

    const upSlots = expandMaskAscending(upMask, profile);
    const downSlots = expandMaskAscending(downMask, profile);
    if (upSlots.length + downSlots.length !== pkt.requested_count) {
      fail('malformed_record', `requested_count mismatch at record ${i}`);
      continue;
    }

    // Strict literal alternating Down(i) at even index 2*k, Up(i) at odd index 2*k+1
    const expectDown = i % 2 === 0;
    const expectedOnset = Math.floor(i / 2);
    if (expectDown && downSlots.length === 0) {
      fail('pilot_geometry_mismatch', `expected Down(${expectedOnset}) at even record index ${i}, got Up`);
      continue;
    }
    if (!expectDown && upSlots.length === 0) {
      fail('pilot_geometry_mismatch', `expected Up(${expectedOnset}) at odd record index ${i}, got Down`);
      continue;
    }

    if (downSlots.length > 0) {
      const expectedSlots = pilotRotatingSlots(downOnsetIdx, expectedWidth);
      const expectedMask = slotsToMask(expectedSlots);
      if (downOnsetIdx !== expectedOnset ||
          downMask !== expectedMask ||
          pkt.authored_down_chord_index_available !== true ||
          pkt.authored_down_chord_index !== downOnsetIdx ||
          pkt.source_action_index !== downOnsetIdx) {
        fail('pilot_geometry_mismatch', `Down packet at record ${i} (onset ${downOnsetIdx}) mask/index mismatch`);
      }

      // Check exact authored Down offset (first=50000 us, cadence=100000 us)
      let authoredOffsetTicks;
      try {
        authoredOffsetTicks = toBigIntStrict(pkt.authored_offset_ticks, `authored_offset_ticks@${i}`);
      } catch (e) {
        fail('authored_timing_inconsistent', e.message);
        authoredOffsetTicks = null;
      }
      const expectedOffsetUs = PILOT_FIRST_OFFSET_US + downOnsetIdx * PILOT_CADENCE_US;
      const expectedOffsetMin = usToTicksFloor(expectedOffsetUs, freqHz);
      const expectedOffsetMax = usToTicksExactOrCeil(expectedOffsetUs, freqHz);
      if (authoredOffsetTicks === null ||
          authoredOffsetTicks < expectedOffsetMin - exactFreqRoundingTol ||
          authoredOffsetTicks > expectedOffsetMax + exactFreqRoundingTol) {
        fail(
          'authored_timing_inconsistent',
          `Down(${downOnsetIdx}) authored_offset_ticks=${authoredOffsetTicks} outside expected ${expectedOffsetUs} us [${expectedOffsetMin}..${expectedOffsetMax}]`,
        );
      }

      // Require authoritative authored_lateness_ticks and cross-check consistent D across all 150 onsets
      if (pkt.authored_lateness_available !== true) {
        fail('authored_timing_inconsistent', `Down(${downOnsetIdx}) missing authoritative authored_lateness_available`);
      } else {
        try {
          const lateness = toBigIntStrict(pkt.authored_lateness_ticks, `authored_lateness_ticks@${i}`);
          const D = S - lateness;
          if (D <= 0n) {
            fail('authored_timing_inconsistent', `Down(${downOnsetIdx}) derived D=${D} <= 0`);
          } else if (firstDownD === null) {
            firstDownD = D;
            firstDownAuthOffset = authoredOffsetTicks;
          } else if (authoredOffsetTicks !== null && firstDownAuthOffset !== null) {
            const actualDeltaD = D - firstDownD;
            const expectedDeltaD = authoredOffsetTicks - firstDownAuthOffset;
            const diff = actualDeltaD >= expectedDeltaD ? actualDeltaD - expectedDeltaD : expectedDeltaD - actualDeltaD;
            if (diff > exactFreqRoundingTol) {
              fail(
                'authored_timing_inconsistent',
                `Down(${downOnsetIdx}) inconsistent D progression: D_i - D_0 = ${actualDeltaD}, expected ${expectedDeltaD}`,
              );
            }
          }
        } catch (e) {
          fail('authored_timing_inconsistent', e.message);
        }
      }

      downCompletionByOnset.set(downOnsetIdx, C);
      downOnsetIdx++;
    } else {
      const expectedSlots = pilotRotatingSlots(upOnsetIdx, expectedWidth);
      const expectedMask = slotsToMask(expectedSlots);
      if (upOnsetIdx !== expectedOnset || upMask !== expectedMask) {
        fail('pilot_geometry_mismatch', `Up packet at record ${i} (onset ${upOnsetIdx}) mask/order mismatch`);
      }

      // Check completion-anchored hold floor against prior Down(upOnsetIdx) completion C
      const priorDownC = downCompletionByOnset.get(upOnsetIdx);
      if (priorDownC === undefined) {
        fail('pilot_geometry_mismatch', `Up(${upOnsetIdx}) has no prior Down(${upOnsetIdx}) completion record`);
      } else if (S + exactFreqRoundingTol < priorDownC + expectedHoldTicksFloor) {
        fail(
          'hold_floor_violation',
          `Up(${upOnsetIdx}) pre-call S=${S} precedes prior Down completion C=${priorDownC} + H=${expectedHoldTicksFloor} ticks`,
        );
      }

      // Strict mandatory UpSlotTrace sidecar verification
      const sidecar = upSlotByRecord.get(pkt.trace_record_index);
      if (!sidecar || sidecar.length !== upSlots.length) {
        fail('up_slot_attribution_mismatch', `missing or count-mismatched UpSlotTrace for record ${pkt.trace_record_index}`);
      } else {
        consumedSidecarRecords.add(pkt.trace_record_index);
        for (let k = 0; k < upSlots.length; k++) {
          const tr = sidecar[k];
          let trComp;
          let trNotBefore;
          try {
            trComp = toBigIntStrict(tr.completion_qpc_ticks, `up_slot.completion@${i}.${k}`);
            trNotBefore = toBigIntStrict(tr.up_not_before_offset_ticks, `up_slot.up_not_before@${i}.${k}`);
          } catch (e) {
            fail('up_slot_attribution_mismatch', e.message);
            continue;
          }
          if (tr.slot !== upSlots[k].slot ||
              tr.scan_code !== upSlots[k].scan_code ||
              tr.source_action_index !== upOnsetIdx ||
              trComp !== C ||
              trNotBefore + exactFreqRoundingTol < expectedHoldTicksFloor) {
            fail(
              'up_slot_attribution_mismatch',
              `UpSlotTrace mismatch at record ${pkt.trace_record_index} idx=${k}: slot=${tr.slot}/${upSlots[k].slot} sc=${tr.scan_code}/${upSlots[k].scan_code} origin=${tr.source_action_index}/${upOnsetIdx} comp=${trComp}/${C} not_before=${trNotBefore}/${expectedHoldTicksFloor}`,
            );
          }
        }
      }
      upOnsetIdx++;
    }

    // Match receiver events strictly & verify receiver observation is not earlier than S beyond 1-tick QPC uncertainty
    const matchedUps = [];
    let matched = true;
    for (const s of upSlots) {
      if (rxCursor >= receiverEvents.length) {
        fail('missing_receiver_event', `missing key_release for slot ${s.slot} at record ${i}`);
        matched = false;
        break;
      }
      const ev = receiverEvents[rxCursor++];
      if (ev.kind !== 'key_release' || ev.scan_code !== s.scan_code || ev.extended !== s.extended) {
        fail('wrong_scan_code_or_extended', `seq ${ev.sequence} expected key_release sc=0x${s.scan_code.toString(16)}, got ${ev.kind} sc=0x${ev.scan_code.toString(16)}`);
        matched = false;
        break;
      }
      if (ev.qpc + 1n < S) {
        fail('receiver_precedes_sender', `key_release seq ${ev.sequence} R=${ev.qpc} precedes sender S=${S} beyond 1-tick uncertainty`);
      }
      matchedUps.push({ slot: s.slot, ev });
    }
    const matchedDowns = [];
    if (matched) {
      for (const s of downSlots) {
        if (rxCursor >= receiverEvents.length) {
          fail('missing_receiver_event', `missing key_press for slot ${s.slot} at record ${i}`);
          matched = false;
          break;
        }
        const ev = receiverEvents[rxCursor++];
        if (ev.kind !== 'key_press' || ev.scan_code !== s.scan_code || ev.extended !== s.extended) {
          fail('wrong_scan_code_or_extended', `seq ${ev.sequence} expected key_press sc=0x${s.scan_code.toString(16)}, got ${ev.kind} sc=0x${ev.scan_code.toString(16)}`);
          matched = false;
          break;
        }
        if (ev.qpc + 1n < S) {
          fail('receiver_precedes_sender', `key_press seq ${ev.sequence} R=${ev.qpc} precedes sender S=${S} beyond 1-tick uncertainty`);
        }
        matchedDowns.push({ slot: s.slot, ev });
      }
    }
    if (!matched) continue;

    // Timing metrics (never fabricate missing wait_target from E!)
    metrics.c_minus_s.push(C - S);
    if (pkt.physical_eligibility_qpc_available === true) {
      metrics.s_minus_e.push(S - toBigIntStrict(pkt.physical_eligibility_qpc_ticks, `E@${i}`));
    }
    if (pkt.wake_qpc_available === true) {
      const W = toBigIntStrict(pkt.wake_qpc_ticks, `W@${i}`);
      metrics.s_minus_w.push(S - W);
      if (pkt.timer_arm_target_qpc_available === true) {
        const waitTarget = toBigIntStrict(pkt.timer_arm_target_qpc_ticks, `waitTarget@${i}`);
        metrics.w_minus_wait_target.push(W - waitTarget);
      }
    }

    if (matchedUps.length > 0) {
      metrics.up_skew.push(matchedUps[matchedUps.length - 1].ev.qpc - matchedUps[0].ev.qpc);
      for (const { slot, ev } of matchedUps) {
        const gen = activeSlotGen.get(slot);
        if (!gen) {
          fail('duplicate_down_or_orphan_up', `orphan Up on inactive slot ${slot} at record ${i}`);
          continue;
        }
        metrics.per_key_receiver_hold.push(ev.qpc - gen.down_r_qpc);
        chordGenerations.get(gen.chord_index)?.up_qpc.push(ev.qpc);
        activeSlotGen.delete(slot);
        lastReleasedSlot.set(slot, { up_r_qpc: ev.qpc });
      }
    }

    if (matchedDowns.length > 0) {
      const chordIndex = pkt.authored_down_chord_index;
      const r1 = matchedDowns[0].ev.qpc;
      const rn = matchedDowns[matchedDowns.length - 1].ev.qpc;
      if (pkt.authored_lateness_available === true) {
        try {
          const D = S - toBigIntStrict(pkt.authored_lateness_ticks, `lateness@${i}`);
          metrics.s_minus_d.push(S - D);
          metrics.r1_minus_d.push(r1 - D);
          metrics.rn_minus_d.push(rn - D);
        } catch {
          // Already recorded above
        }
      }
      metrics.r1_minus_c.push(r1 - C);
      metrics.down_skew.push(rn - r1);

      const chordRec = { down_qpc: [], up_qpc: [], width: matchedDowns.length };
      for (const { slot, ev } of matchedDowns) {
        if (activeSlotGen.has(slot)) {
          fail('duplicate_down_or_orphan_up', `duplicate Down on already-active slot ${slot} at record ${i}`);
          continue;
        }
        const prevRel = lastReleasedSlot.get(slot);
        if (prevRel) metrics.receiver_release_gap.push(ev.qpc - prevRel.up_r_qpc);
        activeSlotGen.set(slot, { chord_index: chordIndex, down_r_qpc: ev.qpc });
        chordRec.down_qpc.push(ev.qpc);
      }
      chordGenerations.set(chordIndex, chordRec);
    }
  }

  // Final completeness assertions
  if (consumedSidecarRecords.size !== upSlotByRecord.size || totalSidecarIndexed !== expectedUpSlotCount) {
    fail('up_slot_attribution_mismatch', `orphan or missing UpSlotTrace records: consumed=${consumedSidecarRecords.size} indexed_records=${upSlotByRecord.size}`);
  }
  if (downOnsetIdx !== PILOT_ONSET_COUNT || upOnsetIdx !== PILOT_ONSET_COUNT) {
    fail('pilot_geometry_mismatch', `incomplete pilot onsets: down=${downOnsetIdx}/${PILOT_ONSET_COUNT} up=${upOnsetIdx}/${PILOT_ONSET_COUNT}`);
  }
  if (rxCursor < receiverEvents.length) {
    fail('extra_receiver_event', `${receiverEvents.length - rxCursor} extra unconsumed receiver event(s)`);
  }
  for (const [slot, gen] of activeSlotGen.entries()) {
    fail('unclosed_generation', `unclosed generation on slot ${slot} (chord_index=${gen.chord_index})`);
  }

  for (const [, chordRec] of chordGenerations.entries()) {
    if (chordRec.down_qpc.length === chordRec.width && chordRec.up_qpc.length === chordRec.width) {
      let maxDown = chordRec.down_qpc[0];
      for (const t of chordRec.down_qpc) if (t > maxDown) maxDown = t;
      let minUp = chordRec.up_qpc[0];
      for (const t of chordRec.up_qpc) if (t < minUp) minUp = t;
      metrics.common_chord_hold.push(minUp > maxDown ? minUp - maxDown : 0n);
    }
  }
  if (metrics.common_chord_hold.length !== PILOT_ONSET_COUNT) {
    fail('pilot_geometry_mismatch', `common_chord_hold sample count ${metrics.common_chord_hold.length} != ${PILOT_ONSET_COUNT}`);
  }
  if (metrics.s_minus_d.length !== PILOT_ONSET_COUNT) {
    fail('authored_timing_inconsistent', `s_minus_d sample count ${metrics.s_minus_d.length} != ${PILOT_ONSET_COUNT}`);
  }

  const totalInvalid = Object.values(invalidCounts).reduce((a, b) => a + b, 0);
  const measurableForSkew = invalidCounts.legacy_no_qpc === 0 && invalidCounts.frequency_mismatch === 0;
  const valid = totalInvalid === 0 && measurableForSkew;

  return {
    schema_version: 3,
    pilot_identity: 'ChordWidthPilotV1',
    protocol: EXPECTED_RECEIVER_OBSERVER_MODE,
    run_id: runId ?? null,
    expected_width: expectedWidth,
    expected_hold_ticks: expectedHoldTicks.toString(),
    qpc_frequency_hz: Number.isSafeInteger(senderFreq) ? senderFreq : null,
    tick_uncertainty_us: Number.isSafeInteger(senderFreq) && senderFreq > 0 ? Number((1_000_000 / senderFreq).toFixed(4)) : null,
    valid,
    accepted_for_aggregation: valid,
    measurable_for_skew: measurableForSkew,
    total_invalid_count: totalInvalid,
    invalid_counts: invalidCounts,
    invalid_reasons: invalidReasons,
    metrics: valid
      ? {
          s_minus_d: summarizeTicks(metrics.s_minus_d, freqHz),
          s_minus_e: summarizeTicks(metrics.s_minus_e, freqHz),
          w_minus_wait_target: summarizeTicks(metrics.w_minus_wait_target, freqHz, 'NOT_MEASURABLE'),
          s_minus_w: summarizeTicks(metrics.s_minus_w, freqHz, 'NOT_MEASURABLE'),
          c_minus_s: summarizeTicks(metrics.c_minus_s, freqHz),
          r1_minus_d: summarizeTicks(metrics.r1_minus_d, freqHz),
          rn_minus_d: summarizeTicks(metrics.rn_minus_d, freqHz),
          r1_minus_c: summarizeTicks(metrics.r1_minus_c, freqHz),
          down_skew: summarizeTicks(metrics.down_skew, freqHz),
          up_skew: summarizeTicks(metrics.up_skew, freqHz),
          per_key_receiver_hold: summarizeTicks(metrics.per_key_receiver_hold, freqHz),
          receiver_release_gap: summarizeTicks(metrics.receiver_release_gap, freqHz, 'NOT_MEASURABLE'),
          common_chord_hold: summarizeTicks(metrics.common_chord_hold, freqHz),
        }
      : null,
  };
}

export function runCli(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i++) {
    const key = argv[i];
    if (key.startsWith('--')) {
      const val = argv[i + 1];
      if (val === undefined || val.startsWith('--')) {
        throw new Error(`flag ${key} requires a value`);
      }
      args[key.slice(2)] = val;
      i++;
    } else {
      throw new Error(`unexpected positional argument: ${key}`);
    }
  }
  for (const req of ['report', 'invocation', 'sink-ready', 'sink-events', 'observer-summary', 'width']) {
    if (!args[req]) throw new Error(`missing required flag --${req}`);
  }
  const width = Number.parseInt(args.width, 10);

  // Report is JSONL (`rt-native-acceptance.jsonl`) with exactly 1 non-empty line.
  const reportLines = fs
    .readFileSync(args.report, 'utf8')
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(Boolean);
  if (reportLines.length !== 1) {
    throw new Error(`expected exactly 1 JSONL record in --report, got ${reportLines.length}`);
  }
  const acceptanceReport = JSON.parse(reportLines[0]);

  const rawInvocation = fs.readFileSync(args.invocation, 'utf8');
  const invocation = JSON.parse(rawInvocation);
  const invocationStartFiletimeExact = extractExactIntegerField(rawInvocation, 'process_start_time_filetime');

  const rawSinkReady = fs.readFileSync(args['sink-ready'], 'utf8');
  const sinkReady = JSON.parse(rawSinkReady);
  const sinkReadyStartFiletimeExact = extractExactIntegerField(rawSinkReady, 'process_start_time_filetime');

  const receiverJsonlLines = fs.readFileSync(args['sink-events'], 'utf8').split(/\r?\n/);
  const observerSummary = JSON.parse(fs.readFileSync(args['observer-summary'], 'utf8'));

  let expectedSourceSha = args['expected-source-sha'] ?? null;
  let expectedSourceTreeSha = args['expected-source-tree'] ?? null;
  let expectedBinarySha256 = args['expected-binary-sha256'] ?? null;
  if (args.manifest) {
    const manifest = JSON.parse(fs.readFileSync(args.manifest, 'utf8'));
    if (manifest.qualification_profile !== 'ChordWidthPilotV1' || manifest.source_role !== 'Candidate') {
      throw new Error('manifest qualification_profile or source_role is invalid for ChordWidthPilotV1');
    }
    expectedSourceSha = expectedSourceSha ?? manifest.source_revision;
    expectedSourceTreeSha = expectedSourceTreeSha ?? manifest.source_tree_sha;
    expectedBinarySha256 = expectedBinarySha256 ?? manifest.binary_sha256;
  }

  const result = analyzePilotRun({
    acceptanceReport,
    invocation,
    sinkReady,
    receiverJsonlLines,
    observerSummary,
    expectedWidth: width,
    expectedSourceSha,
    expectedSourceTreeSha,
    expectedBinarySha256,
    sinkReadyStartFiletimeExact,
    invocationStartFiletimeExact,
  });
  const json = JSON.stringify(result, null, 2);
  if (args.output) {
    fs.writeFileSync(args.output, json + '\n', 'utf8');
  }
  return result;
}

const isMain =
  process.argv[1] &&
  path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url));
if (isMain) {
  try {
    const res = runCli(process.argv.slice(2));
    console.log(JSON.stringify(res, null, 2));
    process.exit(res.valid ? 0 : 1);
  } catch (err) {
    console.error(`analyze_chord_reception error: ${err.message}`);
    process.exit(2);
  }
}
