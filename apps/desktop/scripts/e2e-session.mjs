import { spawn, spawnSync } from 'node:child_process';
import process from 'node:process';

export function loopbackUrl(server) {
  const address = server?.address?.() ?? server?.address;
  if (!address || typeof address === 'string' || address.address !== '127.0.0.1') {
    throw new Error('Expected an owned IPv4 loopback listener');
  }
  return `http://127.0.0.1:${address.port}`;
}

export function terminateProcess(child) {
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

export function runPlaywright(executable, args, options) {
  const child = spawn(executable, args, options);
  return new Promise((resolve, reject) => {
    const stop = () => terminateProcess(child);
    process.once('SIGINT', stop);
    process.once('SIGTERM', stop);
    function cleanup() {
      process.removeListener('SIGINT', stop);
      process.removeListener('SIGTERM', stop);
    }
    child.once('error', (error) => {
      cleanup();
      reject(error);
    });
    child.once('exit', (code, signal) => {
      cleanup();
      resolve(signal ? 1 : (code ?? 1));
    });
  });
}
