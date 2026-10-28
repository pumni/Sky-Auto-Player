import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';
import { chromium } from 'playwright';
/* global window, document, innerWidth, devicePixelRatio, location, requestAnimationFrame, cancelAnimationFrame, KeyboardEvent */

// Manual Windows/WebView2 qualification, never a shared-CI FPS gate. Only the
// launched application is controlled. Pointer input uses Windows SendInput;
// the application's normal native bridge and dry-run playback remain active.
const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const options = Object.fromEntries(
  process.argv.slice(2).map((arg) => arg.replace(/^--/, '').split('=')),
);
const exe = resolve(options.exe ?? join(root, 'rust/target/debug/sky_desktop_shell.exe'));
const output = resolve(options.output ?? join(tmpdir(), 'sky-workbench-profile'));
const label = options.label ?? 'measurement';
const port = 9233;
mkdirSync(output, { recursive: true });
if (options.prepare === 'true') {
  const index = join(root, 'desktop/dist/index.html');
  const html = readFileSync(index, 'utf8');
  if (html.includes('data-resize-profile')) throw new Error('Profiling preload already installed');
  const preload = `<script data-resize-profile>
    window.__resizeCommits = 0;
    window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = { supportsFiber: true, inject: () => 1,
      onCommitFiberRoot: () => { window.__resizeCommits++; }, onCommitFiberUnmount: () => {} };
    localStorage.setItem('sky.ui.workbench.v5', JSON.stringify({ version: 5, navigatorPreference: 'expanded', expandedNavigatorWidth: 240, utilityWidth: 280, utilityOpen: true }));
  </script>`;
  writeFileSync(index, html.replace('<head>', `<head>${preload}`));
  console.log(
    'Installed measurement-only preload in dist/index.html; rebuild native executable before profiling.',
  );
  process.exit(0);
}
const sourceHead = spawnSync('git', ['rev-parse', 'HEAD'], {
  cwd: root,
  encoding: 'utf8',
}).stdout.trim();
const exeSha256 = createHash('sha256').update(readFileSync(exe)).digest('hex');

const pointerProgram = String.raw`
$ErrorActionPreference = 'Stop'
$request = [Console]::In.ReadToEnd() | ConvertFrom-Json
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class WorkbenchPointer {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L,T,R,B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X,Y; }
  [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx,dy; public uint data,flags,time; public UIntPtr extra; }
  [StructLayout(LayoutKind.Explicit, Size=32)] public struct UNION { [FieldOffset(0)] public MOUSEINPUT mouse; }
  [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public UNION data; }
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr w,int x,int y,int width,int height,bool repaint);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr w);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr w,int mode);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr w,IntPtr after,int x,int y,int width,int height,uint flags);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT point);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window,out uint pid);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr w,ref POINT p);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
  [DllImport("user32.dll")] public static extern uint SendInput(uint count,INPUT[] inputs,int size);
  [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
  public static void Move(int x,int y,uint button) {
    var input = new INPUT { type=0, data=new UNION { mouse=new MOUSEINPUT {
      dx=(int)Math.Round(x*65535.0/(GetSystemMetrics(0)-1)),
      dy=(int)Math.Round(y*65535.0/(GetSystemMetrics(1)-1)),flags=0x8001|button } } };
    if(SendInput(1,new[]{input},Marshal.SizeOf<INPUT>())!=1) throw new Exception("SendInput failed");
  }
}
'@
[void][WorkbenchPointer]::SetThreadDpiAwarenessContext([IntPtr](-4))
$window = (Get-Process -Id $request.pid).MainWindowHandle
if ($window -eq 0) { throw 'The launched application has no window' }
[void][WorkbenchPointer]::ShowWindow($window,9)
[void][WorkbenchPointer]::SetWindowPos($window,[IntPtr]::Zero,0,0,0,0,0x13)
if ($request.mode -eq 'position') {
  [void][WorkbenchPointer]::MoveWindow($window,0,0,$request.width,$request.height,$true)
  [void][WorkbenchPointer]::SetForegroundWindow($window)
} else {
  [void][WorkbenchPointer]::SetForegroundWindow($window)
  # A normal activation click is permitted only when the visible titlebar point
  # belongs to this launched PID. Never click through another application.
  $activation = New-Object WorkbenchPointer+POINT
  $activation.X = 100; $activation.Y = 20
  [void][WorkbenchPointer]::ClientToScreen($window,[ref]$activation)
  [uint32]$activationPid = 0
  [void][WorkbenchPointer]::GetWindowThreadProcessId([WorkbenchPointer]::WindowFromPoint($activation),[ref]$activationPid)
  if ($activationPid -eq $request.pid) {
    [WorkbenchPointer]::Move($activation.X,$activation.Y,2)
    [WorkbenchPointer]::Move($activation.X,$activation.Y,4)
  }
  $focusWait = [Diagnostics.Stopwatch]::StartNew()
  do {
    [uint32]$foregroundPid = 0
    [void][WorkbenchPointer]::GetWindowThreadProcessId([WorkbenchPointer]::GetForegroundWindow(),[ref]$foregroundPid)
    if ($foregroundPid -eq $request.pid) { break }
    if ($focusWait.ElapsedMilliseconds -gt 60000) { throw 'Cannot safely focus the launched profile window' }
    Start-Sleep -Milliseconds 50
  } while ($true)
  $origin = New-Object WorkbenchPointer+POINT
  [void][WorkbenchPointer]::ClientToScreen($window,[ref]$origin)
  $startX = $origin.X + [int]$request.x
  $y = $origin.Y + [int]$request.y
  [WorkbenchPointer]::Move($startX,$y,0)
  Start-Sleep -Milliseconds 100
  [WorkbenchPointer]::Move($startX,$y,2)
  $timer = [Diagnostics.Stopwatch]::StartNew()
  try {
    while ($timer.ElapsedMilliseconds -lt $request.duration) {
      $phase = $timer.Elapsed.TotalMilliseconds / $request.duration
      $triangle = if ($phase -lt 0.5) { $phase*2 } else { (1-$phase)*2 }
      [WorkbenchPointer]::Move(($startX + [int]($request.delta*$triangle)),$y,0)
      Start-Sleep -Milliseconds 4
    }
  } finally { [WorkbenchPointer]::Move($startX,$y,4) }
}
`;

