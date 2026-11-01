import { existsSync, readFileSync } from 'node:fs';
import { spawn, spawnSync } from 'node:child_process';
import { basename, dirname, resolve } from 'node:path';
import { constants as osConstants } from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const exactVersion = /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$/;
const rustToolchainPath = 'rust/rust-toolchain.toml';
const bunManifestPath = 'site/package.json';

function invalid(message) {
  throw new Error(message);
}

function tomlValue(line, key) {
  const match = new RegExp(`^${key}\\s*=\\s*(.*?)\\s*$`).exec(line);
  if (!match) invalid(`Malformed Rust toolchain setting: ${key}`);
  return match[1];
}

function quotedString(raw, key) {
  const match = /^"([^"\\]*)"$/.exec(raw);
  if (!match) invalid(`Rust toolchain ${key} must be a plain quoted string`);
  return match[1];
}

function stringArray(raw, key) {
  if (!/^\[\s*(?:"[^"\\]*"\s*(?:,\s*"[^"\\]*"\s*)*)?\]$/.test(raw)) {
    invalid(`Rust toolchain ${key} must be a simple string array`);
  }
  const values = [...raw.matchAll(/"([^"\\]*)"/g)].map((match) => match[1]);
  if (values.some((value) => !value) || new Set(values).size !== values.length) {
    invalid(`Rust toolchain ${key} contains an empty or duplicate value`);
  }
  return values;
}

export function parseRustToolchain(source) {
  const settings = new Map();
  let inToolchain = false;
  let toolchainTables = 0;
  for (const [index, rawLine] of source.replace(/^\uFEFF/, '').split(/\r?\n/).entries()) {
    const line = rawLine.split('#', 1)[0].trim();
    if (!line) continue;
    if (line.startsWith('[')) {
      if (line !== '[toolchain]') invalid(`Unsupported Rust toolchain table at line ${index + 1}`);
      if (++toolchainTables !== 1) invalid('rust/rust-toolchain.toml has duplicate [toolchain] tables');
      inToolchain = true;
      continue;
    }
    if (!inToolchain) invalid(`Rust toolchain setting appears outside [toolchain] at line ${index + 1}`);
    const key = /^([A-Za-z0-9_-]+)\s*=/.exec(line)?.[1];
    if (!key) invalid(`Unsupported Rust toolchain syntax at line ${index + 1}`);
    if (settings.has(key)) invalid(`rust/rust-toolchain.toml has duplicate ${key} settings`);
    if (!['channel', 'profile', 'targets', 'components'].includes(key)) {
      invalid(`Unsupported Rust toolchain setting: ${key}`);
    }
    settings.set(key, tomlValue(line, key));
  }
  if (toolchainTables !== 1 || settings.size !== 4) {
    invalid('rust/rust-toolchain.toml must define one complete [toolchain] table');
  }
  const channel = quotedString(settings.get('channel'), 'channel');
  if (!exactVersion.test(channel)) invalid('[toolchain].channel must be an exact stable numeric version');
  const profile = quotedString(settings.get('profile'), 'profile');
  if (profile !== 'minimal') invalid('[toolchain].profile must remain minimal');
  const targets = stringArray(settings.get('targets'), 'targets');
  const components = stringArray(settings.get('components'), 'components');
  if (targets.length === 0 || components.length === 0) {
    invalid('Rust toolchain targets and components must not be empty');
  }
  return {
    channel,
    profile,
    targets,
    components,
  };
}

export function parseBunPackageManager(source) {
  if ([...source.matchAll(/"packageManager"\s*:/g)].length !== 1) {
    invalid('site/package.json must contain exactly one packageManager field');
  }
  const manifest = JSON.parse(source);
  if (!manifest || Array.isArray(manifest) || typeof manifest !== 'object') {
    invalid('site/package.json must contain a JSON object');
  }
  const packageManager = manifest.packageManager;
  if (typeof packageManager !== 'string' || !packageManager.startsWith('bun@')) {
    invalid('site/package.json packageManager must use bun@<exact-version>');
  }
  const version = packageManager.slice('bun@'.length);
  if (!exactVersion.test(version)) invalid('site/package.json packageManager must be an exact numeric version');
  return version;
}

function assertNoConflictingOverrides(version, environment) {
  const rustupKeys = Object.keys(environment).filter((key) => key.toUpperCase() === 'RUSTUP_TOOLCHAIN');
  if (rustupKeys.length > 1) invalid('Inherited RUSTUP_TOOLCHAIN override is ambiguous');
  if (rustupKeys.length === 1 && environment[rustupKeys[0]] !== version) {
    invalid(`RUSTUP_TOOLCHAIN conflicts with ${rustToolchainPath} (${environment[rustupKeys[0]]})`);
  }
  for (const key of Object.keys(environment)) {
    if (['RUSTC', 'CARGO_BUILD_RUSTC'].includes(key.toUpperCase())) {
      invalid(`${key} overrides are not supported with the canonical Rust toolchain`);
    }
  }
}

