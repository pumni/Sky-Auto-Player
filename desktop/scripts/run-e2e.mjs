import { spawn, spawnSync } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const desktopRoot = fileURLToPath(new URL('..', import.meta.url));
const viteCli = fileURLToPath(new URL('../node_modules/vite/bin/vite.js', import.meta.url));
const playwrightCli = fileURLToPath(
  new URL('../node_modules/@playwright/test/cli.js', import.meta.url),
);
const server = spawn(process.execPath, [viteCli, '--host', '127.0.0.1', '--port', '4173'], {
  cwd: desktopRoot,
  stdio: 'inherit',
  windowsHide: true,
});
server.unref();

async function waitForServer() {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    try {
      const response = await fetch('http://127.0.0.1:4173/');
      if (response.ok) return;
    } catch {
      // Vite is still starting.
    }
    await delay(250);
  }
  throw new Error('Vite did not become ready within 15 seconds');
}

function stopServer() {
  if (server.pid === undefined) return;
  if (process.platform === 'win32') {
    spawnSync('taskkill', ['/pid', String(server.pid), '/t', '/f'], { stdio: 'ignore' });
  } else {
    server.kill('SIGTERM');
  }
}

try {
  await waitForServer();
  const result = await new Promise((resolve, reject) => {
    const runner = spawn(process.execPath, [playwrightCli, 'test'], {
      cwd: desktopRoot,
      stdio: 'inherit',
      windowsHide: true,
    });
    runner.once('error', reject);
    runner.once('exit', (code, signal) => resolve(signal ? 1 : (code ?? 1)));
  });
  process.exitCode = result;
} finally {
  stopServer();
}
