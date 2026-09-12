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
