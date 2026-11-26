import assert from 'node:assert/strict';
import test from 'node:test';
import {
  CANONICAL_SCAN_CODES,
  EXPECTED_SENDER_TELEMETRY_SCHEMA,
  TRACE_FLAG_RECOVERY,
  adaptAcceptanceReportToSenderInput,
  analyzeRun,
} from './analyze_chord_reception.mjs';

function makeBaseSummary(overrides = {}) {
  return {
    schema_version: 1,
    observer_mode: 'SchedulerQueuedV2',
    observer_version: 'scheduler-receiveonly-queue-v2',
    run_id: 'fixture-run',
    role: 'ReceiveOnly',
    event_log_id: 'log-1',
    target_hwnd: 4096,
    target_pid: 1234,
    qpc_frequency_hz: 10_000_000,
    queue_capacity: 65536,
    events_seen: 0,
    events_enqueued: 0,
    events_written: 0,
    queue_dropped: 0,
    queue_overflow: false,
    queue_drained: true,
    writer_failures: 0,
    observer_internal_failures: 0,
    ...overrides,
  };
}

function makeHeaderLine(overrides = {}) {
  return JSON.stringify({
    schema_version: 4,
    run_id: 'fixture-run',
    role: 'ReceiveOnly',
    event_log_id: 'log-1',
    observer_mode: 'SchedulerQueuedV2',
    observer_version: 'scheduler-receiveonly-queue-v2',
    sequence: 0,
    kind: 'stream_start',
    scan_code: 0,
    extended: false,
    virtual_key: 0,
    message: 0,
    observed_utc: '2026-11-26T00:00:00.0000000Z',
    observed_qpc_ticks: 1_000_000,
    target_hwnd: 4096,
    target_pid: 1234,
    foreground_hwnd: 4096,
    foreground_pid: 1234,
    ...overrides,
  });
}

function makeEventLine(seq, kind, slot, qpcTicks, overrides = {}) {
  return JSON.stringify({
    schema_version: 4,
    run_id: 'fixture-run',
    role: 'ReceiveOnly',
    event_log_id: 'log-1',
    observer_mode: 'SchedulerQueuedV2',
    observer_version: 'scheduler-receiveonly-queue-v2',
    sequence: seq,
    kind,
    scan_code: CANONICAL_SCAN_CODES[slot],
    extended: false,
    virtual_key: 0,
    message: kind === 'key_press' ? 0x0100 : 0x0101,
    observed_utc: '2026-11-26T00:00:00.0000000Z',
    observed_qpc_ticks: qpcTicks,
    target_hwnd: 4096,
    target_pid: 1234,
    foreground_hwnd: 4096,
    foreground_pid: 1234,
    ...overrides,
  });
}

function makeSenderPkt(
  idx,
  upMask,
  downMask,
  {
    authoredOffset,
    E,
    W,
    waitTarget,
    S,
    C,
    chordIndex,
    flags = 0,
    send_status = 0,
    transport_anomaly = false,
    sent_count = null,
    authoredLateness = null,
  }
) {
  const req =
    upMask.toString(2).replace(/0/g, '').length + downMask.toString(2).replace(/0/g, '').length;
  return {
    trace_record_index: idx,
    source_action_index: chordIndex ?? idx,
    event_index: chordIndex ?? idx,
    authored_down_chord_index: chordIndex ?? idx,
    authored_down_chord_index_available: downMask !== 0,
    authored_offset_ticks: authoredOffset,
    physical_eligibility_qpc_ticks: E ?? 0,
    physical_eligibility_qpc_available: E !== undefined,
    wake_qpc_ticks: W ?? 0,
    wake_qpc_available: W !== undefined,
    timer_arm_target_qpc_ticks: waitTarget ?? 0,
    timer_arm_target_qpc_available: waitTarget !== undefined,
    pre_call_qpc_ticks: S,
    pre_call_qpc_available: true,
    sendinput_completion_qpc_ticks: C,
    sendinput_completion_qpc_available: true,
    authored_lateness_ticks: authoredLateness ?? 0,
    authored_lateness_available: authoredLateness !== null,
    up_mask: upMask,
    down_mask: downMask,
    requested_count: req,
    sent_count: sent_count ?? req,
    send_status,
    transport_anomaly,
    flags,
  };
}

