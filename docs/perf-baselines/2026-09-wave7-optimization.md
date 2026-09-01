# Wave 7 Optimization Report

Date recorded: 2026-09-02
Wave 6 baseline: `main@509db0a191c22b8d082945473fa7b58fa6864f41`

This report records the measured Wave 7 experiments completed so far. The
adopted changes are CI critical-path parallelization, removal of redundant
packaged Chromium setup, and restore-only Rust caching for pull-request
packaged jobs. No product, realtime, compiler-profile, or Tauri-feature
behavior was changed.

## Method and baseline

The baseline is the exact post-merge Wave 6 main qualification run:

| Field | Value |
| --- | --- |
| Workflow | `33554867111` |
| Runner images | `ubuntu-24.04`; `windows-2025-vs2026` |
| Rust / Bun | `1.98.0` / `1.4.0` |
| Baseline cache | packaged exact cache miss; restore-key hit |
| Static | 3m36s |
| Windows validation | 8m16s |
| Packaged job | 21m07s |
| Required gate | 8s |
| Workflow creation to completion | 22m07s |
| Artifact | 9,239,431-byte upload; 4,531,763-byte inner ZIP |

The baseline package-required path serialized restricted repository checks in
the packaged job before running `dist`. It included, under the restricted
PATH, `check static`, `check rust`, `check desktop`, `dist`, `verify-dist`,
packaged shell/GUI smoke, updater exact-artifact qualification, and upload.

All comparisons below are provisional. The baseline and candidate use the
same hosted Windows image family, but they are not a controlled ten-run
sample and the candidate validation cache was warm while its packaged cache
was a miss.

## Candidate A — parallel restricted validation and exact dist

### Hypothesis

Move the restricted source/test proof into the required Windows validation
job while keeping the restricted exact production build and package proof in
the packaged job. Package-sensitive changes must force `code_required=true`,
so the moved restricted validation cannot be skipped. This removes serialized
work without removing a gate.

### Implementation

Candidate commits:

| Commit | Purpose |
| --- | --- |
| `c3615ffc9ed7baf0e838a38eaa647c17fd311bc7` | initial Lane A topology candidate; rejected before adoption because its new classifier assertion exposed a Clippy failure and the step lacked explicit intermediate exit checks |
| `f79a955956dd15c213aea662dba6d8f2149f273b` | corrected candidate; Clippy-clean assertions and fail-fast restricted validation |

The final candidate changes only CI classification/workflow routing and the
Wave 7 baseline/report records. Product and realtime source are unchanged.

### Coverage equivalence

| Gate | Wave 6 baseline | Candidate A |
| --- | --- | --- |
| Security / architecture / retirement | static job and packaged restricted proof | static job and required restricted validation |
| Rust fmt, Clippy, workspace tests | packaged restricted `check rust`; Windows job also validated | required restricted `check rust` in Windows job; normal package build remains separate |
| Desktop native checks | packaged restricted `check desktop`; Windows validation | required restricted `check desktop` in Windows job |
| Python-unavailable source/test proof | packaged job | Windows validation job, with `python`, `python3`, `py`, and `uv` unavailable |
| Exact production build | packaged job, restricted PATH | packaged job, restricted PATH |
| Provenance observation | build and copied binaries | unchanged in packaged `dist` |
| `verify-dist` | packaged job | packaged job |
| Shell and GUI smoke | normal and restricted packaged smoke | unchanged, both timed packaged paths |
| Updater exact-artifact suite | packaged job | packaged job |
| Artifact upload / manifest | packaged job | packaged job |

The classifier now makes every package-sensitive path code-required. The
required gate therefore waits for the Windows validation job as well as the
packaged job. The package job no longer runs the three repository checks, but
it still performs the complete restricted production-build and artifact
qualification path.

### Candidate qualification

| Metric | Baseline | Candidate A (`f79a9559`) | Delta | Cache state |
| --- | ---: | ---: | ---: | --- |
| Workflow critical path | 22m07s | 10m59s | -11m08s (-50.3%) | baseline packaged miss; candidate validate hit / packaged miss |
| Static job | 3m36s | 3m37s | +1s | separate job |
| Windows validation job | 8m16s | 6m39s | -1m37s | candidate exact validation cache hit |
| Packaged job | 21m07s | 10m05s | -11m02s | candidate packaged exact cache miss |
| `dist` qualification phase | about 5m19s after restricted checks | 6m12s build/qualification phase | not directly comparable | candidate restricted package path |
| Artifact upload | 9,239,431 bytes | 9,239,259 bytes | -172 bytes | different commit/build |
| Inner ZIP | 4,531,763 bytes | 4,531,747 bytes | -16 bytes | different commit/build |

