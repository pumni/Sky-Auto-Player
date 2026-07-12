# Release v2.3.0

**Tag:** `v2.3.0`
**Python:** `3.14+freethreaded` (recommended) · `>=3.11,<3.15` (compatible)

This release overhauls the timing engine to better handle chords and polyphony, adds an adaptive lead-time estimator with exponential forgetting, and tightens the dispatch hot path for lower jitter on free-threaded CPython.

---

## ✨ Features

- **Per-batch lead and polyphonic estimator** — each chord batch now computes its own down-lead using an adaptive logic that accounts for the number of simultaneous notes. The `SendLatencyEstimator` is extended with polyphony buckets and a linear lead model, so chords of different sizes are dispatched with the right anticipation instead of a single fixed offset. (`712c02e`)
- **Exponential forgetting for the latency estimator** — introduces a `lin_forget` factor and RLS-style accumulators, so the estimator tracks recent network/system jitter instead of a stale long-term mean. State serialization and warm-up updated accordingly. (`1a94ce2`)
- **New song: Comedy** — complete note data added to the bundled `songs/` library. (`9c8f09c`)

## 🚀 Performance

- **Trimmed dispatch hot-path allocations** and tightened the spin loop in `orchestration/dispatch_loop.py`, reducing per-action overhead on the tight `perf_counter_ns` loop. (`7cdb8fb`)

## 🐛 Fixes

- **Safe latency deque snapshot** — the UI now retries and falls back to an empty list when the latency deque is mutated mid-iteration, instead of raising `RuntimeError` under load. (`4191b83`)
- **Disable lead cache in DryRunBackend** — `_lead_cache_enabled` now returns `False` when the active backend is `DryRunBackend`, preventing a stale cached lead from skewing dry-run timing previews. (`911d123`)

## ♻️ Refactor

- **Immutable domain models** — relevant dataclasses are now `@dataclass(frozen=True, slots=True)`; duplicate scan codes raise `RuntimeError` instead of a bare `assert`; hold validation now consults `policy.min_hold`. (`bdde9f8`)
- **`ActionKind` enum in tests** — string key kinds replaced with the `ActionKind` enum throughout the test suite. (`4a22cc0`)
- **Runtime state replaces legacy globals** in `main`; the scheduler's grouping and sorting is simplified (events grouped by time and kind, up-before-down priority via boolean key). (`822c439`, `0a32628`)
- **Lint and typing tightening** — ruff config extended, pyright set to standard mode with strict evaluation flags, redundant float casts removed, helpers cleaned up. (`90459b7`, `7f3a9d7`, `0b571eb`)
- **Timing profile tuning** — balanced and audience-safe profile hold-frame thresholds adjusted for consistency. (`18c62da`, `dcb0890`)

## 📚 Documentation

- Repository documentation overhauled: refined feature list, installation instructions, tuning presets, and a new FAQ section. (`dcb0890`)

## 🧰 Chores

- **Python bumped to `3.14+freethreaded`** — the no-GIL build cleanly decouples the dispatch loop from the Textual UI thread. Stock CPython 3.14 still works; see `docs/tuning-presets.md` for non-standard environments. (`3e5aa77`)
- Version bumped to `2.3.0` in `pyproject.toml` and lockfile. (`018d380`)

---

## 📦 Assets

- `Sky-Player-v2.3.0.zip` — standalone Windows build. Extract anywhere and run `Sky-Player.exe`.

## ⚠️ Notes

- **Upgrade from 2.2.x:** configs and tuning are forward-compatible. If you previously pinned a custom spin-threshold or FPS in config, it is still honored; defaults have only been simplified, not removed.
- As always, automatically playing music sheets may violate Thatgamecompany's Terms of Service — use responsibly.
