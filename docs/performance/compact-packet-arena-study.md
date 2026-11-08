# Frozen compact INPUT arena — offline layout experiment (#632)

This is **not a production optimization**. The test-support-only Windows
module `sky_dispatch_win32::input::compact_study` copies the **already built**
Win32 `INPUT` values from `PreparedPhysicalPacket` into a contiguous arena
indexed by checked `(u32 offset, u8 length, PhysicalPacket)` descriptors.

After freeze, the arena owns immutable input and descriptor vectors. Tests
compare the *initialized keyboard fields* of every input event against the
original, not raw C struct bytes (which may contain uninitialized padding).
Mixed packet recovery uses the same canonical Up prefix; cases from 1 to 15
events, a maximal 7 Up / 8 Down packet, and corrupt descriptors/events are
tested. No SendInput is invoked by these tests or by the arena builder.

## Initial qualification (from rust/ on Windows)

```powershell
cargo test --locked -p sky_dispatch_win32 --features test-support compact_study -- --nocapture
cargo test --locked -p sky_player --features test-support --test rt_dispatch_no_alloc
```

The frozen study deliberately does **not** replace the 1,392-byte
`PreparedDispatchEntry` nor the existing 608-byte inline
`PreparedPhysicalPacket` in production. A comparison between the inline
packet-component bytes and the allocated compact arena is **not** a measure
of process RSS or a guaranteed whole-stream saving. No hot-path allocation,
SendInput, runtime final authorization, physical completion floor, or tracked
cleanup behavior has changed.

## Next phase

With the Win32 type/semantic prototype passing CI, add a separate test-support
integration that walks the **actual frozen prepared stream** of the 110
successfully planned built-in songs, records the total arena reserved bytes,
checks every original materialized packet, and compares with the original
prepared Vec allocation. Only after measured full-layout and Windows timing
A/B qualify should a separately gated production layout be considered.

See [#621](https://github.com/pumni/Sky-Auto-Player/issues/621) and
[#632](https://github.com/pumni/Sky-Auto-Player/issues/632).
