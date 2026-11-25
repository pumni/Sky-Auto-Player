---
key: sheet-formats
locale: en
slug: sheet-formats
title: Supported Sheet Formats
description: >-
  Sky Auto Player supports JSON, .skysheet, and JSON-compatible TXT files exported from
  the Sky Music editor. Learn what each format contains and how to add songs correctly.
summary: >-
  Sky Auto Player reads JSON, .skysheet, and JSON-compatible TXT files imported through the Library.
  All three formats originate from the Sky Music editor and share the same underlying data structure.
category: getting-started
order: 2
published: '2026-08-08'
updated: '2026-11-26'
draft: false
related:
  - windows-setup
  - how-it-works
  - troubleshooting
evidence:
  - category: implementation
    label: README — supported formats and workflow
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/README.md
  - category: architecture
    label: Domain models — schedule and timing frames
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/rust/crates/sky_app_core/src/song.rs
---

## Overview

Sky Auto Player reads sheet files that you export from the
[Sky Music editor](https://specy.github.io/skyMusic/) (also called Sky Music Nightly).
Three file formats are accepted:

| Format              | Extension   | Notes                              |
| ------------------- | ----------- | ---------------------------------- |
| JSON                | `.json`     | Default export from the editor     |
| Sky sheet           | `.skysheet` | Editor's native binary/JSON format |
| JSON-compatible TXT | `.txt`      | Text file that contains valid JSON |

All three share the same underlying data structure — the extension determines how the file
is detected; the parser reads the JSON content regardless.

## Import song files

Create or select a playlist in the desktop Library, then choose **Add songs** and import local files or a folder. Imports reference the original files in place; the app does not move or delete them. If a source file is moved or removed, the imported entry may no longer be available.

## Getting a sheet file

1. Open [Sky Music Nightly](https://specy.github.io/skyMusic/).
2. Find or create a song arrangement.
3. Use the editor's **Export** function to save as JSON or `.skysheet`.
4. Import the exported file through the desktop Library.

## Common issues

**File not appearing in the Library**
: Check that the file was imported into a playlist and that its source is still available at the original path. Verify the extension is `.json`, `.skysheet`, or `.txt`; other extensions are not recognised.

**"Failed to parse" error**
: The file content is not valid JSON. Open the file in a text editor to confirm it
contains a JSON object, not plain text or binary data.

**Song plays notes out of order or skips sections**
: The sheet may contain timing data that does not align with the configured FPS or tempo.
Review the Tempo and FPS values in the Player Dock's **Playback profile**. Windows scheduling
and the game's input sampling still affect when a sent key is registered.

## Playback profile

The Player Dock's **Playback profile** controls app playback defaults; these values are not
stored per song or embedded in sheet files. **Base Hold** offers 1, 1.25, and 1.5 frames, and
**Tempo** offers 0.90×, 0.95×, 1.0×, 1.05×, and 1.10×. **Advanced timing** offers FPS values
of 30, 60, 90, 120, 144, 165, or 240 and a timing margin from 0 to 3,000 µs in 100 µs steps.
