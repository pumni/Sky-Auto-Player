import { statSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { join } from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
import { preview } from 'vite';
import { loopbackUrl, runPlaywright } from './e2e-session.mjs';

const desktopRoot = fileURLToPath(new URL('..', import.meta.url));
const viteCli = fileURLToPath(new URL('../node_modules/vite/bin/vite.js', import.meta.url));
const playwrightCli = fileURLToPath(
  new URL('../node_modules/@playwright/test/cli.js', import.meta.url),
);
const bundleDist = join(desktopRoot, 'dist');
const bundleIndex = join(bundleDist, 'index.html');
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

  previewServer = await preview({
    root: desktopRoot,
    preview: { host: '127.0.0.1', port: 0, strictPort: true },
  });
  const bundleUrl = loopbackUrl(previewServer.httpServer);
  console.log(`[desktop bundle e2e] owned Vite preview: ${bundleUrl}`);
  const code = await runPlaywright(process.execPath, [playwrightCli, 'test'], {
    cwd: desktopRoot,
    env: {
      ...process.env,
      PLAYWRIGHT_BASE_URL: bundleUrl,
      PLAYWRIGHT_TEST_DIR: './tests/e2e-bundle',
    },
    stdio: 'inherit',
    windowsHide: true,
  });
  assertSuccess('Production-bundle Playwright smoke', { code });
} finally {
  if (previewServer) await previewServer.close();
}