function makeHealthyPairTelemetry() {
  return {
    run_id: 'fixture-run',
    schema_version: EXPECTED_SENDER_TELEMETRY_SCHEMA,
    qpc_frequency_hz: 10_000_000,
    epoch_qpc_ticks: 10_000_000,
    truncated: false,
    dropped: 0,
    observer_queue_dropped: 0,
    up_slot_records_expected: 1,
    up_slot_records_dropped: 0,
    up_slot_accounting_status: 'valid',
    records: [
      makeSenderPkt(0, 0, 0b1, {
        authoredOffset: 100_000,
        E: 10_100_000,
        S: 10_100_010,
        C: 10_100_050,
        chordIndex: 0,
      }),
      makeSenderPkt(1, 0b1, 0, {
        authoredOffset: 271_670,
        E: 10_271_720,
        S: 10_271_730,
        C: 10_271_770,
      }),
    ],
    up_slot_records: [
      {
        trace_record_index: 1,
        slot: 0,
        scan_code: CANONICAL_SCAN_CODES[0],
        source_action_index: 0,
        authored_offset_ticks: 271_670,
        up_not_before_offset_ticks: 271_720,
        floor_displacement_ticks: 50,
        pre_call_qpc_ticks: 10_271_730,
        completion_qpc_ticks: 10_271_770,
        residual_ticks: 10,
        confirmed: true,
      },
    ],
  };
}

test('contract-shaped schema-28 acceptance report adapter with UpSlotTrace and schema-4 sink JSONL', () => {
  const epoch = 20_000_000;
  const D = epoch + 500_000;
  const U = D + 200 + 171_670;
  const acceptanceReport = {
    run_id: 'pilot-w5-01',
    source_sha: 'e7293877a6ce58a387cb8edfe0a88399a85d2b15',
    binary_sha256: '0'.repeat(64),
    details: {
      session_id: 'sess-001',
      lifecycle_measurement: { qpc_frequency_hz: 10_000_000 },
      native_timing_telemetry: {
        schema_version: 28,
        qpc_frequency_hz: 10_000_000,
        truncated: false,
        attempted: 2,
        accepted: 2,
        dropped: 0,
        observer_queue_dropped: 0,
        up_slot_records_expected: 5,
        up_slot_records_dropped: 0,
        up_slot_accounting_status: 'valid',
        records: [
          makeSenderPkt(0, 0, 0b11111, {
            authoredOffset: 500_000,
            E: D,
            W: D + 15,
            waitTarget: D,
            S: D + 40,
            C: D + 200,
            chordIndex: 0,
            authoredLateness: 40, // allows resolving D = S - 40 even without explicit epoch_qpc_ticks
          }),
          makeSenderPkt(1, 0b11111, 0, {
            authoredOffset: 671_670,
            E: U,
            W: U + 10,
            waitTarget: U,
            S: U + 30,
            C: U + 180,
          }),
        ],
        up_slot_records: [0, 1, 2, 3, 4].map((slot) => ({
          trace_record_index: 1,
          slot,
          scan_code: CANONICAL_SCAN_CODES[slot],
          source_action_index: 0,
          authored_offset_ticks: 671_670,
          up_not_before_offset_ticks: 671_870,
          floor_displacement_ticks: 200,
          pre_call_qpc_ticks: U + 30,
          completion_qpc_ticks: U + 180,
          residual_ticks: 30,
          confirmed: true,
        })),
      },
    },
  };

  const senderTelemetry = adaptAcceptanceReportToSenderInput(acceptanceReport);
  const lines = [
    makeHeaderLine({ run_id: 'pilot-w5-01' }),
    ...[0, 1, 2, 3, 4].map((s, idx) =>
      makeEventLine(idx + 1, 'key_press', s, D + 100 + idx * 50, { run_id: 'pilot-w5-01' })
    ),
    ...[0, 1, 2, 3, 4].map((s, idx) =>
      makeEventLine(idx + 6, 'key_release', s, U + 100 + idx * 50, { run_id: 'pilot-w5-01' })
    ),
  ];
  const summary = makeBaseSummary({
    run_id: 'pilot-w5-01',
    events_seen: 10,
    events_enqueued: 10,
    events_written: 10,
  });

  const res = analyzeRun({ senderTelemetry, receiverJsonlLines: lines, observerSummary: summary });
  assert.equal(res.valid, true);
  assert.equal(res.run_id, 'pilot-w5-01');
  assert.equal(res.session_id, 'sess-001');
  assert.equal(res.metrics.s_minus_d.us.p50, 4);
  assert.equal(res.metrics.down_skew.us.p50, 20);
  assert.equal(res.metrics.up_skew.us.p50, 20);
  assert.equal(res.metrics.per_key_receiver_hold.us.p50, 17187);
  assert.equal(res.metrics.common_chord_hold.us.p50, 17167);
});

