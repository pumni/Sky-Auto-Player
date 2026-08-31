# Wave 2 native application-services boundary

This document records the current Wave 2 integration boundary at the accepted
Wave 1 baseline `main@a2eb48dd5e921cf13f9223fdcc8c27364c204907`.

## Direction

```text
Tauri delivery (`sky_desktop_shell`)
    ├── existing Python Core compatibility transport
    ├── `sky_app_core` (pure application/domain rules)
    └── `sky_native_adapters` (filesystem and native outer effects)
             └── `sky_updater` remains the hardened apply/rollback owner
```

`sky_app_core` has no delivery, Python, Windows, player, filesystem, or
network implementation dependency. `sky_native_adapters` is an outer
composition dependency and is forbidden from becoming an application-policy
dumping ground.

## Wave 2 evidence

The Rust settings service models normalized application values and performs
validate-before-save atomic patches. `JsonSettingsStore` preserves unknown
configuration keys, supports the current schema and legacy timing selection,
and uses an atomic replacement boundary. `FileCatalogSource` performs bounded
filesystem enumeration; `CatalogIndex` owns opaque IDs, generation checks,
path-free rows, normalization, and deterministic substring behavior.

Committed fixtures under `tests/fixtures/wave2/` are generated deliberately
from the current Python implementation by `scripts/generate_wave2_fixtures.py`.
They are evidence, not a CI-time regeneration step.

## Command and event ownership

`desktop/src-tauri/src/command_ownership.rs` is the explicit matrix for every
current Core method. All live commands remain Python-owned in this wave while
the Python Core retains cached settings and rich catalog/detail authority.
The delivery layer rejects an unlisted method; it never performs an implicit
native-then-Python fallback.

The native event mux is bounded and maps only `CatalogChanged` into the
existing `UiEvent` wire shape. The existing Python Core supervisor remains the
live source for current UI events. No frontend command, DTO, envelope, or
event schema is changed by this foundation.

## Deliberate cutover blockers

Settings routing stays Python-owned until a live cache-coherence protocol is
proved. Writing the same `config.json` from Rust while a running Python Core
continues using its cached `AppConfig` would create split-brain state.

Catalog fuzzy routing also stays Python-owned until a Rust ranker is proven
equivalent to RapidFuzz `WRatio`. The Rust index therefore exposes an explicit
fuzzy port and a safe substring/shadow path; it does not substitute a merely
similar algorithm and does not silently fall back on errors.

Update policy is represented as pure, fixture-backed channel/throttle logic.
The hardened native updater continues to own download, verification,
transaction, rollback, recovery, and handoff behavior.

Playback execution, realtime scheduling, `SendInput`, QPC/MMCSS, focus
admission, cleanup, Python runtime deletion, and Textual removal are outside
this wave.
