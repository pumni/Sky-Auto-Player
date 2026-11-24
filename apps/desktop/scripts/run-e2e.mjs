import process from 'node:process';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import { loopbackUrl, runPlaywright } from './e2e-session.mjs';

const desktopRoot = fileURLToPath(new URL('..', import.meta.url));
const playwrightCli = fileURLToPath(
  new URL('../node_modules/@playwright/test/cli.js', import.meta.url),
);
const server = await createServer({
  root: desktopRoot,
  server: { host: '127.0.0.1', port: 0, strictPort: true },
});
try {
  await server.listen();
  const url = loopbackUrl(server.httpServer);
  console.log(`[desktop e2e] owned Vite server: ${url}`);
  process.exitCode = await runPlaywright(
    process.execPath,
    [playwrightCli, 'test', ...process.argv.slice(2)],
    {
      cwd: desktopRoot,
      env: { ...process.env, PLAYWRIGHT_BASE_URL: url },
      stdio: 'inherit',
      windowsHide: true,
    },
  );
} finally {
  await server.close();
}
