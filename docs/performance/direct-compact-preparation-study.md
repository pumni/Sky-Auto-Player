# Direct compact prepared-stream study — issue #632

## Scope and safety

This is a **test-support-only offline experiment** on top of the qualified
[#635 ownership projection](../performance/compact-prepared-replacement-study.md).
It does not switch the shipping prepared stream or Win32 sender.

The new builder invokes the **existing** production coordinator planner for
each authored boundary, validates exactly the same authored-only physical
commit, deferred release and overlapping-mask conditions as the inline builder,
then immediately:

1. Moves all non-packet fields and frozen authored commit into a compact
   prepared entry (the same type as #635).
2. Appends the *temporary* planner-produced Win32 INPUT packet into a
   contiguous descriptor+INPUT arena.
3. Applies the same frozen metadata/success commit to its simulator.
4. At the end, enforces no deferred pending release, rebuilds the reset
   coordinator through the canonical checked constructor, freezes the arena
   and verifies packet indices, masks, event and entry counts.

No full `Vec<PreparedDispatchEntry>` with inline 608-byte packets is
constructed in this path. **One inline packet per NextDispatchPlan is still
temporarily materialized.** The builder allocates during preparation, not
at a real-time dispatch boundary. No compact dispatch path is shipped or
invoked; this is not evidence for pre-call authorization, cleanup or sender
latency equivalence.

The original builder is **not modified**. The alternate uses the same
production admission-budget estimate, exact prepared-entry reservation and
an explicit initial arena hint of two INPUT records per compiled batch,
with checked growth for any 1..15-event physical packet. That reserve
heuristic is a variable to study, **not a proven optimal capacity policy**.
The frozen arena exposes slices by checked packet index, never mutable
Vec pointers.

## Qualification on pinned Windows x64

```powershell
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
# From rust/
cargo test --locked -p sky_player --features test-support direct_compact -- --nocapture
cargo test --locked -p sky_player --features test-support compact_replacement -- --nocapture
cargo test --locked -p sky_dispatch_win32 --features test-support compact_study -- --nocapture
cargo test --locked -p sky_player --features test-support --test rt_dispatch_no_alloc
# From repository root, after frozen Bun + calibration generation:
cargo xtask check all
# From rust/, offline built-in corpus; no real input:
cargo run --locked --release -p sky_player --features test-support --example rt_corpus_packet_probe -- --output <TEMP-json-path>
```

New unit tests build **two independent coordinators** for the direct compact
and inline-control paths; only the *test comparison* transcodes the inline
control. They compare the complete Debug fingerprint of the stored compact
entry (including metadata ordering, authored deadlines and frozen commits),
packet mask/INPUT-count/Up-prefix spans, and reset coordinator heap payload.
Fixtures cover stale Up-only metadata, all 1..15 chord widths, 7 Up + 8 Down
maximal mixed boundaries, and cancellation at both stream and coordinator
reset checkpoints. These are semantic structural checks, not actual
SendInput or QPC-policy tests.

## New additive built-in corpus report

On Windows, `rt_corpus_packet_probe` now reports for each successfully
prepared song:

- Existing inline stream reserved-entry bytes and existing **duplicate**
  offline component arena remain unchanged.
- New `direct_compact_preparation_study` reports compact entry size/count,
  actual entry capacity, reserved entry/descriptor/INPUT bytes, total reserved
  bytes, histogram, frozen coordinator heap payload and one-shot build duration.
- The top-level `direct_compact_preparation_study` aggregates **separate
  per-song** retained buffer capacities and the sum of unpaired build durations.
  Invariants reject mismatched metadata/physical counts, histogram, INPUT count
  and rebuilt coordinator payload against the inline control.

**These are NOT RSS/private/peak process bytes.** The probe runs both
representations sequentially in one process after cloning a coordinator.
Summed per-song capacities are not simultaneously allocated. The one-shot
durations are unpaired, affected by ordering and allocator/cache state; they
cannot justify an A/B speedup claim.

## Still required before any production consideration

Run inline-only vs direct-only in **separate fresh processes** on one pinned
Windows host, using ≥5 interleaved matched trials per workload (110
preparable songs and synthetic 100/1k/10k generations, including large
chords). Distinguish setup/compile from preparation+reset, record actual
allocator calls and bytes, process peak private commit and working set,
RSS/peak with clearly identified Win32 counters, reserve fragmentation and
cold/warm order, all on exact SHA/binary hashes. Compare timing quantiles and
safety invariants with a real dispatch feature-flag candidate **only after**
offline evidence warrants a separate production implementation PR.

No input hooks, game memory access, real SendInput, keyboard output or
in-game receiver acknowledgement is involved in this study.
