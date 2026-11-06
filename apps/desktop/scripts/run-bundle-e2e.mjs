import { statSync } from 'node:fs';
import { spawn, spawnSync } from 'node:child_process';
import { join } from 'node:path';
import process from 'node:process';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

const desktopRoot = fileURLToPath(new URL('..', import.meta.url));
const viteCli = fileURLToPath(new URL('../node_modules/vite/bin/vite.js', import.meta.url));
const playwrightCli = fileURLToPath(
  new URL('../node_modules/@playwright/test/cli.js', import.meta.url),
);
const bundleDist = join(desktopRoot, 'dist');
const bundleIndex = join(bundleDist, 'index.html');
const bundleUrl = 'http://127.0.0.1:4174';
const reuseFlag = '--reuse-existing-build';
const cliArgs = process.argv.slice(2);
const reuseExistingBuild = cliArgs.length === 1 && cliArgs[0] === reuseFlag;
let previewServer;

if (cliArgs.length > 0 && !reuseExistingBuild) {
  throw new Error(`Unknown option(s): ${cliArgs.join(' ')}. Supported option: ${reuseFlag}`);
}

function waitForExit(child) {
  return new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('exit', (code, signal) => resolve({ code, signal }));
  });
}

function assertSuccess(label, result) {
  if (result.signal || result.code !== 0) {
    throw new Error(`${label} failed with ${result.signal ?? `exit code ${result.code}`}`);
  }
}

async function waitForServer(server) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    if (server.exitCode !== null) {
      throw new Error(`Vite preview exited before becoming ready with code ${server.exitCode}`);
    }
    try {
      const response = await fetch(`${bundleUrl}/`);
      if (response.ok) return;
    } catch {
      // Vite preview is still starting.
    }
    await delay(250);
  }
  throw new Error('Vite preview did not become ready within 15 seconds');
}

function stopServer(server) {
  if (!server?.pid) return;
  if (process.platform === 'win32') {
    spawnSync('taskkill', ['/pid', String(server.pid), '/t', '/f'], { stdio: 'ignore' });
  } else {
    server.kill('SIGTERM');
  }
}

function assertExistingBuild() {
  let hasDistDirectory = false;
  let hasIndexFile = false;
  try {
    hasDistDirectory = statSync(bundleDist).isDirectory();
    hasIndexFile = statSync(bundleIndex).isFile();
  } catch {
    // The diagnostic below explains how to produce the required bundle.
  }
  if (!hasDistDirectory || !hasIndexFile) {
    throw new Error(
      `Existing production bundle is missing: expected ${bundleIndex}. Run "bun run build:web" or "bun run check" first.`,
    );
  }
}

try {
  if (reuseExistingBuild) {
    assertExistingBuild();
  } else {
    const build = spawn(process.execPath, [viteCli, 'build', '--config', 'vite.config.ts'], {
      cwd: desktopRoot,
      stdio: 'inherit',
      windowsHide: true,
    });
    assertSuccess('Production Vite build', await waitForExit(build));
  }

  previewServer = spawn(
    process.execPath,
    [
      viteCli,
      'preview',
      '--config',
      'vite.config.ts',
      '--host',
      '127.0.0.1',
      '--port',
      '4174',
      '--strictPort',
    ],
    {
      cwd: desktopRoot,
      stdio: 'inherit',
      windowsHide: true,
    },
  );
  await waitForServer(previewServer);

  const runner = spawn(process.execPath, [playwrightCli, 'test'], {
    cwd: desktopRoot,
    env: {
      ...process.env,
      PLAYWRIGHT_BASE_URL: bundleUrl,
      PLAYWRIGHT_TEST_DIR: './tests/e2e-bundle',
    },
    stdio: 'inherit',
    windowsHide: true,
  });
  assertSuccess('Production-bundle Playwright smoke', await waitForExit(runner));
} finally {
  stopServer(previewServer);
}