Candidate workflow and jobs:

| Job | ID | Duration | Result |
| --- | ---: | ---: | --- |
| Classify validation layers | `100031910173` | 32s | PASS |
| Static and security gates | `100032087798` | 3m37s | PASS |
| Windows compatibility and unit tests | `100032087781` | 6m39s | PASS |
| Packaged frozen unsigned app smoke | `100032087678` | 10m05s | PASS |
| Required CI gate | `100035127542` | 11s | PASS |
| Workflow | `33560612129` | 10m59s | PASS |

The restricted validation log proves all four discovery commands were absent
and that `check static`, `check rust`, and `check desktop` completed with
PASS. The packaged log proves `dist`, `verify-dist`, both normal/restricted
shell and GUI smoke paths, updater qualification, and upload completed with
PASS. The candidate package's Rust-cache save reported a key reservation
collision during post-job cleanup; it did not affect qualification or artifact
contents and is not counted as a cache optimization.

### Candidate artifact inspection

The uploaded candidate artifact was downloaded from Actions and inspected,
not inferred from the generated summary:

| Field | Value |
| --- | --- |
| Artifact ID | `9821354646` |
| Upload size | 9,239,259 bytes |
| Upload digest | `sha256:b3998dcd0549b67590c9381ed755b76072b0fa0fce4f3d79b0645144a3fa8238` |
| Inner ZIP | `Sky-Auto-Player-v3.5.0.zip` |
| Inner ZIP size / SHA-256 | 4,531,747 bytes / `feda06896120cf6b00162c16f8c7d0dacd689a85a13d8c41e46c90eff4ce9edc` |
| MANIFEST SHA-256 | `626695d3f6a006cf68e2706febec075f1d5b8ace6a6a0f5ab224996fc2eaacc5` |
| Portable / managed entries | 102 / 101 |

The downloaded tree had no missing manifest entries, no unexpected managed
entries (the only un-managed file was `MANIFEST.json`), and zero file
hash/size mismatches. The ZIP entry set matched the release tree exactly.
The negative scan found zero Core/Python/runtime/test artifacts. Desktop and
calibration metadata observed from the copied binaries reported:

```text
repo_head                         = f79a955956dd15c213aea662dba6d8f2149f273b
desktop.native_build_commit       = f79a955956dd15c213aea662dba6d8f2149f273b
calibration.source_git_sha        = f79a955956dd15c213aea662dba6d8f2149f273b
calibration.native_build_id       = f79a955956dd15c213aea662dba6d8f2149f273b
MANIFEST.git_head                 = f79a955956dd15c213aea662dba6d8f2149f273b
MANIFEST.native_build_commit      = f79a955956dd15c213aea662dba6d8f2149f273b
PROVENANCE.repo_head              = f79a955956dd15c213aea662dba6d8f2149f273b
PROVENANCE.native_build_commit    = f79a955956dd15c213aea662dba6d8f2149f273b
native_source_fingerprint         = 6aa3f9d6f05f1c1778d3cdfba1e7cf2a5ba8867d7259501fefc9d42bd11c56d9
```

### Decision

`ADOPTED`, subject to final exact-HEAD qualification after this report is
committed. The observed reduction is large, coverage is preserved, and the
change is independently revertible in the CI/classifier files. No claim is
made that the engineering p50/p95 targets are met.

## Candidate A.1 — remove redundant packaged Chromium installation

### Hypothesis

The packaged job no longer runs browser E2E after Candidate A, and `cargo
xtask dist` does not invoke Playwright. Removing only the packaged Chromium
installation should save setup work without changing packaged Tauri GUI
smoke, restricted-PATH qualification, updater qualification, or artifact
contents. The validation job must retain its Chromium installation because it
still runs the browser E2E suite.

### Implementation and qualification

