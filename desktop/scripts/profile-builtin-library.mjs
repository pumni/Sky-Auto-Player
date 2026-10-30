import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createServer } from 'node:net';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { cpus, release, tmpdir } from 'node:os';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

/* global window, document, location, HTMLElement, requestAnimationFrame */

// Reproducible Windows/WebView2 library profile. It launches only the selected
// repository shell, writes fixtures only under a new temporary app-data root,
// invokes catalog commands, and never starts playback or sends input.
const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const options = Object.fromEntries(
  process.argv.slice(2).map((arg) => arg.replace(/^--/, '').split('=')),
);
const exe = resolve(options.exe ?? join(root, 'rust/target/debug/sky_desktop_shell.exe'));
const output = resolve(options.output ?? join(tmpdir(), 'sky-builtin-library-profile.json'));
if (!existsSync(exe)) throw new Error(`Tauri shell is missing: ${exe}`);
if (existsSync(output)) throw new Error(`Refusing to overwrite existing evidence: ${output}`);
mkdirSync(dirname(output), { recursive: true });

const sourceHead = spawnSync('git', ['rev-parse', 'HEAD'], {
  cwd: root,
  encoding: 'utf8',
}).stdout.trim();
const sourceBranch = spawnSync('git', ['branch', '--show-current'], {
  cwd: root,
  encoding: 'utf8',
}).stdout.trim();
const sourceStatus = spawnSync('git', ['status', '--porcelain=v1'], {
  cwd: root,
  encoding: 'utf8',
});
if (sourceStatus.status !== 0) {
  throw new Error(`Cannot verify source worktree status: ${sourceStatus.stderr.trim()}`);
}
if (sourceBranch !== 'feat/builtin-library-paging' || sourceStatus.stdout.trim()) {
  throw new Error(
    `Profile requires a clean committed feat/builtin-library-paging worktree; branch=${sourceBranch} status=${sourceStatus.stdout.trim()}`,
  );
}
const exeSha256 = createHash('sha256').update(readFileSync(exe)).digest('hex');
if (process.platform === 'win32') {
  const activeShells = spawnSync(
    'pwsh',
    [
      '-NoProfile',
      '-Command',
      "Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'sky_desktop_shell.exe' } | Select-Object ProcessId,ExecutablePath | ConvertTo-Json -Compress",
    ],
    { encoding: 'utf8', windowsHide: true },
  );
  if (activeShells.status !== 0) {
    throw new Error(`Cannot verify that no Sky shell is active: ${activeShells.stderr.trim()}`);
  }
  if (activeShells.stdout.trim()) {
    throw new Error(
      `Refusing isolated profile while an existing Sky shell is active: ${activeShells.stdout.trim()}`,
    );
  }
}
const widths = [96, 1_000, 10_000];
const samples = 21;
const delay = (ms) => new Promise((done) => setTimeout(done, ms));

function distribution(values) {
  const sorted = values.toSorted((left, right) => left - right);
  const at = (fraction) =>
    sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * fraction))];
  return {
    count: sorted.length,
    p50_ms: Number(at(0.5).toFixed(3)),
    p90_ms: Number(at(0.9).toFixed(3)),
    min_ms: Number(sorted[0].toFixed(3)),
    max_ms: Number(sorted.at(-1).toFixed(3)),
  };
}

async function availablePort() {
  const server = createServer();
  await new Promise((done, fail) => {
    server.once('error', fail);
    server.listen(0, '127.0.0.1', done);
  });
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('Could not reserve a loopback port');
  await new Promise((done, fail) => server.close((error) => (error ? fail(error) : done())));
  const probe = await fetch(`http://127.0.0.1:${address.port}/json/version`, {
    signal: AbortSignal.timeout(500),
  }).then(
    () => true,
    () => false,
  );
  if (probe) throw new Error('Reserved WebView2 debugging port is already in use');
  return address.port;
}

function processMemory(pid) {
  if (process.platform !== 'win32') return { available: false, reason: 'Windows-only' };
  const command = `$p=Get-Process -Id ${pid}; [pscustomobject]@{working_set_bytes=$p.WorkingSet64; private_bytes=$p.PrivateMemorySize64; handles=$p.HandleCount; threads=$p.Threads.Count} | ConvertTo-Json -Compress`;
  const result = spawnSync('pwsh', ['-NoProfile', '-Command', command], {
    encoding: 'utf8',
    windowsHide: true,
  });
  if (result.status !== 0) return { available: false, error: result.stderr.trim() };
  return JSON.parse(result.stdout.trim());
}

