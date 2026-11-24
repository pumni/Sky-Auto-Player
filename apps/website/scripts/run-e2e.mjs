import { existsSync } from 'node:fs';
import { spawn, spawnSync } from 'node:child_process';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
import { build, preview } from 'astro';

const websiteRoot = fileURLToPath(new URL('..', import.meta.url));
const distIndex = fileURLToPath(new URL('../dist/index.html', import.meta.url));
const playwrightCli = fileURLToPath(
  new URL('../node_modules/@playwright/test/cli.js', import.meta.url),
);
const forwardedArgs = process.argv.slice(2);

function terminateProcess(child) {
  if (!child || child.killed || child.exitCode !== null) return;
  if (process.platform === 'win32' && child.pid) {
    try {
      spawnSync('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
    } catch {
      child.kill();
    }
  } else {
    child.kill('SIGTERM');
  }
}

function runPlaywright(args, env) {
  const child = spawn(process.execPath, [playwrightCli, 'test', ...args], {
    cwd: websiteRoot,
    env,
    stdio: 'inherit',
    windowsHide: true,
  });

  return new Promise((resolve, reject) => {
    const stop = () => terminateProcess(child);
    process.once('SIGINT', stop);
    process.once('SIGTERM', stop);

    function cleanup() {
      process.removeListener('SIGINT', stop);
      process.removeListener('SIGTERM', stop);
    }

    child.once('error', (err) => {
      cleanup();
      reject(err);
    });

    child.once('exit', (code, signal) => {
      cleanup();
      resolve(signal ? 1 : (code ?? 1));
    });
  });
}

if (!process.env.CI && !existsSync(distIndex)) {
  console.log('[website e2e] building static distribution...');
  await build({ root: websiteRoot });
}

let previewServer;
try {
  previewServer = await preview({
    root: websiteRoot,
    server: { host: '127.0.0.1', port: 0 },
  });

  const address = previewServer.server?.address?.();
  if (!address || typeof address === 'string' || address.address !== '127.0.0.1') {
    throw new Error('Expected an owned IPv4 loopback listener for website preview');
  }

  const port = address.port;
  const rootUrl = `http://127.0.0.1:${port}/Sky-Auto-Player`;
  console.log(`[website e2e] owned Astro preview: ${rootUrl}/`);

  const exitCode = await runPlaywright(forwardedArgs, {
    ...process.env,
    PLAYWRIGHT_BASE_URL: rootUrl,
  });

  process.exitCode = exitCode;
} finally {
  if (previewServer) {
    await previewServer.stop();
  }
}
