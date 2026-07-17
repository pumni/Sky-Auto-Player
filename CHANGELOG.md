# Changelog

All notable changes to Sky Player are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [2.3.4] - 2026-07-17

### Changed

- **Refactored the dispatch loop and playback engine** for cleaner focus handling
  and tighter spin-threshold management.
- **Reworked timer management** in the main loop and playback supervisor for
  improved accuracy and performance.
- **Isolated the dispatch core** behind a structural interface, decoupling it
  from platform backends so the scheduler stays pure and unit-testable.
- **Removed the deprecated alias** for the abort input method in `DispatchLoop`.
- **Added an update-flow simulator** (`simulate_update.py`) for exercising
  update scenarios without a live network.
- **Documented the UI CPU/RAM optimization plan** for the 2026-07 workstream.

### Performance

- **Phase 1–3 hot-path hardening**: telemetry flush, cheap focus gate,
  symmetric reprobe, and uncontaminated overshoot samples — lowering tail
  latency on the dispatch spin path.

### Fixed

- **Phase 1 correctness**: focus gate, pause owner, clock, and estimator
  adjusted to remove residual bias and timing drift.

### Housekeeping

- **Backstop the O(polyphony) memory hardening** of `RuntimeDispatchCoordinator`
  introduced in commit `26d9b00`. That fix reduced `status_by_generation` from
  O(note_count) to O(polyphony) (≤ ~30 live entries); this release adds the
  regression coverage the original fix lacked.

## [2.3.3] - 2026-07-15

### Added

- `tests/test_runtime_dispatch_bounded_memory.py` — regression tests asserting that
  `RuntimeDispatchCoordinator.status_by_generation` stays bounded by polyphony
  (≤ 2 × scan_code_space) regardless of song length.
- Hardening assertion in `RuntimeDispatchCoordinator.generation_status_counts()`
  against silent counter drift (terminal + non-terminal > generation_count).
- Direct-drive instrumentation in `scripts/mem_attrlite.py` and
  `scripts/mem_engine_attr.py`; the previous approach inspected
  `engine._runtime_coordinator` post-play, which is `None` after `play()` returns.
- `CHANGELOG.md`.
- Bidirectional-invariant docstring on `status_by_generation`.

### Housekeeping

- Backstop the O(polyphony) memory hardening of `RuntimeDispatchCoordinator`
  introduced in commit `26d9b00`. That fix reduced `status_by_generation` from
  O(note_count) to O(polyphony) (≤ ~30 live entries); this release adds the
  regression coverage the original fix lacked.
