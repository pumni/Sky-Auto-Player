# Real-song offline frozen compact arena measurement — #632

This follow-up to [#633](https://github.com/pumni/Sky-Auto-Player/pull/633)
extends the existing verified 112-song corpus probe, without modifying the
production sender, physical timing, focus authority or tracked cleanup.

Only on Windows with `--features test-support`, each successfully prepared
corpus song additionally constructs a **second** contiguous Win32 INPUT arena
from its original immutable `PreparedDispatchStream`. The secondary arena
is never referenced by runtime dispatch. A checked descriptor stores the
original physical packet mask, 32-bit input offset, and physical event count.
Before the secondary arena is dropped, every original physical packet is
checked against its compact descriptor, initialized event count, and mixed
Up-prefix identity. Metadata-only entries never consume INPUT slots.

## Qualification

From the validation worktree's `rust/` directory:

```powershell
cargo test --locked -p sky_dispatch_win32 --features test-support compact_study -- --nocapture
cargo test --locked -p sky_player --features test-support --test rt_dispatch_no_alloc
cargo run --release --locked -p sky_player --features test-support --example rt_corpus_packet_probe -- --output "$env:TEMP\real-song-arena-study.json"
```

Always perform frozen Bun install, native calibration build, `cargo fmt
--check` and `cargo xtask check all` before the corpus probe.

On Windows, each successful song now has `compact_arena_study` fields
for descriptor/input bytes **actually reserved by the compact arena**,
the original inline packet **component** bytes (`physical packets * 608`),
and one-shot copy/structural verification time in nanoseconds.
The aggregate `compact_arena_component_study` sums only prepared songs.
Every original histogram, rejection outcome and SHA256 provenance field
remains unchanged; unsupported/rejected entries remain accounted.

**Interpretation limits:** This is a second allocation made *after* original
inline preparation. The reported difference is a **packet-component-only
hypothetical layout comparison**, not a measured whole-stream or resident
memory reduction. It does not measure peak process RSS, win32 SendInput
latency, successful game ingestion, or startup speedup. The copy/verify
durations are unpaired one-shot costs, not interleaved A/B performance
benchmarks. If the component-level comparison justifies further work,
design a separate in-situ alternate prepared entry representation and
pair memory/startup/RT timing on the same hardware before considering
a shipping change.
