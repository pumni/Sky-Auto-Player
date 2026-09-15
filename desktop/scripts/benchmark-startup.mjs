import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const defaultRuns = 10;
const defaultTimeoutMs = 60_000;
const requiredMarkers = [
  'process.entry',
  'tauri.builder.start',
  'tauri.setup.start',
  'tauri.setup.end',
  'frontend.entry',
  'react.initialize.start',
  'events.subscribe.start',
  'events.subscribe.end',
  'native.create.start',
  'native.create.end',
  'settings.load.start',
  'settings.load.end',
  'manifest.load.start',
  'manifest.load.end',
  'bootstrap.start',
  'bootstrap.end',
  'settings.reload.start',
  'settings.reload.end',
  'catalog.compose.start',
  'catalog.compose.end',
  'catalog.index.start',
  'catalog.index.end',
  'catalog.ready',
  'react.shell_ready',
  'react.catalog_ready',
];

function parseArgs(argv) {
  const options = {
    exe: null,
    installRoot: null,
    output: null,
    runs: defaultRuns,
    timeoutMs: defaultTimeoutMs,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = () => {
      index += 1;
      if (index >= argv.length) throw new Error(`Missing value for ${arg}`);
      return argv[index];
    };
    if (arg === '--exe') options.exe = next();
    else if (arg === '--install-root') options.installRoot = next();
    else if (arg === '--output') options.output = next();
    else if (arg === '--runs') options.runs = Number(next());
    else if (arg === '--timeout-ms') options.timeoutMs = Number(next());
    else if (arg === '--help') {
      console.log(
        'Usage: bun run benchmark:startup -- --exe <packaged-exe> [--install-root <root>] [--output <json>] [--runs 10]',
      );
      process.exit(0);
    } else throw new Error(`Unknown argument: ${arg}`);
  }
  if (!Number.isInteger(options.runs) || options.runs < 10) {
    throw new Error('--runs must be an integer of at least 10 measured runs');
  }
  if (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 5_000) {
    throw new Error('--timeout-ms must be an integer of at least 5000');
  }
  return options;
}

function findExecutable(explicit) {
  if (explicit) {
    const path = resolve(explicit);
    if (!existsSync(path)) throw new Error(`Packaged executable does not exist: ${path}`);
    return path;
  }
  const candidates = [
    join(desktopRoot, '..', 'rust', 'target', 'dist', 'sky_desktop_shell.exe'),
    join(desktopRoot, '..', 'rust', 'target', 'dist', 'sky-auto-player.exe'),
    join(desktopRoot, '..', 'rust', 'target', 'release', 'sky_desktop_shell.exe'),
  ];
  const path = candidates.find((candidate) => existsSync(candidate));
  if (!path) {
    throw new Error('Packaged executable not found; pass --exe <path> from the dist build.');
  }
  return resolve(path);
}

function writeSong(path, title) {
  writeFileSync(
    path,
    JSON.stringify({ name: title, songNotes: [{ time: 0, key: '1Key0' }] }),
    'utf8',
  );
}

function createFixtures(root) {
  const fixtures = {};
  const minimalRoot = join(root, 'minimal');
  mkdirSync(join(minimalRoot, 'songs'), { recursive: true });
  fixtures.minimal = { importedRoot: null, entries: 0, expectedImport: null };

  const representativeRoot = join(root, 'representative-import');
  for (const folder of ['alpha', 'beta'])
    mkdirSync(join(representativeRoot, folder), { recursive: true });
  for (let index = 0; index < 12; index += 1) {
    writeSong(
      join(representativeRoot, index % 2 === 0 ? 'alpha' : 'beta', `song-${index}.json`),
      `Representative ${index}`,
    );
  }
  fixtures.representative = {
    importedRoot: representativeRoot,
    entries: 12,
    expectedImport: { directories_visited: 3, files_visited: 12, supported_files: 12 },
  };

  const largeRoot = join(root, 'large-import');
  const directoryCount = 40;
  const filesPerDirectory = 32;
  for (let directory = 0; directory < directoryCount; directory += 1) {
    const directoryRoot = join(largeRoot, `directory-${directory}`);
    mkdirSync(directoryRoot, { recursive: true });
    for (let file = 0; file < filesPerDirectory; file += 1) {
      writeSong(join(directoryRoot, `song-${file}.json`), `Synthetic ${directory}-${file}`);
    }
  }
  fixtures.large = {
    importedRoot: largeRoot,
    entries: directoryCount * filesPerDirectory,
    expectedImport: {
      directories_visited: directoryCount + 1,
      files_visited: directoryCount * filesPerDirectory,
      supported_files: directoryCount * filesPerDirectory,
    },
  };
  return fixtures;
}