test('01 single key healthy with signed R1 < C', () => {
  const epoch = 10_000_000;
  const D = epoch + 500_000;
  const senderTelemetry = {
    run_id: 'fixture-run',
    schema_version: 28,
    qpc_frequency_hz: 10_000_000,
    epoch_qpc_ticks: epoch,
    records: [
      makeSenderPkt(0, 0, 0b1, {
        authoredOffset: 500_000,
        E: D,
        W: D + 20,
        waitTarget: D,
        S: D + 50,
        C: D + 200,
        chordIndex: 0,
      }),
      makeSenderPkt(1, 0b1, 0, {
        authoredOffset: 671_670,
        E: D + 200 + 171_670,
        W: D + 200 + 171_690,
        waitTarget: D + 200 + 171_670,
        S: D + 200 + 171_710,
        C: D + 200 + 171_810,
      }),
    ],
  };
  const lines = [
    makeHeaderLine(),
    makeEventLine(1, 'key_press', 0, D + 180),
    makeEventLine(2, 'key_release', 0, D + 200 + 171_800),
  ];
  const res = analyzeRun({
    senderTelemetry,
    receiverJsonlLines: lines,
    observerSummary: makeBaseSummary({ events_seen: 2, events_enqueued: 2, events_written: 2 }),
  });
  assert.equal(res.valid, true);
  assert.equal(res.metrics.s_minus_d.us.p50, 5);
  assert.equal(res.metrics.r1_minus_c.us.p50, -2);
  assert.equal(res.metrics.common_chord_hold.us.p50, 17182);
});

test('02 15-key chord common intersection strictly shorter than every per-key hold', () => {
  const epoch = 10_000_000;
  const D = epoch + 500_000;
  const U = D + 171_670;
  const mask15 = 0x7fff;
  const senderTelemetry = {
    run_id: 'fixture-run',
    schema_version: 28,
    qpc_frequency_hz: 10_000_000,
    epoch_qpc_ticks: epoch,
    records: [
      makeSenderPkt(0, 0, mask15, {
        authoredOffset: 500_000,
        E: D,
        W: D + 10,
        waitTarget: D,
        S: D + 40,
        C: D + 1600,
        chordIndex: 0,
      }),
      makeSenderPkt(1, mask15, 0, {
        authoredOffset: 671_670,
        E: U,
        W: U + 10,
        waitTarget: U,
        S: U + 40,
        C: U + 1600,
      }),
    ],
  };
  const lines = [makeHeaderLine()];
  let seq = 1;
  for (let i = 0; i < 15; i++) lines.push(makeEventLine(seq++, 'key_press', i, D + 100 + i * 100));
  for (let i = 0; i < 15; i++) lines.push(makeEventLine(seq++, 'key_release', i, U + 100 + i * 100));
  const res = analyzeRun({
    senderTelemetry,
    receiverJsonlLines: lines,
    observerSummary: makeBaseSummary({ events_seen: 30, events_enqueued: 30, events_written: 30 }),
  });
  assert.equal(res.valid, true);
  assert.equal(res.metrics.down_skew.us.p50, 140);
  assert.equal(res.metrics.per_key_receiver_hold.us.min, 17167);
  assert.equal(res.metrics.common_chord_hold.us.p50, 17027);
  assert.ok(res.metrics.common_chord_hold.us.p50 < res.metrics.per_key_receiver_hold.us.min);
});

