# Library V1 Native Contract

This contract covers the native-backed Collections and Local Import workflow
used by the Windows desktop Library Navigator.

## Ownership

`LibraryManifestV1` is native library data stored beside the application
settings in `library-manifest.json`. It contains collection membership and
explicit imported references. The native layer canonicalizes and validates
paths; React receives only opaque IDs, display names, availability, collection
names, counts, catalog rows, and catalog generation. Collection membership and
canonical paths never enter frontend state, browser storage, or path-bearing
DTOs.

Import is reference-only. Registering an existing file or folder does not
copy, move, or delete it. Removing an import removes only the manifest
reference. Missing references are skipped during catalog composition so the
remaining library stays usable.

## Manifest

```text
LibraryManifestV1 {
  version: 1
  imports: [{ source_id, canonical_path, kind: file | folder }]
  collections: [{ id, name, song_ids }]
}
```

The manifest is validated at load and saved through an atomic native replace.
Collection and import IDs are opaque lowercase hexadecimal identifiers. The
domain bounds collection count, name length, membership size, import count,
and persisted path length.

## IPC surface

The native command names are:

- `library.list_collections`
- `library.create_collection`
- `library.rename_collection`
- `library.delete_collection`
- `library.add_songs`
- `library.remove_songs`
- `library.import_local_files`
- `library.import_local_folder`
- `library.remove_import`

`library.list_collections` also returns the path-free `imported_sources`
projection. Each item contains `source_id`, `kind`, native-derived
`display_name`, `song_count`, and `availability` (`available` or `missing`).
This lets the navigator remove or explain missing references after restart
without retaining filesystem paths in React.

The two import commands open the native Tauri file dialog and return
path-free results. Catalog composition combines the configured songs
directory with imported file/folder references and reuses
`CatalogIndex::search_with_allowed_ids` for future collection sources.

Catalog search sources are discriminated values rather than a growing string
enum:

```text
{ kind: smart, id: all | liked }
{ kind: collection, id: collection-id }
{ kind: imported, id: source-id }
```

Collection and imported-source searches validate their opaque IDs and resolve
their song ID allow-lists in native state before calling the existing catalog
primitive. Imported membership and the composed catalog are rebuilt together
for one catalog generation. Missing imports remain visible with zero songs and
do not prevent All Songs from loading.

Removing an import removes only its native manifest reference; it never deletes
or moves the referenced file or folder. Collection mutations return summaries,
not full membership arrays, so the navigator remains bounded independently of
collection size.
