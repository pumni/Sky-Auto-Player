# Late Down tolerance native physical acceptance

This evidence records Windows interactive runs of the project's production
`sky_player` dispatch path against the project-owned `ReceiveOnly` native
acceptance sink. It qualifies the sender, cutoff propagation, authored packet
timing, focus safety, and cleanup behavior. It does **not** qualify Sky game
consumption or prove that a suspected song-position miss is fixed.

The primary run below was rebuilt and executed on the exact implementation
head. This evidence archive is a documentation-only commit after that run; the
earlier implementation-point run remains archived in its sibling directory.

## Run identity

| Field | Value |
| --- | --- |
| Run ID | `input-reliability-20260912T210236-153c82dc` |
| Source revision | `713d999e0698a77b2a9a11fb6936038d2f22aa47` |
| Tracked source tree clean | Yes |
| Release harness | `rust/target/release/rt-native-acceptance.exe` |
| Harness SHA-256 | `f81684f69b3e96e0eaba946a5d98d91017314c401e0c1d2a13981a30bc3f21f2` |
| Runner SHA-256 | `8d2c77330ad8c8ced886f4a9059565b321effcf53f1cd5fb5b73c8dc039af237` |
| Scenarios | 17 requested, 17 PASS |
| ReceiveOnly sink events | 136 keyboard events; sequence 1–136, continuous |
| Focus probe | 0 keyboard events |

`summary.json`, all 17 native reports, per-scenario invocation ledgers, both
ready records, both raw event logs, runner output, configuration, and their
checksums are preserved in
`input-reliability-20260912T210236-153c82dc/`. Each invocation ledger
binds its report to the exact `ReceiveOnly` event-log ID and contiguous sink
sequence range; the full sink log was independently checked for run ID,
event-log ID, and sequence continuity.
Text artifacts are stored with LF line endings, so `SHA256SUMS.txt` applies to
the same bytes on Windows and non-Windows checkouts.

## Scenario results

All 13 established physical scenarios passed at 60 FPS, 1.0-frame Base Hold,
800 µs Timing Margin, and 500 µs Late Down tolerance:

```text
canonical-single       canonical-chord       canonical-max-chord
hold                   rapid-retrigger       mixed-up-down
target-hwnd-change     pause-resume          stop-cleanup
skip-cleanup           cleanup-full-release  w4-noncanonical
focus-loss
```

The four focused cutoff cases held Timing Margin at 800 µs and varied only
Late Down tolerance:

| Late Down tolerance | Authored Hold | Authored Release Gap | Authored packet targets (µs) | Result |
| ---: | ---: | ---: | --- | --- |
| 500 µs | 17,467 µs | 17,467 µs | 50,000 → 67,467 → 84,934 → 102,401 | PASS |
| 1,000 µs | 17,467 µs | 17,467 µs | 50,000 → 67,467 → 84,934 → 102,401 | PASS |
| 2,000 µs | 17,467 µs | 17,467 µs | 50,000 → 67,467 → 84,934 → 102,401 | PASS |
| 5,000 µs | 17,467 µs | 17,467 µs | 50,000 → 67,467 → 84,934 → 102,401 | PASS |

Every report returned the selected cutoff unchanged. All reports ended with no
active or stuck keys, no missed Down/backlog/hard-late boundaries, no partial
SendInput calls, and no zero-progress SendInput failures. The intentional
focus-loss case recorded its expected focus rejection; the inert probe still
received no keyboard events.

## Reproduction

From the repository root on an interactive Windows desktop, build the release
acceptance runner and execute the archived `runner.ps1` from this directory.
The runner creates a fresh `.benchmarks/physical-input-reliability/<run-id>`
folder, starts and validates a project-owned receive-only sink and inert focus
probe, runs the matrix and sweep, and stops both windows afterward.

```powershell
cargo build --locked --release --manifest-path rust/Cargo.toml -p sky_player --features real-input-acceptance --bin rt-native-acceptance
pwsh.exe -NoProfile -File docs/perf-baselines/2026-09-late-down-tolerance-native-acceptance-data/input-reliability-20260912T210236-153c82dc/runner.ps1
```