function createAppData(root, fixture, persistent) {
  mkdirSync(join(root, 'songs'), { recursive: true });
  if (!fixture.importedRoot) return;
  const manifest = {
    version: 1,
    imports: [
      {
        source_id: '0123456789abcdef0123456789abcdef',
        canonical_path: resolve(fixture.importedRoot),
        kind: 'folder',
      },
    ],
    collections: [],
  };
  writeFileSync(
    join(root, 'library-manifest.json'),
    `${JSON.stringify(manifest, null, 2)}\n`,
    'utf8',
  );
  if (!persistent) writeFileSync(join(root, 'config.json'), '{"schema_version":3}\n', 'utf8');
}

function waitForExit(child, timeoutMs) {
  return new Promise((resolvePromise, reject) => {
    let timer = setTimeout(() => {
      if (process.platform === 'win32' && child.pid) {
        spawnSync('taskkill', ['/pid', String(child.pid), '/t', '/f'], { stdio: 'ignore' });
      } else {
        child.kill('SIGTERM');
      }
      reject(new Error(`packaged startup run exceeded ${timeoutMs}ms`));
    }, timeoutMs);
    child.once('error', (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.once('exit', (code, signal) => {
      clearTimeout(timer);
      resolvePromise({ code, signal });
    });
  });
}

function assertSafeInteger(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`${label} must be a non-negative safe integer`);
  }
}

function assertMarkerOrder(first, markers, label) {
  let previousMarker = null;
  let previousElapsed = -1;
  for (const marker of markers) {
    const event = first.get(marker);
    if (!event) throw new Error(`${label} is missing ${marker}`);
    if (event.elapsed_us < previousElapsed) {
      throw new Error(`${label} is out of order: ${previousMarker} before ${marker}`);
    }
    previousMarker = marker;
    previousElapsed = event.elapsed_us;
  }
}