test('03 mixed Up prefix stratified without double-counting whole-packet time', () => {
  const epoch = 10_000_000;
  const D0 = epoch + 500_000;
  const D1 = D0 + 172_000;
  const U1 = D1 + 172_000;
  const senderTelemetry = {
    run_id: 'fixture-run',
    schema_version: 28,
    qpc_frequency_hz: 10_000_000,
    epoch_qpc_ticks: epoch,
    records: [
      makeSenderPkt(0, 0, 0b00011, {
        authoredOffset: 500_000,
        E: D0,
        S: D0 + 20,
        C: D0 + 100,
        chordIndex: 0,
      }),
      makeSenderPkt(1, 0b00011, 0b11100, {
        authoredOffset: 672_000,
        E: D1,
        S: D1 + 30,
        C: D1 + 350,
        chordIndex: 1,
      }),
      makeSenderPkt(2, 0b11100, 0, {
        authoredOffset: 844_000,
        E: U1,
        S: U1 + 20,
        C: U1 + 150,
      }),
    ],
  };
  const lines = [
    makeHeaderLine(),
    makeEventLine(1, 'key_press', 0, D0 + 60),
    makeEventLine(2, 'key_press', 1, D0 + 80),
    makeEventLine(3, 'key_release', 0, D1 + 100),
    makeEventLine(4, 'key_release', 1, D1 + 130),
    makeEventLine(5, 'key_press', 2, D1 + 200),
    makeEventLine(6, 'key_press', 3, D1 + 240),
    makeEventLine(7, 'key_press', 4, D1 + 290),
    makeEventLine(8, 'key_release', 2, U1 + 80),
    makeEventLine(9, 'key_release', 3, U1 + 100),
    makeEventLine(10, 'key_release', 4, U1 + 120),
  ];
  const res = analyzeRun({
    senderTelemetry,
    receiverJsonlLines: lines,
    observerSummary: makeBaseSummary({ events_seen: 10, events_enqueued: 10, events_written: 10 }),
  });
  assert.equal(res.valid, true);
  assert.equal(res.mixed_by_up_prefix_width.up_prefix_2.r1_minus_d.us.p50, 20);
  assert.equal(res.mixed_by_up_prefix_width.up_prefix_2.down_skew.us.p50, 9);
});

test('04-06 immediate/late wake, equal timestamps, repeated key release gap, negative R1-C', () => {
  const epoch = 10_000_000;
  const D0 = epoch + 100_000;
  const U0 = D0 + 171_670;
  const D1 = U0 + 1_000;
  const U1 = D1 + 171_670;
  const senderTelemetry = {
    run_id: 'fixture-run',
    schema_version: 28,
    qpc_frequency_hz: 10_000_000,
    epoch_qpc_ticks: epoch,
    records: [
      // Immediate (no W)
      makeSenderPkt(0, 0, 0b11, { authoredOffset: 100_000, E: D0, S: D0, C: D0 + 500, chordIndex: 0 }),
      makeSenderPkt(1, 0b11, 0, { authoredOffset: 271_670, E: U0, S: U0, C: U0 + 500 }),
      // Late wake (+500us)
      makeSenderPkt(2, 0, 0b1, {
        authoredOffset: 272_670,
        E: D1,
        W: D1 + 5000,
        waitTarget: D1,
        S: D1 + 5050,
        C: D1 + 5500,
        chordIndex: 1,
      }),
      makeSenderPkt(3, 0b1, 0, { authoredOffset: 444_340, E: U1, S: U1, C: U1 + 500 }),
    ],
  };
  const lines = [
    makeHeaderLine(),
    makeEventLine(1, 'key_press', 0, D0 + 120), // R1 - C = -38us
    makeEventLine(2, 'key_press', 1, D0 + 120), // equal timestamp -> skew = 0
    makeEventLine(3, 'key_release', 0, U0 + 120),
    makeEventLine(4, 'key_release', 1, U0 + 166_950),
    makeEventLine(5, 'key_press', 0, D1 + 5120), // release gap = (D1+5120)-(U0+120) = 6000 ticks = 600us
    makeEventLine(6, 'key_release', 0, U1 + 120),
  ];
  const res = analyzeRun({
    senderTelemetry,
    receiverJsonlLines: lines,
    observerSummary: makeBaseSummary({ events_seen: 6, events_enqueued: 6, events_written: 6 }),
  });
  assert.equal(res.valid, true);
  assert.equal(res.metrics.down_skew.us.min, 0);
  assert.equal(res.metrics.r1_minus_c.us.min, -38);
  assert.equal(res.metrics.w_minus_wait_target.us.max, 500);
  assert.equal(res.metrics.receiver_release_gap.us.p50, 600);
});

