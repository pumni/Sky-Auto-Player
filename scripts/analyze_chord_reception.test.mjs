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
  PILOT_HOLD_US,
  PILOT_CADENCE_US,
  pilotRotatingSlots,
  slotsToMask,
  analyzePilotRun,
} from './analyze_chord_reception.mjs';

const QPC_FREQ = 10_000_000;
const US_TO_TICKS = 10;
const RUN_ID = 'pilot-test-run-001';
const EVENT_LOG_ID = 'evtlog-test-001';
const PID = 42424;
const HWND = 196608;
const START_FILETIME = 133900000000000000;
const SOURCE_SHA = 'bf00158ddad6d5702ccfc8ea5efb21bdd33a755c';
const BINARY_SHA = 'a'.repeat(64);

function buildValidPilotFixture(width, opts = {}) {
  const onsets = opts.onsets ?? PILOT_ONSET_COUNT;
  const senderRecords = [];
  const upSlotRecords = [];
  const receiverLines = [];

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
    process_start_time_filetime: START_FILETIME,
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

  const baseQpc = 10_000_000;
  let traceIndex = 0;

  for (let i = 0; i < onsets; i++) {
    const slots = pilotRotatingSlots(i, width);
    const mask = slotsToMask(slots);

    const downDueQpc = baseQpc + i * PILOT_CADENCE_US * US_TO_TICKS;
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
      due_qpc_available: true,
      due_qpc_ticks: downDueQpc,
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
      const recvDownQpc = downPreQpc + (20 + k * 4) * US_TO_TICKS;
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

    const upDueQpc = downPostQpc + PILOT_HOLD_US * US_TO_TICKS;
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
      due_qpc_available: true,
      due_qpc_ticks: upDueQpc,
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
        confirmed: true,
      });
      const recvUpQpc = upPreQpc + (22 + k * 4) * US_TO_TICKS;
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

  const totalKeyEvents = onsets * 2 * width;
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

  const acceptanceReport = {
    run_id: RUN_ID,
    verdict: 'Pass',
    source_sha: SOURCE_SHA,
    binary_sha256: BINARY_SHA,
    details: {
      workload_definition_identity: 'chord-width-pilot-v1',
      pilot_chord_width: width,
      pilot_cadence_us: PILOT_CADENCE_US,
      pilot_onset_count: PILOT_ONSET_COUNT,
      scenario_hold_us: PILOT_HOLD_US,
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
      snapshot: {
        active_count: 0,
        final_release_obligation_mask: 0,
        terminal_error: null,
        sendinput_partial_events: 0,
        sendinput_zero_progress_failures: 0,
        release_outcome: {
          released_successfully: true,
        },
      },
    },
  };

  return {
    acceptanceReport,
    sinkReady,
    receiverJsonlLines: receiverLines,
    observerSummary,
    expectedWidth: width,
    expectedSourceSha: SOURCE_SHA,
    expectedBinarySha256: BINARY_SHA,
  };
}

test('accepts valid 150-onset pilot fixtures for widths 1, 5, and 15', () => {
  for (const width of [1, 5, 15]) {
    const fixture = buildValidPilotFixture(width);
    const res = analyzePilotRun(fixture);
    assert.equal(res.valid, true, `width=${width} reasons=${JSON.stringify(res.invalid_reasons)}`);
    assert.equal(res.accepted_for_aggregation, true);
    assert.equal(res.total_invalid_count, 0);
    assert.equal(res.metrics.down_skew.n, 150);
    assert.equal(res.metrics.up_skew.n, 150);
    assert.equal(res.metrics.common_chord_hold.n, 150);
    assert.equal(res.metrics.per_key_receiver_hold.n, 150 * width);
    assert.equal(res.metrics.w_minus_wait_target.n, 300);
    if (width === 1) {
      assert.equal(res.metrics.down_skew.us.max, 0);
      assert.equal(res.metrics.up_skew.us.max, 0);
    } else {
      assert.ok(res.metrics.down_skew.us.p50 > 0);
      assert.ok(res.metrics.common_chord_hold.us.min > 0);
    }
  }
});

test('never falls back wait_target to E when timer_arm_target_qpc_available is false', () => {
  const fixture = buildValidPilotFixture(5, { omitTimerArmTarget: true });
  const res = analyzePilotRun(fixture);
  assert.equal(res.valid, true, JSON.stringify(res.invalid_reasons));
  assert.equal(res.metrics.w_minus_wait_target.n, 0);
  assert.equal(res.metrics.w_minus_wait_target.us, null);
});

test('rejects empty records and zero receiver events (metrics === null)', () => {
  const fixture = buildValidPilotFixture(1);
  fixture.acceptanceReport.details.native_timing_telemetry.records = [];
  fixture.acceptanceReport.details.native_timing_telemetry.up_slot_records = [];
  fixture.receiverJsonlLines = [fixture.receiverJsonlLines[0]];
  const res = analyzePilotRun(fixture);
  assert.equal(res.valid, false);
  assert.equal(res.accepted_for_aggregation, false);
  assert.equal(res.metrics, null);
  assert.ok(res.total_invalid_count >= 2);
});

