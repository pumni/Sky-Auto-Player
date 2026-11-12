#!/usr/bin/env node

import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const MAX_INPUT_BYTES = 10 * 1024 * 1024;
const MAX_PAGES = 100;
const SHA_PATTERN = /^[0-9a-f]{40}$/i;
const CATEGORY_PATTERN = /^[a-z0-9][a-z0-9._-]{0,63}$/;

function fail(message) {
  throw new Error(message);
}

function isRecord(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function requireString(value, label, { optional = false } = {}) {
  if (optional && (value === null || value === undefined || value === '')) return null;
  if (typeof value !== 'string' || value.length === 0) fail(`${label} must be a non-empty string.`);
  return value;
}

function parseTimestamp(value, label, { optional = false } = {}) {
  if (optional && (value === null || value === undefined || value === '')) return null;
  if (typeof value !== 'string' || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(value)) {
    fail(`${label} must be an ISO timestamp or null.`);
  }
  const milliseconds = Date.parse(value);
  if (!Number.isFinite(milliseconds)) fail(`${label} is not a valid timestamp.`);
  return milliseconds;
}

function secondsBetween(start, end, label) {
  if (start === null || end === null) return null;
  if (end < start) fail(`${label} ends before it starts.`);
  return (end - start) / 1000;
}

function round(value, digits = 2) {
  if (value === null) return null;
  const factor = 10 ** digits;
  return Math.round((value + Number.EPSILON) * factor) / factor;
}

function normalizeOs(raw) {
  if (typeof raw !== 'string') return 'unknown';
  const os = raw.toLowerCase();
  if (os.includes('windows')) return 'windows';
  if (os.includes('ubuntu')) return 'ubuntu';
  return 'other';
}

function normalizeStep(step, jobLabel) {
  if (!isRecord(step)) fail(`${jobLabel} contains a malformed step.`);
  const name = requireString(step.name, `${jobLabel} step name`);
  const start = parseTimestamp(step.started_at, `${jobLabel}/${name} started_at`, { optional: true });
  const end = parseTimestamp(step.completed_at, `${jobLabel}/${name} completed_at`, { optional: true });
  return {
    number: Number.isSafeInteger(step.number) ? step.number : null,
    name,
    status: typeof step.status === 'string' ? step.status : 'unknown',
    conclusion: typeof step.conclusion === 'string' ? step.conclusion : null,
    started_at: step.started_at ?? null,
    completed_at: step.completed_at ?? null,
    duration_seconds: round(secondsBetween(start, end, `${jobLabel}/${name}`)),
  };
}

function normalizeJob(job) {
  if (!isRecord(job)) fail('A job record is malformed.');
  const id = String(job.id ?? '');
  if (!/^\d+$/.test(id)) fail('A job id is not a decimal identifier.');
  const name = requireString(job.name, `job ${id} name`);
  const start = parseTimestamp(job.started_at, `job ${id} started_at`, { optional: true });
  const end = parseTimestamp(job.completed_at, `job ${id} completed_at`, { optional: true });
  if (job.steps !== undefined && !Array.isArray(job.steps)) fail(`job ${id} steps must be an array.`);
  const conclusion = typeof job.conclusion === 'string' ? job.conclusion.toLowerCase() : null;
  const resultStatus = conclusion ?? (typeof job.status === 'string' ? job.status.toLowerCase() : 'unknown');
  const excluded = resultStatus === 'skipped' || resultStatus === 'cancelled' || resultStatus === 'canceled';
  // GitHub can return inverted timestamps for a skipped node (it has no runner
  // occupancy). Keep that duration unknown and excluded instead of fabricating
  // a negative interval or rejecting otherwise complete run metadata.
  const duration = excluded ? null : secondsBetween(start, end, `job ${id}`);
  const runnerMinutesEligible = duration !== null && !excluded;

  return {
    id,
    name,
    status: typeof job.status === 'string' ? job.status.toLowerCase() : 'unknown',
    conclusion,
    runner_os: normalizeOs(job.runner_os),
    runner_name: typeof job.runner_name === 'string' ? job.runner_name : null,
    started_at: job.started_at ?? null,
    completed_at: job.completed_at ?? null,
    duration_seconds: round(duration),
    runner_minutes: runnerMinutesEligible ? round(duration / 60, 4) : 0,
    runner_minutes_included: runnerMinutesEligible,
    steps: (job.steps ?? []).map((step) => normalizeStep(step, `job ${id}`)),
  };
}

function validatePages(pages, runId) {
  if (!Array.isArray(pages) || pages.length === 0 || pages.length > MAX_PAGES) {
    fail(`Run ${runId} must include between 1 and ${MAX_PAGES} job pages.`);
  }
  const firstCount = pages[0]?.total_count;
  if (!Number.isSafeInteger(firstCount) || firstCount < 0) fail(`Run ${runId} has an invalid jobs total_count.`);

  const jobs = [];
  for (let index = 0; index < pages.length; index += 1) {
    const page = pages[index];
    if (!isRecord(page) || page.page !== index + 1 || page.total_count !== firstCount || !Array.isArray(page.jobs)) {
      fail(`Run ${runId} job pagination is malformed or incomplete at page ${index + 1}.`);
    }
    jobs.push(...page.jobs);
  }
  if (jobs.length !== firstCount) {
    fail(`Run ${runId} job pagination is partial: expected ${firstCount}, received ${jobs.length}.`);
  }
  const ids = new Set();
  for (const job of jobs) {
    const id = String(job?.id ?? '');
    if (!/^\d+$/.test(id) || ids.has(id)) fail(`Run ${runId} has a missing or duplicate job id.`);
    ids.add(id);
  }
  return jobs;
}

export function normalizeSample(sample) {
  if (!isRecord(sample) || !isRecord(sample.run)) fail('A sample must contain run metadata.');
  const run = sample.run;
  const runId = String(run.id ?? '');
  if (!/^\d+$/.test(runId)) fail('A run id is not a decimal identifier.');
  const category = requireString(sample.category, `run ${runId} category`);
  if (!CATEGORY_PATTERN.test(category)) fail(`Run ${runId} category contains unsupported characters.`);
  const repo = requireString(sample.repo, `run ${runId} repository`);
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo)) fail(`Run ${runId} repository is invalid.`);
  const headSha = requireString(run.head_sha, `run ${runId} head_sha`);
  if (!SHA_PATTERN.test(headSha)) fail(`Run ${runId} head_sha must be a full 40-character SHA.`);
  const event = requireString(run.event, `run ${runId} event`);
  const attempt = run.run_attempt;
  if (!Number.isSafeInteger(attempt) || attempt < 1) fail(`Run ${runId} run_attempt must be a positive integer.`);
  const createdMs = parseTimestamp(run.created_at, `run ${runId} created_at`);
  const startedMs = parseTimestamp(run.run_started_at, `run ${runId} run_started_at`, { optional: true });
  const updatedMs = parseTimestamp(run.updated_at, `run ${runId} updated_at`, { optional: true });
  const explicitCompletedMs = parseTimestamp(run.completed_at, `run ${runId} completed_at`, { optional: true });
  const rawJobs = validatePages(sample.job_pages, runId);
  const jobs = rawJobs.map(normalizeJob);
  const conclusion = typeof run.conclusion === 'string' ? run.conclusion.toLowerCase() : null;
  const status = typeof run.status === 'string' ? run.status.toLowerCase() : 'unknown';
  const failedJobConclusions = new Set(['failure', 'timed_out', 'action_required']);
  const contradictoryJobs = status === 'completed' && conclusion === 'success'
    ? jobs.filter((job) => failedJobConclusions.has(job.conclusion))
    : [];
  if (contradictoryJobs.length > 0) {
    const details = contradictoryJobs.map((job) => `${job.id}:${job.conclusion}`).join(', ');
    fail(`Run ${runId} reports success but has conflicting failed job conclusion(s): ${details}.`);
  }

  const jobCompletionTimes = jobs
    .map((job) => parseTimestamp(job.completed_at, `job ${job.id} completed_at`, { optional: true }))
    .filter((value) => value !== null);
  const latestJobCompletion = jobCompletionTimes.length ? Math.max(...jobCompletionTimes) : null;
  const isComplete = status === 'completed' || conclusion !== null;
  const completedMs = explicitCompletedMs ?? (isComplete ? updatedMs ?? latestJobCompletion : null);
  const completedAtSource = explicitCompletedMs !== null
    ? 'workflow_run.completed_at'
    : updatedMs !== null && isComplete
      ? 'workflow_run.updated_at'
      : latestJobCompletion !== null && isComplete
        ? 'latest_job.completed_at'
        : null;
  const jobSeconds = jobs.reduce((sum, job) => sum + (job.runner_minutes_included ? job.duration_seconds : 0), 0);
  const osSeconds = (os) => jobs.reduce((sum, job) => sum + (job.runner_minutes_included && job.runner_os === os ? job.duration_seconds : 0), 0);
  const result = {
    run_id: runId,
    category,
    repo,
    html_url: `https://github.com/${repo}/actions/runs/${runId}`,
    workflow_id: Number.isSafeInteger(run.workflow_id) ? run.workflow_id : null,
    workflow_name: typeof run.name === 'string' ? run.name : null,
    head_sha: headSha.toLowerCase(),
    event,
    conclusion,
    status,
    run_attempt: attempt,
    created_at: run.created_at,
    run_started_at: run.run_started_at ?? null,
    completed_at: completedMs === null ? null : new Date(completedMs).toISOString(),
    completed_at_source: completedAtSource,
    queue_seconds: round(secondsBetween(createdMs, startedMs, `run ${runId} queue`)),
    wall_clock_seconds: round(secondsBetween(startedMs, completedMs, `run ${runId} wall clock`)),
    job_count: jobs.length,
    job_conclusions: jobs.reduce((counts, job) => {
      const key = job.conclusion ?? job.status;
      counts[key] = (counts[key] ?? 0) + 1;
      return counts;
    }, {}),
    runner_seconds: round(jobSeconds),
    runner_minutes: round(jobSeconds / 60, 4),
    runner_minutes_by_os: {
      windows: round(osSeconds('windows') / 60, 4),
      ubuntu: round(osSeconds('ubuntu') / 60, 4),
      other: round(osSeconds('other') / 60, 4),
      unknown: round(osSeconds('unknown') / 60, 4),
    },
    jobs,
    critical_path_observation: (() => {
      const longest = jobs.filter((job) => job.duration_seconds !== null)
        .toSorted((left, right) => right.duration_seconds - left.duration_seconds)[0];
      if (!longest) return null;
      const longestStep = longest.steps.filter((step) => step.duration_seconds !== null)
        .toSorted((left, right) => right.duration_seconds - left.duration_seconds)[0] ?? null;
      return {
        job_id: longest.id,
        job_name: longest.name,
        job_seconds: longest.duration_seconds,
        longest_step_name: longestStep?.name ?? null,
        longest_step_seconds: longestStep?.duration_seconds ?? null,
        method: 'longest observed job and its longest observed step; workflow dependency DAG is not exposed by the collected endpoint',
      };
    })(),
    failures: jobs.filter((job) => job.conclusion === 'failure' || job.conclusion === 'timed_out' || job.conclusion === 'action_required')
      .map((job) => ({
        job_id: job.id,
        job_name: job.name,
        conclusion: job.conclusion,
        classification: job.name === 'Sky Auto Player — required CI gate' ? 'aggregate_required_gate' : 'failed_job_details_require_logs',
        html_url: `${run.html_url ?? `https://github.com/${repo}/actions/runs/${runId}`}/job/${job.id}`,
      })),
    sccache_stats: {
      available: false,
      reason: 'Actions run/job metadata does not contain the step-summary body; job logs were not downloaded.',
    },
  };
  return result;
}

