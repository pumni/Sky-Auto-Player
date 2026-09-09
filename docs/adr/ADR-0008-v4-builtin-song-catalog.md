# ADR-0008: V4 Built-in Song Catalog

- Status: Accepted for implementation
- Date: 2026-09-09
- Scope: V4 desktop catalog, Tauri packaging, native library composition, and release qualification

## Context

V4 uses the Tauri NSIS package as the canonical Windows distribution and keeps mutable application state under the OS application-data boundary. The repository also contains a curated `songs/` tree that was shipped in the V3 portable package, but the current V4 package does not include that catalog and the native runtime resolves the default `songs` setting to the user-owned application-data music directory.

A fresh V4 install therefore starts with an empty primary song directory even though the repository contains a built-in catalog. Copying those product-owned files into the user-owned directory at first launch would restore the visible behavior but would mix ownership and introduce synchronization, overwrite, rename, deletion, and upgrade policy that V4 otherwise avoids.

V4 has not been officially released. This decision therefore defines the V4 contract directly; no compatibility migration for prior V4 production installs is required.

## Decision

### 1. Built-in songs are immutable application resources

Built-in songs are application-owned assets managed by the release. User songs are user-owned assets managed outside the immutable application resource set.

The canonical ownership split is:

```text
Tauri application resources
  builtin-songs/
    manifest.json
    sheets/
      ... curated built-in sheets ...

OS application data
  songs/                user-owned primary library only
  config.json
  library-manifest.json
  cache/
  logs/
```

The installer/updater owns the packaged built-in resources. It must not copy, merge, seed, overwrite, or delete built-in songs inside the user's `songs/` directory.

The repository `songs/` tree remains the source catalog. Packaging may map it to a different resource layout; source-tree layout is not required to match installed resource layout.

### 2. Built-in catalog membership is manifest-driven

The packaged built-in catalog is declared by a committed manifest. Runtime membership is not discovered by scanning every supported file under the resource directory.

Schema V1 has this logical form:

```json
{
  "schema_version": 1,
  "songs": [
    {
      "id": "9f02e91d412c4ac4a90fe4703babc261",
      "path": "sheets/All Of Me.json",
      "title": "All Of Me",
      "sha256": "..."
    }
  ],
  "retired_songs": [
    {
      "id": "681b5a67d18146e791bd53d53967f09c",
      "last_title": "Old Song"
    }
  ]
}
```

The manifest is committed and reviewed. CI and release tooling may verify or update derived fields through explicit developer commands, but the release build must not silently invent product identity or regenerate the authoritative catalog during packaging.

Active and retired IDs are disjoint. IDs are unique across all identities ever issued by the built-in catalog.

### 3. Built-in identity is stable and independent of filesystem location and content bytes

Existing user and imported songs keep their current path-derived identity policy.

Each built-in catalog item receives one opaque stable song ID represented as exactly 32 lowercase hexadecimal characters. A built-in ID is assigned once and remains stable across:

- application reinstall or install-location changes;
- resource directory changes;
- filename or relative-path renames;
- title corrections;
- sheet-format conversion;
- timing, chord, metadata, or other content corrections that remain the same product catalog item.

The SHA-256 field identifies exact asset bytes for integrity and qualification. It is not the song identity. A content change therefore changes `sha256` but does not, by itself, change `id`.

A genuinely different arrangement or product catalog item receives a new ID even when its title matches an earlier item.

### 4. Retired identities remain reserved

Retiring a built-in song removes it from the active resource catalog but does not erase or reuse its ID. The ID moves to the retired identity ledger.

Likes and playlist memberships are not rewritten when an item is retired. Existing Library behavior intentionally permits well-formed song IDs that are temporarily absent from the active catalog. Such references become dormant and may become active again if the same built-in catalog item is restored with its original ID.

Restoring the same logical catalog item reuses the retired ID. Replacing it with a different arrangement creates a new ID and leaves the old ID retired.

Runtime does not expose retired items as playable catalog entries.

### 5. Immutable resources and mutable paths remain separate dependencies

`AppPaths` continues to describe mutable/runtime/user filesystem ownership.

A distinct immutable resource dependency, conceptually `AppResources`, carries the packaged built-in catalog location. The Tauri shell resolves `BaseDirectory::Resource` and injects the resolved path into the native composition root. `sky_app_core` and `sky_native_adapters` must not acquire a Tauri dependency merely to resolve packaged resources.

This keeps the dependency direction:

```text
Tauri shell
  -> resolved AppResources
  -> NativeDesktopRuntime
  -> sky_native_adapters / sky_app_core
```

### 6. Sources declare identity; CatalogIndex arbitrates identity

The catalog core accepts source entries that can represent either:

- path-derived identity for user/imported assets; or
- validated stable identity for built-in assets.

The final song-ID resolution and conflict rules remain owned by `CatalogIndex` rather than by the filesystem composer.

The public entry API should make invalid stable IDs difficult to construct, for example through validated constructors rather than a freely constructible optional ID field.

Catalog conflict semantics are:

```text
same canonical path + same resolved identity
  -> duplicate source reference; index once

same canonical path + different resolved identity
  -> identity/path conflict; reject the new generation

same resolved identity + different canonical path
  -> identity collision; reject the new generation
```

The composer must not silently deduplicate an identity conflict before the index can validate it.

