import { existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repoRoot = resolve(desktopRoot, '..');
const defaultExecutable = join(repoRoot, 'rust', 'target', 'debug', 'sky_desktop_shell.exe');

function run(program, args, options = {}) {
  const result = spawnSync(program, args, {
    cwd: repoRoot,
    encoding: 'utf8',
    windowsHide: true,
    ...options,
  });
  return {
    ...result,
    stdout: result.stdout?.trim() ?? '',
    stderr: result.stderr?.trim() ?? '',
  };
}

function commandText(program, args, { allowEmpty = false } = {}) {
  const result = run(program, args);
  if (result.error || result.status !== 0 || (!allowEmpty && !result.stdout)) {
    const detail = result.error?.message ?? result.stderr ?? `exit code ${result.status}`;
    throw new Error(`${program} ${args.join(' ')} failed: ${detail}`);
  }
  return result.stdout;
}

function parseArgs(argv) {
  let executable = defaultExecutable;
  let explicitExecutable = false;
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--exe') {
      index += 1;
      if (!argv[index]) throw new Error('Missing value for --exe');
      executable = resolve(desktopRoot, argv[index]);
      explicitExecutable = true;
    } else if (arg === '--help') {
      console.log('Usage: bun run dev:build-info [-- --exe <path>]');
      process.exit(0);
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }
  return { executable, explicitExecutable };
}

function normalizedCommit(value) {
  return value.toLowerCase().replace(/-dirty$/, '');
}

let executable;
try {
  const parsedArgs = parseArgs(process.argv.slice(2));
  executable = parsedArgs.executable;
  const head = commandText('git', ['rev-parse', '--verify', 'HEAD']).toLowerCase();
  const dirty = commandText('git', ['status', '--porcelain'], { allowEmpty: true }).length > 0;
  const rustc = commandText('rustc', ['--version']);

  console.log(`repository_head: ${head}`);
  console.log(`worktree: ${dirty ? 'dirty' : 'clean'}`);
  console.log(`expected_debug_executable: ${defaultExecutable}`);
  console.log(`executable_checked: ${executable}`);
  console.log(`rustc: ${rustc}`);

  let failed = false;
  let incomplete = false;
  if (!existsSync(executable)) {
    const state = parsedArgs.explicitExecutable ? 'FAIL' : 'INCOMPLETE';
    console.log(`embedded_build_info: ${state} (executable does not exist)`);
    failed = parsedArgs.explicitExecutable;
    incomplete = !parsedArgs.explicitExecutable;
  } else {
    const result = run(executable, ['--selftest-build-info'], { timeout: 30_000 });
    if (result.error || result.status !== 0) {
      const detail = result.error?.message ?? result.stderr ?? `exit code ${result.status}`;
      console.log(`embedded_build_info: FAIL (${detail})`);
      failed = true;
    } else {
      try {
        const metadata = JSON.parse(result.stdout);
        const embedded = String(metadata.native_build_commit ?? '');
        console.log(`embedded_native_build_commit: ${embedded || 'missing'}`);
        if (normalizedCommit(embedded) !== head) {
          console.log(`freshness: FAIL (embedded commit does not match HEAD ${head})`);
          failed = true;
        } else {
          console.log('freshness: PASS (embedded commit matches HEAD)');
        }
      } catch (error) {
        console.log(`embedded_build_info: FAIL (invalid JSON: ${error.message})`);
        failed = true;
      }
    }
  }

  const result = failed ? 'FAIL' : incomplete ? 'INCOMPLETE' : 'PASS';
  console.log(`result: ${result}`);
  process.exitCode = failed ? 1 : incomplete ? 2 : 0;
} catch (error) {
  console.error(`build freshness diagnostic failed: ${error.message}`);
  process.exitCode = 1;
}
