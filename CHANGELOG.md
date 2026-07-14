# Changelog

All notable changes to Sky Player are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
