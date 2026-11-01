# Built-in song audit and candidate review

This directory records a reproducible structural audit of every active song
in `builtin-songs/manifest.json`. It does not rank songs or establish musical
fidelity. A high structural match is a listening lead only.

## Rebuild the inventory

From the repository root, run:

```powershell
cargo xtask song-audit inventory
```

The command uses the existing Rust source decoder, 15-key planner, catalog
validator, and a fixed 60 FPS / one-frame hold / zero-margin policy. It fails
if a manifest source is missing, undeclared files exist, a checksum differs,
an active source fails validation, or the audit does not cover every manifest
entry. It rewrites `inventory.json`, `inventory.csv`, and
`inventory-summary.md` deterministically.

To write the three files elsewhere, use
`cargo xtask song-audit inventory --output-dir <directory>`.

## Metric definitions

Inventory and comparison documents use schema version 2 and distinguish these
stages:

- `source_note_count` counts source note records: legacy `songNotes` entries
  or note entries across versioned source tracks.
- `decoded_canonical_event_count` counts events emitted by the authoritative
  source decoder. It can exceed source note records when one source record
  expands into multiple canonical events, such as layered legacy keys.
- `unique_physical_time_key_count` counts unique scheduled onset / physical
  Sky key pairs after physical-key mapping and deduplication. Distinct
  `1KeyN` / `2KeyN` source layers may map to one physical key; those removed
  extras are reported as `source_layer_collision_extra_event_count`.
- `output_physical_note_count` counts notes in the planner's output schedule.
  It can differ from unique physical source events because planner output
  preserves its own compatibility and scheduling semantics.
- `output_action_count` counts scheduled key actions. These are separate from
  source notes, canonical events, physical events, and scheduled notes.
- `authored_distinct_onset_count` counts distinct source-event onset times.

Schema version 2 removes the former `authored_source_event_count` field,
which duplicated source note count while implying a different stage.

- **Physical onsets, chords, polyphony and fast repeats** are computed on the
  deduplicated physical event stream. Planner output actions and diagnostics
  are reported separately; these counts are not interchangeable.
- **Active seconds** spans first to last decoded event. Onset density and
  maximum rest use that span; neither is a quality threshold.
- **Multi-track loss** is reported only where the planner can determine the
  source has tracks that the 15-key plan cannot preserve. `null` means not
  measured or not applicable, not zero loss.
- Missing or unsupported measures remain `null` and are accompanied by parser,
  planner, or warning fields. A file can be present in the 112-row inventory
  while its planner status reports no playable notes.

The manifest title/path/ID/hash are catalog identity. Source title and author
fields are included as written in source JSON and are not independent
verification of composition or arrangement authorship.

## Compare an existing sheet with a local candidate

Download a candidate yourself from a pinned source revision, then run:

```powershell
cargo xtask song-audit compare `
  --current "songs\<current-file>.json" `
  --candidate "<local-candidate-file>" `
  --output "<comparison-report>.json"
```

Both files are strict-decoded and passed through the existing planner. The
report includes input SHA-256 values, source schema/version/pitch/tracks,
source note and decoded canonical event counts, physical-key collisions, planner losses,
normalized highest-key melody and interval sketches, normalized onset and
inter-onset timing sketches, phrase-break proxies, quartile coverage,
chord density, maximum chord, initial-offset time/key overlap, and pitch
signals. It reports composition-section completeness as
`not_measured_human_listening_required`.

These signals have narrow limits: the highest key at an onset is only a melody
proxy; normalized timing can hide absolute tempo differences; quartiles do not
identify intro, verse, chorus, or outro; and note overlap or interval
similarity is not an acoustic comparison. The report deliberately marks
musical fidelity unassessed and does not select a candidate.

Interval edit similarity and normalized timing overlap each have a
deterministic budget of 4,000,000 pairwise operations. If a signal would exceed
its budget, comparison returns an error naming that signal and its input sizes;
no comparison report is written for that invocation. Inputs within budget keep
the exact comparison calculation.

## Human listening and provenance

For a listening review, record the exact upstream commit and path, downloaded
file SHA-256, source attribution/format/pitch, named sections and timestamps
heard on both versions, and the reason for any difference. Listen through a
Sky-compatible sheet composer/player such as the upstream project's suggested
Sky Music Nightly Composer. Mark any unreviewed segment as unknown. No
commercial recording is needed or included in this repository.

The candidate matrix pins its comparison corpus to
[`Ai-Vonie/Sky1984-Sheets-Collection` commit
`fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea`](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/tree/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea).
The upstream README says the collection may be incomplete or outdated, asks
users to listen in a Sky composer when verifying sheets, and distinguishes
sheet compilation terms from rights in original compositions and arrangements
([pinned README](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/blob/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea/README.md)).
Accordingly, filenames/metadata here are provenance clues, not a legal or
authorship determination. Only comparison metrics and provenance are checked
in; downloaded candidate source files are temporary and are not product
assets.

## Artifacts

- `inventory.json` and `inventory.csv`: complete active-catalog metrics.
- `inventory-summary.md`: human-readable screening flags and collision rows.
- `candidate-review-matrix.md`: priority-case findings, confidence and
  listening gaps.
- `candidate-review-matrix.csv`: exact current/candidate byte hashes, pinned
  source paths and structural signals for each compared pair.