The runner uses real Windows `SendInput` against the validated test HWND. It
does not send input to Sky. A real-game A/B at the repeatable missed note and
export of that session's packet-identified sender trace remain necessary to
decide whether the game rejects a packet shape or whether late-Down admission
is implicated. Production batching and keyboard encoding were not changed.

## Review follow-up: desktop Diagnostics and release-gap stress

The desktop physical smoke at the reviewed implementation source verified the
previously missing Diagnostics wiring. During a 12-second playback to a separate
`ReceiveOnly` sink, the UI showed `Physical session: Yes`, `Player attached:
Yes`, `Sender samples: 20`, `Sender backend: Healthy`, and a measured maximum
pre-call lateness of `96 µs`. The ready record and 20-event log are archived in
[`diagnostics-desktop-smoke-20260912T225520/`](diagnostics-desktop-smoke-20260912T225520/),
with the live UI capture. The app was launched from the worktree immediately
before those exact source files were committed as `bb30148c404146821bbd9ef6b44a55b5cc258051`;
the report records the parent Git head present at launch and the commit that
contains the captured source. This smoke is a wiring check, not a gameplay
reliability test.

The focused `release-gap-stress` run used that exact implementation commit,
60 FPS, 1.0-frame Base Hold, 800 µs Timing Margin, and 500 µs Late Down
tolerance. It completed 512 release-gap observations but **failed
qualification**: four observed Up-completion-to-next-Down-pre-call gaps fell
below the fixed one-frame floor. At a 10 MHz QPC frequency, the smallest gap was
16.2742 ms against the 16.667 ms floor. The observed count was `4/512`
(`0.78125%`). The sink also recorded one extra KeyDown (`1,027` events in the
authorized window against `1,026` expected); the harness correctly failed on
that event. SendInput partial/zero-progress counts, dropped keys, stuck keys,
and focus losses were all zero. The report, full ready record, and raw event log
are archived in
[`release-gap-stress-20260912T231500-bb30148c/`](release-gap-stress-20260912T231500-bb30148c/).

An earlier run stayed paused and did not join; it collected zero release-gap
samples and is preserved separately as
[`release-gap-stress-inconclusive-20260912T230900-be9c1d68/`](release-gap-stress-inconclusive-20260912T230900-be9c1d68/).
It is excluded from the observed anomaly count. All stress runs used the same
500 µs tolerance and 800 µs Timing Margin; these results say nothing about
whether changing the cutoff improves the release-gap observations. They do
establish that a below-floor observation is no longer reported as a clean
`PASS`, and that the current stress workload is **not qualifying** for release.
The original 17-scenario and cutoff-sweep evidence above remains historical
evidence from its separately identified source revision.

## Exact-head Windows regression follow-up

The official Windows runner was rebuilt and run on the clean exact source
head `b8bbe42d84837c440dd2e51cc90b490298aaa760`. Its fail-fast run passed the
12 default scenarios and the 500 µs sweep case, then correctly stopped at the
1,000 µs sweep case: that case had one release-gap sample, and it was below the
fixed one-frame floor (`15.1804 ms` observed against `16.667 ms`). The full
runner report, invocation ledger, raw sink and probe logs, ready records, and
summary are archived in
[`input-reliability-20260912T235553-7daf4359/`](input-reliability-20260912T235553-7daf4359/).

The remaining default `focus-loss` regression was run separately on the same
exact head and passed; its inert focus probe received zero keyboard events.
Its complete raw evidence is in
[`input-reliability-focus-loss-20260912T235844-465b29a6/`](input-reliability-focus-loss-20260912T235844-465b29a6/).
Together, all 13 default Windows regression scenarios have exact-head PASS
evidence. The 2,000 and 5,000 µs sweep cases were not reached after the
1,000 µs qualification failure. The 500 and 1,000 µs cases each had only one
release-gap sample, so their different verdicts do not show that changing the
cutoff improves or worsens reliability. The 512-sample stress result above is
the relevant release-gap qualification evidence, and it fails.