function percentile(sorted, value) {
  const rank = (sorted.length - 1) * value;
  const lower = Math.floor(rank);
  const upper = Math.ceil(rank);
  if (lower === upper) return sorted[lower];
  return sorted[lower] + (sorted[upper] - sorted[lower]) * (rank - lower);
}

export function summarizeSamples(runs) {
  const groups = new Map();
  for (const run of runs) {
    const key = [run.category, run.event, run.workflow_id ?? 'unknown-workflow', run.conclusion ?? 'incomplete'].join('|');
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(run);
  }
  return [...groups.entries()].map(([key, samples]) => {
    const comparable = samples.map((sample) => sample.wall_clock_seconds).filter((value) => value !== null).sort((a, b) => a - b);
    return {
      group: key,
      sample_count: comparable.length,
      run_ids: samples.map((sample) => sample.run_id),
      wall_clock_seconds_p50: comparable.length >= 5 ? round(percentile(comparable, 0.5)) : null,
      wall_clock_seconds_p95: comparable.length >= 20 ? round(percentile(comparable, 0.95)) : null,
      availability: comparable.length >= 20 ? 'p50 and p95' : comparable.length >= 5 ? 'p50 only; p95 requires 20 comparable samples' : 'omitted; p50 requires 5 comparable samples and p95 requires 20',
    };
  });
}

