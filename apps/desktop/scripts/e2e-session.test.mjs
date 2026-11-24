import assert from 'node:assert/strict';
import { createServer } from 'node:net';
import process from 'node:process';
import test from 'node:test';
import { loopbackUrl, runPlaywright, terminateProcess } from './e2e-session.mjs';

test('two owned loopback listeners receive distinct actual URLs', async () => {
  const first = createServer();
  const second = createServer();
  try {
    for (const server of [first, second]) {
      await new Promise((resolve, reject) => {
        server.once('error', reject);
        server.listen(0, '127.0.0.1', resolve);
      });
    }
    assert.notEqual(loopbackUrl(first), loopbackUrl(second));
    assert.match(loopbackUrl(first), /^http:\/\/127\.0\.0\.1:\d+$/);
    assert.throws(() => loopbackUrl(createServer()), /owned IPv4/);
  } finally {
    for (const server of [first, second]) {
      if (server.listening) await new Promise((resolve) => server.close(resolve));
    }
  }
});

test('loopbackUrl rejects invalid host or unstarted listener', () => {
  assert.throws(() => loopbackUrl(null), /owned IPv4/);
  assert.throws(() => loopbackUrl({ address: () => null }), /owned IPv4/);
  assert.throws(() => loopbackUrl({ address: () => 'pipe-string' }), /owned IPv4/);
  assert.throws(
    () => loopbackUrl({ address: () => ({ address: '0.0.0.0', port: 1234 }) }),
    /owned IPv4/,
  );
});

test('runner preserves exit status and removes signal listeners', async () => {
  const before = [process.listenerCount('SIGINT'), process.listenerCount('SIGTERM')];
  const code = await runPlaywright(process.execPath, ['-e', 'process.exit(7)'], {
    stdio: 'ignore',
  });
  assert.equal(code, 7);
  assert.deepEqual([process.listenerCount('SIGINT'), process.listenerCount('SIGTERM')], before);
});

test('runner rejects on invalid executable and cleans up signal listeners', async () => {
  const before = [process.listenerCount('SIGINT'), process.listenerCount('SIGTERM')];
  await assert.rejects(
    () => runPlaywright('non-existent-executable-sky-test-12345', [], { stdio: 'ignore' }),
    /ENOENT/,
  );
  assert.deepEqual([process.listenerCount('SIGINT'), process.listenerCount('SIGTERM')], before);
});

test('terminateProcess safely handles null or already exited child', () => {
  assert.doesNotThrow(() => terminateProcess(null));
  assert.doesNotThrow(() => terminateProcess({ killed: true }));
  assert.doesNotThrow(() => terminateProcess({ exitCode: 0 }));
});
