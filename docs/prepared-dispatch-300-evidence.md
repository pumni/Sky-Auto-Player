# Issue #300 evidence

Base: `9a32f0b8ce20d26e0fa31e48353ab6abd0822b3a`

The final PR description records the exact head SHA after this evidence file
and the implementation are committed.

## Prepared stream

- The normal session builds one owned stream before worker startup-ready by
  driving the existing `RuntimeDispatchCoordinator` preparation contract.
- The legal same-key fixture contains 5 immutable entries: 4 physical frames
  and 1 metadata-only entry. The physical sequence is
  `Down(K) -> Up(K) -> Down(K) -> Up(K)`.
- Physical frames have disjoint masks, immutable authored offsets, and
  `deferred_up_mask == 0`.
- The normal precision helper source audit rejects coordinator/planner,
  pending-release, DownBoundaryState, missed-boundary recovery, foreground
  query, lease arithmetic, allocation, formatting, and lock references.
- The startup path materializes the prepared packet payload before the worker
  enters the wait loop. The no-allocation gate covers prepared iteration,
  waiting, and one successful sender transaction.

## Deterministic acceptance

All semantic expectations passed, including:

`single_note`, `maximum_valid_atomic_chord`, `dense_different_key_boundaries`,
`legal_same_key_sequence`, `late_wake_inside_normal_tolerance`,
`late_wake_outside_normal_tolerance`,
`late_first_boundary_pressures_following_boundary`,
`stop_interrupt_race_at_target`, `intentionally_injected_zero_progress_drop`,
`completion_floor_control`, and `completion_floor_delayed`.

The current report has:

- prepared physical boundaries: 16;
- successful sends: 14;
- non-send/drop boundaries: 2;
- timeline rebases: 0;
- normal completion-feedback `PhysicalWindowExpired`: false;
- host real-wait non-dispatches: 0;
- host real-wait overdue observations: 0;
- host real-wait transport anomalies: 0;
- the only deterministic transport anomaly is the intentional
  `ZeroProgress` fail-closed scenario;
- all +2 ms, +10 ms, +50 ms, and +100 ms overdue fixtures sent exactly once
  without `UnobservedBacklog`, `DownExpiredBeforeSend`, sender-window
  expiration, or normal `PhysicalWindowExpired`.

The completion control/delayed pair retains different completion latency for
packet N (8 us versus 40,000 us) while reporting the same authored schedule,
packet N+1 target, packet-not-before, wait target, send eligibility, and
classification. Both floor masks remain zero and delayed completion does not
move later scheduling.

## Fresh host run

Command environment: Rust `rustc 1.98.1 (48a229cea 2026-09-01)`, QPC
frequency 10,000,000 Hz, production calibrated wait policy, 10,000 baseline
observations, same laptop.

The latest prepared-path run was acceptance-clean and statistics-eligible:

| measurement (us) | p50 | p95 | p99 | p99.9 | max |
| --- | ---: | ---: | ---: | ---: | ---: |
| `pre_call_qpc - target_qpc` | 15 | 22 | 38 | 246 | 25,794 |
| `completion_qpc - pre_call_qpc` | 1 | 2 | 3 | 5 | 151 |

The first prepared-path run was also acceptance-clean: the corresponding
`pre_call-target` values were `15 / 22 / 43 / 3326 / 24283` and completion
values were `1 / 2 / 3 / 6 / 58`.

Because the maximum tail is host-noise sensitive, an interleaved control was
run from the exact base with the same command and 10,000 observations. The
first control was `25 / 38 / 64 / 3382 / 19269` for
`pre_call-target`; the repeated control was `25 / 41 / 195 / 1223 / 22581`.
The prepared path improved the central and percentile values in both pairs;
the isolated maximum varied in both arms and is reported without scheduler
tuning. No delivery regression, non-dispatch, overdue observation, or host
transport anomaly occurred.

## Verification

- `cargo test -p sky_player --lib`: 293 passed.
- `cargo test -p sky_player --features test-support --test rt_dispatch_no_alloc`:
  23 passed.
- prepared normal dispatch focused tests: 4 passed.
- prepared Win32 sender tests: 10 passed.
- stabilized lifecycle tests: 1 passed each.
- watchdog focused tests: 4 passed; disabled-lease test: 1 passed.
- `cargo xtask check static`: PASS.
- `cargo xtask check rust`: PASS (fmt, clippy `-D warnings`, all-features
  workspace tests).
- final GitHub required CI result is recorded in the PR description after the
  push.

No MMCSS, affinity, spin threshold, timer, focus cadence, watchdog timeout,
completion-floor, or late-send policy was changed. Strict/diagnostic dynamic
dispatch remains available. The benchmark remains a deterministic mock
transport plus real `HybridWaiter` qualification; it is not Raw Input or
game-observed latency.
