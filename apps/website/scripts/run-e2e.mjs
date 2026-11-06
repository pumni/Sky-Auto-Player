const rootUrl = 'http://127.0.0.1:4321/Sky-Auto-Player/';
const astroCli = './node_modules/astro/bin/astro.mjs';
const playwrightCli = './node_modules/@playwright/test/cli.js';
const forwardedArgs = process.argv.slice(2);

async function run(command, env = process.env) {
  const processHandle = Bun.spawn(command, {
    cwd: import.meta.dir + '/..',
    env,
    stdin: 'inherit',
    stdout: 'inherit',
    stderr: 'inherit',
  });
  const exitCode = await processHandle.exited;
  if (exitCode !== 0) {
    throw new Error(`${command.join(' ')} exited with code ${exitCode}`);
  }
}

async function waitForPreview(server) {
  const deadline = Date.now() + 120_000;
  while (Date.now() < deadline) {
    if (server.exitCode !== null) {
      throw new Error(`Astro preview exited before becoming ready (code ${server.exitCode})`);
    }
    try {
      const response = await fetch(rootUrl);
      if (response.ok) return;
    } catch {
      // Preview is still starting.
    }
    await Bun.sleep(100);
  }
  throw new Error(`Timed out waiting for ${rootUrl}`);
}

async function portIsInUse() {
  try {
    await fetch(rootUrl);
    return true;
  } catch {
    return false;
  }
}

async function stopPreview(server) {
  if (process.platform === 'win32') {
    const taskkill = Bun.spawn(['taskkill', '/pid', String(server.pid), '/T', '/F'], {
      cwd: import.meta.dir + '/..',
      stdout: 'ignore',
      stderr: 'ignore',
    });
    await taskkill.exited;
    return;
  }
  server.kill();
  await server.exited;
}

if (!process.env.CI) {
  await run(['bun', 'run', 'build']);
}

if (await portIsInUse()) {
  throw new Error(`Port 4321 is already serving ${rootUrl}; stop the existing preview first`);
}

const server = Bun.spawn(['node', astroCli, 'preview', '--host', '0.0.0.0', '--port', '4321'], {
  cwd: import.meta.dir + '/..',
  env: process.env,
  stdin: 'ignore',
  stdout: 'inherit',
  stderr: 'inherit',
});

let exitCode = 1;
try {
  await waitForPreview(server);
  const tests = Bun.spawn(['node', playwrightCli, 'test', ...forwardedArgs], {
    cwd: import.meta.dir + '/..',
    env: process.env,
    stdin: 'inherit',
    stdout: 'inherit',
    stderr: 'inherit',
  });
  exitCode = await tests.exited;
} finally {
  await stopPreview(server);
}

process.exit(exitCode);
