import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import process from 'node:process';
import test from 'node:test';
import {
  isPidAlive,
  isProcessGroupAlive,
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
      try { process.kill(sentinel.pid, 'SIGKILL'); } catch {}
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
  // Script ignores SIGTERM after signaling handler readiness
  const stubbornScript = `
    process.on('SIGTERM', () => {});
    console.log('READY');
    setInterval(() => {}, 1000);
  `;

  const session = spawnOwnedProcess(process.execPath, ['-e', stubbornScript], {
    stdio: ['ignore', 'pipe', 'ignore'],
  });

  try {
    // Wait for the handler to actually be attached
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('Child startup timeout')), 5000);
      session.child.stdout.on('data', (chunk) => {
        if (chunk.toString().includes('READY')) {
          clearTimeout(timeout);
          resolve();
        }
      });
      session.child.once('error', (err) => {
        clearTimeout(timeout);
        reject(err);
      });
    });

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

test('terminate is idempotent when called multiple times concurrently', async () => {
  const session = spawnOwnedProcess(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {
    stdio: 'ignore',
  });

  try {
    assert.ok(session.pid && isPidAlive(session.pid));

    const [t1, t2] = await Promise.all([
      session.terminate({ graceTimeoutMs: 500 }),
      session.terminate({ graceTimeoutMs: 500 }),
    ]);

    assert.equal(t1, undefined);
    assert.equal(t2, undefined);
    const dead = await waitForPidExit(session.pid, 2000);
    assert.ok(dead);
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

test('runner-interruption regression: outer signal during runOwnedCommand cleans up tree with stubborn descendant, leaves sentinel unharmed', async () => {
  const sentinel = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {
    stdio: 'ignore',
  });

  let sessionInstance;
  let childPid;
  let grandchildPid;

  try {
    assert.ok(sentinel.pid && isPidAlive(sentinel.pid), 'Sentinel process must be alive');

    const script = `
      const { spawn } = require('node:child_process');
      const grand = spawn(process.execPath, ['-e', \`
        process.on('SIGTERM', () => {});
        console.log('GRANDCHILD_READY:' + process.pid);
        setInterval(() => {}, 1000);
      \`], { stdio: ['ignore', 'pipe', 'inherit'] });

      grand.stdout.on('data', (d) => {
        process.stdout.write(d);
        console.log('PARENT_READY:' + process.pid);
      });

      process.on('SIGTERM', () => {
        process.exit(0);
      });

      setInterval(() => {}, 1000);
    `;

    const customFactory = (executable, args, options) => {
      sessionInstance = spawnOwnedProcess(executable, args, options);
      return sessionInstance;
    };

    const commandPromise = runOwnedCommand(
      process.execPath,
      ['-e', script],
      {
        stdio: ['ignore', 'pipe', 'inherit'],
        _sessionFactory: customFactory,
      },
    );

    childPid = sessionInstance.pid;
    assert.ok(childPid && isPidAlive(childPid), 'Child process must be alive');

    grandchildPid = await new Promise((resolve, reject) => {
      let output = '';
      const timeout = setTimeout(() => reject(new Error('Process startup timeout')), 5000);
      sessionInstance.child.stdout.on('data', (chunk) => {
        output += chunk.toString();
        const grandMatch = output.match(/GRANDCHILD_READY:(\d+)/);
        if (grandMatch && output.includes('PARENT_READY')) {
          clearTimeout(timeout);
          resolve(Number.parseInt(grandMatch[1], 10));
        }
      });
      sessionInstance.child.once('error', (err) => {
        clearTimeout(timeout);
        reject(err);
      });
    });

    assert.ok(grandchildPid && isPidAlive(grandchildPid), 'Grandchild process must be alive');

    // Simulate outer runner interruption
    process.emit('SIGTERM');

    const outcome = await commandPromise;
    assert.equal(outcome.code, 1, 'Interruption outcome code must be 1');
    assert.equal(outcome.signal, 'SIGTERM', 'Interruption outcome signal must be SIGTERM');

    const childDead = await waitForPidExit(childPid, 2500);
    const grandDead = await waitForPidExit(grandchildPid, 2500);

    assert.ok(childDead, 'Child process must be dead after interrupt teardown');
    assert.ok(grandDead, 'Stubborn grandchild must be dead after interrupt teardown');

    assert.ok(isPidAlive(sentinel.pid), 'Sentinel process must remain alive');
  } finally {
    if (sentinel.pid && isPidAlive(sentinel.pid)) {
      try { process.kill(sentinel.pid, 'SIGKILL'); } catch {}
    }
    if (childPid && isPidAlive(childPid)) {
      try { process.kill(childPid, 'SIGKILL'); } catch {}
    }
    if (grandchildPid && isPidAlive(grandchildPid)) {
      try { process.kill(grandchildPid, 'SIGKILL'); } catch {}
    }
  }
});

test('runOwnedCommand propagates teardown failure through test seam without unhandled rejection', async () => {
  const initialListeners = process.listenerCount('SIGTERM');

  let sessionInstance;
  const customFactory = (executable, args, options) => {
    sessionInstance = spawnOwnedProcess(executable, args, options);
    sessionInstance.terminate = async () => {
      throw new Error('synthetic teardown failure');
    };
    return sessionInstance;
  };

  const commandPromise = runOwnedCommand(
    process.execPath,
    ['-e', 'setInterval(() => {}, 1000)'],
    {
      stdio: 'ignore',
      _sessionFactory: customFactory,
    },
  );

  try {
    // Trigger signal to cause teardown
    process.emit('SIGTERM');

    await assert.rejects(
      commandPromise,
      /synthetic teardown failure/,
    );
  } finally {
    if (sessionInstance?.pid && isPidAlive(sessionInstance.pid)) {
      try { process.kill(sessionInstance.pid, 'SIGKILL'); } catch {}
    }
  }

  assert.equal(process.listenerCount('SIGTERM'), initialListeners);
});

test('runOwnedCommand propagates startup failure on invalid executable and cleans up signal listeners', async () => {
  const initialListeners = process.listenerCount('SIGINT');
  await assert.rejects(
    async () => {
      await runOwnedCommand('__non_existent_binary_xyz_123__', [], { stdio: 'ignore' });
    },
    (err) => Boolean(err),
  );
  assert.equal(process.listenerCount('SIGINT'), initialListeners);
});
