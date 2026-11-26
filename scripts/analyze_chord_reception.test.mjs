import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import {
  CANONICAL_SCAN_CODES,
  EXPECTED_SENDER_TELEMETRY_SCHEMA,
  EXPECTED_RECEIVER_OBSERVER_MODE,
  EXPECTED_RECEIVER_OBSERVER_VERSION,
  EXPECTED_RECEIVER_EVENT_SCHEMA,
  EXPECTED_RECEIVER_READY_SCHEMA,
  EXPECTED_RECEIVER_SUMMARY_SCHEMA,
  EXPECTED_RECEIVER_QUEUE_CAPACITY,
  PILOT_ONSET_COUNT,
  PILOT_FIRST_OFFSET_US,
  PILOT_HOLD_US,
  PILOT_CADENCE_US,
  PILOT_GAME_FPS,
  PILOT_TIMING_MARGIN_US,
  PILOT_SAMPLE_SECONDS,
  pilotRotatingSlots,
  slotsToMask,
  extractExactIntegerField,
  analyzePilotRun,
} from './analyze_chord_reception.mjs';

const QPC_FREQ = 10_000_000;
const US_TO_TICKS = 10;
const RUN_ID = 'pilot-test-run-001';
const EVENT_LOG_ID = 'evtlog-test-001';
const PID = 42424;
const HWND = 196608;
// Exact 64-bit Windows FILETIME (> Number.MAX_SAFE_INTEGER) to test lossless comparison
const START_FILETIME_STR = '133900000000000001';
const SOURCE_SHA = '77c03d2bdeaf9873c42ae2e7ab9da49e3973585f';
const SOURCE_TREE = '0ad933679b346e2139351b0f2d46fccc9cec7538';
const BINARY_SHA = 'a'.repeat(64);

