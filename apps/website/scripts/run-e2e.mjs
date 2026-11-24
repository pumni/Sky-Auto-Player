import { existsSync } from 'node:fs';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
import { build, preview } from 'astro';
import { loopbackUrl, runPlaywright } from '../../../scripts/process-session.mjs';

const websiteRoot = fileURLToPath(new URL('..', import.meta.url));
const distIndex = fileURLToPath(new URL('../dist/index.html', import.meta.url));
const playwrightCli = fileURLToPath(
  new URL('../node_modules/@playwright/test/cli.js', import.meta.url),
);

const reuseFlag = '--reuse-existing-build';
const cliArgs = process.argv.slice(2);
const reuseExistingBuild = Boolean(process.env.CI) || cliArgs.includes(reuseFlag);
const forwardedArgs = cliArgs.filter((arg) => arg !== reuseFlag);

if (reuseExistingBuild) {
  if (!existsSync(distIndex)) {
    throw new Error(
      `[website e2e] Expected existing prebuilt static distribution at ${distIndex}. Run "bun run build" first.`,
    );
  }
  console.log(
    '[website e2e] reusing prebuilt static distribution (explicit artifact reuse; not proof of current source)...',
  );
} else {
  console.log('[website e2e] building fresh static distribution...');
  await build({ root: websiteRoot });
}

let previewServer;
try {
  previewServer = await preview({
    root: websiteRoot,
    server: { host: '127.0.0.1', port: 0 },
  });

  const rootUrl = `${loopbackUrl(previewServer.server)}/Sky-Auto-Player`;
  console.log(`[website e2e] owned Astro preview: ${rootUrl}/`);

  const exitCode = await runPlaywright(
    process.execPath,
    [playwrightCli, 'test', ...forwardedArgs],
    {
      cwd: websiteRoot,
      env: {
        ...process.env,
        PLAYWRIGHT_BASE_URL: rootUrl,
      },
      stdio: 'inherit',
      windowsHide: true,
    },
  );

  process.exitCode = exitCode;
} finally {
  if (previewServer) {
    await previewServer.stop();
  }
}
