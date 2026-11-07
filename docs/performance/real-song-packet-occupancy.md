# Offline real-song prepared packet occupancy

## Scope

Test-support-only evidence for [#621](https://github.com/pumni/Sky-Auto-Player/issues/621).
`rt_corpus_packet_probe` reads the validated 112-entry built-in manifest,
checks every song file against its pinned SHA-256, runs the **existing**
`sky_app_core::song::build_song_plan`, converts authored actions exactly as
the native session does, invokes the **production**
`compile_runtime_intents` and the test-support prepared-stream builder,
and reports actual prebuilt Win32 `INPUT` event count histograms.

The frozen policy matches `cargo xtask song-audit inventory`:
**60 FPS, 1 hold frame, 0 microseconds margin, tempo scale 1**.
The default modern plan selects all playable tracks at the source pitch.
No input is injected, no playback worker starts, and no production
sender/guard/cleanup code changes.

### Usage on Windows

Run from the repository's `rust/` directory, with a TEMP output path:

```powershell
cargo test --locked -p sky_player --features test-support --example rt_corpus_packet_probe
cargo run --release --locked -p sky_player --features test-support --example rt_corpus_packet_probe -- --output "$env:TEMP\sky-corpus-packets.json"
```

## Output and interpretation

The JSON records `source_revision`, `manifest_sha256`, the fixed timing
policy, a result for **every manifest song**, per-song preparation and
compiler outcomes, per-song 16-bin histograms (0..15), and a weighted
aggregate histogram over **only successfully prepared songs**.
`prepared_packet_histogram[i]` counts actual frozen physical packet
entries with `i` prebuilt input events; metadata-only entries are excluded.

Failed source/planner/compile/coordinator/preparation cases remain in
`songs` with typed `outcome` and `reason`; no rejected song is silently
counted as successfully prepared. `compiled_mixed_boundary_count`
describes the compiler's intended mixed boundaries, not a separate
measurement of executed physical sends.

The archived song-audit inventory contains **source onset** chord metrics.
These are different from the prepared packet histogram. Even successfully
prepared offline packets do **not** establish that a real game receives the
same events on time. No measurements of SendInput latency, CPU, RSS,
runtime physical hold floors or gameplay are claimed.

Do not start a packed arena refactor until the prepared histogram,
compiler rejection rate, and per-song distributions are reviewed.