function buildValidPilotFixture(width, opts = {}) {
  const onsets = opts.onsets ?? PILOT_ONSET_COUNT;
  const senderRecords = [];
  const upSlotRecords = [];
  const receiverLines = [];
  const totalKeyEvents = onsets * 2 * width;

  const sinkReady = {
    schema_version: EXPECTED_RECEIVER_READY_SCHEMA,
    event_schema_version: EXPECTED_RECEIVER_EVENT_SCHEMA,
    run_id: RUN_ID,
    role: 'ReceiveOnly',
    sink_kind: 'SkyAutoPlayer.NativeAcceptanceSink',
    target_class: 'project-owned-non-game-receive-only',
    target_name: 'Sky Auto Player — Native Acceptance Sink',
    game_target: false,
    title: 'Sky Auto Player — Native Acceptance Sink',
    process: 'native_acceptance_sink.ps1',
    input_policy: 'receives_only; benchmark must use SendInput',
    pid: PID,
    hwnd: HWND,
    process_start_time_filetime: Number(START_FILETIME_STR),
    event_log_id: EVENT_LOG_ID,
    observer_mode: EXPECTED_RECEIVER_OBSERVER_MODE,
    observer_version: EXPECTED_RECEIVER_OBSERVER_VERSION,
    observer_queue_capacity: EXPECTED_RECEIVER_QUEUE_CAPACITY,
    observer_qpc_frequency_hz: QPC_FREQ,
  };

  let seq = 0;
  receiverLines.push(
    JSON.stringify({
      schema_version: EXPECTED_RECEIVER_EVENT_SCHEMA,
      run_id: RUN_ID,
      role: 'ReceiveOnly',
      event_log_id: EVENT_LOG_ID,
      target_hwnd: HWND,
      target_pid: PID,
      sequence: seq++,
      kind: 'stream_start',
      observer_mode: EXPECTED_RECEIVER_OBSERVER_MODE,
      observer_version: EXPECTED_RECEIVER_OBSERVER_VERSION,
      observed_qpc_ticks: 1_000_000,
    }),
  );

  const epochQpc = 10_000_000;
  let traceIndex = 0;

  for (let i = 0; i < onsets; i++) {
    const slots = pilotRotatingSlots(i, width);
    const mask = slotsToMask(slots);

    const authoredOffsetUs = PILOT_FIRST_OFFSET_US + i * PILOT_CADENCE_US;
    const authoredOffsetTicks = authoredOffsetUs * US_TO_TICKS;
    const downDueQpc = epochQpc + authoredOffsetTicks;
    const downWakeQpc = downDueQpc + 20 * US_TO_TICKS;
    const downPreQpc = downDueQpc + 30 * US_TO_TICKS;
    const downPostQpc = downPreQpc + (15 + width * 3) * US_TO_TICKS;

    const downTraceIdx = traceIndex++;
    senderRecords.push({
      trace_record_index: downTraceIdx,
      flags: 0,
      send_status: 0,
      transport_anomaly: false,
      send_attempts: 1,
      requested_count: width,
      sent_count: width,
      up_mask: 0,
      down_mask: mask,
      authored_down_chord_index_available: true,
      authored_down_chord_index: i,
      source_action_index: i,
      authored_offset_ticks: authoredOffsetTicks,
      authored_lateness_available: true,
      authored_lateness_ticks: downPreQpc - downDueQpc,
      physical_eligibility_qpc_available: true,
      physical_eligibility_qpc_ticks: downDueQpc,
      wake_qpc_available: true,
      wake_qpc_ticks: downWakeQpc,
      timer_arm_target_qpc_available: opts.omitTimerArmTarget ? false : true,
      timer_arm_target_qpc_ticks: opts.omitTimerArmTarget ? 0 : downDueQpc,
      pre_call_qpc_available: true,
      pre_call_qpc_ticks: downPreQpc,
      sendinput_completion_qpc_available: true,
      sendinput_completion_qpc_ticks: downPostQpc,
    });

    for (let k = 0; k < slots.length; k++) {
      const slot = slots[k];
      const sc = CANONICAL_SCAN_CODES[slot];
      // R can be < C (observed during SendInput) as long as R >= S - 1
      const recvDownQpc = downPreQpc + (10 + k * 2) * US_TO_TICKS;
      const ev = {
        schema_version: EXPECTED_RECEIVER_EVENT_SCHEMA,
        run_id: RUN_ID,
        role: 'ReceiveOnly',
        event_log_id: EVENT_LOG_ID,
        target_hwnd: HWND,
        target_pid: PID,
        foreground_hwnd: HWND,
        foreground_pid: PID,
        sequence: seq++,
        kind: 'key_press',
        message: 0x0100,
        scan_code: sc,
        extended: false,
        observer_mode: EXPECTED_RECEIVER_OBSERVER_MODE,
        observer_version: EXPECTED_RECEIVER_OBSERVER_VERSION,
        observed_qpc_ticks: recvDownQpc,
      };
      if (opts.omitObservedQpc) {
        delete ev.observed_qpc_ticks;
      }
      receiverLines.push(JSON.stringify(ev));
    }

    const holdTicks = PILOT_HOLD_US * US_TO_TICKS;
    const upDueQpc = downPostQpc + holdTicks;
    const upWakeQpc = upDueQpc + 18 * US_TO_TICKS;
    const upPreQpc = upDueQpc + 28 * US_TO_TICKS;
    const upPostQpc = upPreQpc + (14 + width * 3) * US_TO_TICKS;

    const upTraceIdx = traceIndex++;
    senderRecords.push({
      trace_record_index: upTraceIdx,
      flags: 0,
      send_status: 0,
      transport_anomaly: false,
      send_attempts: 1,
      requested_count: width,
      sent_count: width,
      up_mask: mask,
      down_mask: 0,
      authored_down_chord_index_available: false,
      authored_down_chord_index: 0,
      source_action_index: i,
      authored_offset_ticks: authoredOffsetTicks + holdTicks,
      authored_lateness_available: false,
      authored_lateness_ticks: 0,
      physical_eligibility_qpc_available: true,
      physical_eligibility_qpc_ticks: upDueQpc,
      wake_qpc_available: true,
      wake_qpc_ticks: upWakeQpc,
      timer_arm_target_qpc_available: opts.omitTimerArmTarget ? false : true,
      timer_arm_target_qpc_ticks: opts.omitTimerArmTarget ? 0 : upDueQpc,
      pre_call_qpc_available: true,
      pre_call_qpc_ticks: upPreQpc,
      sendinput_completion_qpc_available: true,
      sendinput_completion_qpc_ticks: upPostQpc,
    });

    for (let k = 0; k < slots.length; k++) {
      const slot = slots[k];
      const sc = CANONICAL_SCAN_CODES[slot];
      upSlotRecords.push({
        trace_record_index: upTraceIdx,
        slot,
        scan_code: sc,
        source_action_index: i,
        authored_offset_ticks: authoredOffsetTicks + holdTicks,
        up_not_before_offset_ticks: holdTicks,
        floor_displacement_ticks: 0,
        pre_call_qpc_ticks: upPreQpc,
        completion_qpc_ticks: upPostQpc,
        residual_ticks: 0,
        confirmed: true,
      });
      const recvUpQpc = upPreQpc + (12 + k * 2) * US_TO_TICKS;
      const ev = {
        schema_version: EXPECTED_RECEIVER_EVENT_SCHEMA,
        run_id: RUN_ID,
        role: 'ReceiveOnly',
        event_log_id: EVENT_LOG_ID,
        target_hwnd: HWND,
        target_pid: PID,
        foreground_hwnd: HWND,
        foreground_pid: PID,
        sequence: seq++,
        kind: 'key_release',
        message: 0x0101,
        scan_code: sc,
        extended: false,
        observer_mode: EXPECTED_RECEIVER_OBSERVER_MODE,
        observer_version: EXPECTED_RECEIVER_OBSERVER_VERSION,
        observed_qpc_ticks: recvUpQpc,
      };
      if (opts.omitObservedQpc) {
        delete ev.observed_qpc_ticks;
      }
      receiverLines.push(JSON.stringify(ev));
    }
  }

  const observerSummary = {
    schema_version: EXPECTED_RECEIVER_SUMMARY_SCHEMA,
    run_id: RUN_ID,
    role: 'ReceiveOnly',
    event_log_id: EVENT_LOG_ID,
    target_hwnd: HWND,
    target_pid: PID,
    observer_mode: EXPECTED_RECEIVER_OBSERVER_MODE,
    observer_version: EXPECTED_RECEIVER_OBSERVER_VERSION,
    queue_capacity: EXPECTED_RECEIVER_QUEUE_CAPACITY,
    qpc_frequency_hz: QPC_FREQ,
    stream_header_flushed: true,
    events_flushed: true,
    consumer_stopped: true,
    queue_drained: true,
    observer_startup_failures: 0,
    writer_failures: 0,
    observer_internal_failures: 0,
    queue_dropped: 0,
    queue_overflow: false,
    events_seen: totalKeyEvents,
    events_enqueued: totalKeyEvents,
    events_written: totalKeyEvents,
    handler_latency_p50_us: 12,
    handler_latency_p95_us: 25,
    handler_latency_p99_us: 40,
    handler_latency_max_us: 55,
  };

  // Actual Rust evidence_report.rs::write_report + flat snapshot_report.rs::snapshot_json shape
  const acceptanceReport = {
    status: 'PASS',
    scenario: 'chord-width-pilot-v1',
    admission_variant: 'control-a',
    run_id: RUN_ID,
    timing_margin_us: PILOT_TIMING_MARGIN_US,
    sample_seconds: PILOT_SAMPLE_SECONDS,
    game_fps: PILOT_GAME_FPS,
    min_hold_us: PILOT_HOLD_US,
    reason: 'pilot fixture pass',
    details: {
      active_count: 0,
      possibly_active_count: 0,
      failed_release_count: 0,
      final_release_obligation_mask: 0,
      terminal_error: null,
      sendinput_partial_events: 0,
      sendinput_zero_progress_failures: 0,
      release_outcome: {
        attempted_mask: 0,
        transport_anomaly: false,
        released_successfully: true,
        stuck_mask: 0,
        verification_inconclusive: false,
        attempts: 0,
      },
      calibration_placement: 'baseline',
      timing_observer_for_qualification: true,
      workload_definition_identity: 'chord-width-pilot-v1',
      pilot_chord_width: width,
      pilot_cadence_us: PILOT_CADENCE_US,
      pilot_onset_count: PILOT_ONSET_COUNT,
      scenario_hold_us: PILOT_HOLD_US,
      sink_event_log_id: EVENT_LOG_ID,
      expected_sink_event_count: totalKeyEvents,
      observed_sink_event_count: totalKeyEvents,
      sink_event_count: totalKeyEvents,
      native_timing_telemetry_available: true,
      native_timing_telemetry: {
        schema_version: EXPECTED_SENDER_TELEMETRY_SCHEMA,
        qpc_frequency_hz: QPC_FREQ,
        truncated: false,
        dropped: 0,
        observer_queue_dropped: 0,
        attempted: senderRecords.length,
        accepted: senderRecords.length,
        records: senderRecords,
        up_slot_accounting_status: 'valid',
        up_slot_records_expected: upSlotRecords.length,
        up_slot_records_dropped: 0,
        up_slot_records: upSlotRecords,
      },
    },
  };

  // Actual run_native_acceptance_case.ps1 invocation.json shape
  const invocation = {
    run_id: RUN_ID,
    qualification_profile: 'ChordWidthPilotV1',
    workload: 'chord-width-pilot-v1',
    workload_version: 'chord-width-pilot-v1',
    source_revision: SOURCE_SHA,
    source_tree_sha: SOURCE_TREE,
    source_tree_clean: true,
    configured_H_us: PILOT_HOLD_US,
    harness_source_revision: SOURCE_SHA,
    harness_source_tree_sha: SOURCE_TREE,
    harness_source_role: 'Candidate',
    harness_binary_sha256: BINARY_SHA,
    scenario: 'chord-width-pilot-v1',
    calibration_placement: 'baseline',
    cpu_contention: false,
    timing_margin_us: PILOT_TIMING_MARGIN_US,
    game_fps: PILOT_GAME_FPS,
    sample_seconds: PILOT_SAMPLE_SECONDS,
    exit_code: 0,
    status: 'PASS',
    event_log_id: EVENT_LOG_ID,
    owner_approved_receive_only_run: true,
    report_window_match: true,
    receiveonly_observer_mode: EXPECTED_RECEIVER_OBSERVER_MODE,
    receiveonly_observer_version: EXPECTED_RECEIVER_OBSERVER_VERSION,
    target_identity: {
      role: 'ReceiveOnly',
      game_target: false,
      pid: PID,
      hwnd: HWND,
      process_start_time_filetime: Number(START_FILETIME_STR),
      event_log_id: EVENT_LOG_ID,
    },
  };

  return {
    acceptanceReport,
    invocation,
    sinkReady,
    receiverJsonlLines: receiverLines,
    observerSummary,
    expectedWidth: width,
    expectedSourceSha: SOURCE_SHA,
    expectedSourceTreeSha: SOURCE_TREE,
    expectedBinarySha256: BINARY_SHA,
    sinkReadyStartFiletimeExact: START_FILETIME_STR,
    invocationStartFiletimeExact: START_FILETIME_STR,
  };
}

