# Song planning

sky_app_core::song::build_song_plan is the domain entry point from a sheet
payload to a playback-ready source plan. It does not access files, UI state,
Win32 input, or realtime scheduling internals. Callers provide the already
materialized timing policy used by the existing scheduler.

## Source path and defaults

Flat legacy songNotes uses the compatibility path:
parse_song_json followed by build_schedule_with_policy. The returned
SongPlan::LegacyCompatibility identifies this path. Its parser and schedule
semantics remain the compatibility baseline; its conversion status is ready
while physical risk remains separately reported.

Versioned sheets are decoded only through decode_song_json. A malformed
authoritative source returns its typed decode error; the planner never retries
the lossy songNotes projection.

For versioned sources, PianoPlannerOptions has these defaults:

- mode: None selects MergePlayable, which considers every source track.
- target_pitch: None selects the source song pitch and marks the target pitch
  as an assumption in the result. The application does not detect actual
  in-game tuning.
- tempo_scale: 1.0 preserves authored onset timing.

SingleTrack { track_id } selects a source track by its stable
SourceTrack.index. Duplicate instrument names do not make a track ambiguous;
the stable track id identifies which one to keep. Unknown or duplicated track
ids fail closed. Notes from other tracks remain at their original musical
times in the source diagnostics and are listed as selection omissions.

Conversion mode and target pitch apply to versioned sources. Flat legacy
sources retain the compatibility parser and scheduler regardless of those
conversion options.

## Piano mapping

Legacy SkyKeyIndex events already identify a physical button index from 0
through 14, so the planner passes that index through directly.

Modern note numbers are mapped only when their source pitchability is
Pitched and the sounding note number exactly matches one of the fifteen
standard Sky Piano notes at the selected target pitch. The C table is
[60, 62, 64, 65, 67, 69, 71, 72, 74, 76, 77, 79, 81, 83, 84]; other target
pitch tables apply the selected semitone offset. No nearest-note substitution,
clamping, modulo mapping, octave fold, or automatic transposition is performed.

Source Unpitched and Unresolved events are reported separately from
TargetPianoUnplayable events. Percussion and unresolved source identities are
not reinterpreted as correctly pitched Piano notes.

## Events, provenance, and loss status

PianoOnset retains the authored onset_us, resulting scheduled_onset_us, target
button/scan code, and all source event provenance that maps to the physical
onset. Events are ordered by onset and stable source identity. Exact same
scheduled-onset/button events are merged into one physical onset while
retaining each source note and track in provenance.

PianoPlanDiagnostics keeps serialized source tuples distinct from canonical
decoded events, including legacy layer expansion. Its counts reconcile as:

    decoded events = selected events + omitted events
    selected events = resolved + source unpitched + source unresolved
    resolved events = mapped source events + target-unplayable events
    mapped source events = unique mapped onsets + merged duplicates

Track omissions, unpitched or unresolved source notes, target-unplayable
notes, and mapped authored sustains that become one-shot Piano notes each have
an exact PlanLoss reason and source provenance. A lossy plan with playable
events is requires_confirmation; a plan with no playable event is unsupported;
a conversion with no losses is ready. An empty source also reports
NoPlayableNotes. Decoder source_warnings remain separate from physical_risk.

Authored sustain is retained as a diagnostic only. Physical Piano key holds
still come from the supplied MaterializedTimingPolicy; note duration does not
change hold, release, FPS, or margin behavior.

## Existing schedule and risk path

The planner passes source microsecond onsets and target scan codes into the
existing schedule builder. The requested tempo scale is applied once when the
builder materializes schedule times. The plan retains both source and
scheduled onsets so this transformation is visible. Same-key repeats are not
delayed to avoid conflicts: impossible_same_key_repeats, physical timing
conflicts, and RiskReport come from the existing scheduler and analyzer.
Conversion status does not override a downstream preparation or physical-risk
gate.

## Regression evidence

rust/crates/sky_app_core/tests/piano_planner.rs covers C and D target pitch,
v4 nominal versus v5 absolute notes, all fifteen simultaneous keys, merged
provenance, duplicate instrument names with explicit track selection, loss
reconciliation, unplayable and unpitched notes, sustain degradation, repeated
same-key onsets, tempo scaling, stable planning order, and legacy compatibility.
The checked-in piano_planner_cases.json provides small synthetic golden
inputs. legacy_song_corpus.rs separately pins the existing 106-song schedule
fingerprints.

The planner consumes source and target pitch facts only. It does not change
sky_dispatch_core, sky_player, native timing, focus admission, or Windows
SendInput behavior.