async function invoke(page, command, params) {
  return page.evaluate(
    ({ commandName, commandParams }) =>
      window.__TAURI_INTERNALS__.invoke(commandName, { params: commandParams }),
    { commandName: command, commandParams: params },
  );
}

async function invokeTiming(page, command, params, count = 1) {
  return page.evaluate(
    async ({ commandName, commandParams, count: iterations }) => {
      const timings = [];
      let response;
      for (let index = 0; index < iterations; index += 1) {
        const started = performance.now();
        response = await window.__TAURI_INTERNALS__.invoke(commandName, {
          params: commandParams,
        });
        timings.push(performance.now() - started);
      }
      return { timings, response };
    },
    { commandName: command, commandParams: params, count },
  );
}

function validateViewportResponse(response, request, expectedIds, phase) {
  const width = request.lastIndex - request.firstIndex + 1;
  if (!response?.accepted) throw new Error(`${phase} viewport response was not accepted`);
  if (
    response.generation !== request.generation ||
    response.first_index !== request.firstIndex ||
    response.last_index !== request.lastIndex ||
    response.selected_song_id !== request.selectedSongId
  ) {
    throw new Error(`${phase} viewport response identity did not match its request`);
  }
  const actualIds = response.items.map((item) => item.song_id);
  if (width !== expectedIds.length || actualIds.length !== width) {
    throw new Error(
      `${phase} viewport response width mismatch: expected ${width}, got ${actualIds.length}`,
    );
  }
  if (actualIds.some((songId, index) => songId !== expectedIds[index])) {
    throw new Error(`${phase} viewport response ordered IDs did not match its request`);
  }
  if (new Set(actualIds).size !== actualIds.length) {
    throw new Error(`${phase} viewport response contained duplicate song IDs`);
  }
  return actualIds;
}

async function pageMetrics(page, cdp, shellPid) {
  const [heap, performanceMetrics, dom, shell] = await Promise.all([
    cdp.send('Runtime.getHeapUsage').catch(() => null),
    cdp.send('Performance.getMetrics').catch(() => null),
    page.evaluate(() => ({
      nodes: document.querySelectorAll('*').length,
      rows: document.querySelectorAll('.track-table-row').length,
      scrollHeight: document.querySelector('.track-table')?.scrollHeight ?? 0,
      clientHeight: document.querySelector('.track-table')?.clientHeight ?? 0,
      scrollTop: document.querySelector('.track-table')?.scrollTop ?? 0,
    })),
    processMemory(shellPid),
  ]);
  const metricMap = Object.fromEntries(
    (performanceMetrics?.metrics ?? []).map(({ name, value }) => [name, value]),
  );
  return {
    v8_heap_bytes: heap?.usedSize ?? null,
    js_heap_used_bytes: metricMap.JSHeapUsedSize ?? null,
    dom_nodes: dom.nodes,
    mounted_track_rows: dom.rows,
    table_scroll_height_px: dom.scrollHeight,
    table_client_height_px: dom.clientHeight,
    table_scroll_top_px: dom.scrollTop,
    shell_process: shell,
  };
}

function fixtureRow(index) {
  const number = String(index + 1).padStart(5, '0');
  return JSON.stringify({
    name: `Package D fixture song ${number}`,
    songNotes: [{ time: 0, key: '1Key0' }],
  });
}

function ensureOwnedTempPath(path) {
  const tempRoot = resolve(tmpdir());
  const absolute = resolve(path);
  const rel = relative(tempRoot, absolute);
  if (
    !rel ||
    rel === '..' ||
    rel.startsWith(`..${sep}`) ||
    !rel.startsWith('sky-builtin-library-')
  ) {
    throw new Error(`Refusing to remove a path outside this harness temp root: ${absolute}`);
  }
}

const report = {
  source_head: sourceHead,
  source_branch: sourceBranch,
  source_worktree_clean: true,
  build_command:
    'cargo +1.99.1 build --manifest-path desktop/src-tauri/Cargo.toml --locked --features packaged-assets --bin sky_desktop_shell',
  executable: exe,
  executable_sha256: exeSha256,
  captured_at_utc: new Date().toISOString(),
  host: {
    platform: process.platform,
    os_release: release(),
    arch: process.arch,
    cpu: cpus()[0]?.model ?? 'unknown',
    logical_processors: cpus().length,
  },
  harness:
    'Windows Tauri debug shell with packaged-assets and WebView2; 21 sequential exact-warm samples plus 21 alternating accepted base→shift+1→base pairs per available window; explicit ordered song IDs; no playback or gameplay input',
  fixture:
    'Package D fixture v1: one deterministic one-note JSON user song per row; 96/1000/10000 user rows; built-in resource remains the verified 96-song catalog',
  sizes: [],
  cleanup_issues: [],
};