test('accepts valid 150-onset actual-schema pilot fixtures for widths 1, 5, and 15', () => {
  for (const width of [1, 5, 15]) {
    const fixture = buildValidPilotFixture(width);
    const res = analyzePilotRun(fixture);
    assert.equal(res.valid, true, `width=${width} reasons=${JSON.stringify(res.invalid_reasons)}`);
    assert.equal(res.accepted_for_aggregation, true);
    assert.equal(res.total_invalid_count, 0);
    assert.equal(res.metrics.s_minus_d.n, 150);
    assert.equal(res.metrics.down_skew.n, 150);
    assert.equal(res.metrics.up_skew.n, 150);
    assert.equal(res.metrics.common_chord_hold.n, 150);
    assert.equal(res.metrics.per_key_receiver_hold.n, 150 * width);
    assert.equal(res.metrics.w_minus_wait_target.status, 'MEASURED');
    assert.equal(res.metrics.w_minus_wait_target.n, 300);
  }
});

test('reports NOT_MEASURABLE instead of fabricating wait_target when timer_arm_target_qpc_available is false', () => {
  const fixture = buildValidPilotFixture(5, { omitTimerArmTarget: true });
  const res = analyzePilotRun(fixture);
  assert.equal(res.valid, true, JSON.stringify(res.invalid_reasons));
  assert.equal(res.metrics.w_minus_wait_target.status, 'NOT_MEASURABLE');
  assert.equal(res.metrics.w_minus_wait_target.n, 0);
  assert.equal(res.metrics.w_minus_wait_target.us, null);
});

