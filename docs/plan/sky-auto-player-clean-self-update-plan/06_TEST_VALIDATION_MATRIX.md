# Test, Validation, E2E and Defender Qualification

## Repository gates

Final candidate must pass:

```powershell
uv run ruff check .
uv run pyright
uv run pytest
uv run --env-file .env python scripts/audit_security_mandates.py
uv run --env-file .env python scripts/audit_free_threaded_wheels.py
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo check --manifest-path rust/Cargo.toml --workspace --all-targets --all-features
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --workspace --all-features
```

Build/package changes also require frozen build and selftests.

## Native updater unit tests

### Release/parser

- exact target tag accepted;
- mismatched tag rejected;
- draft rejected;
- stable prerelease rejected;
- beta prerelease allowed;
- exact asset names required;
- missing ZIP rejected;
- missing SHA rejected;
- duplicate expected asset fails closed.

### Sidecar

- correct single record;
- upper/lower hex;
- wrong filename;
- malformed length;
- multiple records;
- checksum mismatch.

### Archive/path

- `../` traversal;
- absolute drive;
- UNC;
- ADS;
- reserved device names;
- trailing dot/space;
- case collision;
- duplicate path;
- file/directory collision;
- symlink;
- oversized entry/archive;
- too many entries.

### Manifest

- wrong app;
- wrong version;
- wrong executable;
- dirty build when production forbids it;
- duplicate path;
- unsafe path;
- missing file;
- extra file;
- wrong size;
- wrong hash;
- unsupported schema.

### Preserve policy

- `config.json` preserved;
- `.env` preserved;
- `songs/**` preserved;
- `logs/**` preserved;
- case-insensitive matching;
- `songs-old/` must not accidentally match `songs/`.

### Transaction planning

- replace;
- add;
- managed orphan;
- unknown unmanifested file untouched;
- preserved path excluded;
- missing/corrupt old manifest handled fail-closed for deletion.

## Fault-injection integration tests

Disposable install directory.

Inject failure at:

1. backup first file;
2. backup Nth file;
3. journal write;
4. first copy;
5. middle copy;
6. orphan delete;
7. post-copy hash;
8. rollback remove-new;
9. rollback restore;
10. result write;
11. restart.

Assert filesystem state, not only error return.

## Power-loss simulation

Terminate updater after durable `prepared` journal exists.

Next run must:

```text
detect prepared transaction
→ recover before new update
→ verify recovered install
```

Test leftover `committed` journal separately.

## Packaged E2E upgrade

Build/use disposable versions A and B.

Install A into temporary directory.

Create user state:

```text
config.json
.env
songs/custom.skysheet
logs/custom.log
unknown-user-file.txt
```

Update A → B.

Assert:

- app is B;
- managed files match B manifest;
- old managed orphan removed;
- `config.json` unchanged;
- `.env` unchanged;
- songs unchanged;
- logs unchanged;
- unknown unmanifested file unchanged;
- installed updater is B updater;
- transaction cleaned;
- structured result says success.

## Rollback E2E

Install A.

Attempt B with injected failure after mutation begins.

Assert:

- rollback restores exact A managed hashes;
- user state unchanged;
- result says rolled back;
- old app can restart.

## Update launch/playback safety

Test:

- update while idle;
- update request while playback active;
- graceful stop;
- Rust cleanup completes;
- updater launches only after cleanup;
- app exits after successful updater launch;
- updater launch failure leaves app running.

Never shortcut mandatory key release.

## Hotkey Windows tests

- all default hotkeys register;
- each action emitted once per physical press;
- no repeat storm;
- unregister succeeds;
- next session can register same keys;
- conflict is visible and atomic.

## Signing tests

With production signer configured:

- all own PE signed;
- valid chain;
- expected publisher;
- tampered PE fails;
- unsigned own PE fails;
- manifest generated after signing;
- modifying PE after manifest produces hash failure.

## Package assertions

Canonical release must satisfy:

```text
one application ZIP
exact filename
embedded MANIFEST
external MANIFEST match
exact SHA sidecar
no updater.bat
no installer/updater.ps1
no updater PowerShell tests
no build/test cache
```

## Security regression search

Active production/update architecture should not contain required use of:

```text
SetWindowsHookEx
pynput
python-keyboard
ExecutionPolicy Bypass
updater.bat
installer/updater.ps1
```

`SendInput` remains the input injection mechanism.

Scope searches so archived historical docs do not create meaningless failures.

## Defender qualification

Use a clean Windows 10/11 VM or owned runner with:

- Defender real-time protection ON;
- current definition versions recorded;
- no exclusions;
- no manual allow-list;
- no previously restored/allowed copy of the candidate.

Record:

```text
Windows build
Defender platform version
Defender engine version
Defender intelligence version
timestamp
git SHA
release version
```

Scan separately:

```text
Sky-Auto-Player.exe
Sky-Auto-Player-Updater.exe
native_calibration.exe
sky_player_rs*.pyd
Sky-Auto-Player-vX.Y.Z.zip
```

Do not infer child cause from ZIP-only result.

## Evidence matrix

| Phase | Git SHA | App EXE | Updater EXE | Pyd | ZIP |
|---|---|---|---|---|---|
| baseline | | | n/a | | |
| hotkey/focus | | | n/a | | |
| clean native updater package | | | | | |
| signed | | | | | |
| source bootloader | | | | | |
| narrow PyInstaller | | | | | |

Record exact detection names and hashes.

## Defender scan helper

A local helper may invoke the installed Microsoft Defender CLI to scan a specific candidate and collect metadata.

It must never:

- disable protection;
- add exclusions;
- restore/allow detection automatically.

Do not assume GitHub-hosted Windows runner is a meaningful Defender qualification environment.

## False-positive submission

If the final clean/signed artifact is still detected, submit the **exact final artifact/hash** to Microsoft as a software-developer false positive.

Do not respond by adding evasion techniques.

## Release GO

GO only if:

- repository gates green;
- updater unit/fault tests green;
- packaged update E2E green;
- rollback E2E green;
- signing verification green;
- manifest/package verification green;
- no prohibited old updater assets;
- Defender qualification evidence captured.
