# Song source decoding

`sky_app_core::song::decode_song_json` is the strict, source-preserving JSON
decoder. `detect_song_format` reports the selected schema without decoding its
events. The existing `parse_song_json` and scheduling API remain available for
the checked-in song catalog; this decoder does not silently replace that
compatibility path.

## Format selection

The decoder accepts a JSON object or an array containing exactly one object.
An empty array and an array with multiple songs are errors. A present `type`
must agree with `data.isComposedVersion`, when that historical family flag is
present. With no `type`, the supported `data.appName` and boolean
`data.isComposedVersion` can identify a historical family when its authoritative
event container is present. A missing version defaults to v1 only for the
unambiguous old container (`columns` for composed or `notes` for recorded),
without incompatible modern fields. A modern `tracks` shape without a version
is rejected; the decoder never guesses between v4 and v5. Contradictory or
malformed versioned data is never retried as the legacy projection, even when
`songNotes` is also present.

An object with neither a versioned event container nor a historical family
flag is the flat legacy format when it contains `songNotes`. The `.txt` song
extension still carries JSON here; this decoder does not treat it as ABC
notation or another text format.

| Source format | Authoritative event data | Source identity interpretation |
| --- | --- | --- |
| Flat legacy | `songNotes[{key,time,l?}]` | `Key0`–`Key14` and `1KeyN`/`2KeyN`/`3KeyN` name physical Sky keys and optional source layers. An explicit `l` bitmask overrides a key prefix. |
| Composed v1 | `columns`, `instruments`, `bpm` | Legacy note index plus reversed binary layer mask; instruments are names. |
| Composed v2 | `columns`, `instruments`, `bpm` | Legacy note index plus hexadecimal layer mask; instruments are names. |
| Composed v3 | `columns`, `instruments`, `bpm` | Same note and mask encoding as v2; instrument objects may override the song pitch. |
| Composed v4 | `columnTempos`, `tracks`, `bpm` | Track note tuples contain a column index, nominal source note ID, and optional column span. |
| Composed v5 | `columnTempos`, `tracks`, `bpm` | Same track and span structure as v4, with absolute note numbers. |
| Recorded v1 | `notes`, optional `instruments` | Legacy note index and timestamp in milliseconds. Its optional numeric layer field retains the source format's decimal-as-hex interpretation; absent or zero means layer one. If the roster is absent, one default source instrument is allowed only for masks confined to bit 0. |
| Recorded v2 | `notes`, optional `instruments` | Legacy note index, timestamp in milliseconds, and hexadecimal layer mask. An absent roster permits one default source instrument only for masks confined to bit 0. |
| Recorded v3 | `tracks[].notes` | Nominal source note ID, timestamp in milliseconds, and optional duration in milliseconds. |
| Recorded v4 | `tracks[].notes` | Absolute note number, timestamp in milliseconds, and optional duration in milliseconds. |

Versioned formats require `data.appName` to identify the source game as `Sky`
or `Genshin`. The source pitch is one of C, Db, D, Eb, E, F, Gb, G, Ab, A,
Bb, or B; a non-empty track pitch overrides the song pitch, while an empty
track pitch inherits it.
Legacy flat keys are already physical Sky key positions and are not converted
through an instrument table.

## Canonical events

Each `CanonicalEvent` retains its stable source ordinal and position, source
track, onset and duration in microseconds, raw note identity, pitch identity,
and source layer or span information when present. Events are ordered by onset;
equal-onset events retain source order. Layered legacy notes expand into one
event per selected track while retaining the shared source ordinal.

Flat legacy pitches are `SkyKeyIndex` values. Versioned source notes are
`NoteNumber` values with a source pitchability state (`pitched`, `unpitched`, or
`unresolved`). V4 nominal IDs are converted through the frozen source-game and
instrument tables. V5/v4 absolute note numbers remain unchanged. This decoder
does not choose or emit a target Sky key; [song planning](song-planning.md) is
the separate conversion step.

Composed column durations use `60,000 / bpm` milliseconds multiplied by the
column tempo changer (1, 1/2, 1/4, or 1/8). The first composed onset is 100 ms.
The decoder accumulates exact floating-point millisecond boundaries, rounds
each emitted boundary to the nearest millisecond, then converts it to
microseconds. A `span` of 1 is a one-shot with zero duration; spans greater than
1 use the difference between rounded start and end boundaries. These rules
apply to composed v1 through v5. Recorded and flat timestamps are stored in
milliseconds and are converted directly to microseconds; recorded songs do not
get an additional BPM conversion.

`DecodeDiagnostics` reports authored source notes, expanded canonical events,
tracks, columns, sustain, skipped empty masks, and potential same-key collisions
in the flat physical-key format. Empty masks are preserved as diagnostics and
do not emit events.

## Validation and source tables

The decoder rejects unsupported versions and games, encrypted inputs, unknown
instruments, malformed note tuples and masks, invalid pitches and timing,
out-of-range columns/spans, and ambiguous multi-song arrays. It applies these
resource caps before event expansion:

- 16 MiB input payload
- 200,000 source notes and 200,000 composed columns
- 16 source tracks and 200,000 expanded events

Composed v1–v3 layer masks and recorded v1–v2 masks with an explicit roster
must select only declared tracks. Recorded v1–v2 may omit `instruments`; in
that case the decoder creates one source-game default track and accepts masks
confined to bit 0. Any mask selecting a higher track is rejected. An explicitly
empty roster or an entry without a required instrument name remains invalid.

Source tables are frozen from [Specy's `genshin-music` reference data](https://github.com/Specy/genshin-music/tree/d41f8a703955ba4ed1e1c4450407080bb3ae364f)
at commit `d41f8a703955ba4ed1e1c4450407080bb3ae364f`. The decoder keeps only the
numeric pitch and instrument mapping facts required for source identity; it
does not copy that project's implementation code. Unrecognized source
instrument names fail rather than being substituted with another named
instrument.

## Regression evidence

`rust/crates/sky_app_core/tests/song_source_decoding.rs` uses small, inspectable
fixtures for flat legacy and composed/recorded versions 1 through 5/4. The two
historical-header fixtures are synthetic and preserve the official historical
field shapes without importing full third-party song assets. Coverage includes
layer expansion, absent-roster defaults, source-game conversion, nominal versus
absolute notes, one-shot spans, composed timing, and fail-closed cases.

`rust/crates/sky_app_core/tests/legacy_song_corpus.rs` pins all 106 checked-in
song assets by input SHA-256 and freezes the existing `parse_song_json` plus
schedule output under 60 FPS, one hold frame, zero user margin, and no tempo
scaling. This guards the prior catalog API while the new decoder is introduced
as a separate path.