test('lossless 64-bit FILETIME comparison detects 1-tick mismatch beyond Number.MAX_SAFE_INTEGER', () => {
  const rawA = '{"process_start_time_filetime": 133900000000000001}';
  const rawB = '{"process_start_time_filetime": 133900000000000002}';
  assert.equal(extractExactIntegerField(rawA, 'process_start_time_filetime'), '133900000000000001');
  assert.equal(extractExactIntegerField(rawB, 'process_start_time_filetime'), '133900000000000002');
  // In IEEE-754 double, both round to the same Number, proving why lossless extraction is required!
  const fixture = buildValidPilotFixture(1);
  fixture.invocationStartFiletimeExact = '133900000000000002';
  const res = analyzePilotRun(fixture);
  assert.equal(res.valid, false);
  assert.equal(res.metrics, null);
  assert.ok(res.invalid_counts.identity_mismatch > 0);
});

test('rejects missing invocation.json or missing mandatory provenance expectations', () => {
  const fNoInv = buildValidPilotFixture(1);
  fNoInv.invocation = null;
  assert.equal(analyzePilotRun(fNoInv).valid, false);

  const fNoSrc = buildValidPilotFixture(1);
  fNoSrc.expectedSourceSha = null;
  assert.equal(analyzePilotRun(fNoSrc).valid, false);

  const fNoBin = buildValidPilotFixture(1);
  fNoBin.expectedBinarySha256 = null;
  assert.equal(analyzePilotRun(fNoBin).valid, false);
});