export function parseSnapshotText(text) {
  if (Buffer.byteLength(text, 'utf8') > MAX_INPUT_BYTES) fail(`Input exceeds the ${MAX_INPUT_BYTES}-byte safety limit.`);
  let snapshot;
  try {
    snapshot = JSON.parse(text);
  } catch {
    fail('Input is not valid JSON.');
  }
  if (!isRecord(snapshot) || snapshot.schema_version !== 1 || !Array.isArray(snapshot.samples)) {
    fail('Input must be a ci-metrics snapshot with schema_version 1 and a samples array.');
  }
  if (snapshot.samples.length === 0 || snapshot.samples.length > 500) fail('Input must contain between 1 and 500 samples.');
  const ids = new Set();
  const runs = snapshot.samples.map(normalizeSample);
  const snapshotRepo = snapshot.repo === undefined ? runs[0].repo : requireString(snapshot.repo, 'snapshot repository');
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(snapshotRepo) || runs.some((run) => run.repo !== snapshotRepo)) {
    fail('Snapshot repository does not match its run samples.');
  }
  for (const run of runs) {
    const key = `${run.repo}/${run.run_id}/${run.run_attempt}`;
    if (ids.has(key)) fail(`Input contains a duplicate run attempt: ${key}.`);
    ids.add(key);
  }
  return { schema_version: 1, repo: snapshotRepo, runs, comparable_samples: summarizeSamples(runs) };
}

