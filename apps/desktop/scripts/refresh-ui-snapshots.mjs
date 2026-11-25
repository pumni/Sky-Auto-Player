import { spawnSync } from 'node:child_process';
import { copyFile, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
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
  'appearance-settings-light.png': 'candidate-appearance-preview-light-1366x768.png',
  'compact-admission.png': 'candidate-admission-640x448.png',
};
const vqcScreenshots = {
  'tooltip-open.png': 'candidate-tooltip-settings-1366x768.png',
  'scrollbar-idle-workbench.png': 'candidate-scrollbar-idle-workbench.png',
  'scrollbar-visible-scrolling.png': 'candidate-scrollbar-scrolling-workbench.png',
  'scrollbar-visible-dragged.png': 'candidate-scrollbar-dragging-workbench.png',
  'profile-expanded-800x560.png': 'candidate-profile-expanded-800x560.png',
  'profile-select-open-800x560.png': 'candidate-profile-select-open-800x560.png',
  'profile-select-open-light-800x560.png': 'candidate-profile-select-open-light-800x560.png',
  'settings-categories-640x448.png': 'candidate-settings-nav-640x448.png',
  'settings-categories-640x320-scrolled.png': 'candidate-settings-nav-640x320-scrolled.png',
  'settings-select-open-640x320.png': 'candidate-settings-select-open-640x320.png',
  'settings-playback-dark.png': 'candidate-settings-playback-dark-1366x768.png',
  'settings-playback-light.png': 'candidate-settings-playback-light-1366x768.png',
  'settings-appearance-dark.png': 'candidate-settings-appearance-dark-1366x768.png',
  'settings-appearance-light.png': 'candidate-settings-appearance-light-1366x768.png',
  'settings-diagnostics-dark.png': 'candidate-settings-diagnostics-dark-1366x768.png',
  'settings-diagnostics-light.png': 'candidate-settings-diagnostics-light-1366x768.png',
  'settings-updates-dark.png': 'candidate-settings-updates-dark-1366x768.png',
  'settings-updates-light.png': 'candidate-settings-updates-light-1366x768.png',
  'settings-donate-dark.png': 'candidate-settings-donate-dark-1366x768.png',
  'settings-donate-light.png': 'candidate-settings-donate-light-1366x768.png',
  'settings-about-dark.png': 'candidate-settings-about-dark-1366x768.png',
  'settings-about-light.png': 'candidate-settings-about-light-1366x768.png',
  'inspector-details-tab.png': 'candidate-inspector-details-1366x768.png',
  'inspector-runtime-performance.png': 'candidate-runtime-diagnostics-1366x768.png',
  'inspector-runtime-timing.png': 'candidate-runtime-timing-1366x768.png',
  'inspector-events-tab.png': 'candidate-runtime-events-1366x768.png',
  'light-workbench.png': 'candidate-light-1366x768.png',
  'palette-comparison-contact-sheet.png': 'candidate-palette-comparison-contact-sheet.png',
  'workbench-palette-contact-sheet.png': 'candidate-workbench-palette-contact-sheet.png',
  'transport-play-idle.png': 'candidate-transport-play-idle-1366x768.png',
  'transport-pause-playing.png': 'candidate-transport-pause-playing-1366x768.png',
  'transport-resume-paused.png': 'candidate-transport-resume-paused-1366x768.png',
  'transport-long-title-compact.png': 'candidate-transport-long-title-640x448.png',
  'admission-light-640x448.png': 'candidate-admission-light-640x448.png',
  'recovery-dark-800x560.png': 'candidate-recovery-dark-800x560.png',
  'recovery-light-800x560.png': 'candidate-recovery-light-800x560.png',
  'update-dark-800x560.png': 'candidate-update-dark-800x560.png',
  'update-light-800x560.png': 'candidate-update-light-800x560.png',
};