| Commit | Purpose |
| --- | --- |
| `34ef2f7431b10991181021ac0dca234cc7b53ab5` | Remove packaged-job Chromium installation only; retain validation-job installation |

The exact A.1 workflow was `33567068463` on `34ef2f74`, with all five
required jobs passing. The validation log still contains `Install Chromium
for desktop browser smoke` and desktop checks pass. The packaged log has no
Chromium/Playwright installation, while `dist`, both normal and restricted
shell/GUI smoke paths, the updater exact-artifact suite, and upload all pass.

| Metric | Before A.1 (`2aeb9e30`) | A.1 (`34ef2f74`) | Delta | Cache state |
| --- | ---: | ---: | ---: | --- |
| Packaged job | 10m06s | 10m18s | +12s | both restore-key-hit/no exact cache; hosted-run variance |
| Packaged Chromium setup | present, about 23s | absent | about -23s setup work | validation install unchanged |
| Artifact upload | 9,239,364 bytes | 9,239,341 bytes | -23 bytes | different commit |

The total packaged-job comparison is not a controlled speedup because cache
restore/build timing varied. The direct setup removal is nevertheless
verified by the exact logs, and no packaged phase consumed Playwright.

### A.1 artifact inspection

The uploaded artifact was downloaded and inspected independently:

| Field | Value |
| --- | --- |
| Artifact ID | `9823765983` |
| Upload size | 9,239,341 bytes |
| Upload digest | `sha256:66be676402ad678facdc13bfb2fd6d0a103b0ecdd844b65a922750391e8fe48b` |
| Inner ZIP size / SHA-256 | 4,531,763 bytes / `64e83bd2ce5b3221537b7664a657410585071052dc1211a8f4a85ba333c83830` |
| MANIFEST SHA-256 | `46f1e280e8aaa142c244530ac564a56762efeb8a2f4a52f3f64ae4ad066894f2` |
| Portable / managed entries | 102 / 101 |

The release tree and ZIP each contain the expected 102 files; 101 managed
entries have zero missing/unexpected/hash/size mismatches. The runtime/test
negative scan is zero. Observed desktop/calibration provenance and the
manifest/provenance commit are all `34ef2f7431b10991181021ac0dca234cc7b53ab5`;
the native source fingerprint remains
`6aa3f9d6f05f1c1778d3cdfba1e7cf2a5ba8867d7259501fefc9d42bd11c56d9`.

### A.1 decision

`ADOPTED`. This is a narrow, independently revertible workflow cleanup with
coverage preserved. It does not justify any p50/p95 claim.

## Candidate B — pull-request packaged Rust cache restore-only policy

### Hypothesis

The packaged job restores the existing Rust cache but, on pull requests,
spends about 145 seconds after artifact upload cleaning/compressing a cache
that commonly collides with another writer. Setting the packaged action's
`save-if` to false for pull requests should keep restore behavior and all
build/package coverage while removing that post-job critical-path write. Main
pushes and manual runs remain the cache population path.

### A/B implementation

| Commit | Purpose |
| --- | --- |
| `c1fc52eccccbde5b9e0826f4291150427e87c5e5` | Set packaged Rust cache `save-if: ${{ github.event_name != 'pull_request' }}` |

The A/B baseline was the A.1 workflow `33567068463` (`save-if: true`); the
candidate workflow was `33568432155` on `c1fc52ec` (`save-if: false`). Both
used the same 842 MiB restore-key-hit cache path and completed the complete
restricted package qualification.

| Metric | Baseline A.1 | Candidate B | Delta | Cache state |
| --- | ---: | ---: | ---: | --- |
| Packaged job | 10m18s | 6m17s | -4m01s | both restore-key hit; host restore/build noise remains |
| Cache restore | about 35s | about 32s | -3s | about 842 MiB restored in both |
| Upload completed | 22:44:17Z | 23:00:05Z | n/a | qualification and upload unchanged |
| Post Rust cache | 22:44:18–22:46:43Z (145s) | no save phase | about -145s | `save-if=false` on PR |
| Workflow creation to completion | 11m12s | 10m11s | -1m01s | validation was slower in candidate B |