async function pointer(request) {
  const child = spawn(
    'pwsh',
    ['-NoProfile', '-EncodedCommand', Buffer.from(pointerProgram, 'utf16le').toString('base64')],
    { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] },
  );
  let error = '';
  child.stderr.on('data', (chunk) => {
    error += chunk;
  });
  child.stdin.end(JSON.stringify(request));
  await new Promise((done, fail) => {
    child.on('error', fail);
    child.on('exit', (code) => (code === 0 ? done() : fail(new Error(error))));
  });
}
const delay = (ms) => new Promise((done) => setTimeout(done, ms));
function distribution(values) {
  const sorted = values.toSorted((a, b) => a - b);
  const percentile = (p) =>
    sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))] ?? null;
  return {
    count: sorted.length,
    p50: percentile(0.5),
    p95: percentile(0.95),
    p99: percentile(0.99),
    max: sorted.at(-1) ?? null,
  };
}

const previous =
  options.resume === 'true'
    ? JSON.parse(readFileSync(join(output, `${label}.json`), 'utf8'))
    : null;
if (previous && previous.exeSha256 !== exeSha256)
  throw new Error('Cannot resume measurements from a different executable');
const results = previous?.results ?? [];
const counts = options.counts ? options.counts.split(',').map(Number) : [12, 1280];
if (counts.some((count) => ![12, 1280].includes(count)))
  throw new Error('Only 12/1280 fixture sizes are supported');