function validateTrace(events, fixture) {
  if (events.length === 0) throw new Error('telemetry run produced no events');
  const first = new Map();
  let previousElapsed = -1;
  for (const [index, event] of events.entries()) {
    assertSafeInteger(event.elapsed_us, `event ${index} elapsed_us`);
    if (event.elapsed_us < previousElapsed) {
      throw new Error(`telemetry elapsed_us is not monotonic at event ${index}`);
    }
    previousElapsed = event.elapsed_us;
    if (!first.has(event.marker)) first.set(event.marker, event);
  }

  const missing = requiredMarkers.filter((marker) => !first.has(marker));
  if (missing.length > 0) {
    throw new Error(`telemetry run is missing markers: ${missing.join(', ')}`);
  }

  assertMarkerOrder(
    first,
    ['process.entry', 'tauri.builder.start', 'tauri.setup.start', 'tauri.setup.end'],
    'Tauri setup markers',
  );
  assertMarkerOrder(
    first,
    [
      'events.subscribe.start',
      'events.subscribe.end',
      'native.create.start',
      'settings.load.start',
      'settings.load.end',
      'manifest.load.start',
      'manifest.load.end',
      'native.create.end',
    ],
    'native subscription markers',
  );
  assertMarkerOrder(
    first,
    [
      'bootstrap.start',
      'bootstrap.end',
      'catalog.compose.start',
      'catalog.compose.end',
      'catalog.index.start',
      'catalog.index.end',
      'catalog.ready',
    ],
    'bootstrap markers',
  );
  assertMarkerOrder(
    first,
    ['bootstrap.end', 'settings.reload.start', 'settings.reload.end'],
    'explicit settings refresh markers',
  );

  const frontendMarkers = [
    'frontend.entry',
    'react.initialize.start',
    'react.shell_ready',
    'react.catalog_ready',
  ];
  let previousFrontendElapsed = -1;
  for (const marker of frontendMarkers) {
    const event = first.get(marker);
    assertSafeInteger(event.frontend_elapsed_us, `${marker} frontend_elapsed_us`);
    if (event.frontend_elapsed_us < previousFrontendElapsed) {
      throw new Error(`frontend timestamps are not monotonic at ${marker}`);
    }
    previousFrontendElapsed = event.frontend_elapsed_us;
  }

  const sourceNames = [
    ...new Set(
      events
        .map((event) =>
          event.marker.match(/^(catalog\.(?:builtin|user|import\.source_[0-9]+))\.(?:start|end)$/),
        )
        .filter(Boolean)
        .map((match) => match[1]),
    ),
  ].sort();
  const expectedSourceNames = ['catalog.builtin', 'catalog.user'];
  if (fixture.expectedImport) expectedSourceNames.push('catalog.import.source_0');
  expectedSourceNames.sort();
  if (JSON.stringify(sourceNames) !== JSON.stringify(expectedSourceNames)) {
    throw new Error(
      `source marker set mismatch: expected ${expectedSourceNames.join(', ')}, got ${sourceNames.join(', ')}`,
    );
  }

  const sourceCounters = new Map();
  for (const sourceName of sourceNames) {
    const start = first.get(`${sourceName}.start`);
    const end = first.get(`${sourceName}.end`);
    if (!start || !end) throw new Error(`source ${sourceName} must have start and end markers`);
    if (end.elapsed_us < start.elapsed_us) {
      throw new Error(`source ${sourceName} markers are out of order`);
    }
    assertSafeInteger(end.duration_ms, `${sourceName}.end duration_ms`);
    for (const field of ['directories_visited', 'files_visited', 'supported_files']) {
      assertSafeInteger(end[field], `${sourceName}.end ${field}`);
    }
    if (end.supported_files > end.files_visited) {
      throw new Error(`${sourceName}.end supported_files exceeds files_visited`);
    }
    sourceCounters.set(sourceName, end);
  }

  const user = sourceCounters.get('catalog.user');
  if (user.directories_visited !== 1 || user.files_visited !== 0 || user.supported_files !== 0) {
    throw new Error('catalog.user counters do not match the empty user fixture');
  }
  if (fixture.expectedImport) {
    const imported = sourceCounters.get('catalog.import.source_0');
    for (const [field, expected] of Object.entries(fixture.expectedImport)) {
      if (imported[field] !== expected) {
        throw new Error(
          `catalog.import.source_0 ${field} expected ${expected}, got ${imported[field]}`,
        );
      }
    }
  }
  return { first, sourceCounters };
}

async function runOnce(exe, installRoot, appDataRoot, telemetryPath, fixture, timeoutMs) {
  const child = spawn(exe, ['--selftest-desktop-gui'], {
    cwd: installRoot,
    env: {
      ...process.env,
      SKY_INSTALL_ROOT: installRoot,
      SKY_APP_DATA_ROOT: appDataRoot,
      SKY_STARTUP_TELEMETRY_PATH: telemetryPath,
    },
    stdio: 'ignore',
    windowsHide: true,
  });
  const result = await waitForExit(child, timeoutMs);
  const events = readFileSync(telemetryPath, 'utf8')
    .trim()
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line) => JSON.parse(line));
  if (result.signal || result.code !== 0) {
    throw new Error(
      `packaged startup run failed with ${result.signal ?? `exit code ${result.code}`}`,
    );
  }
  const { first, sourceCounters } = validateTrace(events, fixture);
  const elapsed = (marker) => first.get(marker).elapsed_us;
  const duration = (start, end) => elapsed(end) - elapsed(start);
  const frontendElapsed = (marker) => first.get(marker).frontend_elapsed_us;
  const frontendDuration = (start, end) => frontendElapsed(end) - frontendElapsed(start);
  const catalogSources = [...sourceCounters.entries()].map(([source, event]) => ({
    marker: `${source}.end`,
    duration_ms: event.duration_ms,
    directories_visited: event.directories_visited,
    files_visited: event.files_visited,
    supported_files: event.supported_files,
  }));
  return {
    process_to_shell_ready_ms: duration('process.entry', 'react.shell_ready') / 1000,
    frontend_initialize_to_shell_ready_ms:
      frontendDuration('react.initialize.start', 'react.shell_ready') / 1000,
    native_create_ms: duration('native.create.start', 'native.create.end') / 1000,
    bootstrap_ms: duration('bootstrap.start', 'bootstrap.end') / 1000,
    catalog_compose_ms: duration('catalog.compose.start', 'catalog.compose.end') / 1000,
    catalog_index_ms: duration('catalog.index.start', 'catalog.index.end') / 1000,
    catalog_compose_to_index_ms: duration('catalog.compose.start', 'catalog.index.end') / 1000,
    catalog_ready_ms: elapsed('catalog.ready') / 1000,
    shell_ready_ms: elapsed('react.shell_ready') / 1000,
    catalog_reconciled_ms: elapsed('react.catalog_ready') / 1000,
    catalog_sources: catalogSources,
  };
}