function parseArgs(argv) {
  const options = { runs: [], input: null, outputDir: null, repo: 'pumni/Sky-Auto-Player' };
  let pendingRun = null;
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--help' || arg === '-h') return { help: true };
    if (arg === '--repo' || arg === '--input' || arg === '--output-dir' || arg === '--run-id' || arg === '--category') {
      const value = argv[index + 1];
      if (!value || value.startsWith('--')) fail(`${arg} requires a value.`);
      index += 1;
      if (arg === '--repo') options.repo = value;
      if (arg === '--input') options.input = value;
      if (arg === '--output-dir') options.outputDir = value;
      if (arg === '--run-id') {
        if (pendingRun !== null) fail(`Run ${pendingRun.id} is missing --category.`);
        pendingRun = { id: value };
      }
      if (arg === '--category') {
        if (pendingRun === null) fail('--category must immediately follow a --run-id.');
        pendingRun.category = value;
        options.runs.push(pendingRun);
        pendingRun = null;
      }
      continue;
    }
    fail(`Unknown argument: ${arg}`);
  }
  if (pendingRun !== null) fail(`Run ${pendingRun.id} is missing --category.`);
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(options.repo)) fail('--repo must be owner/name.');
  if (!options.outputDir) fail('--output-dir is required.');
  if ((options.input === null) === (options.runs.length === 0)) fail('Provide either --input or one or more --run-id/--category pairs.');
  if (options.input !== null && options.runs.length) fail('--input cannot be combined with --run-id.');
  const seen = new Set();
  for (const run of options.runs) {
    if (!/^\d+$/.test(run.id) || !CATEGORY_PATTERN.test(run.category)) fail('Run IDs and categories must use their documented formats.');
    if (seen.has(run.id)) fail(`Duplicate --run-id ${run.id}.`);
    seen.add(run.id);
  }
  return options;
}

