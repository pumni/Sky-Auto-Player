import { spawn, spawnSync } from 'node:child_process';
import process from 'node:process';

/**
 * Extracts and validates an owned IPv4 loopback URL from an active server instance.
 * Rejects wildcard (0.0.0.0), non-loopback, or unstarted servers.
 */
export function loopbackUrl(server) {
  const address = server?.address?.() ?? server?.address;
  if (!address || typeof address === 'string' || address.address !== '127.0.0.1') {
    throw new Error('Expected an owned IPv4 loopback listener on 127.0.0.1');
  }
  return `http://127.0.0.1:${address.port}`;
}

/**
 * Safely terminates a child process and its owned descendant tree without
 * affecting unrelated development or system processes.
 *
 * On Windows: Uses taskkill targeting the exact owned PID tree (/T /F).
 * On POSIX: Targets the owned process group or falls back to SIGTERM -> SIGKILL.
 */
export function terminateProcess(child, options = {}) {
  if (!child || child.killed || child.exitCode !== null) return;
  const pid = child.pid;
  if (!pid) return;

  if (process.platform === 'win32') {
    const result = spawnSync('taskkill', ['/pid', String(pid), '/T', '/F'], {
      stdio: 'ignore',
      windowsHide: true,
    });
    // If taskkill encountered an error or unexpected failure (other than process already exited),
    // fall back to direct child kill.
    if (result.error || (result.status !== 0 && result.status !== 128 && result.status !== 255)) {
      try {
        child.kill('SIGKILL');
      } catch {
        // Child already exited.
      }
    }
  } else {
    // POSIX process group or direct child termination
    try {
      // Attempt process group kill first if child was spawned in its own group
      process.kill(-pid, 'SIGTERM');
    } catch {
      try {
        child.kill('SIGTERM');
      } catch {
        // Child already exited.
      }
    }

    const timeoutMs = options.graceTimeoutMs ?? 2000;
    if (timeoutMs > 0) {
      const timer = setTimeout(() => {
        try {
          process.kill(-pid, 'SIGKILL');
        } catch {
          try {
            child.kill('SIGKILL');
          } catch {
            // Already dead.
          }
        }
      }, timeoutMs);
      if (typeof timer.unref === 'function') timer.unref();
    }
  }
}

/**
 * Spawns a child process (such as a Playwright test runner) with signal trapping.
 * Ensures the child and any spawned descendants are cleanly torn down on interrupt.
 */
export function runPlaywright(executable, args, options = {}) {
  const child = spawn(executable, args, options);

  return new Promise((resolve, reject) => {
    let cleanedUp = false;
    const stop = () => terminateProcess(child);

    const onSigInt = () => {
      stop();
    };
    const onSigTerm = () => {
      stop();
    };

    process.once('SIGINT', onSigInt);
    process.once('SIGTERM', onSigTerm);

    function cleanup() {
      if (cleanedUp) return;
      cleanedUp = true;
      process.removeListener('SIGINT', onSigInt);
      process.removeListener('SIGTERM', onSigTerm);
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
