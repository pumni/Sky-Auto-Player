---
key: sheet-formats
locale: en
slug: sheet-formats
title: Supported Sheet Formats
description: >-
  Sky Auto Player supports JSON, .skysheet, and JSON-compatible TXT files exported from
  the Sky Music editor. Learn what each format contains and how to add songs correctly.
summary: >-
  Sky Auto Player reads JSON, .skysheet, and JSON-compatible TXT files from the songs/ folder.
  All three formats originate from the Sky Music editor and share the same underlying data structure.
category: getting-started
order: 2
published: '2026-08-08'
updated: '2026-08-08'
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

## Where to put song files

Place sheet files in the `songs/` folder located next to `Sky-Auto-Player.exe`:

```
Sky-Auto-Player/
├── Sky-Auto-Player.exe
├── config.json
└── songs/
    ├── my-song.json
    ├── another-song.skysheet
    └── third-song.txt
```

After adding files, press **Reload songs** in the desktop Library without restarting the
application.

## Getting a sheet file

1. Open [Sky Music Nightly](https://specy.github.io/skyMusic/).
2. Find or create a song arrangement.
3. Use the editor's **Export** function to save as JSON or `.skysheet`.
4. Drop the exported file into your `songs/` folder.

## Common issues

**File not appearing in the Library**
: Use **Reload songs** in the desktop GUI.
If the file still does not appear, verify the file extension is `.json`, `.skysheet`, or `.txt`.
Other extensions are not recognised.

**"Failed to parse" error**
: The file content is not valid JSON. Open the file in a text editor to confirm it
contains a JSON object, not plain text or binary data.

**Song plays notes out of order or skips sections**
: The sheet may contain timing data that does not align with the configured FPS or tempo.
Try adjusting the tempo multiplier or FPS in the per-song settings.

## Per-song configuration

Sky Auto Player stores per-song settings (hold frame selection, tempo, FPS, theme) separately
from the sheet files. Changing a sheet file does not reset these settings.
