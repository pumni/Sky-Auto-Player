# Library V1 Native Contract

This contract covers the native-backed Playlist workflow used by the Windows
desktop Library Navigator. A local file or folder is an infrastructure source
used to add playable tracks to a playlist; it is never a Library navigation
item.

## Ownership

`LibraryManifestV1` is native library data stored beside the application
settings in `library-manifest.json`. It contains playlist membership and
explicit imported references. The native layer canonicalizes and validates
paths; React receives only opaque IDs, playlist names, counts, catalog rows,
and catalog generation. Imported source references, membership details, and
canonical paths never enter frontend state, browser storage, or path-bearing
DTOs.

Import is reference-only. Registering an existing file or folder does not copy,
move, or delete it. A playlist import transaction records the native import
reference and appends the resulting song IDs to the target playlist in one
atomic manifest save. Missing references are skipped during catalog
composition so the remaining library stays usable.

## Manifest

```text
LibraryManifestV1 {
  version: 1
  imports: [{ source_id, canonical_path, kind: file | folder }]
  collections: [{ id, name, song_ids }] // native compatibility name; product = playlists
}
```

The manifest is validated at load and saved through an atomic native replace.
Playlist and import IDs are opaque lowercase hexadecimal identifiers. The
domain bounds playlist count, name length, membership size, import count, and
persisted path length.

## IPC surface

The native command names are:

- `library.list_playlists`
- `library.create_playlist`
- `library.rename_playlist`
- `library.delete_playlist`
- `library.add_songs`
- `library.remove_songs`
- `library.import_local_files_to_playlist`
- `library.import_local_folder_to_playlist`

`library.list_playlists` returns only the path-free playlist summaries needed by
the navigator. Imported source status and membership remain native-only
infrastructure projections.

The two import commands receive only a target `playlistId` from React, open the
native Tauri file dialog, resolve selected paths in Rust, and return a playlist
summary, imported song count, and catalog generation. The returned DTO never
contains a path or source ID.

Catalog search sources are discriminated values rather than a growing string
enum:

```text
{ kind: smart, id: all | liked }
{ kind: playlist, id: playlist-id }
```

All Songs is restricted to the library membership projection: immutable built-in
resources plus the primary user song directory. Imported assets may remain in
the native playable index, but they do not change the All Songs source. Liked
Songs uses liked IDs, and a playlist uses its persisted song IDs.
All allow-lists are resolved in native state before calling the existing
catalog primitive. Imported membership and the composed playable catalog are
rebuilt together for one catalog generation; missing imports do not prevent All
Songs from loading.

The native composer keeps separate built-in, user, and imported projections.
`library_membership = builtin_membership ∪ user_membership`; frontend DTOs and
navigator sources remain unchanged. Built-in IDs come from the committed V4
resource manifest and user/imported IDs retain the existing canonical-path
policy. The composer does not content-dedupe or remap either ownership class.

## Built-in catalog identity and ownership

The committed `builtin-songs/manifest.json` defines the active packaged
catalog. Its songs are immutable package resources resolved through the native
resource boundary. User songs remain in the mutable AppData `songs/` directory;
installation and updates do not seed, copy, overwrite, or delete built-in songs
there. Native composition keeps packaged resources separate from mutable user
paths.

Each built-in arrangement has a stable ID of exactly 32 lowercase hexadecimal
characters. Its ID is independent of source path and bytes: path renames,
metadata corrections, and source-byte corrections preserve membership under
the same ID, while SHA-256 verifies the bytes and does not define identity.
Retired IDs remain reserved in the manifest ledger; active and retired IDs are
disjoint, and restoring an arrangement reuses its retired ID.

Manifest validation rejects malformed identities and paths, active/retired ID
overlap, undeclared or missing source files, hash mismatches, and unparseable
active songs. Source/build qualification runs
`cargo xtask builtin-catalog verify`; release qualification verifies the
installed resources and a fresh AppData install as specified in
[Tauri packaging](v4-tauri-packaging.md). `CatalogIndex` may collapse repeated
entries only when both canonical path and ID match. It rejects one path
assigned different IDs and one ID assigned different paths; callers must not
discard conflicts before validation. Built-in and user memberships compose for
All Songs, while import-only items do not expand that source. At runtime,
malformed or missing built-in resources are not trusted; valid user and
imported sources remain available for library and playlist use.

Deleting a playlist removes only its membership and native playlist record; it
never deletes songs or local files. Playlist mutations return summaries, not
full membership arrays, so the navigator remains bounded independently of
playlist size.
