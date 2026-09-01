# Wave 7 Optimization Baseline

Date recorded: 2026-09-02
Repository baseline: `main@509db0a191c22b8d082945473fa7b58fa6864f41`

This is the exact post-merge Wave 6 baseline used for the first Wave 7
experiment. It is one natural main qualification run, so all distribution
metrics below are provisional and are not p50/p95 claims.

## Qualification identity

| Field | Value |
| --- | --- |
| Workflow | `33554867111` |
| Event | `push` to `main` |
| Attempt | `1` |
| Ubuntu image | `ubuntu-24.04` |
| Windows image | `windows-2025-vs2026` |
| Rust | `1.98.0` |
| Bun | `1.4.0` |
| Rust cache | restore-key hit; exact cache output `false` |
| Package artifact | 9,239,431 bytes upload; 4,531,763-byte inner ZIP |
| Portable tree | 102 files / 101 managed entries |

## Job and critical-path timing

GitHub job durations from the run metadata:

| Job | ID | Duration |
| --- | ---: | ---: |
| Classify validation layers | `100013096026` | 38s |
| Static and security gates | `100013318132` | 3m36s |
| Windows compatibility and unit tests | `100013317999` | 8m16s |
| Packaged frozen unsigned app smoke | `100013317997` | 21m07s |
| Required CI gate | `100020171153` | 8s |
| Workflow creation to completion | `33554867111` | 22m07s |

The packaged job log gives these major boundaries:

| Packaged phase | Approximate elapsed |
| --- | ---: |
| Restricted environment construction and checks start | 20:24:17 UTC |
| Restricted `check static` completes | 21s after start |
| Restricted `check rust` | 20:24:38–20:31:19 UTC (6m41s) |
| Remaining restricted desktop checks | 20:31:19–20:33:37 UTC (about 2m18s) |
| `cargo xtask dist` qualification | 20:33:37–20:38:56 UTC (about 5m19s) |
| `verify-dist` | about 4s |
| Artifact upload | about 2s |
| Rust cache save in post-job cleanup | about 4m29s |

The current package job serializes the restricted source/test proof before
the exact distribution build. The separate Windows validation job runs much
of the same Rust/desktop coverage concurrently, but it is not restricted-PATH
and therefore is not a substitute for the package proof.

## Baseline coverage

The package-required path currently executes, in the packaged job, under a
restricted PATH with `python`, `python3`, `py`, and `uv` unavailable:

```text
cargo xtask check static
cargo xtask check rust
cargo xtask check desktop
cargo xtask dist --profile dist
cargo xtask verify-dist
packaged shell and GUI smoke
updater exact-artifact qualification
artifact upload
```

The static, validate, package and required aggregate gates remain required
according to the classifier outputs and required-gate truth table.

## Metrics status

```text
metrics_status = provisional
natural comparable main runs = 1
authoritative p50/p95 = unavailable (fewer than 10 representative runs)
```

Engineering targets remain targets only; they are not permission to remove
coverage:

```text
ordinary PR p50 <= 5m
ordinary PR p95 <= 8m
main exact p50 <= 10m
```

The first Wave 7 candidate is restricted validation/package parallelization.
No product code, realtime code, Tauri feature, compiler profile, or cache
strategy is changed by this baseline record.