function git(args) {
  const result = spawnSync('git', args, { cwd: repositoryRoot, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr || 'git command failed');
  return result.stdout.trim();
}

const sha = git(['rev-parse', 'HEAD']);
const desktopTreeSha = git(['rev-parse', 'HEAD:apps/desktop']);
const dirty = git([
  'status',
  '--porcelain',
  '--untracked-files=no',
  '--',
  'apps/desktop/src',
  'apps/desktop/tests',
  'rust',
]);
if (dirty)
  throw new Error(
    'UI source/tests are dirty. Commit changes before refreshing the canonical UI capture.',
  );

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
      GRAPHITE_WORKSPACE_CAPTURE_VIEWS: [
        'populated',
        'compact',
        'overlay',
        'runtime',
        'appearance-preview-light',
        'compact-admission',
        'inspector-details',
        'tooltip',
        'scrollbar-idle',
        'scrollbar-scrolling',
        'scrollbar-dragging',
        'profile-expanded',
        'profile-select',
        'profile-select-light',
        'runtime-timing',
        'runtime-events',
        'settings-nav',
        'settings-nav-tiny',
        'settings-select',
        ...['playback', 'appearance', 'diagnostics', 'updates', 'donate', 'about'].flatMap(
          (category) => [`settings-${category}-dark`, `settings-${category}-light`],
        ),
        'transport-idle',
        'transport-playing',
        'transport-paused',
        'transport-long-title',
        'admission-light',
        'recovery-dark',
        'recovery-light',
        'update-dark',
        'update-light',
        'light',
        ...['graphite', 'aurora', 'minimalist', 'slate', 'cyberpunk', 'classic'].flatMap(
          (palette) => [`palette-${palette}`, `workbench-${palette}`],
        ),
        'palette-contact-sheet',
        'workbench-palette-contact-sheet',
      ].join(','),
    },
  },
);
if (run.status !== 0) throw new Error('Visual capture failed; canonical images were not changed.');