function ghJson(endpoint) {
  const result = spawnSync('gh', ['api', endpoint], {
    encoding: 'utf8',
    windowsHide: true,
    maxBuffer: MAX_INPUT_BYTES,
  });
  if (result.error || result.status !== 0) fail(`GitHub API request failed for ${endpoint}; authenticate with gh and retry. Response details were suppressed.`);
  if (Buffer.byteLength(result.stdout, 'utf8') > MAX_INPUT_BYTES) fail('GitHub API response exceeded the safety limit.');
  try {
    return JSON.parse(result.stdout);
  } catch {
    fail(`GitHub API returned invalid JSON for ${endpoint}.`);
  }
}

function collectRun(repo, runId, category) {
  const run = ghJson(`repos/${repo}/actions/runs/${runId}`);
  if (String(run.id) !== runId) fail(`GitHub returned a different run for requested ID ${runId}.`);
  const jobPages = [];
  let expectedCount = null;
  for (let page = 1; page <= MAX_PAGES; page += 1) {
    const response = ghJson(`repos/${repo}/actions/runs/${runId}/jobs?per_page=100&page=${page}`);
    if (!isRecord(response) || !Number.isSafeInteger(response.total_count) || !Array.isArray(response.jobs)) {
      fail(`GitHub returned malformed job page ${page} for run ${runId}.`);
    }
    if (expectedCount !== null && response.total_count !== expectedCount) fail(`Job count changed during collection for run ${runId}; retry the snapshot.`);
    expectedCount = response.total_count;
    jobPages.push({ page, total_count: response.total_count, jobs: response.jobs });
    const collected = jobPages.reduce((sum, item) => sum + item.jobs.length, 0);
    if (collected === expectedCount) break;
    if (response.jobs.length === 0 || page === MAX_PAGES) fail(`GitHub job pagination was partial for run ${runId}; refusing to emit incomplete metrics.`);
  }
  return { repo, category, run, job_pages: jobPages };
}

function markdown(report) {
  const cell = (value) => String(value ?? 'n/a').replace(/[|\r\n]/g, ' ').replace(/`/g, "'");
  const lines = [
    '# GitHub Actions CI measurements',
    '',
    `Repository: \`${report.repo}\`. Job minutes are elapsed runner occupancy summed across jobs, not GitHub billable minutes or OS-weighted billing units.`,
    '',
    '| Run | Category | Event | Attempt | Result | Head SHA | Wall (s) | Runner (min) | Windows (min) | Ubuntu (min) | Unknown OS (min) |',
    '| --- | --- | --- | ---: | --- | --- | ---: | ---: | ---: | ---: | ---: |',
  ];
  for (const run of report.runs) {
    lines.push(`| [${run.run_id}](${run.html_url}) | ${cell(run.category)} | ${cell(run.event)} | ${run.run_attempt} | ${cell(run.conclusion ?? run.status)} | \`${run.head_sha}\` | ${run.wall_clock_seconds ?? 'n/a'} | ${run.runner_minutes} | ${run.runner_minutes_by_os.windows} | ${run.runner_minutes_by_os.ubuntu} | ${run.runner_minutes_by_os.unknown} |`);
  }
  lines.push('', '## Run details', '');
  for (const run of report.runs) {
    lines.push(`### Run ${run.run_id} — ${run.category}`, '');
    lines.push(`- Source: [GitHub Actions run](${run.html_url})`);
    lines.push(`- Created: ${run.created_at}; started: ${run.run_started_at ?? 'unknown'}; completed: ${run.completed_at ?? 'unknown'} (${run.completed_at_source ?? 'source unavailable'})`);
    lines.push(`- Jobs: ${run.job_count}; conclusions: ${JSON.stringify(run.job_conclusions)}; failure records: ${run.failures.length}`);
    lines.push(`- Observed longest job: ${run.critical_path_observation ? `${cell(run.critical_path_observation.job_name)} (${run.critical_path_observation.job_seconds}s), longest step ${cell(run.critical_path_observation.longest_step_name)} (${run.critical_path_observation.longest_step_seconds ?? 'n/a'}s)` : 'unavailable'}. This is not a dependency-DAG critical path.`);
    if (run.failures.length) lines.push(`- Failure details are classified from job conclusions only; inspect linked job logs for root cause.`);
    lines.push('');
    lines.push('| Job | OS | Conclusion/status | Duration (s) | Runner min included | Longest measured step (s) |', '| --- | --- | --- | ---: | --- | ---: |');
    for (const job of run.jobs) {
      const longestStep = job.steps.filter((step) => step.duration_seconds !== null)
        .reduce((current, step) => current === null || step.duration_seconds > current.duration_seconds ? step : current, null);
      lines.push(`| [${cell(job.name)}](${run.html_url}/job/${job.id}) | ${job.runner_os} | ${cell(job.conclusion ?? job.status)} | ${job.duration_seconds ?? 'n/a'} | ${job.runner_minutes_included ? 'yes' : 'no'} | ${longestStep?.duration_seconds ?? 'n/a'} |`);
    }
    lines.push('');
  }
  lines.push('## Comparable samples', '');
  for (const group of report.comparable_samples) {
    lines.push(`- \`${group.group}\`: n=${group.sample_count}; p50=${group.wall_clock_seconds_p50 ?? 'n/a'}s; p95=${group.wall_clock_seconds_p95 ?? 'n/a'}s (${group.availability}).`);
  }
  lines.push('', 'Runner OS is reported only when the API record contains an explicit OS value; otherwise it is unknown. sccache summaries are not exposed in run/job metadata, and logs were not downloaded.', '');
  return `${lines.join('\n')}\n`;
}

