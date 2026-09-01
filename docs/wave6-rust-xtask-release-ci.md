# Wave 6 — Rust xtask release and CI boundary

Wave 6 starts from the accepted Wave 5 main checkpoint
`b9debfaab043b88423e4efbd381f08ebec726d7e`. The canonical repository and
release path is now Rust plus Bun:

```text
cargo xtask check static
cargo xtask check rust
cargo xtask check desktop
cargo xtask check all
cargo xtask ci classify --full
cargo xtask version check --tag v3.5.0
cargo xtask dist --profile dist --output <directory>
cargo xtask verify-dist --release-dir <release-directory>
```

`rust/xtask` is repository tooling only. It is not linked into the shipped
desktop executable and is not copied into the portable tree. Its process
boundary uses typed `std::process::Command` arguments and checked exit status.

The native product provenance chain remains observed and fail-closed:

```text
repository HEAD
  = observed Sky-Auto-Player.exe metadata
  = observed native_calibration.exe metadata
  = PROVENANCE.native_build_commit
```

The package builder observes metadata from the built binaries and again from
the copied release binaries before writing provenance. It retains MANIFEST
schema 2, runtime-Python negative scanning, deterministic file ordering and
the accepted public executable layout. The disposable updater E2E runner is
used only for qualification and is never copied into the public tree.

Required CI and release workflows do not set up Python, uv, or a virtual
environment. Bun remains the frontend package/build/test tool and Rust remains
the native/compiler/check/release tool. The repository may still contain
optional historical or manual Python tooling while it is migrated in a later
cleanup, but no such file is required for product checks, package assembly,
manifest verification, updater qualification, or tag release.

The canonical version is the native Cargo package version. `cargo xtask
version check` uses the accepted PEP-440-compatible Rust parser and emits
structured `version=` and `is_prerelease=` fields for workflows.

Wave 6 intentionally does not change product DTOs, event names, updater
transaction formats, calibration cache schema, portable layout, realtime
input behavior, or performance configuration.