test('07-16 negative classifications: legacy no-QPC, missing/extra/wrong scan code, identity/freq, drops, malformed/dup, partial/cleanup, sender trace exhaustion & UpSlotTrace mismatch', () => {
  // Legacy no QPC
  const legacyRes = analyzeRun({
    senderTelemetry: makeHealthyPairTelemetry(),
    receiverJsonlLines: [
      JSON.stringify({ schema_version: 2, run_id: 'fixture-run', role: 'ReceiveOnly', event_log_id: 'log-1', sequence: 0, kind: 'stream_start' }),
      JSON.stringify({ schema_version: 2, run_id: 'fixture-run', role: 'ReceiveOnly', event_log_id: 'log-1', sequence: 1, kind: 'key_press', scan_code: 0x15, extended: false }),
    ],
    observerSummary: makeBaseSummary({ events_seen: 1, events_enqueued: 1, events_written: 1 }),
  });
  assert.equal(legacyRes.measurable_for_skew, false);
  assert.equal(legacyRes.metrics, 'NOT_MEASURABLE');

  // Missing + unclosed
  const missingRes = analyzeRun({
    senderTelemetry: makeHealthyPairTelemetry(),
    receiverJsonlLines: [makeHeaderLine(), makeEventLine(1, 'key_press', 0, 10_100_040)],
    observerSummary: makeBaseSummary({ events_seen: 1, events_enqueued: 1, events_written: 1 }),
  });
  assert.equal(missingRes.invalid_counts.missing_receiver_event, 1);
  assert.equal(missingRes.invalid_counts.unclosed_generation, 1);

  // Extra tail event
  const extraRes = analyzeRun({
    senderTelemetry: makeHealthyPairTelemetry(),
    receiverJsonlLines: [
      makeHeaderLine(),
      makeEventLine(1, 'key_press', 0, 10_100_040),
      makeEventLine(2, 'key_release', 0, 10_271_760),
      makeEventLine(3, 'key_press', 1, 10_271_900),
    ],
    observerSummary: makeBaseSummary({ events_seen: 3, events_enqueued: 3, events_written: 3 }),
  });
  assert.equal(extraRes.invalid_counts.extra_receiver_event, 1);

  // Wrong scan code
  const wrongScanRes = analyzeRun({
    senderTelemetry: makeHealthyPairTelemetry(),
    receiverJsonlLines: [makeHeaderLine(), makeEventLine(1, 'key_press', 2, 10_100_040), makeEventLine(2, 'key_release', 0, 10_271_760)],
    observerSummary: makeBaseSummary({ events_seen: 2, events_enqueued: 2, events_written: 2 }),
  });
  assert.equal(wrongScanRes.invalid_counts.wrong_scan_code_or_extended, 1);

  // Wrong role + frequency + identity
  const idRes = analyzeRun({
    senderTelemetry: makeHealthyPairTelemetry(),
    receiverJsonlLines: [makeHeaderLine({ run_id: 'other' }), makeEventLine(1, 'key_press', 0, 10_100_040)],
    observerSummary: makeBaseSummary({ role: 'InertFocusProbe', qpc_frequency_hz: 3_579_545, events_seen: 1, events_enqueued: 1, events_written: 1 }),
  });
  assert.ok(idRes.invalid_counts.identity_mismatch >= 2);
  assert.equal(idRes.invalid_counts.frequency_mismatch, 1);

  // Receiver queue overflow / drops
  const dropRes = analyzeRun({
    senderTelemetry: makeHealthyPairTelemetry(),
    receiverJsonlLines: [makeHeaderLine(), makeEventLine(1, 'key_press', 0, 10_100_040), makeEventLine(2, 'key_release', 0, 10_271_760)],
    observerSummary: makeBaseSummary({ events_seen: 3, events_enqueued: 2, events_written: 2, queue_dropped: 1, queue_overflow: true }),
  });
  assert.equal(dropRes.invalid_counts.observer_health_invalid, 1);

  // Malformed JSON + duplicate sequence
  const malRes = analyzeRun({
    senderTelemetry: makeHealthyPairTelemetry(),
    receiverJsonlLines: [makeHeaderLine(), '{bad-json', makeEventLine(1, 'key_press', 0, 10_100_040), makeEventLine(1, 'key_release', 0, 10_271_760)],
    observerSummary: makeBaseSummary({ events_seen: 2, events_enqueued: 2, events_written: 2 }),
  });
  assert.equal(malRes.invalid_counts.malformed_record, 1);
  assert.equal(malRes.invalid_counts.duplicate_or_out_of_order_sequence, 1);

  // Partial send + cleanup recovery record
  const partialRes = analyzeRun({
    senderTelemetry: {
      run_id: 'fixture-run',
      schema_version: 28,
      qpc_frequency_hz: 10_000_000,
      epoch_qpc_ticks: 10_000_000,
      records: [
        makeSenderPkt(0, 0, 0b11, { authoredOffset: 100_000, E: 10_100_000, S: 10_100_010, C: 10_100_050, send_status: 3, transport_anomaly: true, sent_count: 1 }),
        makeSenderPkt(1, 0b1, 0, { authoredOffset: 100_500, E: 10_100_500, S: 10_100_510, C: 10_100_550, flags: TRACE_FLAG_RECOVERY }),
      ],
    },
    receiverJsonlLines: [makeHeaderLine(), makeEventLine(1, 'key_press', 0, 10_100_040), makeEventLine(2, 'key_release', 0, 10_100_540)],
    observerSummary: makeBaseSummary({ events_seen: 2, events_enqueued: 2, events_written: 2 }),
  });
  assert.equal(partialRes.invalid_counts.partial_or_failed_send, 1);
  assert.equal(partialRes.invalid_counts.cleanup_record, 1);

  // Sender trace capacity exhaustion + UpSlotTrace mismatch
  const exhaustTelemetry = makeHealthyPairTelemetry();
  exhaustTelemetry.truncated = true;
  exhaustTelemetry.up_slot_records_dropped = 1;
  exhaustTelemetry.up_slot_accounting_status = 'invalid';
  exhaustTelemetry.up_slot_records[0].scan_code = CANONICAL_SCAN_CODES[3];
  const exhaustRes = analyzeRun({
    senderTelemetry: exhaustTelemetry,
    receiverJsonlLines: [makeHeaderLine(), makeEventLine(1, 'key_press', 0, 10_100_040), makeEventLine(2, 'key_release', 0, 10_271_760)],
    observerSummary: makeBaseSummary({ events_seen: 2, events_enqueued: 2, events_written: 2 }),
  });
  assert.equal(exhaustRes.valid, false);
  assert.equal(exhaustRes.invalid_counts.sender_trace_incomplete, 1);
  assert.equal(exhaustRes.invalid_counts.up_slot_attribution_mismatch, 1);
});