test('rejects wrong authored offset, inconsistent authored_lateness_ticks / D, hold-floor violation, and R < S - 1', () => {
  const fBadOffset = buildValidPilotFixture(5);
  fBadOffset.acceptanceReport.details.native_timing_telemetry.records[2].authored_offset_ticks += 5000;
  const rBadOffset = analyzePilotRun(fBadOffset);
  assert.equal(rBadOffset.valid, false);
  assert.equal(rBadOffset.metrics, null);
  assert.ok(rBadOffset.invalid_counts.authored_timing_inconsistent > 0);

  const fBadD = buildValidPilotFixture(5);
  fBadD.acceptanceReport.details.native_timing_telemetry.records[2].authored_lateness_ticks += 2500;
  const rBadD = analyzePilotRun(fBadD);
  assert.equal(rBadD.valid, false);
  assert.equal(rBadD.metrics, null);
  assert.ok(rBadD.invalid_counts.authored_timing_inconsistent > 0);

  const fHoldViol = buildValidPilotFixture(5);
  const upPkt = fHoldViol.acceptanceReport.details.native_timing_telemetry.records[1];
  upPkt.pre_call_qpc_ticks -= 50_000;
  const rHoldViol = analyzePilotRun(fHoldViol);
  assert.equal(rHoldViol.valid, false);
  assert.equal(rHoldViol.metrics, null);
  assert.ok(rHoldViol.invalid_counts.hold_floor_violation > 0);

  const fRxEarly = buildValidPilotFixture(1);
  const ev = JSON.parse(fRxEarly.receiverJsonlLines[1]);
  ev.observed_qpc_ticks = fRxEarly.acceptanceReport.details.native_timing_telemetry.records[0].pre_call_qpc_ticks - 50;
  fRxEarly.receiverJsonlLines[1] = JSON.stringify(ev);
  const rRxEarly = analyzePilotRun(fRxEarly);
  assert.equal(rRxEarly.valid, false);
  assert.equal(rRxEarly.metrics, null);
  assert.ok(rRxEarly.invalid_counts.receiver_precedes_sender > 0);
});

test('rejects non-PASS status, unhealthy flat cleanup fields, legacy no-QPC, and unfinalized summary', () => {
  const fFail = buildValidPilotFixture(1);
  fFail.acceptanceReport.status = 'FAIL';
  assert.equal(analyzePilotRun(fFail).valid, false);

  const fCleanup = buildValidPilotFixture(1);
  fCleanup.acceptanceReport.details.release_outcome.released_successfully = false;
  assert.equal(analyzePilotRun(fCleanup).valid, false);

  const fLegacy = buildValidPilotFixture(1, { omitObservedQpc: true });
  const rLegacy = analyzePilotRun(fLegacy);
  assert.equal(rLegacy.valid, false);
  assert.equal(rLegacy.measurable_for_skew, false);
  assert.equal(rLegacy.metrics, null);

  const fSummary = buildValidPilotFixture(1);
  fSummary.observerSummary.consumer_stopped = false;
  assert.equal(analyzePilotRun(fSummary).valid, false);
});

