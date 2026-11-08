# Compact prepared replacement ownership study — #632

This is the **next test-support-only checkpoint after #634**, not a shipping
sender refactor. On Windows with `test-support`, an already-built production
`PreparedDispatchStream` is **consumed** into a distinct compact representation:

- Preserve all non-packet fields from the physical `AuthoredBatchView` in
  original order, including frozen authored commit, deadlines, packet identity,
  recovery descriptor, and dispatch metadata.
- Preserve the metadata-only entries and frozen metadata commits in their
  original positions.
- Move Win32 INPUT records into one frozen arena, with checked packet indices
  and descriptor offsets/lengths; verify physical identity, event conservation,
  packet order, and canonical Mixed Up prefix before exposing the study owner.
- Drop the entire original inline-entry Vec in the conversion, so the **held**
  alternate layout contains no redundant inline 608-byte packet.

## Test

Run on pinned Windows x64 with the repository's test-support feature:

```powershell
cargo test --locked -p sky_player --features test-support compact_replacement -- --nocapture
cargo test --locked -p sky_dispatch_win32 --features test-support compact_study -- --nocapture
cargo test --locked -p sky_player --features test-support --test rt_dispatch_no_alloc
```

The focused test reports type and **reserved buffer** byte sizes for a small
mixed physical/metadata fixture and maximal 7 Up + 8 Down recovery fixture.
Also checks every 1..15-key Down/Up width. This is a structural step before
corpus/paired Windows memory and timing A/B.

**Limitations:** The conversion starts with an **already-prepared inline stream**.
It temporarily owns both forms while copying, so conversion may increase startup
peak and duration. An alternate owner retained after conversion proves nothing
about process RSS, startup wall time, realtime SendInput or game receipt. No
dispatch code consumes the alternate owner. A separate experiment must build
the alternate layout in-situ without staging full inline storage, isolate
process-level measurements, then run interleaved cold/warm A/B and RT safety
gates before any production implementation is considered.