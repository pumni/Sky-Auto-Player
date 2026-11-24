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

export function isPidAlive(pid) {
  if (!pid || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

export function isProcessGroupAlive(pgid) {
  if (!pgid || pgid <= 0) return false;
  if (process.platform === 'win32') {
    return isPidAlive(pgid);
  }
  try {
    process.kill(-pgid, 0);
    return true;
  } catch (err) {
    if (err.code === 'EPERM') return true;
    return false;
  }
}

export async function waitForPidExit(pid, timeoutMs = 2000) {
  const start = performance.now();
  while (performance.now() - start < timeoutMs) {
    if (!isPidAlive(pid)) return true;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  return !isPidAlive(pid);
}

export async function waitForProcessGroupExit(pgid, timeoutMs = 2000) {
  const start = performance.now();
  while (performance.now() - start < timeoutMs) {
    if (!isProcessGroupAlive(pgid)) return true;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  return !isProcessGroupAlive(pgid);
}

/**
 * Explicit session owning a child process and its process tree or group.
 */
export class OwnedProcessSession {
  constructor(child, isGroupLeader) {
    this.child = child;
    this.pid = child.pid;
    this.isGroupLeader = isGroupLeader;
    this.exited = false;
    this.exitCode = null;
    this.exitSignal = null;
    this.terminationPromise = null;

    child.once('exit', (code, signal) => {
      this.exited = true;
      this.exitCode = code;
      this.exitSignal = signal;
    });
  }

  /**
   * Bounded, awaited teardown of the owned process tree.
   * On POSIX: sends SIGTERM to the owned process group (-pgid), awaits exit, and escalates to SIGKILL.
   * On Windows: invokes taskkill targeting the owned PID tree (/T /F), awaits exit, and verifies cleanup.
   */
  async terminate(options = {}) {
    if (this.terminationPromise) {
      return this.terminationPromise;
    }
    this.terminationPromise = this._performTerminate(options);
    return this.terminationPromise;
  }

  async _performTerminate(options) {
    const graceTimeoutMs = options.graceTimeoutMs ?? 1500;
    const killTimeoutMs = options.killTimeoutMs ?? 1000;

    if (process.platform === 'win32') {
      if (!this.pid || !isPidAlive(this.pid)) {
        this.exited = true;
        return;
      }

      // Bounded taskkill timeout. Note: Node's spawnSync timeout parameter is not a guaranteed
      // hard wall-clock deadline; the OS and runtime may take additional time to finish terminating or close stdio.
      const taskkillTimeoutMs = Math.max(graceTimeoutMs, 3000);
      const result = spawnSync('taskkill', ['/pid', String(this.pid), '/T', '/F'], {
        stdio: 'ignore',
        windowsHide: true,
        timeout: taskkillTimeoutMs,
      });

      if (result.error) {
        throw new Error(
          `Failed to execute taskkill for PID ${this.pid}: ${result.error.message}`,
        );
      }

      if (result.status !== 0) {
        const exited = await waitForPidExit(this.pid, graceTimeoutMs);
        if (!exited) {
          try {
            this.child.kill('SIGKILL');
          } catch {
            // child already dead
          }
          await waitForPidExit(this.pid, killTimeoutMs);
        }
        throw new Error(
          `Failed to cleanly terminate owned process tree for PID ${this.pid} (taskkill exit=${result.status})`,
        );
      }

      const exited = await waitForPidExit(this.pid, graceTimeoutMs);
      if (!exited) {
        try {
          this.child.kill('SIGKILL');
        } catch {
          // child already dead
        }
        const killed = await waitForPidExit(this.pid, killTimeoutMs);
        if (!killed) {
          throw new Error(
            `Failed to cleanly terminate owned process tree for PID ${this.pid} within deadline`,
          );
        }
      }
      this.exited = true;
    } else {
      const pgid = this.isGroupLeader ? this.pid : null;
      const target = pgid ? -pgid : this.pid;

      const isAlive = pgid ? isProcessGroupAlive(pgid) : isPidAlive(this.pid);
      if (!isAlive) {
        this.exited = true;
        return;
      }

      try {
        process.kill(target, 'SIGTERM');
      } catch (err) {
        if (err.code !== 'ESRCH') {
          try {
            this.child.kill('SIGTERM');
          } catch {}
        }
      }

      const exitedGracefully = pgid
        ? await waitForProcessGroupExit(pgid, graceTimeoutMs)
        : await waitForPidExit(this.pid, graceTimeoutMs);

      if (!exitedGracefully) {
        try {
          process.kill(target, 'SIGKILL');
        } catch (err) {
          if (err.code !== 'ESRCH') {
            try {
              this.child.kill('SIGKILL');
            } catch {}
          }
        }
        const killed = pgid
          ? await waitForProcessGroupExit(pgid, killTimeoutMs)
          : await waitForPidExit(this.pid, killTimeoutMs);
        if (!killed) {
          throw new Error(
            `Failed to cleanly terminate owned POSIX process group ${target} within deadline`,
          );
        }
      }
      this.exited = true;
    }
  }
}

/**
 * Spawns an explicitly owned child process.
 * On POSIX platforms, sets detached: true so child becomes the leader of its own process group.
 */
export function spawnOwnedProcess(executable, args, options = {}) {
  const isPosix = process.platform !== 'win32';
  const spawnOptions = {
    ...options,
    detached: isPosix ? true : Boolean(options.detached),
    windowsHide: true,
  };

  const child = spawn(executable, args, spawnOptions);
  return new OwnedProcessSession(child, isPosix);
}

/**
 * Runs an owned command to completion with signal trapping and bounded cleanup.
 *
 * Operational assumptions & containment boundaries:
 * - Cooperative normal exit: Supported commands (e.g. test runners, dev scripts) are assumed
 *   to exit cooperatively upon test completion without intentionally detaching orphan daemons.
 *   On normal zero or non-zero command exit, the child process is waited on without full-tree teardown.
 * - Signal-triggered teardown: When outer runner signals (SIGINT / SIGTERM) are received, the session
 *   initiates and awaits an explicit full-tree teardown before completing.
 * - Windows containment limitations: On Windows without external job objects, if a parent process exits
 *   spontaneously before teardown can be executed, child processes dissociate. `taskkill /pid <pid> /T /F`
 *   cannot find descendants whose parent is already dead. Teardown on Windows is therefore best-effort
 *   under uncoordinated parent exit, and escaped descendants cannot be guaranteed contained without an
 *   external supervisor or OS job object.
 *
 * @param {string} executable
 * @param {string[]} args
 * @param {import('node:child_process').SpawnOptions & { _sessionFactory?: typeof spawnOwnedProcess }} [options]
 */
export function runOwnedCommand(executable, args, options = {}) {
  const factory = options._sessionFactory ?? spawnOwnedProcess;
  const session = factory(executable, args, options);
  const child = session.child;

  return new Promise((resolve, reject) => {
    let finished = false;
    let teardownPromise = null;

    const cleanupListeners = () => {
      process.removeListener('SIGINT', handleSignal);
      process.removeListener('SIGTERM', handleSignal);
    };

    const finish = (err, outcome) => {
      if (finished) return;
      finished = true;
      cleanupListeners();
      if (err) {
        reject(err);
      } else {
        resolve(outcome);
      }
    };

    const handleSignal = async () => {
      if (finished) return;
      try {
        if (!teardownPromise) {
          teardownPromise = session.terminate();
        }
        await teardownPromise;
        finish(null, { code: 1, signal: 'SIGTERM' });
      } catch (err) {
        finish(err);
      }
    };

    process.once('SIGINT', handleSignal);
    process.once('SIGTERM', handleSignal);

    child.once('error', async (err) => {
      try {
        if (!teardownPromise) {
          teardownPromise = session.terminate();
        }
        await teardownPromise;
      } catch {}
      finish(err);
    });

    child.once('exit', async (code, signal) => {
      if (teardownPromise) {
        try {
          await teardownPromise;
          finish(null, { code: code ?? 1, signal });
        } catch (err) {
          finish(err);
        }
      } else {
        finish(null, { code: code ?? 1, signal });
      }
    });
  });
}

/**
 * Convenience wrapper for Playwright test runners preserving exit codes.
 */
export async function runPlaywright(executable, args, options = {}) {
  const { code, signal } = await runOwnedCommand(executable, args, options);
  return signal ? 1 : (code ?? 1);
}