The package-job reduction is consistent with removing the observed post-job
save and the candidate log explicitly reports `save-if: false`; no cache
save/cleanup follows artifact upload. The end-to-end workflow delta is not
attributed to this candidate because the validation job took 9m17s versus
6m54s in the baseline. Cold-cache behavior was not separately sampled; this
policy changes only PR saving, not restore or compilation, while push/manual
runs retain saving.

### Candidate B qualification and artifact

All five required jobs in workflow `33568432155` passed. The candidate
packaged job retained restricted `python`/`python3`/`py`/`uv` absence, `dist`,
`verify-dist`, metadata observations, normal/restricted shell and GUI smoke,
updater qualification, and artifact upload.

The uploaded artifact was independently inspected:

| Field | Value |
| --- | --- |
| Artifact ID | `9824206104` |
| Upload size | 9,239,383 bytes |
| Upload digest | `sha256:a7f9607ad1653efb27bc8f6b9a7561c6daf8c214376235d230040fe0e6b48379` |
| Inner ZIP size / SHA-256 | 4,531,748 bytes / `5a719a8bcf55123921bd9c4bd3e5866382f9607868d0ce04c53355d265e86d6c` |
| MANIFEST SHA-256 | `c74bee8fc8af21931897d8d6f5223e9c110a4215aef879f6722c32920a0e10b5` |
| Portable / managed entries | 102 / 101 |

The release tree and ZIP each contain the expected 102 files; 101 managed
entries have zero missing/unexpected/hash/size mismatches. The runtime/test
negative scan is zero. Observed desktop/calibration provenance and the
manifest/provenance commit are all
`c1fc52eccccbde5b9e0826f4291150427e87c5e5`; the native source fingerprint
is unchanged at
`6aa3f9d6f05f1c1778d3cdfba1e7cf2a5ba8867d7259501fefc9d42bd11c56d9`.

### Candidate B decision

`ADOPTED`. Pull-request packaged jobs are restore-only; main/manual package
runs still populate the cache. The change preserves the exact artifact and
all required gates while eliminating the measured PR post-job save. The
workflow and p50/p95 claims remain provisional.

## Other candidates

These candidates were not changed before Lane A was measured. They remain
explicitly unattempted rather than being forced into the same experiment:

| Candidate | Decision | Reason |
| --- | --- | --- |
| Browser path-awareness | `NOT_ATTEMPTED` | audit and Lane A validation first; no evidence yet that browser work can be safely skipped for each path class |
| Static tooling cache | `NOT_ATTEMPTED` | no measured installation bottleneck or isolated A/B yet |
| Packaged Rust cache save policy | `ADOPTED` | PR restore-only A/B removed the measured post-upload save; main/manual runs still save |
| Tauri feature pruning | `NOT_ATTEMPTED` | no feature graph A/B; preserve `wry`/`custom-protocol` and capability semantics |
| `sccache` | `NOT_ATTEMPTED` | no compile-dominant measurement or cache overhead study yet |
| `profile.dist` | `NOT_ATTEMPTED` | highest runtime risk; no optimized-profile RT comparison mechanism added |
| Test binary partitioning | `NOT_ATTEMPTED` | no compile-vs-execution bottleneck measurement |
| Binding modernization | `NOT_ATTEMPTED` | no material binding-generation cost identified |
| Frontend output reuse | `NOT_ATTEMPTED` | avoid stateful reuse without freshness/provenance evidence |

## Runtime and safety

Candidates A, A.1, and B change only CI routing/setup/cache policy. They do not
alter the compiler profile, Tauri features, production binaries, player,
calibration, updater, or artifact layout. Existing Wave 6 authoritative
security, architecture, realtime/no-allocation, focus/release-all, updater,
provenance, and exact artifact suites remain green.

Therefore no new timing number is fabricated:

```text
No runtime build-semantics change; authoritative regression suite remains green.
```

## Metrics status and targets

```text
metrics_status = provisional
natural comparable main runs = 1 baseline run
authoritative p50/p95 = unavailable (fewer than 10 representative natural runs)
```

Engineering targets remain targets, not coverage-reduction permissions:

```text
ordinary PR p50 <= 5m
ordinary PR p95 <= 8m
main exact p50 <= 10m
```

The final exact PR workflow after this report commit will be the qualification
HEAD for human review. No merge, release tag, or follow-on optimization wave
is authorized by this report.
