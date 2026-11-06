import { spawnSync } from 'node:child_process';
import { copyFile, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const desktopRoot = fileURLToPath(new URL('..', import.meta.url));
const repositoryRoot = resolve(desktopRoot, '../..');
const captureDir = resolve(desktopRoot, 'test-results/graphite-workspace-checkpoint');
const destination = resolve(repositoryRoot, 'docs/ui/current');
const screenshots = {
  'workbench.png': 'candidate-populated-1366x768.png',
  'compact.png': 'candidate-workbench-800x560.png',
  'overlay.png': 'candidate-overlay-640x448.png',
  'diagnostics.png': 'candidate-runtime-diagnostics-1366x768.png',
};

function git(args) {
  const result = spawnSync('git', args, { cwd: repositoryRoot, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr || 'git command failed');
  return result.stdout.trim();
}

const sha = git(['rev-parse', 'HEAD']);
const dirty = git(['status', '--porcelain', '--untracked-files=no', '--', 'apps/desktop/src', 'apps/desktop/tests', 'rust']);
if (dirty) throw new Error('UI source/tests are dirty. Commit changes before refreshing the canonical UI capture.');

await rm(captureDir, { recursive: true, force: true });
const run = spawnSync(
  process.execPath,
  [
    resolve(desktopRoot, 'scripts/run-e2e.mjs'),
    'tests/e2e/visual-evidence.spec.ts',
    '--grep',
    'captures selected Graphite workspace evidence views on request',
    '--workers=1',
  ],
  {
    cwd: desktopRoot,
    stdio: 'inherit',
    env: {
      ...process.env,
      GRAPHITE_WORKSPACE_CAPTURE: 'candidate',
      GRAPHITE_WORKSPACE_CAPTURE_VIEWS: 'populated,compact,overlay,runtime',
    },
  },
);
if (run.status !== 0) throw new Error('Visual capture failed; canonical images were not changed.');

for (const file of Object.values(screenshots)) {
  const bytes = await readFile(resolve(captureDir, file));
  if (bytes.length < 24 || bytes.subarray(0, 8).toString('hex') !== '89504e470d0a1a0a') {
    throw new Error('Invalid PNG capture: ' + file);
  }
}
await mkdir(destination, { recursive: true });
for (const [name, captured] of Object.entries(screenshots)) {
  await copyFile(resolve(captureDir, captured), resolve(destination, name));
}
const manifest = {
  status: 'captured',
  source_sha: sha,
  captured_at_utc: new Date().toISOString(),
  environment: 'Playwright Chromium with deterministic desktop mock bridge; not native Tauri/Windows',
  generated_by: 'bun run ui:refresh (apps/desktop)',
  scenes: {
    'workbench.png': 'Graphite workbench, 1366x768 CSS viewport; selected and dry-run playing rows',
    'compact.png': 'Graphite compact workbench, 800x560 CSS viewport',
    'overlay.png': 'Browser-only responsive inspector overlay, 640x448; not supported native window minimum',
    'diagnostics.png': 'Runtime diagnostics, dry run without physical keyboard input, 1366x768',
  },
};
await writeFile(resolve(destination, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
console.log('Updated canonical UI screenshots at ' + sha + ': ' + Object.keys(screenshots).join(', '));