function writeEvidence() {
  const contents = `${JSON.stringify(report, null, 2)}\n`;
  if (!existsSync(output)) {
    writeFileSync(output, contents, { flag: 'wx' });
    return;
  }
  const previous = JSON.parse(readFileSync(output, 'utf8'));
  if (previous.source_head !== sourceHead || previous.executable_sha256 !== exeSha256) {
    throw new Error(`Refusing to update evidence owned by another run: ${output}`);
  }
  writeFileSync(output, contents);
}

for (const size of widths) {
  const port = await availablePort();
  const sandbox = mkdtempSync(join(tmpdir(), 'sky-builtin-library-'));
  ensureOwnedTempPath(sandbox);
  const appData = join(sandbox, 'appdata');
  const userSongs = join(appData, 'songs');
  mkdirSync(userSongs, { recursive: true });
  mkdirSync(join(sandbox, 'webview'), { recursive: true });
  for (let index = 0; index < size; index += 1) {
    writeFileSync(
      join(userSongs, `Package D fixture song ${String(index + 1).padStart(5, '0')}.json`),
      fixtureRow(index),
    );
  }
  writeFileSync(
    join(appData, 'config.json'),
    JSON.stringify({ schema_version: 3, palette: 'graphite', update: { auto_check: false } }),
  );
  writeFileSync(
    join(appData, 'library-manifest.json'),
    JSON.stringify({ version: 1, imports: [], collections: [] }),
  );

  const native = spawn(exe, [], {
    cwd: root,
    windowsHide: true,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: {
      ...process.env,
      SKY_INSTALL_ROOT: root,
      SKY_APP_DATA_ROOT: appData,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
      WEBVIEW2_USER_DATA_FOLDER: join(sandbox, 'webview'),
    },
  });
  let nativeOutput = '';
  native.stdout.on('data', (chunk) => {
    nativeOutput = `${nativeOutput}${chunk}`.slice(-8_000);
  });
  native.stderr.on('data', (chunk) => {
    nativeOutput = `${nativeOutput}${chunk}`.slice(-8_000);
  });
  let browser;
  try {
    for (let attempt = 0; ; attempt += 1) {
      try {
        browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
        break;
      } catch (error) {
        if (attempt > 240 || native.exitCode !== null) {
          throw new Error(
            `Tauri WebView2 failed to start (exit=${native.exitCode}, output=${nativeOutput.trim()}): ${error}`,
            { cause: error },
          );
        }
        await delay(250);
      }
    }
    const page = browser.contexts()[0]?.pages()[0];
    if (!page) throw new Error('Tauri WebView2 page was not found');
    const pageErrors = [];
    page.on('pageerror', (error) => pageErrors.push(error.message));
    try {
      await page.locator('.workbench').waitFor({ timeout: 15_000 });
    } catch (error) {
      const state = await page
        .evaluate(() => ({
          href: location.href,
          title: document.title,
          body: document.body?.innerText.slice(0, 1_000) ?? '',
        }))
        .catch((diagnostic) => ({ error: String(diagnostic) }));
      throw new Error(
        `Tauri workbench did not become visible: page=${JSON.stringify(state)} pageErrors=${JSON.stringify(pageErrors)} shellOutput=${nativeOutput.trim()} cause=${error}`,
        { cause: error },
      );
    }
    if (!(await page.evaluate(() => !!window.__TAURI_INTERNALS__))) {
      throw new Error('Connected page is not the Tauri application');
    }
    await page.locator('.track-table').waitFor({ timeout: 60_000 });
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Performance.enable');

    const search = async (query, offset, source = { kind: 'smart', id: 'all' }) =>
      invoke(page, 'search_songs', {
        query,
        offset,
        limit: 100,
        generation: null,
        source,
      });
    let catalog;
    for (let attempt = 0; attempt < 60; attempt += 1) {
      try {
        catalog = await search('Package D fixture', 0);
        if (catalog.total === size) break;
      } catch {
        // The production catalog scanner may still be composing the fixture.
      }
      await delay(500);
    }
    if (catalog?.total !== size) {
      const diagnostic = await page.evaluate(async () => {
        const invoke = window.__TAURI_INTERNALS__.invoke;
        const [bootstrap, all, builtin] = await Promise.all([
          invoke('bootstrap'),
          invoke('search_songs', {
            params: {
              query: '',
              offset: 0,
              limit: 1,
              generation: null,
              source: { kind: 'smart', id: 'all' },
            },
          }),
          invoke('search_songs', {
            params: {
              query: '',
              offset: 0,
              limit: 1,
              generation: null,
              source: { kind: 'smart', id: 'builtin' },
            },
          }),
        ]);
        return {
          catalog_state: bootstrap.catalog_state,
          generation: bootstrap.catalog_generation,
          all_total: all.total,
          builtin_total: builtin.total,
          all_first_title: all.items[0]?.title ?? null,
        };
      });
      throw new Error(
        `Expected ${size} fixture songs; native fixture query reported ${catalog?.total}; diagnostic=${JSON.stringify(diagnostic)}`,
      );
    }
    const builtin = await search('', 0, { kind: 'smart', id: 'builtin' });
    if (builtin.total !== 96) throw new Error(`Expected 96 Built-in Songs; got ${builtin.total}`);

    const sizeReport = {
      fixture_user_songs: size,
      catalog_total: catalog.total + builtin.total,
      builtin_total: builtin.total,
      cold_start_fixture_search_ms: null,
      search: [],
      viewport: [],
      memory: {},
      frontend_scroll_sweep: null,
    };
    const cdpMetrics = await pageMetrics(page, cdp, native.pid);
    sizeReport.memory.after_native_catalog_load = cdpMetrics;

    for (const [queryName, query] of [
      ['empty', ''],
      ['short', 'Package'],
      ['fuzzy', 'Package D fixture'],
    ]) {
      const first = await invokeTiming(page, 'search_songs', {
        query,
        offset: 0,
        limit: 100,
        generation: catalog.generation,
        source: { kind: 'smart', id: 'all' },
      });
      if (first.response.total !== (query ? size : size + builtin.total)) {
        throw new Error(`${queryName} search total mismatch: ${first.response.total}`);
      }
      if (queryName === 'fuzzy')
        sizeReport.cold_start_fixture_search_ms = Number(first.timings[0].toFixed(3));
      const total = first.response.total;
      const last = Math.max(0, Math.floor(Math.max(0, total - 100) / 100) * 100);
      const middle = Math.floor((total - Math.min(total, 100)) / 200) * 100;
      const offsets = [...new Set([0, middle, last])];
      for (const offset of offsets) {
        const request = {
          query,
          offset,
          limit: 100,
          generation: catalog.generation,
          source: { kind: 'smart', id: 'all' },
        };
        const cold = await invokeTiming(page, 'search_songs', request);
        const warm = await invokeTiming(page, 'search_songs', request, samples);
        sizeReport.search.push({
          query: queryName,
          window: offset === 0 ? 'first' : offset === last ? 'last' : 'middle',
          offset,
          total,
          first_measured_ms: Number(cold.timings[0].toFixed(3)),
          warm: distribution(warm.timings),
        });

        const ids = warm.response.items.map((item) => item.song_id);
        if (ids.length === 0) continue;
        const viewportRequest = {
          generation: catalog.generation,
          firstIndex: offset,
          lastIndex: offset + ids.length - 1,
          selectedSongId: null,
          songIds: ids,
        };
        const coldViewport = await invokeTiming(page, 'set_library_viewport', viewportRequest);
        validateViewportResponse(coldViewport.response, viewportRequest, ids, 'cold');
        const warmViewport = await invokeTiming(
          page,
          'set_library_viewport',
          viewportRequest,
          samples,
        );
        validateViewportResponse(warmViewport.response, viewportRequest, ids, 'exact warm');
        const viewportCase = {
          query: queryName,
          window: offset === 0 ? 'first' : offset === last ? 'last' : 'middle',
          offset,
          rows: ids.length,
          window_width: ids.length,
          first_measured_ms: Number(coldViewport.timings[0].toFixed(3)),
          exact_warm: distribution(warmViewport.timings),
        };
        if (offset + ids.length >= total) {
          const unavailable = {
            status: 'unavailable',
            reason: 'one-row shift cannot preserve the original window width at the result-set end',
          };
          viewportCase.shift_pair_count = 0;
          viewportCase.shift_plus_one = unavailable;
          viewportCase.shift_back = unavailable;
        } else {
          const shiftedRows = await search(query, offset + 1);
          if (shiftedRows.total !== total || shiftedRows.items.length !== ids.length) {
            throw new Error(
              `Shifted search window changed total or width at offset ${offset}: total=${shiftedRows.total} rows=${shiftedRows.items.length}`,
            );
          }
          const shiftedIds = shiftedRows.items.map((item) => item.song_id);
          const overlapWidth = ids.length - 1;
          if (
            overlapWidth <= 0 ||
            ids.slice(1).some((songId, index) => shiftedIds[index] !== songId)
          ) {
            throw new Error(
              `Shifted search window did not have the expected ordered overlap at offset ${offset}`,
            );
          }
          const shiftedRequest = {
            ...viewportRequest,
            firstIndex: offset + 1,
            lastIndex: offset + ids.length,
            songIds: shiftedIds,
          };
          const shiftTimings = [];
          const backTimings = [];
          for (let pair = 0; pair < samples; pair += 1) {
            const shift = await invokeTiming(page, 'set_library_viewport', shiftedRequest);
            validateViewportResponse(
              shift.response,
              shiftedRequest,
              shiftedIds,
              `shift ${pair + 1}`,
            );
            shiftTimings.push(shift.timings[0]);

            const back = await invokeTiming(page, 'set_library_viewport', viewportRequest);
            validateViewportResponse(back.response, viewportRequest, ids, `shift-back ${pair + 1}`);
            backTimings.push(back.timings[0]);
          }
          viewportCase.shift_pair_count = samples;
          viewportCase.ordered_overlap_rows = overlapWidth;
          viewportCase.shift_plus_one = distribution(shiftTimings);
          viewportCase.shift_back = distribution(backTimings);
        }
        sizeReport.viewport.push(viewportCase);
      }
    }

    const searchbox = page.getByLabel('Search All Songs');
    await searchbox.fill('Package D fixture');
    await page.waitForFunction(
      (expected) =>
        document.querySelector('.track-browser-count')?.textContent?.includes(`${expected} songs`),
      size,
      { timeout: 60_000 },
    );
    sizeReport.memory.before_frontend_scroll_sweep = await pageMetrics(page, cdp, native.pid);
    sizeReport.frontend_scroll_sweep = await page.evaluate(async (count) => {
      const table = document.querySelector('.track-table');
      if (!(table instanceof HTMLElement)) throw new Error('TrackTable viewport was not found');
      const targets = [];
      const maximum = Math.max(0, table.scrollHeight - table.clientHeight);
      for (let index = 0; index < count; index += 100) {
        const top = Math.min(maximum, index * 46);
        table.scrollTop = top;
        table.dispatchEvent(new Event('scroll', { bubbles: true }));
        targets.push(top);
        await new Promise(requestAnimationFrame);
      }
      table.scrollTop = 0;
      table.dispatchEvent(new Event('scroll', { bubbles: true }));
      await new Promise(requestAnimationFrame);
      return {
        jumps: targets.length,
        maximum_scroll_top_px: maximum,
        returned_to_start: table.scrollTop === 0,
      };
    }, size);
    await delay(500);
    sizeReport.memory.after_scroll_sweep_and_return = await pageMetrics(page, cdp, native.pid);
    await cdp.detach();
    report.sizes.push(sizeReport);
    console.log(
      `profiled fixture=${size} catalog_total=${sizeReport.catalog_total} builtin_total=${sizeReport.builtin_total} jumps=${sizeReport.frontend_scroll_sweep.jumps}`,
    );
  } finally {
    try {
      await browser?.close();
    } catch {
      // The browser transport may already be closed by the application.
    }
    if (native.exitCode === null) {
      const stopped = spawnSync('taskkill', ['/PID', String(native.pid), '/T', '/F'], {
        encoding: 'utf8',
        windowsHide: true,
      });
      if (stopped.status !== 0 && native.exitCode === null) native.kill();
      await Promise.race([new Promise((done) => native.once('exit', done)), delay(5_000)]);
    }
    ensureOwnedTempPath(sandbox);
    try {
      rmSync(sandbox, { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
    } catch (error) {
      report.cleanup_issues.push({
        sandbox,
        error: error instanceof Error ? `${error.name}: ${error.message}` : String(error),
      });
    }
  }
  writeEvidence();
}

console.log(`Wrote isolated profile evidence to ${output}`);
