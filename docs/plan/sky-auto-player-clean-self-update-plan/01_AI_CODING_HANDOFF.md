# AI Coding Handoff — Execute Clean Native Self-Update Architecture

You are implementing an intentional architecture cutover in `pumni/Sky-Auto-Player`.

## Baseline

This plan was authored against:

- branch: `main`
- commit: `c2f0f573e5087684f4c641a5f2ba4abb478e897c`

Before editing:

1. Read `AGENTS.md` completely.
2. Read `SECURITY.md` completely.
3. Read the current normative docs listed by `AGENTS.md`, especially:
   - `docs/architecture.md`
   - `docs/rt-dispatch-architecture.md`
   - `docs/timing-principles.md`
   - `docs/distribution-and-update.md`
4. Read the actual current source before editing.
5. Run `git rev-parse HEAD` and `git status --short`.

If HEAD differs from the baseline, adapt the implementation to current source. Do not blindly apply old line assumptions.

## This task intentionally changes the current P2 distribution/update architecture

The current repo makes notify-only + external PowerShell apply normative. This task explicitly replaces that model.

Do **not** weaken P0/P1. Instead, the first cutover step must update the normative architecture text so the new native self-update model becomes the repository contract.

Update at least:

- `AGENTS.md` repo map / architecture invariants that reference `updater.bat` / `installer/updater.ps1`;
- `docs/distribution-and-update.md`;
- any other normative doc that makes the old apply path authoritative.

After that documentation cutover, implementation must follow the new contract.

## Final target

One canonical application distribution only:

```text
Sky-Auto-Player-v<version>.zip
Sky-Auto-Player-v<version>.zip.sha256
MANIFEST.json
```

ZIP runtime content includes:

```text
Sky-Auto-Player.exe
Sky-Auto-Player-Updater.exe
native_calibration.exe
MANIFEST.json
README.md
config.json
songs/
_internal/
```

No script updater is shipped.

When the user chooses **Update now**:

1. app ensures playback/input cleanup is complete;
2. app copies the bundled updater to a per-run directory under `%LOCALAPPDATA%\Sky-Auto-Player\update-runs\...`;
3. app launches the temporary updater with install root, parent PID, current version, target version, channel and restart intent;
4. app exits;
5. updater waits for the app process to exit;
6. updater independently refetches the exact GitHub release by target tag;
7. updater downloads canonical ZIP + SHA256 sidecar;
8. updater verifies outer SHA256;
9. updater validates ZIP paths before extraction;
10. updater extracts to staging outside install root;
11. updater validates embedded `MANIFEST.json` and exact staged file set;
12. updater validates Authenticode on project-owned PE payloads;
13. updater creates a durable backup/journal;
14. updater replaces only managed application files;
15. updater never overwrites preserved user state;
16. updater verifies installed hashes;
17. updater commits or rolls back;
18. updater writes a structured result;
19. updater restarts the app when install state is valid.

## Clean cutover — absolutely no legacy support

Delete and do not replace with compatibility shims:

- `updater.bat`;
- `installer/updater.ps1`;
- updater PowerShell tests/actions used only by that updater;
- `Sky-Player.exe` updater compatibility;
- dual-name process/executable resolution;
- bridge release assets;
- legacy package naming;
- PowerShell fallback;
- BAT fallback.

Do not create `legacy.zip`, `portable.zip`, `bridge.zip`, or any second distribution bundle.

Do not add migration logic solely to support pre-cutover updater installations.

Existing users may be required to manually download the first native-updater release once. That is accepted.

## Security constraints

P0 remains immutable:

- no game-file modification;
- no game-memory read/write;
- no DLL injection;
- no debugger attach;
- no anti-cheat bypass;
- no process hooks;
- no keyboard hooks;
- `SendInput` remains the only input injection mechanism.

Do not use `SetWindowsHookEx`, `pynput`, `keyboard`, or similar.

The Defender work is legitimate hardening, not scanner evasion. Do not:

- obfuscate imports/API names;
- dynamically resolve APIs to hide them;
- pack/encrypt executables;
- enable UPX;
- add junk code;
- disable Defender;
- add Defender exclusions;
- weaken archive/hash/signature verification.

## Protected real-time boundary

Treat the current Rust playback dispatcher as protected. Do not change unless compilation/interface necessity requires it:

- scan-code whitelist;
- packet semantics;
- `SendInput` path;
- cleanup/recovery semantics;
- QPC timing;
- wait strategy;
- estimator;
- no-allocation guarantees;
- wake-to-send telemetry semantics.

Rust physical-key state verification used for safety may remain.

## Required execution order

Follow `03_PHASED_IMPLEMENTATION_PLAN.md`.

Do not perform one uncontrolled rewrite. Keep phases reviewable and run gates after every phase.

If a required invariant or test cannot be proven, report `NO-GO` and stop instead of moving forward.

## External signing prerequisite

Do not invent, generate, or self-sign a production certificate.

Implement clean signing boundaries and verification. A production tag release must fail closed until the maintainer provisions an approved Authenticode provider/certificate.

PR/non-release builds may remain explicitly unsigned.

## Completion criteria

Complete only when:

- exactly one ZIP is published;
- no updater BAT/PowerShell is shipped;
- native Rust updater performs transactional self-update;
- `Update now` works from the app;
- user data is preserved;
- project-owned PE files are signed before manifest generation in production;
- manifest is generated after signing;
- global playback hotkeys no longer use continuous `GetAsyncKeyState` polling;
- aggressive automatic focus manipulation is removed;
- production PyInstaller bootloader is built from the exact pinned PyInstaller source version;
- all repository gates pass;
- Defender qualification evidence is recorded without exclusions or bypasses.