function printHelp() {
  return [
    'Read-only GitHub Actions run metrics; writes ci-metrics.json and ci-metrics.md.',
    '  node scripts/ci_metrics.mjs --repo owner/name --run-id ID --category CATEGORY [--run-id ID --category CATEGORY ...] --output-dir DIR',
    '  node scripts/ci_metrics.mjs --input snapshot.json --output-dir DIR',
    'The offline snapshot schema is {"schema_version":1,"repo":"owner/name","samples":[{"repo":"owner/name","category":"...","run":{},"job_pages":[{"page":1,"total_count":N,"jobs":[]}]}]}.',
  ].join('\n');
}

export function main(argv) {
  const options = parseArgs(argv);
  if (options.help) {
    process.stdout.write(`${printHelp()}\n`);
    return;
  }
  let report;
  if (options.input) {
    let input;
    try {
      const buffer = readFileSync(options.input);
      if (buffer.byteLength > MAX_INPUT_BYTES) fail(`Input exceeds the ${MAX_INPUT_BYTES}-byte safety limit.`);
      input = buffer.toString('utf8');
    } catch (error) {
      if (error instanceof Error && error.message.includes('safety limit')) throw error;
      fail('Unable to read the offline JSON input.');
    }
    report = parseSnapshotText(input);
  } else {
    const samples = options.runs.map(({ id, category }) => collectRun(options.repo, id, category));
    report = parseSnapshotText(JSON.stringify({ schema_version: 1, repo: options.repo, samples }));
  }
  const outputDir = resolve(options.outputDir);
  mkdirSync(outputDir, { recursive: true });
  writeFileSync(resolve(outputDir, 'ci-metrics.json'), `${JSON.stringify(report, null, 2)}\n`, 'utf8');
  writeFileSync(resolve(outputDir, 'ci-metrics.md'), markdown(report), 'utf8');
  process.stdout.write(`Wrote ${report.runs.length} run(s) to ${outputDir}.\n`);
}

const currentPath = process.argv[1] ? resolve(process.argv[1]) : null;
const thisPath = fileURLToPath(import.meta.url);
if (currentPath === thisPath) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`ci-metrics: ${error instanceof Error ? error.message : 'unknown error'}\n`);
    process.exitCode = 1;
  }
}