test('rejects non-Pass report verdict and unhealthy snapshot', () => {
  const f1 = buildValidPilotFixture(5);
  f1.acceptanceReport.verdict = 'Fail';
  const r1 = analyzePilotRun(f1);
  assert.equal(r1.valid, false);
  assert.equal(r1.metrics, null);

  const f2 = buildValidPilotFixture(5);
  f2.acceptanceReport.details.snapshot.release_outcome.released_successfully = false;
  const r2 = analyzePilotRun(f2);
  assert.equal(r2.valid, false);
  assert.equal(r2.metrics, null);
});

test('rejects missing, unconfirmed, orphan, and mismatched UpSlotTrace records', () => {
  const fMissing = buildValidPilotFixture(5);
  fMissing.acceptanceReport.details.native_timing_telemetry.up_slot_records.pop();
  const rMissing = analyzePilotRun(fMissing);
  assert.equal(rMissing.valid, false);
  assert.equal(rMissing.metrics, null);

  const fUnconf = buildValidPilotFixture(5);
  fUnconf.acceptanceReport.details.native_timing_telemetry.up_slot_records[0].confirmed = false;
  const rUnconf = analyzePilotRun(fUnconf);
  assert.equal(rUnconf.valid, false);
  assert.equal(rUnconf.metrics, null);

  const fBadOrigin = buildValidPilotFixture(5);
  fBadOrigin.acceptanceReport.details.native_timing_telemetry.up_slot_records[0].source_action_index = 999;
  const rBadOrigin = analyzePilotRun(fBadOrigin);
  assert.equal(rBadOrigin.valid, false);
  assert.equal(rBadOrigin.metrics, null);

  const fOrphan = buildValidPilotFixture(5);
  fOrphan.acceptanceReport.details.native_timing_telemetry.up_slot_records[0].scan_code = 0x7f;
  const rOrphan = analyzePilotRun(fOrphan);
  assert.equal(rOrphan.valid, false);
  assert.equal(rOrphan.metrics, null);
});

test('rejects duplicate Down, orphan Up, unexpected mixed packet, and S > C', () => {
  const fDupDown = buildValidPilotFixture(1);
  const firstUp = JSON.parse(fDupDown.receiverJsonlLines[2]);
  firstUp.kind = 'key_press';
  firstUp.message = 0x0100;
  fDupDown.receiverJsonlLines[2] = JSON.stringify(firstUp);
  const rDupDown = analyzePilotRun(fDupDown);
  assert.equal(rDupDown.valid, false);
  assert.equal(rDupDown.metrics, null);

  const fOrphanUp = buildValidPilotFixture(1);
  const firstDown = JSON.parse(fOrphanUp.receiverJsonlLines[1]);
  firstDown.kind = 'key_release';
  firstDown.message = 0x0101;
  fOrphanUp.receiverJsonlLines[1] = JSON.stringify(firstDown);
  const rOrphanUp = analyzePilotRun(fOrphanUp);
  assert.equal(rOrphanUp.valid, false);
  assert.equal(rOrphanUp.metrics, null);

  const fMixed = buildValidPilotFixture(5);
  fMixed.acceptanceReport.details.native_timing_telemetry.records[0].up_mask = 1;
  const rMixed = analyzePilotRun(fMixed);
  assert.equal(rMixed.valid, false);
  assert.equal(rMixed.metrics, null);

  const fBadSC = buildValidPilotFixture(5);
  const rec = fBadSC.acceptanceReport.details.native_timing_telemetry.records[0];
  rec.pre_call_qpc_ticks = rec.sendinput_completion_qpc_ticks + 100;
  const rBadSC = analyzePilotRun(fBadSC);
  assert.equal(rBadSC.valid, false);
  assert.equal(rBadSC.metrics, null);
});

test('rejects non-monotonic receiver QPC, duplicate stream_start, and unfinalized summary', () => {
  const fBack = buildValidPilotFixture(5);
  const ev2 = JSON.parse(fBack.receiverJsonlLines[2]);
  const ev1 = JSON.parse(fBack.receiverJsonlLines[1]);
  ev2.observed_qpc_ticks = ev1.observed_qpc_ticks - 1;
  fBack.receiverJsonlLines[2] = JSON.stringify(ev2);
  const rBack = analyzePilotRun(fBack);
  assert.equal(rBack.valid, false);
  assert.equal(rBack.metrics, null);

  const fDupStart = buildValidPilotFixture(1);
  fDupStart.receiverJsonlLines.splice(1, 0, fDupStart.receiverJsonlLines[0]);
  const rDupStart = analyzePilotRun(fDupStart);
  assert.equal(rDupStart.valid, false);
  assert.equal(rDupStart.metrics, null);

  for (const field of ['stream_header_flushed', 'events_flushed', 'consumer_stopped', 'queue_drained']) {
    const fUnfin = buildValidPilotFixture(1);
    fUnfin.observerSummary[field] = false;
    const rUnfin = analyzePilotRun(fUnfin);
    assert.equal(rUnfin.valid, false, `expected failure when ${field}=false`);
    assert.equal(rUnfin.metrics, null);
  }
});

