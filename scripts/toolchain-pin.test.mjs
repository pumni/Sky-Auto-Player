import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';
import {
  assertBunVersion,
  parseBunPackageManager,
  parseBunVersionFile,
  parseRustToolchain,
  readBunPin,
  readPins,
} from './toolchain-pin.mjs';

const validRust = `[toolchain]\nchannel = "7.8.9"\nprofile = "minimal"\ntargets = ["x86_64-pc-windows-msvc"]\ncomponents = ["rustfmt", "clippy"]\n`;
const validBun = '{"name":"fixture","packageManager":"bun@7.8.9"}';

function fixtureRoot() {
  const root = mkdtempSync(join(tmpdir(), 'toolchain pin fixture with spaces-'));
  mkdirSync(join(root, 'rust'));
  mkdirSync(join(root, 'site'));
  writeFileSync(join(root, 'rust/rust-toolchain.toml'), validRust);
  writeFileSync(join(root, '.bun-version'), '7.8.9\n');
  writeFileSync(join(root, 'site/package.json'), validBun);
  return root;
}

test('reads only exact nested Rust and Bun package-manager pins', () => {
  assert.equal(parseRustToolchain(validRust).channel, '7.8.9');
  assert.equal(parseBunVersionFile('7.8.9\n'), '7.8.9');
  assert.equal(parseBunVersionFile('7.8.9'), '7.8.9');
  assert.equal(parseBunPackageManager(validBun), '7.8.9');
  assert.equal(assertBunVersion('7.8.9', '7.8.9'), '7.8.9');
  assert.throws(() => assertBunVersion('7.8.8', '7.8.9'), /Installed Bun version mismatch/);
  assert.throws(() => parseBunPackageManager('{"packageManager":"bun@2.3.4","packageManager":"bun@2.3.5"}'), /exactly one/);
});

test('rejects malformed canonical Bun pin files and stale site mirrors', () => {
  for (const source of ['stable\n', '7.8\n', '07.8.9\n', '7.8.9\n\n', ' 7.8.9\n']) {
    assert.throws(() => parseBunVersionFile(source), /\.bun-version/);
  }

  const root = fixtureRoot();
  try {
    assert.equal(readBunPin(root), '7.8.9');
    writeFileSync(join(root, 'site/package.json'), '{"packageManager":"bun@7.8.8"}');
    assert.throws(() => readBunPin(root), /must mirror \.bun-version/);
    writeFileSync(join(root, 'site/package.json'), '{"packageManager":"bun@latest"}');
    assert.throws(() => readBunPin(root), /exact numeric version/);
    writeFileSync(join(root, '.bun-version'), '7.8.8\n');
    writeFileSync(join(root, 'site/package.json'), validBun);
    assert.throws(() => readBunPin(root), /must mirror \.bun-version/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('rejects malformed, duplicate, floating, and conflicting Rust pin sources', () => {
  for (const source of [
    validRust.replace('channel = "7.8.9"\n', ''),
    validRust.replace('channel = "7.8.9"', 'channel = "stable"'),
    validRust.replace('channel = "7.8.9"', 'channel = "7.8"'),
    validRust.replace('channel = "7.8.9"', 'channel = "07.8.9"'),
    validRust.replace('channel = "7.8.9"\n', 'channel = "7.8.9"\nchannel = "7.8.10"\n'),
    validRust.replace('[toolchain]\n', '[toolchain]\n\n[toolchain]\n'),
    validRust.replace('profile = "minimal"', 'profile = "complete"'),
    validRust.replace('targets = ["x86_64-pc-windows-msvc"]', 'targets = []'),
  ]) assert.throws(() => parseRustToolchain(source));

  const root = fixtureRoot();
  try {
    assert.throws(() => readPins(root, { RUSTUP_TOOLCHAIN: 'stable' }), /conflicts/);
    assert.throws(() => readPins(root, { rustup_toolchain: 'stable' }), /conflicts/);
    assert.throws(() => readPins(root, { RUSTUP_TOOLCHAIN: '7.8.9', rustup_toolchain: '7.8.9' }), /ambiguous/);
    assert.equal(readPins(root, { RUSTUP_TOOLCHAIN: '7.8.9' }).rust.channel, '7.8.9');
    assert.throws(() => readPins(root, { RUSTC: 'rustc.exe' }), /RUSTC overrides/);
    assert.throws(() => readPins(root, { CARGO_BUILD_RUSTC: 'rustc.exe' }), /CARGO_BUILD_RUSTC/);
    writeFileSync(join(root, 'rust-toolchain.toml'), validRust);
    assert.throws(() => readPins(root, {}), /Conflicting root Rust toolchain file/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('rejects a contradictory sibling legacy Rustup file beside the canonical TOML pin', () => {
  const root = fixtureRoot();
  try {
    assert.equal(readPins(root, {}).rust.channel, '7.8.9');
    writeFileSync(join(root, 'rust/rust-toolchain'), '7.8.10\n');
    assert.throws(() => readPins(root, {}), /legacy Rust toolchain file can shadow/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('rejects root Cargo compiler overrides and malformed Bun pins', () => {
  const root = fixtureRoot();
  try {
    mkdirSync(join(root, '.cargo'));
    writeFileSync(join(root, '.cargo/config.toml'), '[build]\nrustc = "other-rustc"\n');
    assert.throws(() => readPins(root, {}), /conflicting Rust compiler override/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
  const alternateConfigRoot = fixtureRoot();
  try {
    mkdirSync(join(alternateConfigRoot, '.cargo'));
    writeFileSync(join(alternateConfigRoot, '.cargo/config'), '[build]\nrustc = "other-rustc"\n');
    assert.throws(() => readPins(alternateConfigRoot, {}), /conflicting Rust compiler override/);
  } finally {
    rmSync(alternateConfigRoot, { recursive: true, force: true });
  }
  for (const source of [
    '{"name":"fixture"}',
    '{"name":"fixture","packageManager":"bun@latest"}',
    '{"name":"fixture","packageManager":"bun@2.3"}',
    '{"nested":{"packageManager":"bun@2.3.4"},"packageManager":"bun@2.3.4"}',
  ]) assert.throws(() => parseBunPackageManager(source));
});

test('exec forwards output, exit status, canonical environment, and spaced arguments', () => {
  const script = resolve('scripts/toolchain-pin.mjs');
  const environment = { ...process.env, RUSTUP_TOOLCHAIN: undefined };
  delete environment.RUSTUP_TOOLCHAIN;
  const child = spawnSync(process.execPath, [
    script,
    'exec',
    '--',
    process.execPath,
    '-e',
    'process.stdout.write(`${process.env.RUSTUP_TOOLCHAIN}|${process.argv[1]}`); process.exit(23)',
    'argument path with spaces',
  ], { cwd: resolve('.'), encoding: 'utf8', env: environment, windowsHide: true });
  assert.equal(child.error, undefined);
  assert.equal(child.status, 23);
  assert.equal(child.stdout, `${readPins().rust.channel}|argument path with spaces`);

  for (const args of [
    ['cargo', '+stable', 'check'],
    ['cargo', '--config', 'build.rustc="other-rustc"', 'check'],
    ['rustup', 'override', 'set', 'stable'],
  ]) {
    const rejected = spawnSync(process.execPath, [script, 'exec', '--', ...args], {
      cwd: resolve('.'),
      encoding: 'utf8',
      env: environment,
      windowsHide: true,
    });
    assert.equal(rejected.status, 1, args.join(' '));
    assert.match(rejected.stderr, /not supported/);
  }
});
