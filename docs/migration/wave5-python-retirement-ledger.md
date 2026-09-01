# Wave 5 Python retirement ledger

Baseline: `1634729acbdc236e0e0964a3fc9f74283a68c1c6`

This ledger covers every Python file under `scripts/`, `src/`, and
`tests/` at the accepted Wave 4 baseline. The machine-readable companion
contains one exact entry per baseline path, plus each new Wave 5 tooling file;
no path is classified twice.

## Counts

- DUPLICATE: 26
- FIXTURE_FROZEN: 2
- MIGRATED: 98
- OBSOLETE: 58
- TOOLING_RETAINED: 26
- TRANSPORT_ONLY: 11

## Classification

- **MIGRATED** — native/TypeScript tests now prove the product invariant.
- **OBSOLETE** — retired presentation, manual experiment, or unsupported
  product surface.
- **TRANSPORT_ONLY** — Python Core/desktop IPC transport or startup guard.
- **DUPLICATE** — stronger direct native/build evidence replaces the bridge.
- **FIXTURE_FROZEN** — committed fixture data remains as test input.
- **TOOLING_RETAINED** — repository-only tooling/guard test deferred to Wave 6.

## Deletion rule

Every deleted Python source, script, or test in the Wave 5 diff appears as an
exact path in the JSON ledger. Retained Python files are explicitly
`TOOLING_RETAINED`; they are not installable product code and do not prove a
missing product behavior. Evidence roots and replacement boundaries are listed
in the JSON entry metadata.

Wave 5 adds three repository-only Python files: the retirement guard, its
direct test, and `release_common.py`. They are explicitly marked
`TOOLING_RETAINED` and are scheduled for the Wave 6 `xtask` migration.

The supported product is native Tauri/Rust. Wave 6 may remove the remaining
release/CI Python orchestration after replacing it with `cargo xtask`.
