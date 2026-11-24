import assert from 'node:assert/strict';
import { createServer } from 'node:net';
import process from 'node:process';
import test from 'node:test';
import { loopbackUrl, runPlaywright } from './e2e-session.mjs';

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

test('runner preserves exit status and removes signal listeners', async () => {
  const before = [process.listenerCount('SIGINT'), process.listenerCount('SIGTERM')];
  const code = await runPlaywright(process.execPath, ['-e', 'process.exit(7)'], {
    stdio: 'ignore',
  });
  assert.equal(code, 7);
  assert.deepEqual([process.listenerCount('SIGINT'), process.listenerCount('SIGTERM')], before);
});
