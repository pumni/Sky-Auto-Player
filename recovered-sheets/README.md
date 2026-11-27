# Recovered legacy projections for import

These are **opt-in, import-only copies** of two historical built-in JSON sheets whose headers declare `composed` v3 while `columns` is empty and `songNotes` contains physical 15-key events.

| Original built-in source | Legacy copy | Preserved source `songNotes` |
| --- | --- | ---: |
| `songs/Stay with me Goblin.json` | `Stay with me Goblin (legacy).json` | 494 |
| `songs/Only_LeeHi.json` | `Only_LeeHi (legacy).json` | 698 |

The recovery keeps each `songNotes` entry and its order, `time`, `key`, and any additional per-note fields unchanged. Only the contradictory versioned metadata/container is excluded. The display title for Goblin is corrected from the old internal metadata `Only` to its catalog name; the Lee Hi copy has the distinguishable title `Only - Lee Hi`.

## How to try

1. Download one or both recovered JSON files from this directory.
2. In Sky Auto Player, create/open a playlist and choose **Add songs → Import files…**.
3. Select the downloaded `.json` file, inspect Song Details, and run **Test playback (no input)** before attempting game playback.
4. Listen to the introduction and other meaningful sections to verify musical fidelity, especially for Goblin, whose original metadata also says `Only`.

## Boundaries

- **No originals changed:** `songs/`, the built-in manifest, original SHA-256 hashes, and historical golden schedule fingerprints are untouched.
- These copies are not packaged as built-in assets; the installed built-in entries will remain `Unplayable` until a separate, owner-approved product integration is made.
- This is a physical-key projection recovery, **not** evidence that the underlying composition, original pitch, layer provenance, tempo, or arrangement is musically faithful.
- The authoritative versioned parser is unchanged and must continue to reject malformed versioned source data rather than silently retrying `songNotes`.
