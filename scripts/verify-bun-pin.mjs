import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const packagePath = resolve(repositoryRoot, "site/package.json");
const packageJson = JSON.parse(readFileSync(packagePath, "utf8"));
const packageManager = packageJson.packageManager;
const match = typeof packageManager === "string"
  ? /^bun@(\d+\.\d+\.\d+)$/.exec(packageManager)
  : null;

if (!match) {
  console.error("site/package.json must declare one exact Bun packageManager version");
  process.exit(1);
}

const expected = match[1];
const actual = Bun.version;
if (actual !== expected) {
  console.error(`bun --version mismatch: expected ${expected} from site/package.json, got ${actual}`);
  process.exit(1);
}

console.log(`bun --version ${actual} matches site/package.json packageManager`);