export function readPins(root = repositoryRoot, environment = process.env) {
  for (const path of ['rust-toolchain.toml', 'rust-toolchain']) {
    if (existsSync(resolve(root, path))) invalid(`Conflicting root Rust toolchain file: ${path}`);
  }
  const legacyRustToolchainPath = 'rust/rust-toolchain';
  if (existsSync(resolve(root, legacyRustToolchainPath))) {
    invalid(
      `Conflicting legacy Rust toolchain file can shadow ${rustToolchainPath}: ${legacyRustToolchainPath}`,
    );
  }
  for (const configName of ['.cargo/config.toml', '.cargo/config']) {
    const cargoConfig = resolve(root, configName);
    if (existsSync(cargoConfig)) {
      const source = readFileSync(cargoConfig, 'utf8');
      if (/^\s*(?:rustc|RUSTUP_TOOLCHAIN)\s*=/m.test(source)) {
        invalid(`${configName} contains a conflicting Rust compiler override`);
      }
    }
  }
  const rust = parseRustToolchain(readFileSync(resolve(root, rustToolchainPath), 'utf8'));
  const bun = parseBunPackageManager(readFileSync(resolve(root, bunManifestPath), 'utf8'));
  assertNoConflictingOverrides(rust.channel, environment);
  return { rust, bun };
}

function toolOutput(command, args, environment) {
  const result = spawnSync(command, args, { encoding: 'utf8', env: environment, windowsHide: true });
  if (result.error) throw result.error;
  if (result.status !== 0) invalid(`${command} ${args.join(' ')} failed with exit code ${result.status}`);
  return (result.stdout ?? '').trim();
}

function verifyRust(version) {
  const active = toolOutput('rustup', ['show', 'active-toolchain'], process.env);
  const rustup = toolOutput('rustup', ['--version'], process.env);
  const rustc = toolOutput('rustc', ['--version'], process.env);
  const cargo = toolOutput('cargo', ['--version'], process.env);
  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  if (!new RegExp(`^${escaped}(?:-|\\s|$)`).test(active)) invalid(`Active Rustup toolchain mismatch: ${active}`);
  if (!new RegExp(`^rustc ${escaped}(?:\\s|$)`).test(rustc)) invalid(`rustc version mismatch: ${rustc}`);
  if (!new RegExp(`^cargo ${escaped}(?:\\s|$)`).test(cargo)) invalid(`Cargo version mismatch: ${cargo}`);
  console.log(`rustup: ${rustup}`);
  console.log(`active toolchain: ${active}`);
  console.log(rustc);
  console.log(cargo);
}

async function executeRustCommand(command, args, version) {
  const executable = basename(command).replace(/\.exe$/i, '').toLowerCase();
  if (executable === 'cargo' && args[0]?.startsWith('+')) {
    invalid('Explicit cargo +toolchain overrides are not supported');
  }
  if (executable === 'cargo') {
    for (let index = 0; index < args.length; index++) {
      const config = args[index] === '--config' ? args[index + 1] : args[index].startsWith('--config=') ? args[index].slice('--config='.length) : '';
      if (/^(?:build\.)?rustc\s*=/.test(config.trim())) {
        invalid('Explicit cargo --config build.rustc overrides are not supported');
      }
    }
  }
  if (executable === 'rustup' && (args[0] === 'run' || args[0] === 'override')) {
    invalid('Explicit rustup run/override commands are not supported');
  }

  const child = spawn(command, args, {
    cwd: process.cwd(),
    env: { ...process.env, RUSTUP_TOOLCHAIN: version },
    stdio: 'inherit',
    windowsHide: true,
  });
  const signalNames = process.platform === 'win32'
    ? ['SIGINT', 'SIGTERM', 'SIGBREAK']
    : ['SIGHUP', 'SIGINT', 'SIGQUIT', 'SIGTERM'];
  const handlers = new Map(signalNames.map((signal) => {
    const handler = () => child.kill(signal);
    process.on(signal, handler);
    return [signal, handler];
  }));

  await new Promise((resolveClose) => {
    child.once('error', (error) => {
      console.error(`Could not start ${command}: ${error.message}`);
      process.exitCode = 1;
    });
    child.once('close', (code, signal) => {
      for (const [name, handler] of handlers) process.off(name, handler);
      if (signal) {
        process.exitCode = 128 + (osConstants.signals[signal] ?? 1);
        if (process.platform !== 'win32' && osConstants.signals[signal]) process.kill(process.pid, signal);
      } else {
        process.exitCode = code ?? 1;
      }
      resolveClose();
    });
  });
}

async function main(args) {
  if (args[0] === 'read' && ['rust', 'bun'].includes(args[1]) && args.length === 2) {
    const pins = readPins();
    console.log(args[1] === 'rust' ? pins.rust.channel : pins.bun);
    return;
  }
  if (args[0] === 'verify' && args[1] === 'rust' && args.length === 2) {
    const { rust } = readPins();
    verifyRust(rust.channel);
    return;
  }
  if (args[0] === 'exec' && args[1] === '--' && args.length >= 3) {
    const { rust } = readPins();
    await executeRustCommand(args[2], args.slice(3), rust.channel);
    return;
  }
  invalid('Usage: node scripts/toolchain-pin.mjs read <rust|bun> | verify rust | exec -- <command> [args...]');
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
