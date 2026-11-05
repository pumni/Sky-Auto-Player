import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { ESLint } from 'eslint';

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const eslint = new ESLint({ cwd: desktopRoot });

async function withFixture(parent, source, run) {
  const directory = mkdtempSync(resolve(parent, 'tooling-policy-'));
  const file = resolve(directory, 'fixture.tsx');
  writeFileSync(file, source);
  try {
    return await run(file, relative(desktopRoot, file).replaceAll('\\', '/'));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

function biomeLint(filePath) {
  const result = spawnSync('bun', ['run', 'biome', 'lint', filePath], {
    cwd: desktopRoot,
    encoding: 'utf8',
    windowsHide: true,
  });
  assert.equal(result.error, undefined, result.error?.message);
  return { status: result.status, output: `${result.stdout ?? ''}${result.stderr ?? ''}` };
}

async function eslintLint(source, filePath) {
  return (await eslint.lintText(source, { filePath }))[0];
}

test('Biome and ESLint reject Tauri imports outside bridge and platform adapters', async () => {
  const invalid = `import { invoke } from '@tauri-apps/api/core';
export function invalidImport() { return invoke('get_state'); }
`;
  const allowed = `import { invoke } from '@tauri-apps/api/core';
export function platformAdapter() { return invoke('get_state'); }
`;

  await withFixture(resolve(desktopRoot, 'src/components'), invalid, async (file, relativePath) => {
    const biome = biomeLint(relativePath);
    assert.notEqual(biome.status, 0, biome.output);
    assert.match(biome.output, /noRestrictedImports/);
    const lint = await eslintLint(invalid, file);
    assert.ok(lint.messages.some((message) => message.ruleId === 'no-restricted-imports'));
  });

  await withFixture(resolve(desktopRoot, 'src/platform'), allowed, async (file, relativePath) => {
    assert.equal(biomeLint(relativePath).status, 0);
    const lint = await eslintLint(allowed, file);
    assert.equal(lint.errorCount, 0);
    assert.equal(lint.warningCount, 0);
  });
});

test('Biome and ESLint reject conditional React Hooks', async () => {
  const invalid = `import { useEffect } from 'react';
export function invalidHook({ enabled }) {
  if (enabled) useEffect(() => {}, []);
  return null;
}
`;
  await withFixture(resolve(desktopRoot, 'src/components'), invalid, async (file, relativePath) => {
    const biome = biomeLint(relativePath);
    assert.notEqual(biome.status, 0);
    assert.match(biome.output, /useHookAtTopLevel/);

    const lint = await eslintLint(invalid, file);
    assert.ok(lint.messages.some((message) => message.ruleId === 'react-hooks/rules-of-hooks'));
  });
});

test('ESLint rejects a React Compiler-invalid state update during render', async () => {
  const invalid = `import { useState } from 'react';
export function InvalidRender() {
  const [count, setCount] = useState(0);
  setCount(count + 1);
  return <div>{count}</div>;
}
`;
  await withFixture(resolve(desktopRoot, 'src/components'), invalid, async (file) => {
    const lint = await eslintLint(invalid, file);
    assert.ok(
      lint.messages.some((message) => message.ruleId === 'react-hooks/set-state-in-render'),
      lint.messages.map((message) => `${message.ruleId}: ${message.message}`).join('\n'),
    );
  });
});

test('generated bridge files remain excluded from both linter gates', async () => {
  const generated = `import { invoke } from '@tauri-apps/api/core';
export const generated = invoke('get_state');
`;
  const parent = resolve(desktopRoot, 'src/bridge/generated');
  const parentExisted = existsSync(parent);
  mkdirSync(parent, { recursive: true });
  try {
    await withFixture(parent, generated, async (file, relativePath) => {
      const biome = biomeLint(relativePath);
      assert.notEqual(biome.status, 0, biome.output);
      assert.match(biome.output, /No files were processed/);
      assert.match(biome.output, /provided but ignored/);

      const lint = await eslintLint(generated, file);
      assert.equal(lint.errorCount, 0);
      assert.equal(
        lint.messages.some((message) => message.ruleId === 'no-restricted-imports'),
        false,
      );
    });
  } finally {
    if (!parentExisted) rmSync(parent, { recursive: true, force: true });
  }
});
