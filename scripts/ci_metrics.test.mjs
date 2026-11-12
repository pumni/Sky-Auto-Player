import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import {
  MAX_INPUT_BYTES,
  normalizeSample,
  parseSnapshotText,
  summarizeSamples,
} from './ci_metrics.mjs';

const fixturePath = resolve('scripts/fixtures/ci-metrics/paginated-attempt-2.json');
const fixtureText = readFileSync(fixturePath, 'utf8');
const fixture = JSON.parse(fixtureText);

test('normalizes paginated jobs, attempt 2, concurrent runner minutes, and unavailable OS', () => {
  const report = parseSnapshotText(fixtureText);
  const [run] = report.runs;
  assert.equal(run.run_attempt, 2);
  assert.equal(run.job_count, 6);
  assert.equal(run.wall_clock_seconds, 180);
  assert.equal(run.runner_seconds, 240);
  assert.notEqual(run.runner_minutes, run.wall_clock_seconds / 60);
  assert.equal(run.runner_minutes_by_os.windows, 1.3333);
  assert.equal(run.runner_minutes_by_os.ubuntu, 2);
  assert.equal(run.runner_minutes_by_os.unknown, 0.6667);
  assert.equal(run.jobs.find((job) => job.id === '900015').runner_os, 'unknown');
  assert.equal(run.jobs.find((job) => job.id === '900013').runner_minutes_included, false);
  assert.equal(run.jobs.find((job) => job.id === '900013').duration_seconds, null);
  assert.equal(run.jobs.find((job) => job.id === '900014').runner_minutes_included, false);
  assert.equal(run.failures.length, 1);
  assert.equal(run.failures[0].classification, 'failed_job_details_require_logs');
  assert.equal(run.conclusion, 'failure');
  assert.equal(run.sccache_stats.available, false);
  assert.equal(run.critical_path_observation.job_name, 'Ubuntu tests');
});

test('keeps missing step times unknown and does not infer runner OS from names', () => {
  const [run] = parseSnapshotText(fixtureText).runs;
  const ubuntu = run.jobs.find((job) => job.id === '900012');
  const unknown = run.jobs.find((job) => job.id === '900015');
  assert.equal(ubuntu.steps[1].duration_seconds, null);
  assert.equal(unknown.runner_os, 'unknown');
});

test('rejects incomplete or malformed pagination rather than producing partial metrics', () => {
  const partial = structuredClone(fixture);
  partial.samples[0].job_pages[1].jobs.pop();
  assert.throws(() => parseSnapshotText(JSON.stringify(partial)), /pagination is partial/);

  const reordered = structuredClone(fixture);
  reordered.samples[0].job_pages[1].page = 3;
  assert.throws(() => parseSnapshotText(JSON.stringify(reordered)), /pagination is malformed or incomplete/);

  assert.throws(() => parseSnapshotText('{broken'), /not valid JSON/);
  assert.throws(() => parseSnapshotText(' '.repeat(MAX_INPUT_BYTES + 1)), /safety limit/);
});

test('rejects invalid durations, run identities, and duplicate run attempts', () => {
  const negativeDuration = structuredClone(fixture);
  negativeDuration.samples[0].job_pages[0].jobs[0].completed_at = '2026-11-12T00:59:00Z';
  assert.throws(() => parseSnapshotText(JSON.stringify(negativeDuration)), /ends before it starts/);

  const invalidSha = structuredClone(fixture);
  invalidSha.samples[0].run.head_sha = '0123';
  assert.throws(() => parseSnapshotText(JSON.stringify(invalidSha)), /full 40-character SHA/);

  const duplicate = structuredClone(fixture);
  duplicate.samples.push(structuredClone(duplicate.samples[0]));
  assert.throws(() => parseSnapshotText(JSON.stringify(duplicate)), /duplicate run attempt/);
  assert.throws(() => normalizeSample({}), /run metadata/);
});

test('does not report a successful run for failed jobs or thin p50/p95 samples', () => {
  const [run] = parseSnapshotText(fixtureText).runs;
  assert.equal(run.conclusion, 'failure');
  assert.equal(run.failures.length, 1);
  assert.equal(run.comparable_samples, undefined);

  const [group] = summarizeSamples([run]);
  assert.equal(group.sample_count, 1);
  assert.equal(group.wall_clock_seconds_p50, null);
  assert.equal(group.wall_clock_seconds_p95, null);
  assert.match(group.availability, /p50 requires 5/);
});

test('computes p50 with five comparable samples and withholds p95 until twenty', () => {
  const [run] = parseSnapshotText(fixtureText).runs;
  const fiveSamples = Array.from({ length: 5 }, (_, index) => ({
    ...run,
    run_id: String(910000 + index),
    wall_clock_seconds: 100 + index * 10,
  }));
  const [five] = summarizeSamples(fiveSamples);
  assert.equal(five.wall_clock_seconds_p50, 120);
  assert.equal(five.wall_clock_seconds_p95, null);

  const [twenty] = summarizeSamples(Array.from({ length: 20 }, (_, index) => ({
    ...run,
    run_id: String(920000 + index),
    wall_clock_seconds: 100 + index,
  })));
  assert.notEqual(twenty.wall_clock_seconds_p95, null);
});

test('CLI writes both deterministic formats from offline input', () => {
  const output = mkdtempSync(join(tmpdir(), 'ci metrics output with spaces-'));
  try {
    const result = spawnSync(process.execPath, [
      resolve('scripts/ci_metrics.mjs'), '--input', fixturePath, '--output-dir', output,
    ], { cwd: resolve('.'), encoding: 'utf8', windowsHide: true });
    assert.equal(result.error, undefined);
    assert.equal(result.status, 0, result.stderr);
    const json = JSON.parse(readFileSync(join(output, 'ci-metrics.json'), 'utf8'));
    const markdown = readFileSync(join(output, 'ci-metrics.md'), 'utf8');
    assert.equal(json.runs[0].run_attempt, 2);
    assert.match(markdown, /not GitHub billable minutes/);
    assert.match(markdown, /Unknown OS/);
  } finally {
    rmSync(output, { recursive: true, force: true });
  }
});