Song-ID format validation should have one core authority shared by catalog and library code. This decision does not require converting persisted or IPC song IDs from strings to a new wire type.

### 7. Catalog composition owns source projections

`FileCatalogSource` remains responsible for enumerating ordinary filesystem sources. It should not grow into the owner of built-in plus user plus imported composition.

A native adapter composition service, conceptually `CatalogComposer`, composes:

- the manifest-driven built-in source;
- the primary user song directory;
- explicit imported file/folder references.

The resulting composition keeps separate native membership projections and one playable entry set. The intended logical shape is:

```text
CatalogComposition
  entries
  library_membership
  builtin_membership
  user_membership
  imported_membership
  imported_status
  builtin_status
```

with the invariant:

```text
library_membership = builtin_membership union user_membership
```

Imported assets may exist in the playable catalog and in playlists, but they do not expand the `All Songs` smart source. This preserves the existing Library V1 contract that `All Songs` is the primary/bundled library projection.

### 8. The initial UI contract does not change

The implementation must not require a new frontend source enum or a new Built-in/My Songs navigator in this work.

The existing smart sources remain:

```text
All Songs
Liked Songs
Playlists
```

`All Songs` resolves to `library_membership`, which is built-in plus primary user songs. The native composition may retain built-in and user projections for diagnostics, tests, and future UI use without exposing them in the current DTO surface.

Individual catalog rows remain path-free and do not need a scalar `source` field. Membership is a projection because a physical song may participate in more than one source context.

### 9. Runtime and release failure policy differ deliberately

Build/package/release qualification is strict. It must reject at least:

- malformed or unsupported manifest schema;
- malformed, uppercase, or duplicate IDs;
- overlap between active and retired IDs;
- unsafe relative paths, traversal, or paths escaping the packaged root;
- unsupported song extensions;
- missing declared resources;
- hash mismatches;
- invalid song parsing;
- manifest/catalog set drift according to the repository's chosen source layout.

Runtime performs enough validation to ensure it never trusts malformed manifest paths or identities. It does not need to hash every built-in song on every normal startup if the exact package has already passed release qualification.

A damaged or unavailable built-in source must not make user-owned songs, imported songs, playlists, or the application unusable. The built-in source fails as a source and records a bounded native diagnostic status; a valid release package is expected never to enter this state.

### 10. Tooling makes identity changes explicit

Repository tooling should provide a pure verification path suitable for CI and explicit developer mutation operations for built-in catalog maintenance.

The intended semantics are:

- adding a new song may generate a new stable ID and SHA-256;
- updating an existing asset preserves its ID and updates derived integrity metadata;
- renaming is explicit and preserves the existing ID;
- retiring is explicit and moves the ID to the retired ledger;
- restoring the same item is explicit and reactivates its prior ID;
- verification never mutates the repository.

The exact CLI surface belongs to implementation, but implicit rename detection, silent retirement, automatic ID reuse, and CI-only generated manifests are out of contract.

### 11. Packaged acceptance must prove the fresh-install behavior

The V4 packaging qualification must include direct evidence that the NSIS product contains and can load the built-in catalog independently of a source checkout and independently of user `AppData\...\songs` contents.

At minimum, packaged acceptance must prove:

1. build the exact canonical NSIS candidate;
2. install under a fresh current-user application-data state;
3. launch the installed application;
4. load the expected built-in catalog count through the native catalog path;
5. prove the user-owned `songs/` directory may remain empty while `All Songs` is non-empty;
6. preserve ordinary user-song behavior when a user song is added;
7. bind this result to the exact packaged candidate used by the existing V4 qualification flow.

Ordinary core and adapter tests must cover stable-ID persistence across path/content changes, active/retired identity validation, composition memberships, duplicate/reference behavior, and conflict rejection.

## Consequences

- Fresh V4 installs expose the curated built-in library without mutating user data.
- Application updates replace built-in resources through the package lifecycle while leaving user-owned songs untouched.
- Likes and playlists can remain stable across built-in renames, corrections, retirement, restoration, reinstall, and install-location changes.
- The catalog gains a second identity source while preserving the existing 32-lowercase-hex persistence/IPC representation.
- Packaging and release qualification become responsible for validating the built-in asset set as part of the product artifact.
- The native catalog model gains explicit source composition rather than continuing to expand `FileCatalogSource` responsibilities.

## Rejected alternatives

### Copy built-ins into the user song directory on first launch

Rejected because it converts application-owned resources into apparently user-owned files and creates synchronization, overwrite, conflict, rename, retirement, and orphan-cleanup policy on every later release.

### Install built-ins directly into the mutable user song directory

Rejected for the same ownership reason and because package/update behavior would become coupled to user-managed storage.

### Download the starter catalog after first launch

Rejected because the catalog is already a release asset, network availability is unnecessary, and an additional remote content/update trust contract is not justified for this requirement.

### Use packaged canonical paths as built-in song IDs

Rejected because reinstall or install-location changes would change Likes and playlist identity.

### Use content hashes as built-in song IDs

Rejected because correcting the same catalog item would change its identity and break durable Likes/playlist references.

### Automatically deduplicate user copies against built-ins by content

Rejected from this work. A user-owned copy and a built-in item have different ownership and may legitimately have different identities. Duplicate-content UX may be improved separately without coupling this architecture to content-based identity remapping.