test('CLI consumes actual-schema files + manifest and exits 0 on valid, 1 on 1-tick FILETIME tamper', () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'sky-pilot-cli-c1-'));
  try {
    const fixture = buildValidPilotFixture(5);
    const reportPath = path.join(tmpDir, 'rt-native-acceptance.jsonl');
    const invPath = path.join(tmpDir, 'invocation.json');
    const readyPath = path.join(tmpDir, 'sink-ready.json');
    const eventsPath = path.join(tmpDir, 'sink-events.jsonl');
    const summaryPath = path.join(tmpDir, 'sink-observer-summary.json');
    const manifestPath = path.join(tmpDir, 'manifest.json');
    const outPath = path.join(tmpDir, 'analysis.json');

    fs.writeFileSync(reportPath, JSON.stringify(fixture.acceptanceReport) + '\n');
    // Write exact 64-bit integer literals for process_start_time_filetime
    const invText = JSON.stringify(fixture.invocation).replace(
      `"process_start_time_filetime":${Number(START_FILETIME_STR)}`,
      `"process_start_time_filetime":${START_FILETIME_STR}`,
    );
    const readyText = JSON.stringify(fixture.sinkReady).replace(
      `"process_start_time_filetime":${Number(START_FILETIME_STR)}`,
      `"process_start_time_filetime":${START_FILETIME_STR}`,
    );
    fs.writeFileSync(invPath, invText);
    fs.writeFileSync(readyPath, readyText);
    fs.writeFileSync(eventsPath, fixture.receiverJsonlLines.join('\n') + '\n');
    fs.writeFileSync(summaryPath, JSON.stringify(fixture.observerSummary));
    fs.writeFileSync(
      manifestPath,
      JSON.stringify({
        qualification_profile: 'ChordWidthPilotV1',
        source_role: 'Candidate',
        source_revision: SOURCE_SHA,
        source_tree_sha: SOURCE_TREE,
        binary_sha256: BINARY_SHA,
      }),
    );

    const cliScript = path.resolve(process.cwd(), 'scripts/analyze_chord_reception.mjs');
    const stdout = execFileSync(
      process.execPath,
      [
        cliScript,
        '--report', reportPath,
        '--invocation', invPath,
        '--sink-ready', readyPath,
        '--sink-events', eventsPath,
        '--observer-summary', summaryPath,
        '--manifest', manifestPath,
        '--width', '5',
        '--output', outPath,
      ],
      { encoding: 'utf8' },
    );
    const parsed = JSON.parse(stdout);
    assert.equal(parsed.valid, true);
    assert.equal(parsed.accepted_for_aggregation, true);
    assert.equal(parsed.metrics.common_chord_hold.n, 150);
    assert.ok(fs.existsSync(outPath));

    // Mutate invocation FILETIME by 1 tick -> lossless check must exit 1
    fs.writeFileSync(invPath, invText.replace(START_FILETIME_STR, '133900000000000002'));
    assert.throws(
      () =>
        execFileSync(
          process.execPath,
          [
            cliScript,
            '--report', reportPath,
            '--invocation', invPath,
            '--sink-ready', readyPath,
            '--sink-events', eventsPath,
            '--observer-summary', summaryPath,
            '--manifest', manifestPath,
            '--width', '5',
          ],
          { encoding: 'utf8', stdio: 'pipe' },
        ),
      (err) => err.status === 1,
    );
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test('separate receiver protocol check: native_acceptance_sink.ps1 -SelfTest emits Compat3 schema-3 stream', () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'sky-sink-selftest-'));
  try {
    const sinkScript = path.resolve(process.cwd(), 'scripts/native_acceptance_sink.ps1');
    execFileSync(
      'pwsh',
      ['-NoProfile', '-File', sinkScript, '-SelfTest', '-SelfTestFixtureRoot', tmpDir],
      { encoding: 'utf8', stdio: 'pipe' },
    );
    const files = [];
    const walk = (dir) => {
      for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) walk(full);
        else files.push(full);
      }
    };
    walk(tmpDir);
    const jsonlFiles = files.filter((f) => f.endsWith('.jsonl'));
    let verifiedCompat3 = false;
    for (const jf of jsonlFiles) {
      const lines = fs
        .readFileSync(jf, 'utf8')
        .split(/\r?\n/)
        .map((l) => l.trim())
        .filter(Boolean)
        .map((l) => JSON.parse(l));
      if (lines.length >= 2 && lines[0].observer_mode === EXPECTED_RECEIVER_OBSERVER_MODE) {
        assert.equal(lines[0].schema_version, EXPECTED_RECEIVER_EVENT_SCHEMA);
        assert.equal(lines[0].kind, 'stream_start');
        assert.equal(lines[0].observer_version, EXPECTED_RECEIVER_OBSERVER_VERSION);
        const keyEv = lines.find((e) => e.kind === 'key_press' || e.kind === 'key_release');
        assert.ok(keyEv);
        assert.equal(keyEv.schema_version, EXPECTED_RECEIVER_EVENT_SCHEMA);
        assert.equal(typeof keyEv.observed_qpc_ticks, 'number');
        assert.ok(keyEv.observed_qpc_ticks > 0);
        verifiedCompat3 = true;
      }
    }
    assert.equal(verifiedCompat3, true);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});