for (const count of counts) {
  if (
    await fetch(`http://127.0.0.1:${port}/json/version`, {
      signal: AbortSignal.timeout(1000),
    }).then(
      () => true,
      () => false,
    )
  )
    throw new Error('Profiling port already in use; refusing to control an existing WebView');
  const sandbox = mkdtempSync(join(tmpdir(), 'sky-resize-native-'));
  const appData = join(sandbox, 'data');
  const songs = join(appData, 'songs');
  mkdirSync(appData);
  mkdirSync(songs);
  for (let index = 0; index < count; index++) {
    const songNotes =
      index === 0
        ? Array.from({ length: 480 }, (_, n) => ({ time: n * 250, key: '1Key0' }))
        : [{ time: 0, key: '1Key0' }];
    writeFileSync(
      join(songs, `Resize fixture ${String(index).padStart(4, '0')}.json`),
      JSON.stringify({ name: `Resize fixture ${String(index).padStart(4, '0')}`, songNotes }),
    );
  }
  writeFileSync(
    join(appData, 'config.json'),
    JSON.stringify({ schema_version: 3, palette: 'graphite' }),
  );
  writeFileSync(
    join(appData, 'library-manifest.json'),
    JSON.stringify({
      version: 1,
      imports: [],
      collections: [],
    }),
  );
  const native = spawn(exe, [], {
    cwd: root,
    windowsHide: true,
    stdio: 'ignore',
    env: {
      ...process.env,
      SKY_INSTALL_ROOT: root,
      SKY_APP_DATA_ROOT: appData,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
      WEBVIEW2_USER_DATA_FOLDER: join(sandbox, 'webview'),
    },
  });
  let browser;
  try {
    for (let attempt = 0; ; attempt++) {
      try {
        browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
        break;
      } catch (error) {
        if (attempt > 80 || native.exitCode !== null) throw error;
        await delay(250);
      }
    }
    const context = browser.contexts()[0];
    const page = context.pages()[0];
    await page.locator('.workbench').waitFor();
    if (!(await page.evaluate(() => !!window.__TAURI_INTERNALS__)))
      throw new Error('Not a native Tauri page');
    if (!(await page.evaluate(() => typeof window.__resizeCommits === 'number')))
      throw new Error('Rebuild with --prepare=true preload first');
    await page
      .getByRole('button', { name: 'Reload library' })
      .evaluate((element) => element.click());
    await delay(1500);
    await page.getByRole('searchbox').fill('Resize fixture');
    await delay(500);
    await delay(3000); // Allow the operator's initial focus click to finish before capture.
    console.log(
      'native setup',
      await page.evaluate(async () => ({
        input: document.querySelector('input[type="search"]').value,
        total: (
          await window.__TAURI_INTERNALS__.invoke('search_songs', {
            params: {
              query: 'Resize fixture',
              offset: 0,
              limit: 1,
              generation: null,
              source: { kind: 'smart', id: 'all' },
            },
          })
        ).total,
      })),
    );
    for (const playback of [false, true]) {
      for (const pane of ['inspector', 'navigator', 'clamp']) {
        const width = pane === 'clamp' ? 1024 : 1366;
        await pointer({
          mode: 'position',
          pid: native.pid,
          width: Math.round(width * 1.25),
          height: 950,
        });
        const actualWidth = await page.evaluate(() => innerWidth);
        await pointer({
          mode: 'position',
          pid: native.pid,
          width: Math.round((2 * width - actualWidth) * 1.25),
          height: 950,
        });
        await page.getByRole('separator', { name: 'Resize utility pane' }).waitFor();
        await page.waitForFunction((width) => innerWidth === width, width);
        const stop = page.getByRole('button', { name: 'Stop', exact: true });
        if (await stop.isEnabled()) {
          await stop.click();
          await delay(300);
        }
        await page
          .getByRole('separator', { name: 'Resize utility pane' })
          .evaluate((element) =>
            element.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true })),
          );
        await page
          .getByRole('separator', { name: 'Resize library navigator' })
          .evaluate((element) =>
            element.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })),
          );
        await page
          .getByRole('row', { name: /Resize fixture 0000/ })
          .evaluate((element) => element.click());
        if (playback) {
          await page.getByRole('button', { name: 'Configure playback profile' }).click();
          await page.getByRole('button', { name: 'Test playback (no input)' }).click();
          await page.waitForFunction(() =>
            document.querySelector('.player-track-status')?.textContent.includes('Playing'),
          );
        }
        const separator = page.getByRole('separator', {
          name: pane === 'navigator' ? 'Resize library navigator' : 'Resize utility pane',
        });
        const box = await separator.boundingBox();
        const cdp = await context.newCDPSession(page);
        await cdp.send('Performance.enable');
        const before = (await cdp.send('Performance.getMetrics')).metrics;
        await cdp.send('Tracing.start', {
          categories: 'devtools.timeline,blink.user_timing',
          transferMode: 'ReturnAsStream',
        });
        await page.evaluate(
          ({ pane }) => {
            const selector =
              pane === 'navigator' ? '.navigator-workbench-pane' : '.utility-workbench-pane';
            const direction = pane === 'navigator' ? 1 : -1;
            const startWidth = document.querySelector(selector).getBoundingClientRect().width;
            window.__resizeSamples = {
              frames: [],
              errors: [],
              age: [],
              events: [],
              commits: 0,
              trusted: true,
            };
            const samples = window.__resizeSamples;
            let startX;
            let previousFrame;
            let active = false;
            let frameId;
            const event = (e) => {
              if (e.type === 'pointerdown' && e.target.closest('[role="separator"]')) {
                active = true;
                startX = e.clientX;
                previousFrame = undefined;
                window.__resizeCommits = 0;
              }
              if (active && e.type === 'pointermove') {
                samples.trusted &&= e.isTrusted;
                samples.events.push({
                  time: performance.now(),
                  width: Math.min(
                    pane === 'clamp' ? 388 : pane === 'navigator' ? 340 : 440,
                    Math.max(
                      pane === 'navigator' ? 200 : 280,
                      startWidth + direction * (e.clientX - startX),
                    ),
                  ),
                });
              }
              if (e.type === 'pointerup') {
                active = false;
                samples.commits = window.__resizeCommits;
              }
            };
            for (const name of ['pointerdown', 'pointermove', 'pointerup'])
              document.addEventListener(name, event, true);
            const sample = (time) => {
              if (active) {
                if (previousFrame !== undefined) samples.frames.push(time - previousFrame);
                previousFrame = time;
                const latest = samples.events.at(-1);
                if (latest) {
                  const actual = document.querySelector(selector).getBoundingClientRect().width;
                  samples.errors.push(Math.abs(actual - latest.width));
                  const matched = samples.events.findLast((e) => Math.abs(actual - e.width) <= 1);
                  if (matched) samples.age.push(performance.now() - matched.time);
                }
              }
              frameId = requestAnimationFrame(sample);
            };
            frameId = requestAnimationFrame(sample);
            window.__stopResizeSamples = () => {
              cancelAnimationFrame(frameId);
              for (const name of ['pointerdown', 'pointermove', 'pointerup'])
                document.removeEventListener(name, event, true);
            };
          },
          { pane },
        );
        await pointer({
          mode: 'drag',
          pid: native.pid,
          x: Math.round((box.x + box.width / 2) * 1.25),
          y: Math.round((box.y + 100) * 1.25),
          delta: (pane === 'navigator' ? 100 : -160) * 1.25,
          duration: 4000,
        });
        await delay(100);
        const samples = await page.evaluate(() => {
          window.__stopResizeSamples();
          return window.__resizeSamples;
        });
        const after = (await cdp.send('Performance.getMetrics')).metrics;
        const completed = new Promise((done) => cdp.once('Tracing.tracingComplete', done));
        await cdp.send('Tracing.end');
        const { stream } = await completed;
        let trace = '';
        for (;;) {
          const chunk = await cdp.send('IO.read', { handle: stream });
          trace += chunk.data;
          if (chunk.eof) break;
        }
        await cdp.send('IO.close', { handle: stream });
        const caseId = `${count}-${playback ? 'playback' : 'idle'}-${pane}`;
        writeFileSync(join(output, `${label}-${caseId}.trace.json.gz`), gzipSync(trace));
        const delta = (name) =>
          ((after.find((m) => m.name === name)?.value ?? 0) -
            (before.find((m) => m.name === name)?.value ?? 0)) *
          1000;
        const paints = JSON.parse(trace)
          .traceEvents.filter((event) => event.name === 'Paint' && event.dur)
          .map((event) => event.dur / 1000);
        console.log('stream', {
          events: samples.events.length,
          frames: samples.frames.length,
          trusted: samples.trusted,
          box,
        });
        if (samples.events.length < 50 || !samples.trusted)
          throw new Error(`No genuine continuous pointer stream in ${caseId}`);
        results.push({
          caseId,
          pointerEvents: samples.events.length,
          reactCommits: samples.commits,
          frameMs: distribution(samples.frames),
          over33ms: samples.frames.filter((ms) => ms > 33).length,
          latestPointerErrorPx: distribution(samples.errors),
          matchedEventAgeMs: distribution(samples.age),
          layoutMs: delta('LayoutDuration'),
          styleMs: delta('RecalcStyleDuration'),
          scriptMs: delta('ScriptDuration'),
          paintTotalMs: paints.reduce((sum, ms) => sum + ms, 0),
          paintMs: distribution(paints),
        });
        console.log(JSON.stringify(results.at(-1)));
        writeFileSync(
          join(output, `${label}.json`),
          JSON.stringify(
            {
              label,
              sourceHead: previous?.sourceHead ?? sourceHead,
              invocationHead: sourceHead,
              exeSha256,
              runtime: await browser.version(),
              results,
            },
            null,
            2,
          ),
        );
        await cdp.detach();
      }
    }
  } catch (error) {
    const page = browser?.contexts()[0]?.pages()[0];
    if (page) {
      console.error(
        await page.evaluate(() => ({
          url: location.href,
          width: innerWidth,
          dpr: devicePixelRatio,
          storage: localStorage.getItem('sky.ui.workbench.v5'),
          body: document.body.innerText.slice(0, 2500),
        })),
      );
      await page.screenshot({ path: join(output, `${label}-failure.png`) });
    }
    throw error;
  } finally {
    if (browser) await browser.close();
    if (native.exitCode === null)
      spawnSync('taskkill', ['/pid', String(native.pid), '/t', '/f'], { windowsHide: true });
  }
}
