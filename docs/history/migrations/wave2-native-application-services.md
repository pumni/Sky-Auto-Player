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

## Wave 2 evidence status

### Proven parity

The settings persisted-read path is covered by committed Python-oracle
fixtures through the real `JsonSettingsStore -> SettingsService` path. The
fixtures cover missing/malformed/non-object files, v2 timing migration
precedence and FPS conversion, v3 coercions, nulls, unknown fields, and
legacy-key removal. Validate-before-save and atomic patch behavior are covered
separately by application-core tests.

The catalog index is covered for opaque IDs, canonical-path de-duplication,
generation checks, path-free rows, supported extensions, Unicode NFKD/mark
removal/`đ` replacement/full `casefold`, deterministic substring ordering,
and Python code-point length and window bounds. These are index/substrings
parity claims only.

### Implemented shadow

`sky_native_adapters` provides filesystem enumeration and persisted config
compatibility, but no live desktop command uses these adapters yet. Update
channel and throttle policy is implemented as a fixture-backed Rust shadow;
the hardened native updater still owns network, verification, transaction,
rollback, recovery, and handoff behavior.

### Explicitly unmigrated

All 21 live desktop commands remain Python-owned. Settings remain Python-owned
because the running Python Core caches its configuration and no live
cache-coherence protocol has been proven. Catalog fuzzy ranking remains
Python-owned until RapidFuzz `WRatio` equivalence is proven; rich song detail
and catalog generation/path authority also remain Python-owned. No native
application event producer exists in this wave.

Committed fixtures under `tests/fixtures/wave2/` are generated deliberately
from the current Python implementation by `scripts/generate_wave2_fixtures.py`.
They are evidence, not a CI-time regeneration step.

## Command and event ownership

`desktop/src-tauri/src/command_ownership.rs` is the explicit matrix for every
current Core method. The delivery layer rejects an unlisted method; it never
performs an implicit native-then-Python fallback.

The application event abstraction and native event mux are deliberately
deferred until the first real native-owned use case can publish through the
stable delivery path. The existing Python Core supervisor remains the live
source for current UI events. No frontend command, DTO, envelope, or event
schema is changed by this foundation.

## Deliberate cutover blockers

Settings routing stays Python-owned until a live cache-coherence protocol is
proved. Writing the same `config.json` from Rust while a running Python Core
continues using its cached `AppConfig` would create split-brain state.

Native services are not eagerly constructed from the process working directory.
The outer adapter remains available for targeted parity tests and will be
constructed only when a concrete native owner has a proven installation-root
contract.

Catalog fuzzy routing stays Python-owned until a Rust ranker is proven
equivalent to RapidFuzz `WRatio`. The Rust index exposes an explicit fuzzy
port and a safe substring/shadow path; it does not substitute a merely similar
algorithm and does not silently fall back on errors.

Playback execution, realtime scheduling, `SendInput`, QPC/MMCSS, focus
admission, cleanup, Python runtime deletion, and Textual removal are outside
this wave.
