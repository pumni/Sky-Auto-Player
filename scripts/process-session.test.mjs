import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import process from 'node:process';
import test from 'node:test';
import {
  isPidAlive,
  loopbackUrl,
  runOwnedCommand,
  runPlaywright,
  spawnOwnedProcess,
  waitForPidExit,
} from './process-session.mjs';

test('loopbackUrl extracts port and rejects invalid addresses', async () => {
  const server = createServer();
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  try {
    const url = loopbackUrl(server);
    assert.match(url, /^http:\/\/127\.0\.0\.1:\d+$/);
  } finally {
    server.close();
  }

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

test('terminate terminates real child and grandchild tree without affecting sentinel', async () => {
  // Spawn an unrelated sentinel process that must survive
  const sentinel = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {
    stdio: 'ignore',
  });

  let session;
  let grandchildPid;

  try {
    assert.ok(sentinel.pid && isPidAlive(sentinel.pid), 'Sentinel process must be alive');

    // Child script spawns a long-lived grandchild process and outputs grandchild PID
    const childScript = `
      const { spawn } = require('node:child_process');
      const grand = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'inherit' });
      console.log('GRANDCHILD_PID:' + grand.pid);
      setInterval(() => {}, 1000);
    `;

    session = spawnOwnedProcess(process.execPath, ['-e', childScript], {
      stdio: ['ignore', 'pipe', 'inherit'],
    });

    grandchildPid = await new Promise((resolve, reject) => {
      let output = '';
      const timeout = setTimeout(() => reject(new Error('Grandchild startup timeout')), 5000);
      session.child.stdout.on('data', (chunk) => {
        output += chunk.toString();
        const match = output.match(/GRANDCHILD_PID:(\d+)/);
        if (match) {
          clearTimeout(timeout);
          resolve(Number.parseInt(match[1], 10));
        }
      });
      session.child.once('error', (err) => {
        clearTimeout(timeout);
        reject(err);
      });
    });

    assert.ok(session.pid && isPidAlive(session.pid), 'Child process must be alive');
    assert.ok(grandchildPid && isPidAlive(grandchildPid), 'Grandchild process must be alive');

    // Terminate the owned session
    await session.terminate({ graceTimeoutMs: 1500 });

    const childDead = await waitForPidExit(session.pid, 2000);
    const grandDead = await waitForPidExit(grandchildPid, 2000);

    assert.ok(childDead, 'Child process must be dead after terminate()');
    assert.ok(grandDead, 'Grandchild process must be dead after terminate()');

    // Verify sentinel was untouched
    assert.ok(isPidAlive(sentinel.pid), 'Unrelated sentinel process must remain alive');
  } finally {
    if (sentinel.pid && isPidAlive(sentinel.pid)) {
      try {
        if (process.platform === 'win32') {
          process.kill(sentinel.pid, 'SIGKILL');
        } else {
          process.kill(sentinel.pid, 'SIGKILL');
        }
      } catch {}
    }
    if (session && session.pid && isPidAlive(session.pid)) {
      await session.terminate().catch(() => {});
    }
    if (grandchildPid && isPidAlive(grandchildPid)) {
      try { process.kill(grandchildPid, 'SIGKILL'); } catch {}
    }
  }
});

test('terminate escalates to SIGKILL if child ignores SIGTERM', async () => {
  // Script ignores SIGTERM
  const stubbornScript = `
    process.on('SIGTERM', () => {});
    setInterval(() => {}, 1000);
  `;

  const session = spawnOwnedProcess(process.execPath, ['-e', stubbornScript], {
    stdio: 'ignore',
  });

  try {
    assert.ok(session.pid && isPidAlive(session.pid), 'Stubborn child must be alive');

    // Terminate with very short grace period to force escalation
    await session.terminate({ graceTimeoutMs: 100, killTimeoutMs: 1500 });

    const dead = await waitForPidExit(session.pid, 2000);
    assert.ok(dead, 'Stubborn child must be terminated after escalation');
  } finally {
    if (session.pid && isPidAlive(session.pid)) {
      try { process.kill(session.pid, 'SIGKILL'); } catch {}
    }
  }
});

test('runOwnedCommand preserves exit codes and cleans up signal listeners', async () => {
  const initialListeners = process.listenerCount('SIGINT');

  const { code } = await runOwnedCommand(
    process.execPath,
    ['-e', 'process.exit(42)'],
    { stdio: 'ignore' },
  );
  assert.equal(code, 42);
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
