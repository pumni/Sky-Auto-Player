import { assertBunVersion, readBunPin } from "./toolchain-pin.mjs";

try {
  const actual = Bun.version;
  assertBunVersion(actual, readBunPin());
  console.log(`bun --version ${actual} matches .bun-version and the site packageManager mirror`);
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