test('rejects legacy schema 3 without observed_qpc_ticks as NOT MEASURABLE', () => {
  const fixture = buildValidPilotFixture(1, { omitObservedQpc: true });
  const res = analyzePilotRun(fixture);
  assert.equal(res.valid, false);
  assert.equal(res.accepted_for_aggregation, false);
  assert.equal(res.measurable_for_skew, false);
  assert.equal(res.metrics, null);
  assert.ok(res.invalid_counts.legacy_no_qpc > 0);
});

test('rejects protocol, role, game_target, identity, and source/binary hash mismatches', () => {
  const fProto = buildValidPilotFixture(1);
  fProto.sinkReady.observer_mode = 'SchedulerQueuedV2';
  assert.equal(analyzePilotRun(fProto).valid, false);

  const fGame = buildValidPilotFixture(1);
  fGame.sinkReady.game_target = true;
  assert.equal(analyzePilotRun(fGame).valid, false);

  const fSrc = buildValidPilotFixture(1);
  fSrc.expectedSourceSha = '0'.repeat(40);
  assert.equal(analyzePilotRun(fSrc).valid, false);

  const fBin = buildValidPilotFixture(1);
  fBin.expectedBinarySha256 = 'b'.repeat(64);
  assert.equal(analyzePilotRun(fBin).valid, false);
});

test('CLI exits 0 on valid files and 1 on invalid files', () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'sky-pilot-cli-'));
  try {
    const fixture = buildValidPilotFixture(5);
    const reportPath = path.join(tmpDir, 'report.json');
    const readyPath = path.join(tmpDir, 'sink-ready.json');
    const eventsPath = path.join(tmpDir, 'sink-events.jsonl');
    const summaryPath = path.join(tmpDir, 'sink-observer-summary.json');
    const outPath = path.join(tmpDir, 'analysis.json');

    fs.writeFileSync(reportPath, JSON.stringify(fixture.acceptanceReport));
    fs.writeFileSync(readyPath, JSON.stringify(fixture.sinkReady));
    fs.writeFileSync(eventsPath, fixture.receiverJsonlLines.join('\n') + '\n');
    fs.writeFileSync(summaryPath, JSON.stringify(fixture.observerSummary));

    const cliScript = path.resolve(process.cwd(), 'scripts/analyze_chord_reception.mjs');
    const stdout = execFileSync(
      process.execPath,
      [
        cliScript,
        '--report',
        reportPath,
        '--sink-ready',
        readyPath,
        '--sink-events',
        eventsPath,
        '--observer-summary',
        summaryPath,
        '--width',
        '5',
        '--expected-source-sha',
        SOURCE_SHA,
        '--expected-binary-sha256',
        BINARY_SHA,
        '--output',
        outPath,
      ],
      { encoding: 'utf8' },
    );
    const parsed = JSON.parse(stdout);
    assert.equal(parsed.valid, true);
    assert.equal(parsed.accepted_for_aggregation, true);
    assert.ok(fs.existsSync(outPath));

    fixture.observerSummary.queue_dropped = 1;
    fs.writeFileSync(summaryPath, JSON.stringify(fixture.observerSummary));
    assert.throws(
      () =>
        execFileSync(
          process.execPath,
          [
            cliScript,
            '--report',
            reportPath,
            '--sink-ready',
            readyPath,
            '--sink-events',
            eventsPath,
            '--observer-summary',
            summaryPath,
            '--width',
            '5',
          ],
          { encoding: 'utf8', stdio: 'pipe' },
        ),
      (err) => err.status === 1,
    );
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test('real native_acceptance_sink.ps1 -SelfTest emits Compat3 schema-3 stream compatible with analyzer contract', () => {
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
    const summaryFiles = files.filter((f) => f.includes('summary') && f.endsWith('.json'));
    assert.ok(jsonlFiles.length > 0, 'SelfTest must emit at least one JSONL event stream');
    assert.ok(summaryFiles.length > 0, 'SelfTest must emit at least one observer summary JSON');

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
        assert.ok(keyEv, 'Compat3 self-test stream should contain key events');
        assert.equal(keyEv.schema_version, EXPECTED_RECEIVER_EVENT_SCHEMA);
        assert.equal(typeof keyEv.observed_qpc_ticks, 'number');
        assert.ok(keyEv.observed_qpc_ticks > 0);
        verifiedCompat3 = true;
      }
    }
    assert.equal(verifiedCompat3, true, 'expected at least one Compat3 schema-3 stream from sink -SelfTest');
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});
