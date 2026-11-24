import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import process from 'node:process';
import test from 'node:test';
import { loopbackUrl, runPlaywright, terminateProcess } from './process-session.mjs';

function isPidAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function waitForCondition(check, timeoutMs = 4000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await check()) return true;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  return false;
}

test('loopbackUrl extracts port and rejects invalid addresses', async () => {
  const server = createServer();
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const url = loopbackUrl(server);
  assert.match(url, /^http:\/\/127\.0\.0\.1:\d+$/);
  server.close();

  assert.throws(() => loopbackUrl(null), /Expected an owned IPv4 loopback listener/);
  assert.throws(
    () => loopbackUrl({ address: () => ({ address: '0.0.0.0', port: 1234 }) }),
    /Expected an owned IPv4 loopback listener/,
  );
  assert.throws(
    () => loopbackUrl({ address: () => null }),
    /Expected an owned IPv4 loopback listener/,
  );
});

test('terminateProcess safely ignores null, undefined, and exited children', () => {
  assert.doesNotThrow(() => terminateProcess(null));
  assert.doesNotThrow(() => terminateProcess(undefined));
  assert.doesNotThrow(() => terminateProcess({ killed: true }));
  assert.doesNotThrow(() => terminateProcess({ exitCode: 0 }));
});

test('terminateProcess terminates real child and grandchild tree without affecting sentinel', async () => {
  // Spawn an unrelated sentinel process that must survive
  const sentinel = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {
    stdio: 'ignore',
  });
  assert.ok(sentinel.pid && isPidAlive(sentinel.pid), 'Sentinel process must be alive');

  // Spawn a parent process which in turn spawns a long-lived grandchild process and prints grandchild PID
  const childScript = `
    const { spawn } = require('node:child_process');
    const grand = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'inherit' });
    console.log('GRANDCHILD_PID:' + grand.pid);
    setInterval(() => {}, 1000);
  `;

  const child = spawn(process.execPath, ['-e', childScript], {
    stdio: ['ignore', 'pipe', 'inherit'],
  });

  const grandchildPid = await new Promise((resolve, reject) => {
    let output = '';
    const timeout = setTimeout(() => reject(new Error('Grandchild startup timeout')), 5000);
    child.stdout.on('data', (chunk) => {
      output += chunk.toString();
      const match = output.match(/GRANDCHILD_PID:(\d+)/);
      if (match) {
        clearTimeout(timeout);
        resolve(Number.parseInt(match[1], 10));
      }
    });
    child.once('error', (err) => {
      clearTimeout(timeout);
      reject(err);
    });
  });

  assert.ok(child.pid && isPidAlive(child.pid), 'Child process must be alive');
  assert.ok(grandchildPid && isPidAlive(grandchildPid), 'Grandchild process must be alive');

  // Now terminate the parent child process
  terminateProcess(child);

  // Verify BOTH child and grandchild are terminated
  const childExited = await waitForCondition(() => !isPidAlive(child.pid));
  const grandExited = await waitForCondition(() => !isPidAlive(grandchildPid));

  assert.ok(childExited, 'Child process must be terminated');
  assert.ok(grandExited, 'Grandchild process must be terminated by tree termination');

  // Verify the unrelated sentinel process is STILL ALIVE
  assert.ok(isPidAlive(sentinel.pid), 'Unrelated sentinel process must remain unaffected');

  // Clean up sentinel
  terminateProcess(sentinel);
  await waitForCondition(() => !isPidAlive(sentinel.pid));
});

test('runPlaywright preserves exit codes and cleans up signal listeners', async () => {
  const initialListeners = process.listenerCount('SIGINT');

  const exitCode = await runPlaywright(
    process.execPath,
    ['-e', 'process.exit(42)'],
    { stdio: 'ignore' },
  );
  assert.equal(exitCode, 42);
  assert.equal(process.listenerCount('SIGINT'), initialListeners);
});

test('runPlaywright rejects on invalid executable and cleans up signal listeners', async () => {
  const initialListeners = process.listenerCount('SIGINT');
  await assert.rejects(
    async () => {
      await runPlaywright('nonexistent-binary-for-sure-xyz-12345', [], { stdio: 'ignore' });
    },
    (error) => Boolean(error),
  );
  assert.equal(process.listenerCount('SIGINT'), initialListeners);
});