for (const file of [...Object.values(screenshots), ...Object.values(vqcScreenshots)]) {
  const bytes = await readFile(resolve(captureDir, file));
  if (bytes.length < 24 || bytes.subarray(0, 8).toString('hex') !== '89504e470d0a1a0a') {
    throw new Error(`Invalid PNG capture: ${file}`);
  }
}
await mkdir(destination, { recursive: true });
for (const [name, captured] of Object.entries(screenshots)) {
  await copyFile(resolve(captureDir, captured), resolve(destination, name));
}
const vqcDestination = resolve(destination, 'vqc');
await mkdir(vqcDestination, { recursive: true });
for (const name of await readdir(vqcDestination)) {
  if (name.endsWith('.png') && !(name in vqcScreenshots)) {
    await rm(resolve(vqcDestination, name));
  }
}
for (const [name, captured] of Object.entries(vqcScreenshots)) {
  await copyFile(resolve(captureDir, captured), resolve(vqcDestination, name));
}
const vqcRows = {
  'tooltip-open.png': 'Settings keyboard-shortcut tooltip on the docked Workbench, 1366x768',
  'scrollbar-idle-workbench.png': 'Workbench with the focused scrollbar fully hidden at idle',
  'scrollbar-visible-scrolling.png': 'The same Workbench during actual wheel scrolling',
  'scrollbar-visible-dragged.png':
    'The same Workbench while the thumb remains actively dragged after scrolling settles',
  'profile-expanded-800x560.png':
    'Playback Profile with Advanced timing expanded at 800x560; all fields fit without scrolling',
  'profile-select-open-800x560.png': 'Playback Profile Base Hold Select open at 800x560 (Graphite)',
  'profile-select-open-light-800x560.png':
    'Playback Profile Base Hold Select open at 800x560 (Light)',
  'settings-categories-640x448.png': 'All six vertical Settings categories visible at 640x448',
  'settings-categories-640x320-scrolled.png': 'Settings category navigation scrolled at 640x320',
  'settings-select-open-640x320.png': 'Settings Dark palette Select open at 640x320',
  'settings-playback-dark.png': 'Settings Playback category in Graphite Dark at 1366x768',
  'settings-playback-light.png': 'Settings Playback category in Graphite Light at 1366x768',
  'settings-appearance-dark.png': 'Settings Appearance category in Graphite Dark at 1366x768',
  'settings-appearance-light.png': 'Settings Appearance category in Graphite Light at 1366x768',
  'settings-diagnostics-dark.png': 'Settings Diagnostics category in Graphite Dark at 1366x768',
  'settings-diagnostics-light.png': 'Settings Diagnostics category in Graphite Light at 1366x768',
  'settings-updates-dark.png': 'Settings Updates category in Graphite Dark at 1366x768',
  'settings-updates-light.png': 'Settings Updates category in Graphite Light at 1366x768',
  'settings-donate-dark.png': 'Settings Donate category in Graphite Dark at 1366x768',
  'settings-donate-light.png': 'Settings Donate category in Graphite Light at 1366x768',
  'settings-about-dark.png': 'Settings About category in Graphite Dark at 1366x768',
  'settings-about-light.png': 'Settings About category in Graphite Light at 1366x768',
  'inspector-details-tab.png': 'Inspector Details selected with the Readiness state visible',
  'inspector-runtime-performance.png':
    'Runtime Performance tab in deterministic dry-run browser state',
  'inspector-runtime-timing.png': 'Runtime Timing tab with the segmented indicator moved to Timing',
  'inspector-events-tab.png':
    'Runtime Events with full-width detail rows beneath their timestamps and names',
  'light-workbench.png':
    '100% undimmed Light Workbench with selected/playing/hovered rows, Runtime indicator, and secondary action',
  'palette-comparison-contact-sheet.png':
    'Rendered Settings Appearance comparison for Light and all six dark palettes',
  'workbench-palette-contact-sheet.png':
    'Undimmed Workbench comparison across Light and all six dark palettes (selected, playing, hovered rows, Runtime indicator, Player, secondary action)',
  'transport-play-idle.png':
    'Idle transport with a selected short title and truthful Selected status',
  'transport-pause-playing.png':
    'Filled Pause control while browser-mock dry-run playback is active',
  'transport-resume-paused.png':
    'Filled Resume control while browser-mock dry-run playback is paused',
  'transport-long-title-compact.png':
    'Compact Player Dock with a long title and readable selected/prepared status',
  'admission-light-640x448.png':
    'Compact 640x448 timing-risk confirmation in Light mode (paired with canonical Dark compact-admission.png)',
  'recovery-dark-800x560.png':
    'Actionable target_not_found playback recovery alert in Graphite Dark at 800x560',
  'recovery-light-800x560.png':
    'Actionable target_not_found playback recovery alert in Graphite Light at 800x560',
  'update-dark-800x560.png': 'Update dialog with release notes in Graphite Dark at 800x560',
  'update-light-800x560.png': 'Update dialog with release notes in Graphite Light at 800x560',
};
const vqcReadme = [
  '# VQC-R2 visual evidence',
  '',
  `Captured from desktop source [\`${sha.slice(0, 12)}\`](https://github.com/pumni/Sky-Auto-Player/commit/${sha}) (desktop tree \`${desktopTreeSha}\`) with Playwright Chromium and the deterministic browser mock bridge. These are rendered app states, not native Tauri or gameplay evidence. The Profile captures use 800x560; the compact Settings captures use 640x448 and 640x320.`,
  '',
  '| Capture | Rendered state |',
  '| --- | --- |',
  ...Object.entries(vqcRows).map(
    ([name, description]) => `| [${name}](${name}) | ${description} |`,
  ),
  '',
  'The palette contact sheet contains the Settings Appearance view in Light and each dark palette. The separate interactive checks cover idle, wheel, keyboard, settled, active drag, release, cancellation, no-overflow unmount, reduced-motion, and forced-colors behavior.',
  '',
].join('\n');
await writeFile(resolve(vqcDestination, 'README.md'), vqcReadme);
const manifest = {
  status: 'captured',
  source_sha: sha,
  desktop_tree_sha: desktopTreeSha,
  captured_at_utc: new Date().toISOString(),
  environment:
    'Playwright Chromium with deterministic desktop mock bridge; not native Tauri/Windows',
  generated_by: 'bun run ui:refresh (apps/desktop)',
  targeted_evidence: {
    directory: 'vqc/',
    source_sha: sha,
    desktop_tree_sha: desktopTreeSha,
    scenes: Object.keys(vqcScreenshots),
  },
  scenes: {
    'workbench.png': 'Graphite workbench, 1366x768 CSS viewport; selected and dry-run playing rows',
    'compact.png': 'Graphite compact workbench, 800x560 CSS viewport',
    'overlay.png':
      'Browser-only responsive inspector overlay, 640x448; not supported native window minimum',
    'diagnostics.png': 'Runtime diagnostics, dry run without physical keyboard input, 1366x768',
    'appearance-settings-light.png':
      'Review-only Appearance settings state with Light active and Graphite saved, 1366x768',
    'compact-admission.png':
      'Compact 640x448 pre-start timing-risk confirmation with readable title and risk badge',
  },
};
await writeFile(resolve(destination, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(
  `Updated UI screenshots at ${sha}: ${Object.keys(screenshots).length} canonical and ${Object.keys(vqcScreenshots).length} targeted captures`,
);
