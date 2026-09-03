# Library V1 Native Contract

This contract is the native foundation for the stacked Collections and Local
Import work. It does not add frontend controls until the persistence and
catalog semantics are complete.

## Ownership

`LibraryManifestV1` is native library data stored beside the application
settings in `library-manifest.json`. It contains collection membership and
explicit imported references. The native layer canonicalizes and validates
paths; React receives only opaque IDs, display names, availability, names, song
IDs, counts, and catalog generation. Canonical paths never enter frontend
state, browser storage, or path-bearing DTOs.

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
projection. Each item contains an opaque `id`, `kind`, native-derived
`display_name`, and current `available` state so a future Import Manager can
remove or explain missing references after restart without retaining
filesystem paths in React.

The two import commands open the native Tauri file dialog and return
path-free results. Catalog composition combines the configured songs
directory with imported file/folder references and reuses
`CatalogIndex::search_with_allowed_ids` for future collection sources.

Catalog search sources are discriminated values rather than a growing string
enum:

```text
{ kind: smart, id: all | liked }
{ kind: collection, id: collection-id }
```

Collection search validates the opaque collection ID and resolves its song ID
allow-list in native state before calling the existing catalog primitive.

Collections and local import remain intentionally UI-gated until the next
stacked implementation adds source selection, collection navigation, and
native-backed mutation workflows.
