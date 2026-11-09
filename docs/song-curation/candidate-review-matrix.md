# Candidate review matrix

This is a screening record for the priority cases named in Issue #579. Its
metrics do not make song-selection or replacement decisions. The owner later
authorized the two P0 replacements recorded below. Exact candidate paths,
input hashes, format/pitch/event counts and comparison signals are in
[`candidate-review-matrix.csv`](candidate-review-matrix.csv).

All candidates below were fetched from the pinned
[`Ai-Vonie/Sky1984-Sheets-Collection` revision
`fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea`](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/tree/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea).
The source-file SHA-256 is recorded per row. Provenance confidence is high for
the fetched bytes and their pinned path; every structural signal is screening
only, and musical-fidelity confidence remains **not assessed**. No A/B
listening was performed, so section names, timestamps, and composition
completeness remain unreviewed for every row.

| Active catalog item | Candidate source(s) | Structural observation (screening only) |
| --- | --- | --- |
| Senorita (Snejok) | v1 and v2 | v1 has higher interval-sketch similarity (0.317 vs 0.124) and normalized timing overlap (0.289 vs 0.191) against the active short sheet. This is not a lead-note or fidelity verdict. |
| Something Just Like This | NRSPlays | 1,052 candidate physical events vs 238 active; interval similarity 0.238, normalized timing overlap 0.278, duration ratio 3.69. |
| Kiss The Rain | KiMuJo | 437 candidate physical events vs 314 active; interval similarity 0.208, normalized timing overlap 0.597. |
| Lovely | Kenny, KiMuJo, v1–v4 | Candidate max chords range from 1 to 8. v4 has 1,413 physical events and max chord 8; the active sheet has 425 physical events. Density/chord size do not identify the best arrangement. |
| Easy On Me | NRSPlays | Active sheet has 363 physical events vs 447 candidate events. The strict current-subset-after-offset signal is false, so the proposed 363-event timed-subset clue is not reproduced by this comparator. |
| In The End | Maple | Interval similarity 0.868 and normalized timing overlap 0.528; candidate has 276 physical events vs 235 active. Listening needed. |
| Memories (both active files) | NRSPlays, Sadion, plain sheet; Miru Chan for `Memories-Maroon 5.json` | Main `Memories.json` and NRSPlays both have 798 physical events; interval and normalized-timing signals are 1.0, while exact relative time/key overlap after onset offset is 0.003 and the subset signal is false. Both pitch fields are B, with an aligned melody button offset of 0. The `Memories-Maroon 5.json` file instead matches the plain 139-event candidate exactly under the offset-relative signal; this is structural evidence only. |
| You Are My Sunshine | Purgatoriun_sky | Candidate has 156 physical events vs 60 active; interval similarity 0.228 and normalized timing overlap 0.306. |
| Jingle Bells | NRSPlays and Rin5743 full | NRSPlays: 266 physical events vs 170 active; Rin full: 96 vs 170. Both are incomplete structural matches under these signals; listening is needed. |
| We Wish You A Merry Christmas | NRSPlays | Both have 161 physical events; interval and normalized timing signals are 1.0, and active events are a subset after onset offset. Current raw count is 180 because source duplicates are retained. |
| Take Me To Church | v1 | Candidate has 748 physical events vs 156 active; interval similarity 0.187 and normalized timing overlap 0.197. |
| Ocean Eyes | KiMuJo | Both have 214 physical events; interval and normalized timing signals are 1.0. Current raw count is 316. Listening and section coverage remain unknown. |
| Howl's Moving Castle (both active files) | Lara RM YT, SCMS, KiMuJo, v1/v2, Merry-Go-Round variants and other pinned alternatives | Main active file has 1,120 physical events. Lara RM YT candidate also has 1,120, exact interval/timing and offset-relative time/key overlap 1.0, and active events are a subset. This reproduces a strong structural alignment clue. The SCMS candidate has the same event count and normalized interval/timing 1.0 but offset-relative time/key overlap 0.002. The shorter active v2 file did not exactly align with tested candidates. |
| Yoru ni kakeru / Racing Into the Night | Haruma and NRSPlays | Haruma has 27 physical events vs 851 in active `Racing into the Night` (interval 0.043); NRSPlays has 2,321 vs 1,035 in the second active variant (interval 0.432, normalized timing 0.600). |
| Comedy | Kigeki | 483 candidate physical events vs 477 active; interval similarity 0.654 and normalized timing overlap 0.687. |
| Last Christmas | NRSPlays easy version | Both have 305 physical events; interval and normalized timing signals are 1.0, but offset-relative time/key overlap is 0.01 and subset is false. |
| Only_LeeHi / Stay with me Goblin (P0 repairs) | Han / AkiCutieGamingMusic | The owner authorized the pinned Han and Aki sources. Active catalog analysis now reports `ready` for both. The structural comparisons against legacy projections below remain historical screening evidence; neither they nor this inventory establish musical fidelity or redistribution rights. |

## Baseline clue outcomes

- The inventory measured 25 files with extra physical time/key events and
  3,435 extras, including 419 same-source layer-expansion extras. The prior
  22-file / 3,185-event clue did not reproduce under the documented counting
  policy.
- The `Howl's Moving Castle.json` count of 1,120 is reproduced. The pinned
  Lara RM YT candidate also has 1,120 physical events and matches the exact
  onset-relative time/key set. This is a strong structural match; it still
  needs human listening.
- The `Memories.json` count of 798 is reproduced. The NRSPlays candidate also
  has 798 physical events and has exact interval and normalized-timing
  signals, while exact onset-relative time/key overlap is 0.003. Both pitch
  fields are B with aligned melody button offset 0. The prior exact-offset
  clue is therefore only partly supported by these tested signals.
- The `easy on me.json` 363-event count is reproduced, while its 363 events do
  not form a strict onset-relative subset of the tested 447-event NRSPlays
  candidate.
- Before the P0 repair, `Stay with me Goblin.json` and `Only_LeeHi.json` had
  empty authoritative Composed v3 columns, no decoded events, and planner
  status `unsupported` (`NoPlayableNotes`). Their byte-identical historical
  sources remain under `tests/fixtures/song_planning/historical_sources/`.
  The active catalog now uses the pinned Han and Aki sources and reports both
  as `ready`; the original-projection comparisons are historical evidence.

## Listening gaps

No human listening verdicts are recorded for the comparisons in this matrix,
including the two owner-selected P0 replacements. A reviewer still needs to
compare the same named intro/verse/chorus/outro sections, note timestamps and
pitch, and confirm whether meaningful composition sections are present. The
automated report intentionally cannot label any candidate `best`, `correct`,
or `faithful`. File-specific redistribution rights also remain unverified.

## Reproduction

Regenerate the complete inventory with `cargo xtask song-audit inventory`.
For an individual pair, download the exact pinned path and run the compare
command documented in [`README.md`](README.md). The CSV's candidate SHA-256
lets a reviewer confirm the downloaded bytes before comparing them. Candidate
source files were temporary during this audit and are not committed.
