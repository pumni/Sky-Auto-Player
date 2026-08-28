# Updater ZIP 8.6.0 qualification

Date: 2026-08-28

Source commit for the qualification: `4d3d4448f856c66e470d69a9f40dbc981de628f6`.

Environment: Windows 11 build `26200`, CPython `3.14.7+freethreaded`, uv
`0.12.7`, Rust `1.98.0`. CPU model capture was unavailable on this host.

## Workload

The input was the locally built Sky Auto Player v3.4.5 release archive, with
235 files and 30,835,159 bytes. Each run performed 10 warmups and 30 measured
iterations. Each iteration used the updater's `validate_zip_file` and
`extract_zip_file` path and wrote to a fresh temporary staging directory.

The benchmark harness was temporary and was removed after qualification; the
numbers below are the measured output, not a synthetic single-file test.

| Configuration | Run | Median (us) | p95 (us) | Max (us) | Total (us) |
| --- | ---: | ---: | ---: | ---: | ---: |
| zip 2.4.2 + existing DEFLATE | 1 | 1,225,344 | 1,436,732 | 1,519,560 | 37,155,405 |
| zip 2.4.2 + existing DEFLATE | 2 | 1,097,057 | 1,258,793 | 6,759,036 | 38,745,606 |
| zip 2.4.2 + existing DEFLATE | 3 | 1,089,364 | 1,458,281 | 1,504,756 | 34,835,580 |
| zip 2.4.2 + existing DEFLATE | 4 | 1,161,318 | 1,447,097 | 1,956,166 | 36,295,267 |
| zip 2.4.2 + existing DEFLATE | 5 | 1,088,264 | 1,283,615 | 3,849,879 | 35,319,997 |
| zip 8.6.0 + `zlib-rs` | 1 | 1,015,414 | 1,069,923 | 1,086,592 | 30,474,468 |
| zip 8.6.0 + `zlib-rs` | 2 | 1,012,312 | 1,079,587 | 1,244,826 | 30,740,054 |
| zip 8.6.0 + `zlib-rs` | 3 | 1,022,204 | 1,603,139 | 3,013,627 | 33,370,925 |
| zip 8.6.0 + `zlib-rs` | 4 | 993,318 | 1,292,528 | 1,444,442 | 31,329,758 |
| zip 8.6.0 + `zlib-rs` | 5 | 1,086,462 | 1,374,329 | 1,578,989 | 33,505,543 |

Across-run medians are approximately:

- median extraction: `1,097,057 -> 1,015,414 us` (`-7.4%`);
- p95: `1,436,732 -> 1,292,528 us` (`-10.0%`);
- max: `1,956,166 -> 1,444,442 us` (`-26.2%`);
- measured total: `36,295,267 -> 31,329,758 us` (`-13.7%`).

The candidate had one p95/max outlier, so this is a measurable qualification
result, not a claim of a guaranteed tail bound. The benchmark does not include
network download time; the updater's download, manifest, hash, transaction,
rollback, and user-state paths were qualified separately by the security E2E.

## Compatibility and security

- `cargo check -p sky_updater --all-targets --all-features --locked`: PASS.
- `cargo test -p sky_updater --all-features --locked`: 58 tests PASS.
- Packaged update and injected rollback E2E: PASS.
- Release manifest verification: PASS.
- Source PyInstaller bootloader build and app smoke tests: PASS.

The candidate changes only the updater ZIP reader/decompression dependency;
public release asset names, manifest format, path validation, hash checks,
transaction layout, rollback behavior, and preserved user state are unchanged.

## Decision

**MERGE NOW**, subject to the normal full-workspace gate. The candidate has a
repeatable median/p95 improvement on a real release archive and passed the
updater safety matrix. Keep the reproducibility result separate: two same-host
builds using the current `Compress-Archive` pipeline were not byte-identical,
so ZIP producer reproducibility remains an investigation item rather than a
silent packaging change.
