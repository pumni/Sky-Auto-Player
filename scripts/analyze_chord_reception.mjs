#!/usr/bin/env node
/**
 * Offline Chord & Receiver Timing Analyzer (R2-P / P1).
 *
 * Zero-dependency Node.js module that joins actual schema-28 sender telemetry
 * (`NativeTelemetryOutput` / `RtTraceRecord` + `UpSlotTrace`) with raw queued
 * ReceiveOnly sink JSONL (schema 4, `observed_qpc_ticks`) and observer summary
 * JSON (`scripts/native_acceptance_sink.ps1`).
 *
 * Invariants:
 * - Signed BigInt QPC arithmetic throughout; converts to microseconds only at
 *   aggregation/reporting. Never subtracts UTC wall clocks.
 * - Joins ordered physical direction/scan_code/extended events strictly by
 *   actual packet masks (Up prefix ascending slots, then Down chord ascending
 *   slots) through the validated physical profile — never by timestamp proximity.
 * - Preserves all invalid classifications and counts explicitly without silently
 *   trimming or dropping tails.
 * - Distinguishes WndProc message arrival from game keyboard-state sampling or
 *   audio acknowledgement.
 */

export const CANONICAL_SCAN_CODES = Object.freeze([
  0x15, 0x16, 0x17, 0x18, 0x19, // Y U I O P (slots 0..4)
  0x23, 0x24, 0x25, 0x26, 0x27, // H J K L ; (slots 5..9)
  0x31, 0x32, 0x33, 0x34, 0x35, // N M , . / (slots 10..14)
]);

export const EXPECTED_SENDER_TELEMETRY_SCHEMA = 28;
export const TRACE_FLAG_RECOVERY = 1 << 1;

function toBigIntStrict(value, label) {
  if (typeof value === 'bigint') return value;
  if (typeof value === 'number' && Number.isSafeInteger(value)) return BigInt(value);
  if (typeof value === 'string' && /^-?\d+$/.test(value)) return BigInt(value);
  throw new Error(`malformed integer for ${label}: ${String(value)}`);
}

function ticksToUs(deltaTicks, freqHz) {
  return (Number(deltaTicks) * 1_000_000) / Number(freqHz);
}

/**
 * Nearest-rank quantile convention (matches native_acceptance_sink.ps1:311):
 * index = Math.max(0, Math.ceil(q * n) - 1) on ascending sorted values.
 */