function percentile(values, ratio) {
  const sorted = [...values].sort((left, right) => left - right);
  const index = Math.min(sorted.length - 1, Math.ceil(ratio * sorted.length) - 1);
  return sorted[index];
}

function summarize(samples) {
  const metricNames = Object.keys(samples[0]).filter((name) => name.endsWith('_ms'));
  return Object.fromEntries(
    metricNames.map((name) => {
      const values = samples.map((sample) => sample[name]);
      return [
        name,
        { median: percentile(values, 0.5), p95: percentile(values, 0.95), raw: values },
      ];
    }),
  );
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const exe = findExecutable(options.exe);
  const installRoot = resolve(options.installRoot ?? dirname(exe));
  const tempRoot = mkdtempSync(join(tmpdir(), 'sky-startup-benchmark-'));
  const fixtures = createFixtures(join(tempRoot, 'fixtures'));
  const report = {
    schema_version: 1,
    executable_name: basename(exe),
    methodology: {
      measured_runs_per_mode: options.runs,
      discarded_warmup_runs_per_mode: 1,
      coldish_definition:
        'Each run uses a fresh app-data root and a fresh manifest; Windows filesystem/OS caches are not cleared.',
      warm_definition:
        'Runs reuse one app-data root per fixture after a discarded warm-up; process restart remains part of every sample.',
      launch:
        '--selftest-desktop-gui with SKY_STARTUP_TELEMETRY_PATH; telemetry is independent of smoke phase logging.',
      fixtures: Object.fromEntries(
        Object.entries(fixtures).map(([name, fixture]) => [
          name,
          { imported_entries: fixture.entries },
        ]),
      ),
    },
    results: {},
  };

  try {
    for (const [fixtureName, fixture] of Object.entries(fixtures)) {
      report.results[fixtureName] = {};
      for (const mode of ['coldish', 'warm']) {
        const modeRoot = join(tempRoot, fixtureName, mode);
        mkdirSync(modeRoot, { recursive: true });
        const warmRoot = join(modeRoot, 'persistent-app-data');
        const samples = [];
        for (let run = 0; run <= options.runs; run += 1) {
          const appDataRoot = mode === 'warm' ? warmRoot : join(modeRoot, `run-${run}`);
          createAppData(appDataRoot, fixture, mode === 'warm');
          const telemetryPath = join(modeRoot, `telemetry-${run}.jsonl`);
          const sample = await runOnce(
            exe,
            installRoot,
            appDataRoot,
            telemetryPath,
            fixture,
            options.timeoutMs,
          );
          if (run > 0) samples.push({ run: run, ...sample });
        }
        report.results[fixtureName][mode] = {
          measured_runs: samples.length,
          summary: summarize(samples),
          samples,
        };
      }
    }
  } finally {
    rmSync(tempRoot, { recursive: true, force: true });
  }

  const encoded = `${JSON.stringify(report, null, 2)}\n`;
  if (options.output) writeFileSync(resolve(options.output), encoded, 'utf8');
  else process.stdout.write(encoded);
}

await main();