export function summarizeTicks(samplesBigInt, freqHz) {
  const n = samplesBigInt.length;
  if (n === 0) {
    return { n: 0, quantile_convention: 'nearest_rank_ceil', ticks: null, us: null };
  }
  const sorted = [...samplesBigInt].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  const at = (q) => sorted[Math.max(0, Math.ceil(q * n) - 1)];
  return {
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
 * Adapt an actual `rt-native-acceptance` JSON report (`details.native_timing_telemetry`
 * schema 28 + metadata) into the normalized sender input expected by `analyzeRun`.
 */
export function adaptAcceptanceReportToSenderInput(report) {
  const details = report?.details ?? report;
  const telemetry = details?.native_timing_telemetry ?? report?.native_timing_telemetry ?? report;
  const runId = report?.run_id ?? details?.run_id ?? telemetry?.run_id;
  const epochQpc =
    details?.playback_epoch_qpc_ticks ??
    telemetry?.epoch_qpc_ticks ??
    details?.snapshot?.playback_epoch_qpc_ticks ??
    null;

  return {
    run_id: runId,
    session_id: details?.session_id ?? telemetry?.session_id ?? null,
    source_sha: report?.source_sha ?? details?.source_sha ?? null,
    binary_sha256: report?.binary_sha256 ?? details?.binary_sha256 ?? null,
    schema_version: telemetry?.schema_version,
    qpc_frequency_hz: telemetry?.qpc_frequency_hz ?? details?.lifecycle_measurement?.qpc_frequency_hz,
    epoch_qpc_ticks: epochQpc,
    truncated: Boolean(telemetry?.truncated),
    dropped: telemetry?.dropped ?? 0,
    observer_queue_dropped: telemetry?.observer_queue_dropped ?? 0,
    up_slot_records_expected: telemetry?.up_slot_records_expected ?? 0,
    up_slot_records_dropped: telemetry?.up_slot_records_dropped ?? 0,
    up_slot_accounting_status: telemetry?.up_slot_accounting_status ?? 'valid',
    records: Array.isArray(telemetry?.records) ? telemetry.records : [],
    up_slot_records: Array.isArray(telemetry?.up_slot_records) ? telemetry.up_slot_records : [],
  };
}

/**
 * Analyze one run from schema-28 sender telemetry + raw queued ReceiveOnly JSONL
 * + observer summary JSON.
 */
export function analyzeRun({
  senderTelemetry,
  receiverJsonlLines,
  observerSummary,
  physicalProfile,
}) {
  const profile =
    physicalProfile ?? CANONICAL_SCAN_CODES.map((scan_code) => ({ scan_code, extended: false }));

  const invalidReasons = [];
  const invalidCounts = {
    identity_mismatch: 0,
    frequency_mismatch: 0,
    schema_mismatch: 0,
    legacy_no_qpc: 0,
    sender_trace_incomplete: 0,
    observer_health_invalid: 0,
    malformed_record: 0,
    duplicate_or_out_of_order_sequence: 0,
    wrong_scan_code_or_extended: 0,
    missing_receiver_event: 0,
    extra_receiver_event: 0,
    partial_or_failed_send: 0,
    cleanup_record: 0,
    unclosed_generation: 0,
    overlapping_packet_masks: 0,
    up_slot_attribution_mismatch: 0,
  };

  // 1. Validate identity, schema, and clock-domain provenance
  const runId = senderTelemetry?.run_id;
  const senderFreq = senderTelemetry?.qpc_frequency_hz;
  const summaryFreq = observerSummary?.qpc_frequency_hz;

  if (
    senderTelemetry?.schema_version !== undefined &&
    senderTelemetry.schema_version !== EXPECTED_SENDER_TELEMETRY_SCHEMA
  ) {
    invalidCounts.schema_mismatch++;
    invalidReasons.push(
      `unexpected sender telemetry schema_version=${senderTelemetry.schema_version} (expected ${EXPECTED_SENDER_TELEMETRY_SCHEMA})`
    );
  }
  if (!runId || observerSummary?.run_id !== runId) {
    invalidCounts.identity_mismatch++;
    invalidReasons.push(`run_id mismatch: sender=${runId} summary=${observerSummary?.run_id}`);
  }
  if (observerSummary?.role !== 'ReceiveOnly') {
    invalidCounts.identity_mismatch++;
    invalidReasons.push(`invalid receiver role: ${observerSummary?.role}`);
  }
  if (
    !Number.isSafeInteger(senderFreq) ||
    senderFreq <= 0 ||
    !Number.isSafeInteger(summaryFreq) ||
    summaryFreq <= 0 ||
    senderFreq !== summaryFreq
  ) {
    invalidCounts.frequency_mismatch++;
    invalidReasons.push(
      `QPC frequency mismatch or invalid: sender=${senderFreq} receiver=${summaryFreq}`
    );
  }

  // 2. Validate sender trace completeness & capacity bounds
  if (
    senderTelemetry?.truncated === true ||
    (senderTelemetry?.dropped ?? 0) > 0 ||
    (senderTelemetry?.observer_queue_dropped ?? 0) > 0 ||
    (senderTelemetry?.up_slot_records_dropped ?? 0) > 0 ||
    (senderTelemetry?.up_slot_accounting_status !== undefined &&
      senderTelemetry.up_slot_accounting_status !== 'valid')
  ) {
    invalidCounts.sender_trace_incomplete++;
    invalidReasons.push(
      `sender trace incomplete: truncated=${senderTelemetry?.truncated} dropped=${senderTelemetry?.dropped} observer_queue_dropped=${senderTelemetry?.observer_queue_dropped} up_slot_dropped=${senderTelemetry?.up_slot_records_dropped} up_slot_status=${senderTelemetry?.up_slot_accounting_status}`
    );
  }

  // 3. Validate receiver observer summary health
  if (
    observerSummary?.queue_dropped > 0 ||
    observerSummary?.queue_overflow === true ||
    observerSummary?.queue_drained === false ||
    observerSummary?.writer_failures > 0 ||
    observerSummary?.observer_internal_failures > 0 ||
    observerSummary?.events_written !== observerSummary?.events_seen
  ) {
    invalidCounts.observer_health_invalid++;
    invalidReasons.push(
      `receiver observer health invalid: dropped=${observerSummary?.queue_dropped} overflow=${observerSummary?.queue_overflow} drained=${observerSummary?.queue_drained} writer_failures=${observerSummary?.writer_failures} internal_failures=${observerSummary?.observer_internal_failures} seen=${observerSummary?.events_seen} written=${observerSummary?.events_written}`
    );
  }

  // 4. Parse receiver JSONL strictly
  let streamHeaderSeen = false;
  const receiverEvents = [];
  let expectedSeq = 1;
  const seenSeq = new Set();

  for (let i = 0; i < receiverJsonlLines.length; i++) {
    const raw = receiverJsonlLines[i];
    if (typeof raw !== 'string' || raw.trim() === '') continue;
    let rec;
    try {
      rec = JSON.parse(raw);
    } catch {
      invalidCounts.malformed_record++;
      invalidReasons.push(`malformed JSON at line ${i + 1}`);
      continue;
    }

    if (rec.kind === 'stream_start') {
      streamHeaderSeen = true;
      if (
        rec.run_id !== runId ||
        rec.role !== 'ReceiveOnly' ||
        rec.event_log_id !== observerSummary?.event_log_id
      ) {
        invalidCounts.identity_mismatch++;
        invalidReasons.push(`stream_start identity mismatch at line ${i + 1}`);
      }
      if (rec.observed_qpc_ticks === undefined || rec.observed_qpc_ticks === null) {
        invalidCounts.legacy_no_qpc++;
        invalidReasons.push('legacy stream_start without observed_qpc_ticks (NOT MEASURABLE)');
      }
      continue;
    }

    if (rec.kind !== 'key_press' && rec.kind !== 'key_release') {
      invalidCounts.malformed_record++;
      invalidReasons.push(`unknown event kind at line ${i + 1}: ${rec.kind}`);
      continue;
    }
    if (
      rec.run_id !== runId ||
      rec.role !== 'ReceiveOnly' ||
      rec.event_log_id !== observerSummary?.event_log_id
    ) {
      invalidCounts.identity_mismatch++;
      invalidReasons.push(`event identity mismatch at line ${i + 1}`);
    }
    if (
      observerSummary?.target_hwnd !== undefined &&
      rec.target_hwnd !== undefined &&
      (rec.target_hwnd !== observerSummary.target_hwnd ||
        rec.target_pid !== observerSummary.target_pid)
    ) {
      invalidCounts.identity_mismatch++;
      invalidReasons.push(`event target HWND/PID mismatch at seq ${rec.sequence}`);
    }
    if (
      !Number.isSafeInteger(rec.sequence) ||
      rec.sequence !== expectedSeq ||
      seenSeq.has(rec.sequence)
    ) {
      invalidCounts.duplicate_or_out_of_order_sequence++;
      invalidReasons.push(
        `sequence anomaly at line ${i + 1}: got ${rec.sequence}, expected ${expectedSeq}`
      );
    }
    seenSeq.add(rec.sequence);
    expectedSeq = Number.isSafeInteger(rec.sequence) ? rec.sequence + 1 : expectedSeq + 1;

    if (rec.observed_qpc_ticks === undefined || rec.observed_qpc_ticks === null) {
      invalidCounts.legacy_no_qpc++;
      invalidReasons.push(
        `legacy event without observed_qpc_ticks at seq ${rec.sequence} (NOT MEASURABLE)`
      );
      continue;
    }
    let qpc;
    try {
      qpc = toBigIntStrict(rec.observed_qpc_ticks, `observed_qpc_ticks@${rec.sequence}`);
      if (qpc <= 0n) throw new Error('non-positive QPC');
    } catch (e) {
      invalidCounts.malformed_record++;
      invalidReasons.push(`malformed observed_qpc_ticks at seq ${rec.sequence}: ${e.message}`);
      continue;
    }
    receiverEvents.push({
      sequence: rec.sequence,
      kind: rec.kind,
      scan_code: rec.scan_code,
      extended: Boolean(rec.extended),
      qpc,
    });
  }

  if (!streamHeaderSeen && receiverEvents.length > 0) {
    invalidCounts.malformed_record++;
    invalidReasons.push('receiver JSONL missing stream_start header');
  }

  // Index UpSlotTrace records by trace_record_index when present
  const upSlotTraceByRecord = new Map();
  for (const trace of senderTelemetry?.up_slot_records ?? []) {
    const list = upSlotTraceByRecord.get(trace.trace_record_index) ?? [];
    list.push(trace);
    upSlotTraceByRecord.set(trace.trace_record_index, list);
  }

  const freqHz = BigInt(
    Number.isSafeInteger(senderFreq) && senderFreq > 0 ? senderFreq : 10_000_000
  );
  const epochQpc =
    senderTelemetry?.epoch_qpc_ticks !== null && senderTelemetry?.epoch_qpc_ticks !== undefined
      ? toBigIntStrict(senderTelemetry.epoch_qpc_ticks, 'epoch_qpc_ticks')
      : null;
  const records = senderTelemetry?.records ?? [];

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
  const mixedByUpPrefixWidth = new Map();

  const activeSlotGen = new Map();
  const lastReleasedSlot = new Map();
  const chordGenerations = new Map();

  let rxCursor = 0;
  let validPacketCount = 0;
  let validChordCount = 0;

  for (const pkt of records) {
    const isRecovery = ((pkt.flags ?? 0) & TRACE_FLAG_RECOVERY) !== 0;
    if (isRecovery) {
      invalidCounts.cleanup_record++;
      invalidReasons.push(`cleanup/recovery record at trace_record_index=${pkt.trace_record_index}`);
      const upSlots = expandMaskAscending(pkt.up_mask ?? 0, profile);
      for (const s of upSlots) {
        if (
          rxCursor < receiverEvents.length &&
          receiverEvents[rxCursor].kind === 'key_release' &&
          receiverEvents[rxCursor].scan_code === s.scan_code
        ) {
          rxCursor++;
          activeSlotGen.delete(s.slot);
        }
      }
      continue;
    }

    const healthy =
      pkt.send_status === 0 &&
      !pkt.transport_anomaly &&
      pkt.sent_count === pkt.requested_count &&
      pkt.pre_call_qpc_available &&
      pkt.sendinput_completion_qpc_available;

    if (!healthy) {
      invalidCounts.partial_or_failed_send++;
      invalidReasons.push(
        `partial/failed/anomalous send at trace_record_index=${pkt.trace_record_index} status=${pkt.send_status} sent=${pkt.sent_count}/${pkt.requested_count}`
      );
      rxCursor += pkt.sent_count ?? 0;
      continue;
    }

    const upMask = pkt.up_mask ?? 0;
    const downMask = pkt.down_mask ?? 0;
    if ((upMask & downMask) !== 0) {
      invalidCounts.overlapping_packet_masks++;
      invalidReasons.push(`overlapping up/down mask at trace_record_index=${pkt.trace_record_index}`);
      continue;
    }

    const upSlots = expandMaskAscending(upMask, profile);
    const downSlots = expandMaskAscending(downMask, profile);
    if (upSlots.length + downSlots.length !== pkt.requested_count) {
      invalidCounts.malformed_record++;
      invalidReasons.push(`requested_count mismatch at trace_record_index=${pkt.trace_record_index}`);
      continue;
    }

    // Cross-check UpSlotTrace sidecar when provided for this packet
    if (upSlots.length > 0 && upSlotTraceByRecord.has(pkt.trace_record_index)) {
      const sidecar = upSlotTraceByRecord.get(pkt.trace_record_index);
      if (sidecar.length !== upSlots.length) {
        invalidCounts.up_slot_attribution_mismatch++;
        invalidReasons.push(
          `UpSlotTrace count mismatch at trace_record_index=${pkt.trace_record_index}: sidecar=${sidecar.length} up_mask_bits=${upSlots.length}`
        );
      } else {
        for (let k = 0; k < upSlots.length; k++) {
          if (sidecar[k].slot !== upSlots[k].slot || sidecar[k].scan_code !== upSlots[k].scan_code) {
            invalidCounts.up_slot_attribution_mismatch++;
            invalidReasons.push(
              `UpSlotTrace slot/scan_code mismatch at trace_record_index=${pkt.trace_record_index} idx=${k}`
            );
          }
        }
      }
    }

    let packetMatched = true;
    const matchedUps = [];
    for (const s of upSlots) {
      if (rxCursor >= receiverEvents.length) {
        invalidCounts.missing_receiver_event++;
        invalidReasons.push(
          `missing receiver key_release for slot ${s.slot} (sc=0x${s.scan_code.toString(16)})`
        );
        packetMatched = false;
        break;
      }
      const ev = receiverEvents[rxCursor++];
      if (ev.kind !== 'key_release' || ev.scan_code !== s.scan_code || ev.extended !== s.extended) {
        invalidCounts.wrong_scan_code_or_extended++;
        invalidReasons.push(
          `mismatch at seq ${ev.sequence}: expected key_release sc=0x${s.scan_code.toString(16)} ext=${s.extended}, got ${ev.kind} sc=0x${ev.scan_code.toString(16)} ext=${ev.extended}`
        );
        packetMatched = false;
        break;
      }
      matchedUps.push({ slot: s.slot, ev });
    }

    const matchedDowns = [];
    if (packetMatched) {
      for (const s of downSlots) {
        if (rxCursor >= receiverEvents.length) {
          invalidCounts.missing_receiver_event++;
          invalidReasons.push(
            `missing receiver key_press for slot ${s.slot} (sc=0x${s.scan_code.toString(16)})`
          );
          packetMatched = false;
          break;
        }
        const ev = receiverEvents[rxCursor++];
        if (ev.kind !== 'key_press' || ev.scan_code !== s.scan_code || ev.extended !== s.extended) {
          invalidCounts.wrong_scan_code_or_extended++;
          invalidReasons.push(
            `mismatch at seq ${ev.sequence}: expected key_press sc=0x${s.scan_code.toString(16)} ext=${s.extended}, got ${ev.kind} sc=0x${ev.scan_code.toString(16)} ext=${ev.extended}`
          );
          packetMatched = false;
          break;
        }
        matchedDowns.push({ slot: s.slot, ev });
      }
    }

    if (!packetMatched) continue;
    validPacketCount++;

    const S = toBigIntStrict(pkt.pre_call_qpc_ticks, 'pre_call_qpc_ticks');
    const C = toBigIntStrict(pkt.sendinput_completion_qpc_ticks, 'sendinput_completion_qpc_ticks');
    const E = pkt.physical_eligibility_qpc_available
      ? toBigIntStrict(pkt.physical_eligibility_qpc_ticks, 'physical_eligibility_qpc_ticks')
      : null;
    const W = pkt.wake_qpc_available ? toBigIntStrict(pkt.wake_qpc_ticks, 'wake_qpc_ticks') : null;
    const waitTarget = pkt.timer_arm_target_qpc_available
      ? toBigIntStrict(pkt.timer_arm_target_qpc_ticks, 'timer_arm_target_qpc_ticks')
      : E;

    metrics.c_minus_s.push(C - S);
    if (E !== null) metrics.s_minus_e.push(S - E);
    if (W !== null) {
      metrics.s_minus_w.push(S - W);
      if (waitTarget !== null) metrics.w_minus_wait_target.push(W - waitTarget);
    }

    if (matchedUps.length > 0) {
      const upR1 = matchedUps[0].ev.qpc;
      const upRn = matchedUps[matchedUps.length - 1].ev.qpc;
      metrics.up_skew.push(upRn - upR1);
      for (const { slot, ev } of matchedUps) {
        const gen = activeSlotGen.get(slot);
        if (gen) {
          metrics.per_key_receiver_hold.push(ev.qpc - gen.down_r_qpc);
          const chordRec = chordGenerations.get(gen.chord_index);
          if (chordRec) chordRec.up_qpc.push(ev.qpc);
          activeSlotGen.delete(slot);
        }
        lastReleasedSlot.set(slot, { up_r_qpc: ev.qpc });
      }
    }

    if (matchedDowns.length > 0) {
      validChordCount++;
      // Resolve authored Down target D:
      // 1) explicit authored_down_target_qpc_ticks if present
      // 2) epoch_qpc + authored_offset_ticks (dispatch_loop.rs:1304 uses down_authored_target for mixed)
      // 3) S - authored_lateness_ticks when authored_lateness_available is true and epoch_qpc omitted
      let D = null;
      if (pkt.authored_down_target_qpc_ticks !== undefined) {
        D = toBigIntStrict(pkt.authored_down_target_qpc_ticks, 'authored_down_target_qpc_ticks');
      } else if (epochQpc !== null && pkt.authored_offset_ticks !== undefined) {
        D = epochQpc + toBigIntStrict(pkt.authored_offset_ticks, 'authored_offset_ticks');
      } else if (pkt.authored_lateness_available && pkt.authored_lateness_ticks !== undefined) {
        D = S - toBigIntStrict(pkt.authored_lateness_ticks, 'authored_lateness_ticks');
      }

      const r1 = matchedDowns[0].ev.qpc;
      const rn = matchedDowns[matchedDowns.length - 1].ev.qpc;
      const downSkew = rn - r1;

      if (D !== null) {
        metrics.s_minus_d.push(S - D);
        metrics.r1_minus_d.push(r1 - D);
        metrics.rn_minus_d.push(rn - D);
      }
      metrics.r1_minus_c.push(r1 - C);
      metrics.down_skew.push(downSkew);

      if (upSlots.length > 0) {
        const key = String(upSlots.length);
        if (!mixedByUpPrefixWidth.has(key)) {
          mixedByUpPrefixWidth.set(key, {
            s_minus_d: [],
            r1_minus_d: [],
            rn_minus_d: [],
            down_skew: [],
          });
        }
        const bucket = mixedByUpPrefixWidth.get(key);
        if (D !== null) {
          bucket.s_minus_d.push(S - D);
          bucket.r1_minus_d.push(r1 - D);
          bucket.rn_minus_d.push(rn - D);
        }
        bucket.down_skew.push(downSkew);
      }

      const chordIndex = pkt.authored_down_chord_index_available
        ? pkt.authored_down_chord_index
        : pkt.trace_record_index;
      const chordRec = { down_qpc: [], up_qpc: [], width: matchedDowns.length };
      for (const { slot, ev } of matchedDowns) {
        const prevRel = lastReleasedSlot.get(slot);
        if (prevRel) {
          metrics.receiver_release_gap.push(ev.qpc - prevRel.up_r_qpc);
        }
        activeSlotGen.set(slot, { chord_index: chordIndex, down_r_qpc: ev.qpc, down_c_qpc: C });
        chordRec.down_qpc.push(ev.qpc);
      }
      chordGenerations.set(chordIndex, chordRec);
    }
  }

  if (rxCursor < receiverEvents.length) {
    const extra = receiverEvents.length - rxCursor;
    invalidCounts.extra_receiver_event += extra;
    invalidReasons.push(`${extra} extra unconsumed receiver event(s) at tail`);
  }

  for (const [slot, gen] of activeSlotGen.entries()) {
    invalidCounts.unclosed_generation++;
    invalidReasons.push(`unclosed generation on slot ${slot} (chord_index=${gen.chord_index})`);
  }

  for (const [, chordRec] of chordGenerations.entries()) {
    if (chordRec.down_qpc.length === chordRec.width && chordRec.up_qpc.length === chordRec.width) {
      let maxDown = chordRec.down_qpc[0];
      for (const t of chordRec.down_qpc) if (t > maxDown) maxDown = t;
      let minUp = chordRec.up_qpc[0];
      for (const t of chordRec.up_qpc) if (t < minUp) minUp = t;
      const common = minUp > maxDown ? minUp - maxDown : 0n;
      metrics.common_chord_hold.push(common);
    }
  }

  const totalInvalid = Object.values(invalidCounts).reduce((a, b) => a + b, 0);
  const measurableForSkew =
    invalidCounts.legacy_no_qpc === 0 && invalidCounts.frequency_mismatch === 0;

  const mixedStratified = {};
  for (const [prefixWidth, bucket] of mixedByUpPrefixWidth.entries()) {
    mixedStratified[`up_prefix_${prefixWidth}`] = {
      s_minus_d: summarizeTicks(bucket.s_minus_d, freqHz),
      r1_minus_d: summarizeTicks(bucket.r1_minus_d, freqHz),
      rn_minus_d: summarizeTicks(bucket.rn_minus_d, freqHz),
      down_skew: summarizeTicks(bucket.down_skew, freqHz),
    };
  }

  return {
    run_id: runId,
    session_id: senderTelemetry?.session_id ?? null,
    source_sha: senderTelemetry?.source_sha ?? null,
    binary_sha256: senderTelemetry?.binary_sha256 ?? null,
    qpc_frequency_hz: Number(freqHz),
    tick_uncertainty_us: Number((1_000_000 / Number(freqHz)).toFixed(4)),
    valid: totalInvalid === 0,
    measurable_for_skew: measurableForSkew,
    valid_packet_count: validPacketCount,
    valid_chord_count: validChordCount,
    total_invalid_count: totalInvalid,
    invalid_counts: invalidCounts,
    invalid_reasons: invalidReasons,
    metrics: measurableForSkew
      ? {
          s_minus_d: summarizeTicks(metrics.s_minus_d, freqHz),
          s_minus_e: summarizeTicks(metrics.s_minus_e, freqHz),
          w_minus_wait_target: summarizeTicks(metrics.w_minus_wait_target, freqHz),
          s_minus_w: summarizeTicks(metrics.s_minus_w, freqHz),
          c_minus_s: summarizeTicks(metrics.c_minus_s, freqHz),
          r1_minus_d: summarizeTicks(metrics.r1_minus_d, freqHz),
          rn_minus_d: summarizeTicks(metrics.rn_minus_d, freqHz),
          r1_minus_c: summarizeTicks(metrics.r1_minus_c, freqHz),
          down_skew: summarizeTicks(metrics.down_skew, freqHz),
          up_skew: summarizeTicks(metrics.up_skew, freqHz),
          per_key_receiver_hold: summarizeTicks(metrics.per_key_receiver_hold, freqHz),
          receiver_release_gap: summarizeTicks(metrics.receiver_release_gap, freqHz),
          common_chord_hold: summarizeTicks(metrics.common_chord_hold, freqHz),
        }
      : 'NOT_MEASURABLE',
    mixed_by_up_prefix_width: measurableForSkew ? mixedStratified : 'NOT_MEASURABLE',
  };
}
