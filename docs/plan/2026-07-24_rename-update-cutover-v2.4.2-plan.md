# Plan: Post-rename update cutover → v2.4.2

> **Status:** PROPOSAL (not yet implemented).
> **Date:** 2026-07-24.
> **Target version:** `2.4.2` (`pyproject.toml` `[project].version` + git tag `v2.4.2`).
> **Audience:** AI coding agents (and human reviewers).
> **Non-normative:** This plan is a proposal. On conflict, `AGENTS.md`, `SECURITY.md`,
> and the P2 docs in `docs/INDEX.md` win. After implementation, update the P2 doc
> `docs/distribution-and-update.md` in the same change set.
>
> **Implements:** full update support after the Sky Player → Sky Auto Player rename,
> closing the live asset-name gap and giving pre-rename installs a one-shot bridge
> path into the new brand.

---

## 0. Objective

Ship **v2.4.2** such that **all** of the following are true on Windows 10/11 x64:

1. **Fresh installs** of `Sky-Auto-Player-v2.4.2.zip` run `Sky-Auto-Player.exe` and can
   self-update via `updater.bat` against future releases that use the new asset names.
2. **Post-rename local/main builds** (already looking for `Sky-Auto-Player-v*.zip`) can
   update once `v2.4.2` is published with the new names (today they fail exit 2 against
   live `v2.4.1` assets still named `Sky-Player-v*`).
3. **Pre-rename field installs** (`Sky-Player.exe` + old `updater.bat` from 2.4.0/2.4.1)
   can run their **existing** `updater.bat` once, land on 2.4.2 binaries + the new
   updater, and thereafter follow the canonical path — **without** manual zip surgery
   and **without** losing `config.json` or `songs/`.
4. Docs, CHANGELOG, README, landing/FAQ strings, and the normative distribution doc
   agree with the shipped contract.
5. No P0 security regression (SendInput-only, HTTPS allow-list, SHA256-before-mutate,
   MANIFEST fail-closed, preserve-list).

---

## 1. Evidence (why this plan exists)

Verified against live GitHub + current tree on 2026-07-24:

| Fact | Value |
|---|---|
| Repo | `pumni/Sky-Auto-Player` (old `pumni/Sky-Player` → HTTP 301) |
| Latest release tag | `v2.4.1` |
| Live assets | `Sky-Player-v2.4.1.zip`, `Sky-Player-v2.4.1.zip.sha256`, `MANIFEST.json` |
| Live MANIFEST | `app=Sky-Player`, `executable=Sky-Player.exe` |
| Code on `main` / local | expects `Sky-Auto-Player-v*.zip`, `Sky-Auto-Player.exe`, repo `Sky-Auto-Player` |
| Simulated asset pick (post-rename updater vs live latest) | **FAIL exit 2** (new zip name missing) |
| Normative doc path | `AGENTS.md` / `INDEX.md` point at `docs/distribution-and-update.md`, but HEAD may have moved it to `docs/archive/` — **must restore** |
| CHANGELOG Unreleased | documents rename; currently claims updater-assisted migration is **not** supported — **this plan reverses that for the single 2.4.2 bridge** |
| `pyproject.toml` version today | `2.4.1` → bump to `2.4.2` only in the version phase |

Chicken-and-egg for pre-rename users:

- Old updater selects **exact** asset name `Sky-Player-v{ver}.zip`.
- Old updater requires staging root to contain **`Sky-Player.exe`**.
- Canonical post-rename zip has only `Sky-Auto-Player.exe` → old updater cannot apply it.
- Therefore v2.4.2 **must dual-publish a legacy bridge zip** whose layout the old
  updater accepts, and that bridge zip **must also install the new updater +
  `Sky-Auto-Player.exe`** so the second update uses the canonical path.

---

## 2. Immutable execution guardrails

### 2.1 P0 / architecture (never violate)

1. No game tampering, memory read, hooks, injection, or anti-cheat bypass.
2. `SendInput` remains the only input mechanism; do not add keyboard libraries.
3. External updater remains the **only** apply path; do **not** reintroduce in-app
   auto-apply / self-overwrite of running binaries.
4. HTTPS host allow-list, SHA256-before-mutate, MANIFEST fail-closed, and
   preserve-list (`config.json` content except allowed update fields; never mutate
   `songs/`; keep `logs/` skip) stay mandatory.
5. `installer/updater.ps1` **MUST keep UTF-8 BOM** (`EF BB BF`). Any edit that strips
   the BOM is a release blocker (PS 5.1 parse failure).
6. Domain/orchestration stay free of `ctypes` / Win32.
7. No new runtime PyPI dependencies. Dev-only tools already in the tree are fine.

### 2.2 AGENTS.md “Ask first” surfaces touched by this plan

This plan is the user’s **explicit authorization** to edit the following when
executing the plan (do not re-ask unless expanding scope beyond this document):

- `installer/updater.ps1` and `installer/Tests/*`
- `updater.bat`
- `.github/workflows/release.yml`
- `src/build_app.py` (packaging only; do **not** extend `Sky-Auto-Player.spec` `excludes`)
- `docs/distribution-and-update.md` (restore + update)
- Version pair for **this release only**: `pyproject.toml` `version` → `2.4.2`
  (no change to `requires-python` / `.python-version`)

Do **not** edit without a new explicit ask:

- `scripts/audit_security_mandates.py` / `.config/security_audit_baseline.json`
- `Sky-Auto-Player.spec` `excludes` / COLLECT strategy
- `tests/golden_schedules/`, `perf-baselines/*`
- Winget community PR publication (manifest file may be version-bumped in-tree only)

### 2.3 Hard non-goals (do not implement)

- Authenticode / EV signing.
- System installer, Start Menu, `.skysheet` association (Phase 4 of mpv plan).
- Delta / differential updates.
- Auto-killing unrelated processes or force-close without `-ForceClose`.
- Reintroducing `update.auto_apply` / in-app zip apply.
- Renaming the Python package import path (`sky_music`) or Rust module names.
- Supporting macOS/Linux.
- Migrating `%LOCALAPPDATA%\Sky-Player\updater.log` (leave old log; new log dir is fine).
- Changing scheduler / SendInput / free-threaded interpreter pins.
- “Support forever” dual naming — bridge is **one transition release family**
  (see §3 decision D3 for sunset).

### 2.4 Coding discipline for AI agents

1. **One phase at a time.** Do not merge phases into one commit/PR unless the phase
   map marks them as a single commit unit.
2. **Tests first** for every behavior change: write/adjust a failing test, then fix.
3. Prefer surgical diffs; no drive-by refactors in UI, scheduler, or platform.
4. After any edit to `installer/updater.ps1`, re-verify **BOM present** and run the
   PS 5.1 parse check from `release.yml`.
5. Security-sensitive surfaces (`updater.ps1` allow-list, preserve-list, MANIFEST
   gate) must be **read and verified directly** by the primary agent — do not accept
   a subagent summary as proof.
6. Use `uv run` for all Python; never `pip install` / manual `.venv` activate.
7. Do not push tags or create GitHub Releases until Phase 8 runbook and the user
   explicitly approve the tag push.
8. Conventional commits: one logical change per commit where practical
   (`fix(updater):…`, `build(release):…`, `docs:…`, `chore(release): 2.4.2`).

---

## 3. Locked product decisions

| ID | Decision | Rationale |
|---|---|---|
| **D1** | Canonical brand remains **Sky Auto Player** / `Sky-Auto-Player.exe` / zip `Sky-Auto-Player-v{ver}.zip`. | Matches renamed repo and current code. |
| **D2** | v2.4.2 release publishes **two portable zips** (each with its own `.sha256`): **canonical** + **legacy bridge**. Top-level release also keeps a **canonical** `MANIFEST.json` asset (as today). | Old updaters hard-require `Sky-Player-v{ver}.zip` + `Sky-Player.exe`. |
| **D3** | Bridge is temporary: dual-publish **at least v2.4.2**; may continue for 2.4.x patches if needed. **Remove dual-publish no earlier than the first 2.5.0 release**, and only after CHANGELOG + distribution doc announce the sunset (≥ 30 days notice preferred). | Avoid permanent dual-name debt. |
| **D4** | Legacy bridge zip layout (must pass **old** 2.4.0/2.4.1 updater checks): contains **both** `Sky-Player.exe` and `Sky-Auto-Player.exe` as **byte-identical copies** of the same build, plus the **new** `updater.bat` / `installer/updater.ps1`, `MANIFEST.json` with `executable` set to `Sky-Player.exe` (so old `Get-RunningVersion` / layout checks work), version `2.4.2`. | One run of old updater installs new brand + new updater; next update uses canonical zip and orphan-cleanup can drop `Sky-Player.exe`. |
| **D5** | New `updater.ps1` asset selection order: (1) `Sky-Auto-Player-v{ver}.zip` + sidecar, else (2) `Sky-Player-v{ver}.zip` + sidecar. Never mix zip from one name with sha256 from the other. | Post-rename and bridge both work; no cross-hash. |
| **D6** | New `updater.bat` / path init accept **either** exe next to the bat: prefer `Sky-Auto-Player.exe`, else `Sky-Player.exe`. If neither exists → exit 1 with a clear message. | Bridge installs and mixed folders work. |
| **D7** | Process guard checks **both** process names (`Sky-Auto-Player`, `Sky-Player`), still scoped to processes whose path parent equals install root. | Avoid locked-file failures on either brand. |
| **D8** | `Compare-Version` / `Get-RunningVersion` use the resolved primary exe (D6 preference). | PEP 440 path stays single. |
| **D9** | In-app notify remains notify-only; banner text must mention: close app → run `updater.bat` → reopen **Sky Auto Player**. Add a short note that pre-2.4.2 Sky Player installs use the same `updater.bat` once (bridge). Do **not** auto-download in-app. | Keeps mpv model. |
| **D10** | Preserve-list unchanged: never overwrite user `config.json` wholesale; only patch `update.last_check_ts` + `update.last_notified_version`. Never touch `songs/` or `logs/`. | P2 invariant. |
| **D11** | Orphan cleanup may delete `Sky-Player.exe` on a later canonical update when it is absent from the new MANIFEST — **desired**. Must never delete preserve-list paths. | Completes brand migration. |
| **D12** | Remove production `DEBUG updater.ps1: …` host messages (or gate behind env `SKY_UPDATER_DEBUG=1`). | Hygiene. |
| **D13** | Restore `docs/distribution-and-update.md` to docs root if archived; set header version to **2.4.2**; document dual-publish + bridge + sunset. | Hierarchy of truth. |
| **D14** | Version bump is **2.4.2** only (patch): rename cutover + updater bridge, no feature train. | Semver honesty. |

---

## 4. Target contracts (post-2.4.2)

### 4.1 GitHub Release assets for tag `v2.4.2`

Required uploads:

1. `Sky-Auto-Player-v2.4.2.zip` — canonical portable tree
2. `Sky-Auto-Player-v2.4.2.zip.sha256` — ASCII sidecar (`{hash}  Sky-Auto-Player-v2.4.2.zip`)
3. `MANIFEST.json` — **canonical** manifest (app `Sky-Auto-Player`, executable `Sky-Auto-Player.exe`)
4. `Sky-Player-v2.4.2.zip` — **legacy bridge** portable tree (D4)
5. `Sky-Player-v2.4.2.zip.sha256` — sidecar for the bridge zip only

Optional later: GitHub Release body checklist (Phase 7).

### 4.2 Canonical zip root (minimum)

```text
Sky-Auto-Player.exe
updater.bat
installer/updater.ps1
MANIFEST.json          # executable=Sky-Auto-Player.exe, app=Sky-Auto-Player, version=2.4.2
config.json            # defaults only; user file preserved on update
songs/                 # defaults/sample only; user tree preserved on update
_internal/…            # PyInstaller payload
README.md              # if OPTIONAL_ASSETS copied
```

Must **not** require `Sky-Player.exe` in the canonical zip.

### 4.3 Legacy bridge zip root (minimum)

Same as canonical, **plus**:

```text
Sky-Player.exe         # byte-identical copy of Sky-Auto-Player.exe
MANIFEST.json          # executable=Sky-Player.exe, app may be "Sky-Auto-Player" or dual;
                       # version=2.4.2; files[] must hash-match every staged file that is listed
```

Both exes **must** have identical SHA256. Bridge MANIFEST must list every file the
fail-closed checker needs (same rules as today). If `files[]` omits the main exe
(current `build_app.write_release_manifest` excludes `exe_name`), keep that behavior
**consistent** for both packages, but ensure orphan logic does not brick the install
mid-copy (existing order: backup → orphan delete → copy is OK if the new exe is in
the copy set).

### 4.4 Updater identity resolution (new script)

Pseudocode (normative for implementers):

```text
function Resolve-PrimaryExe(installRoot):
  candidates = [
    installRoot/Sky-Auto-Player.exe,
    installRoot/Sky-Player.exe,
  ]
  for c in candidates:
    if file exists: return c
  fail exit 1

function Resolve-ProcessNames():
  return ["Sky-Auto-Player", "Sky-Player"]

function Select-ReleaseAssets(release, version):
  pairs = [
    ("Sky-Auto-Player-v{version}.zip", "Sky-Auto-Player-v{version}.zip.sha256"),
    ("Sky-Player-v{version}.zip",      "Sky-Player-v{version}.zip.sha256"),
  ]
  for (zip, sha) in pairs:
    if both assets present on release: return (zip, sha)
  fail exit 2 "missing zip or sha256"

function Resolve-StagingRoot(extractDir):
  if extractDir/Sky-Auto-Player.exe exists: return extractDir
  if extractDir/Sky-Player.exe exists: return extractDir
  if single child dir contains either exe: return that dir
  fail exit 5
```

Repo / API remain:

```text
owner = pumni
repo  = Sky-Auto-Player
UA    = sky-auto-player-updater
```

Log dir remains `%LOCALAPPDATA%\Sky-Auto-Player\updater.log`.

### 4.5 User journeys (acceptance stories)

| ID | Start state | Action | End state |
|---|---|---|---|
| J1 | Empty folder, download canonical 2.4.2 zip | Extract, run exe | App runs; brand Sky Auto Player |
| J2 | J1 install | Close; `updater.bat` when 2.4.3+ canonical exists | Updates; config/songs preserved |
| J3 | Field `Sky-Player` 2.4.1 install (old updater) | Close; run existing `updater.bat` after 2.4.2 published | Becomes 2.4.2 with both exes + new updater; config/songs preserved |
| J4 | J3 after success | Run new `updater.bat` for a later canonical-only release | Updates via `Sky-Auto-Player-v*`; `Sky-Player.exe` may be orphan-removed |
| J5 | Post-rename broken build (new updater, only old assets on GitHub) | After 2.4.2 publish, `updater.bat` | Finds canonical zip; updates |
| J6 | App running | `updater.bat` without `-ForceClose` | Exit 4; no mutation |
| J7 | Tampered zip hash | Any updater | Exit 3; no mutation |
| J8 | Missing MANIFEST in zip | New updater | Exit 5; no mutation |

---

## 5. Phase map

| Phase | Goal | Primary files | Risk if skipped |
|---|---|---|---|
| **0** | Inventory freeze + failing regression tests for bridge contracts | `installer/Tests/*`, `tests/test_update_*.py`, maybe new `tests/test_release_asset_names.py` | Silent contract drift |
| **1** | Dual-name identity + asset selection + process guard in `updater.ps1` / `updater.bat` | `installer/updater.ps1`, `updater.bat`, Pester | Breaks J3–J5 |
| **2** | Bridge package builder in `build_app` | `src/build_app.py`, unit tests | Cannot produce D4 zip |
| **3** | `release.yml` dual upload + checksums | `.github/workflows/release.yml` | Live GitHub still one name |
| **4** | In-app / CLI messaging + skip dead DEBUG | `update_service.py`, modals/README strings as needed, `updater.ps1` | User confusion |
| **5** | Docs restore + normative contract + CHANGELOG 2.4.2 notes | `docs/distribution-and-update.md`, `docs/INDEX.md`, `CHANGELOG.md`, `README.md`, FAQ if needed | Hierarchy of truth lies |
| **6** | Version bump to 2.4.2 + local full gates | `pyproject.toml`, version surfaces | Wrong tag lock |
| **7** | Manual Windows E2E matrix (J1–J8 subset) | local dist + fake root / real API | Ship blind |
| **8** | Tag `v2.4.2`, watch release workflow, post-publish verify | git tag, GitHub | Incomplete cutover |

**Do not start Phase N+1 until Phase N acceptance is green.**  
Phases 0–6 are code/docs; 7 is human/agent manual on Windows; 8 requires **user approval** before tag push.

Suggested commit grouping (after tests green per phase):

1. `test(updater): freeze rename bridge contracts` (Phase 0)
2. `fix(updater): dual-name identity and asset fallback` (Phase 1)
3. `build: emit legacy Sky-Player bridge zip` (Phase 2)
4. `build(ci): publish dual release zips` (Phase 3)
5. `fix(ui/docs): rename update messaging; drop updater DEBUG` (Phase 4–5 may split)
6. `chore(release): bump version to 2.4.2` (Phase 6)

---

## 6. Phase 0 — Inventory freeze + failing tests

### 6.1 Goal

Encode the post-rename + bridge contracts as tests that **fail on today’s code**
where behavior is missing, and **pass** for already-correct pieces.

### 6.2 Work

1. Read (do not skim summaries):
   - `installer/updater.ps1` (header contract, `Assert-HttpsUrl`, `Copy-UpdateTree`,
     asset selection, process gate, BOM)
   - `updater.bat`
   - `src/build_app.py` (`APP_NAME`, `write_release_manifest`, updater asset copy)
   - `.github/workflows/release.yml` (stage + upload)
   - `src/sky_music/domain/update_checker.py` (`DEFAULT_REPO`)
   - `docs/archive/distribution-and-update.md` or `docs/distribution-and-update.md`
2. Add / extend **Pester** tests in `installer/Tests/updater.Tests.ps1` (new `Describe` blocks):

   | Test name (suggested) | Assert |
   |---|---|
   | `Resolve-PrimaryExe prefers Sky-Auto-Player.exe` | When both exist, Auto wins |
   | `Resolve-PrimaryExe falls back to Sky-Player.exe` | When only legacy exists |
   | `Resolve-PrimaryExe fails when neither exists` | Non-zero / throw per implementation |
   | `Select-ReleaseAssets prefers canonical pair` | When both pairs present |
   | `Select-ReleaseAssets falls back to legacy pair` | When only `Sky-Player-v*` present |
   | `Select-ReleaseAssets refuses mixed pairs` | Zip new + sha old → fail |
   | `Process guard detects legacy Sky-Player in install root` | Mock/process stub if already patterned; else unit-level helper |
   | `Staging accepts Sky-Player.exe-only layout` | Bridge extract |
   | `Staging accepts Sky-Auto-Player.exe-only layout` | Canonical extract |
   | `Preserve-list still skips songs/ and config.json body` | Existing tests must remain green |

   If helpers do not exist yet, **extract pure functions** in Phase 1; in Phase 0 you may
   add tests as `pending` only if Pester cannot bind — prefer real failing tests against
   new function names you will add in Phase 1 (write the function stubs that throw
   `NotImplemented` so tests fail for the right reason).

3. Add **pytest** coverage:

   - `tests/test_update_checker.py`: default repo remains `Sky-Auto-Player`; asset
     fixtures use `Sky-Auto-Player-v*.zip` names (already mostly true — freeze them).
   - New `tests/test_build_bridge_package.py` (or extend build tests if present):
     given a fake release dir with `Sky-Auto-Player.exe` + dummy files, the bridge
     builder (Phase 2) produces:
     - both exes, identical hash
     - MANIFEST `version == 2.4.2` (or injected version)
     - MANIFEST `executable == Sky-Player.exe`
     - canonical builder still has `executable == Sky-Auto-Player.exe` and **no**
       requirement for `Sky-Player.exe`

   Until Phase 2 lands, the bridge builder test should fail on `ImportError` /
   missing function — that is desired.

4. Snapshot inventory comment at top of the Pester describe: list current hard-coded
   strings expected after Phase 1 (`Sky-Auto-Player`, fallback `Sky-Player`).

### 6.3 Acceptance

- New tests exist and fail for missing bridge behavior (or fail on missing symbols).
- Existing updater/update_checker tests still collected.
- No production behavior change yet (stubs only if required).

### 6.4 Gate

```powershell
uv run pytest tests/test_update_checker.py tests/test_update_service.py tests/test_update_config.py -q
# Pester (expect new tests fail until Phase 1):
# Install-Module Pester 5.x if needed — same pins as release.yml
Invoke-Pester -Path .\installer\Tests\updater.Tests.ps1 -PassThru
```

---

## 7. Phase 1 — Updater dual-name support

### 7.1 Goal

Make `updater.bat` + `installer/updater.ps1` implement D5–D8, D12; keep all security
gates; turn Phase 0 tests green.

### 7.2 Work (ordered)

1. **`updater.bat`**
   - Accept install folder if **either** `Sky-Auto-Player.exe` or `Sky-Player.exe` exists.
   - Error text must name both and tell user to run from the install folder.
   - Keep `pwsh` prefer + `powershell` fallback + `-ExecutionPolicy Bypass -File`.
   - Pass through `%*` unchanged (`-Channel`, `-DryRun`, `-ForceClose`, `-Restart`).

2. **`installer/updater.ps1`**
   - Keep BOM.
   - Implement helpers from §4.4 (`Resolve-PrimaryExe`, process name list,
     `Select-ReleaseAssets`, staging resolution). Prefer small functions for Pester.
   - Replace hard-coded single-name assumptions in:
     - `Initialize-Paths` / `Get-ExePath`
     - process gate block
     - asset selection block
     - staging layout check
     - restart / “reopen” messages (say “Sky Auto Player” in user-facing text;
       mention legacy exe only in debug logs)
   - `Compare-Version` still delegates to resolved primary exe `--compare-versions`.
   - Remove unconditional `Write-Host "DEBUG updater.ps1: …"` (D12). If retained,
     only when `$env:SKY_UPDATER_DEBUG` is `1`.
   - Do **not** weaken `Assert-HttpsUrl`, SHA256 zip check, `Test-ManifestIntegrity`,
     or preserve-list.
   - Fake-root test mode (`SKY_UPDATER_FAKE_ROOT`) must keep working for Pester.

3. **User-facing strings** on success:
   - Prefer: `DONE: updated to v{ver}. Reopen Sky-Auto-Player.exe …`
   - If primary was legacy-only install that now has Auto exe after copy, still tell
     user to open `Sky-Auto-Player.exe` when present.

4. Re-run BOM check after save:

```powershell
$b = [IO.File]::ReadAllBytes((Resolve-Path installer\updater.ps1))
if (-not ($b[0]-eq 0xEF -and $b[1]-eq 0xBB -and $b[2]-eq 0xBF)) { throw 'BOM missing' }
```

### 7.3 Acceptance

- Phase 0 Pester tests green.
- Manual dry logic: with only legacy assets in a fake release.json, selection returns
  legacy pair; with both, canonical pair.
- PS 5.1 parse gate (from `release.yml`) passes.
- Preserve-list tests still pass.

### 7.4 Gate

```powershell
# BOM + PS 5.1 parse (copy from release.yml)
# Pester full updater tests
# uv run pytest tests/test_update_*.py -q
```

### 7.5 Explicit non-changes

- Do not change GitHub owner/repo away from `pumni` / `Sky-Auto-Player`.
- Do not delete orphan preserve-list entries.
- Do not call `Stop-Process` without `-ForceClose`.

---

## 8. Phase 2 — Bridge package builder (`build_app`)

### 8.1 Goal

From a finished canonical `dist/Sky-Auto-Player-v{ver}/` tree, produce a **legacy
bridge tree + zip inputs** without a second PyInstaller run.

### 8.2 Work

1. In `src/build_app.py` (or a tightly scoped helper module imported only by
   `build_app` — prefer keep in `build_app.py` unless file size forces split):

   ```text
   build_legacy_bridge_dir(canonical_dir: Path, version: str) -> Path
   ```

   Algorithm (normative):

   1. Create `dist/Sky-Player-v{ver}-bridge/` (or under `dist/.bridge/…`); wipe if exists.
   2. Copy tree from canonical dir (files only as today; use same ignore rules — no
      `Tests/`).
   3. Ensure `Sky-Auto-Player.exe` exists; else fail hard.
   4. Copy `Sky-Auto-Player.exe` → `Sky-Player.exe` (byte-identical).
   5. Rewrite `MANIFEST.json`:
      - `version` = `{ver}`
      - `app` = `Sky-Auto-Player` (canonical product identity)
      - `executable` = `Sky-Player.exe`
      - `executable_sha256` = hash of `Sky-Player.exe` (same as Auto)
      - Rebuild `files[]` the same way `write_release_manifest` does, with
        `exe_name="Sky-Player.exe"` **or** include both exes consistently with
        current exclude rules — **pick one rule and test it**:
        - Recommended: call shared manifest writer with `exe_name="Sky-Player.exe"`
          so legacy `Get-RunningVersion` reads `MANIFEST.version` and ProductVersion
          fallback still works via either exe.
   6. Return bridge dir path.

2. CLI flag (optional but useful):

   - `python -m build_app --manifest` always builds canonical (unchanged).
   - Add `--bridge-legacy` (default **True** for release safety) to also emit bridge
     dir. Or always emit bridge when `--manifest` is set — **prefer always-on when
     `--manifest`** so release.yml cannot forget.

3. Do **not** change `Sky-Auto-Player.spec` excludes.

4. Unit-test with tmp_path fake exe bytes + fake files (no PyInstaller).

### 8.3 Acceptance

- Canonical dir has Auto exe, no requirement for Player exe.
- Bridge dir has **both** exes, `Get-FileHash` equal.
- Bridge MANIFEST `executable == Sky-Player.exe`, `version` correct.
- `updater.bat` + `installer/updater.ps1` present in both trees.
- Phase 0 bridge pytest green.

### 8.4 Gate

```powershell
uv run pytest tests/test_build_bridge_package.py -q
# Optional local:
# uv run --env-file .env python -m build_app --manifest
# then inspect dist\Sky-Auto-Player-v* and bridge dir
```

---

## 9. Phase 3 — Release workflow dual publish

### 9.1 Goal

`.github/workflows/release.yml` uploads the five assets in §4.1 for every `v*` tag
while dual-publish is enabled (D3).

### 9.2 Work

1. After existing “Build + manifest + smoke test” step, ensure bridge dir exists
   (either built inside `build_app --manifest` or a new step calling a small Python
   entry). Prefer **single** `build_app --manifest` producing both trees to avoid drift.

2. Stage step (extend current Compress-Archive logic):

   ```text
   canonical:
     dist/Sky-Auto-Player-v$ver/  →  $RUNNER_TEMP/Sky-Auto-Player-v$ver.zip
     sha256 sidecar
     copy canonical MANIFEST.json → $RUNNER_TEMP/MANIFEST.json

   bridge:
     dist/<bridge-dir>/  →  $RUNNER_TEMP/Sky-Player-v$ver.zip
     sha256 sidecar for that zip only
   ```

3. Attest **all** zip + sha + canonical MANIFEST subjects (add bridge zip + bridge sha
   to `subject-path`).

4. `softprops/action-gh-release` `files:` list includes all five artifacts.

5. Keep: free-threaded audit, ruff, pyright, security audit, pytest, Pester, PS 5.1
   parse, tag↔version lock, prerelease bit via `packaging.version`.

6. Add a workflow assertion step before upload:

   ```powershell
   $required = @(
     "Sky-Auto-Player-v$ver.zip",
     "Sky-Auto-Player-v$ver.zip.sha256",
     "Sky-Player-v$ver.zip",
     "Sky-Player-v$ver.zip.sha256",
     "MANIFEST.json"
   )
   # each must exist in RUNNER_TEMP; fail if not
   ```

### 9.3 Acceptance

- YAML valid; required names exact (case-sensitive).
- No step still references only `Sky-Player-v` as the sole zip.
- Comment in workflow pointing at this plan §3 D3 sunset.

### 9.4 Gate

- `actionlint` if available; else manual review.
- Do **not** push a real tag yet.

---

## 10. Phase 4 — Product messaging + DEBUG cleanup

### 10.1 Goal

Users understand how to update across the rename; no debug noise.

### 10.2 Work

1. `format_update_banner` (`update_service.py`) — keep Sky Auto Player wording; ensure
   lines include `updater.bat`. Optional one-liner:
   `If you still have Sky-Player.exe, run updater.bat once to migrate.`
2. `UpdateBannerModal` / settings copy — no claim of in-app apply.
3. CLI `--check-update` output should remain accurate (repo + version).
4. Finish D12 DEBUG cleanup if not done in Phase 1.
5. Do not change throttle / channel policy semantics.

### 10.3 Acceptance

- Banner snapshot/tests updated if they assert exact strings.
- No `DEBUG updater.ps1` on normal runs.

### 10.4 Gate

```powershell
uv run pytest tests/test_update_service.py tests/test_update_checker.py -q
uv run ruff check src/sky_music/orchestration/update_service.py src/sky_music/ui
```

---

## 11. Phase 5 — Documentation cutover

### 11.1 Goal

Normative + user docs match D1–D14 and version 2.4.2.

### 11.2 Work

1. **Restore** `docs/distribution-and-update.md` to docs root if it currently lives
   only under `docs/archive/`:
   - `git mv docs/archive/distribution-and-update.md docs/distribution-and-update.md`
     when that is the history; or copy content and delete archive copy **only if**
     archive was a mistaken move — prefer `git mv` to preserve history.
2. Update that doc:
   - Header version **2.4.2**
   - Release artefact section: five assets during bridge window; note sunset D3
   - Updater dual-name behavior
   - Migration section: pre-2.4.2 Sky Player → run `updater.bat` once after 2.4.2
   - TEMP path naming may still say sky-update-* (match code)
   - Process guard both exe names
3. `docs/INDEX.md`:
   - Fix distribution bullet if still saying `Sky-Player-v…` triple only
   - Add this plan under Active References as **Proposed / in progress** until Done
4. `CHANGELOG.md`:
   - Move Unreleased rename notes under `## [2.4.2] - YYYY-MM-DD` when releasing
   - **Replace** the “updater-assisted migrations are not supported” warning with the
     bridge story (one-shot via dual zip + dual-name updater)
   - Document dual-publish and preserve-list unchanged
5. `README.md` install/update sections: canonical zip name; note legacy users can
   keep using `updater.bat` after 2.4.2 is out
6. Landing/FAQ (`docs/index.html`, `docs/faq.html`, `docs/vi/*`) only if they still
   claim impossible migration or old zip-only names — keep surgical
7. `manifests/p/pumni/SkyPlayer/pumni.SkyPlayer.yaml`: bump version metadata to
   2.4.2 and InstallerUrl to **canonical** `Sky-Auto-Player-v2.4.2.zip` when known;
   leave `InstallerSha256` placeholder unless hashing a real artifact — do **not**
   publish winget PR in this plan

### 11.3 Acceptance

- `Test-Path docs/distribution-and-update.md` → True
- AGENTS.md links resolve
- CHANGELOG no longer contradicts bridge support for 2.4.2

### 11.4 Gate

Manual doc review + link paths. No need for full pytest if only docs (still run if
mixed commit).

---

## 12. Phase 6 — Version bump to 2.4.2

### 12.1 Goal

Single source version = `2.4.2` everywhere release tooling reads.

### 12.2 Work

1. `pyproject.toml` → `version = "2.4.2"`
2. Confirm `src/sky_music/_version.py` is generated by `build_app` (do not hand-edit
   if generated).
3. Any hard-coded `2.4.1` in **normative** docs headers updated to `2.4.2`.
4. Do **not** change `.python-version` / `requires-python`.
5. Run version consistency tests if present (`tests/test_version_consistency.py`).

### 12.3 Acceptance

```powershell
uv run python -c "import tomllib,pathlib; assert tomllib.loads(pathlib.Path('pyproject.toml').read_text(encoding='utf-8'))['project']['version']=='2.4.2'"
uv run pytest tests/test_version_consistency.py tests/test_version_cli.py -q
```

### 12.4 Full pre-tag gate (mandatory)

```powershell
uv run --env-file .env python scripts/audit_free_threaded_wheels.py
uv run ruff check .
uv run pyright
uv run --env-file .env python scripts/audit_security_mandates.py
uv run pytest
# Pester updater tests (Pester 5.x pins as release.yml)
# PS 5.1 parse of installer/updater.ps1
# BOM check
```

Optional but strongly recommended before tag:

```powershell
uv run --env-file .env python -m build_app --manifest
# smoke is inside build_app unless --skip-test
```

---

## 13. Phase 7 — Windows E2E matrix (manual / agent on Windows)

### 13.1 Goal

Prove J1–J8 on a real Windows host before tagging, using local artifacts.

### 13.2 Fixtures

1. Build canonical + bridge via `build_app --manifest`.
2. Create two install dirs under `%TEMP%\sap-e2e\`:
   - `canonical-install\` — extract canonical zip
   - `legacy-install\` — simulate 2.4.1: copy **old** layout if available, or
     construct: `Sky-Player.exe` (can use bridge’s Player exe from an older build
     if present), **old** updater scripts from git tag `v2.4.1` if needed for true
     J3. Minimum J3 simulation: install dir with only `Sky-Player.exe` name + **old**
     asset selector expectations exercised via `SKY_UPDATER_FAKE_ROOT` serving
     `Sky-Player-v2.4.2.zip` built from bridge.

### 13.3 Cases (minimum)

| Case | Steps | Expect |
|---|---|---|
| E1 | Canonical install; run `--selftest-textual` | exit 0 |
| E2 | Canonical `updater.bat -DryRun` against fake root with newer canonical assets | DryRun pass; no file mutation |
| E3 | Legacy-shaped install + fake root with **only** bridge zip pair | Updates to 2.4.2; `config.json` user keys preserved; `songs/` untouched |
| E4 | After E3, ensure `Sky-Auto-Player.exe` exists and new updater present | paths exist |
| E5 | Running exe + updater without ForceClose | exit 4 |
| E6 | Wrong sha256 in fake sidecar | exit 3; install unchanged |
| E7 | Zip without MANIFEST | new updater exit 5 |
| E8 | Compare-Version via exe | exit codes 0/1/2/3 match pytest |

Record results in the PR description or a short `docs/plan` as-built note (not a new
normative doc).

### 13.4 Gate

All minimum cases pass; any failure blocks Phase 8.

---

## 14. Phase 8 — Publish runbook (user-approved)

### 14.1 Preconditions

- Phases 0–7 green.
- Working tree clean; commits on the intended branch.
- User explicitly says to tag/publish.

### 14.2 Steps

```powershell
# 1. Confirm version
uv run python -c "import tomllib,pathlib; print(tomllib.loads(pathlib.Path('pyproject.toml').read_text(encoding='utf-8'))['project']['version'])"
# → 2.4.2

# 2. Tag (annotated)
git tag -a v2.4.2 -m "v2.4.2: rename update cutover (canonical + legacy bridge)"

# 3. Push branch + tag (ONLY with user approval)
git push origin HEAD
git push origin v2.4.2
```

4. Watch `Release` workflow on GitHub until green.
5. Post-publish verify:

```powershell
$h = @{ 'User-Agent'='sap-cutover'; 'Accept'='application/vnd.github.v3+json' }
$r = Invoke-RestMethod -Uri 'https://api.github.com/repos/pumni/Sky-Auto-Player/releases/tags/v2.4.2' -Headers $h
$r.assets | ForEach-Object Name
# Must include:
#   Sky-Auto-Player-v2.4.2.zip
#   Sky-Auto-Player-v2.4.2.zip.sha256
#   Sky-Player-v2.4.2.zip
#   Sky-Player-v2.4.2.zip.sha256
#   MANIFEST.json
```

6. Download both zips; verify sidecar hashes; spot-check each contains expected exe(s).
7. Optional: run field J3 on a real 2.4.1 install folder backup.

### 14.3 Rollback

| Failure | Action |
|---|---|
| Workflow fails before upload | Fix forward; delete failed tag only if not public yet (`git tag -d` / remote delete) with user approval |
| Wrong assets uploaded | Publish a fixed `v2.4.3` rather than mutating history if users may have downloaded; avoid rewriting public tags |
| Bridge zip breaks old updater | Hotfix scripts + `v2.4.3` dual-publish; document manual canonical install as interim |

### 14.4 After publish

- Mark this plan **Status: IMPLEMENTED** with date + release URL.
- Update `docs/INDEX.md` plan bullet to “shipped in 2.4.2”.
- Do not remove dual-publish until D3 sunset criteria met.

---

## 15. File touch list (expected)

| Path | Phases | Notes |
|---|---|---|
| `installer/updater.ps1` | 1, 4 | BOM preserved; dual-name; no DEBUG |
| `updater.bat` | 1 | either-exe gate |
| `installer/Tests/updater.Tests.ps1` | 0, 1 | bridge contracts |
| `installer/Tests/WriteUpdateFields.Tests.ps1` | 0–1 | only if path helpers change |
| `src/build_app.py` | 2 | bridge builder; `--manifest` emits bridge |
| `tests/test_build_bridge_package.py` | 0, 2 | new |
| `tests/test_update_*.py` | 0, 4 | string/repo freezes |
| `.github/workflows/release.yml` | 3 | five assets + attest |
| `src/sky_music/orchestration/update_service.py` | 4 | banner |
| `src/sky_music/ui/textual_app/modals.py` | 4 | only if copy requires |
| `docs/distribution-and-update.md` | 5 | restore + 2.4.2 contract |
| `docs/INDEX.md` | 5, 8 | plan entry + status |
| `CHANGELOG.md` | 5–6 | 2.4.2 section |
| `README.md` | 5 | install/update |
| `pyproject.toml` | 6 | version 2.4.2 |
| `manifests/.../pumni.SkyPlayer.yaml` | 5 | optional version URL bump |
| Landing/FAQ HTML | 5 | only if stale |

**Out of scope paths:** scheduler, platform inputs, `Sky-Auto-Player.spec` excludes,
security audit script, golden schedules.

---

## 16. Risk register

| Risk | Severity | Mitigation |
|---|---|---|
| BOM stripped on edit | P0 release break | BOM check in Phase 1 gate + release.yml |
| Dual zip forgotten in workflow | P0 field break | required-assets assert step |
| Bridge exe not byte-identical | Medium | hash equality test |
| Orphan delete removes user files outside preserve-list | Medium | document; do not expand orphan scope in this plan |
| Old updater lacks MANIFEST fail-closed of newer script | Low | old 2.4.x still has sha256 zip check; bridge ships new script for next time |
| SmartScreen on unsigned exe | Existing | out of scope |
| GitHub rate limit during checks | Low | existing throttle |
| Attest subject list incomplete | Medium | include all five files |
| Version tag mismatch | High | existing verlock step |
| Docs left in archive | Medium | Phase 5 restore gate |
| Permanent dual-name debt | Low | D3 sunset |

---

## 17. Definition of Done

1. `pyproject.toml` version is `2.4.2` and tag `v2.4.2` release is published with
   **all five** assets in §4.1.
2. Canonical zip: `Sky-Auto-Player.exe` only (plus normal payload); updates via
   `Sky-Auto-Player-v*` work (J1/J2/J5).
3. Bridge zip: both exes identical; old-style name selectable; J3 demonstrated.
4. Preserve-list intact; security audit green; Pester + pytest green; BOM + PS 5.1 OK.
5. `docs/distribution-and-update.md` at docs root documents bridge + sunset.
6. CHANGELOG 2.4.2 documents rename cutover **with** updater-assisted one-shot bridge
   (no longer “not supported”).
7. No P0 mandate violated; no unrelated refactors.

---

## 18. Implementation checklist (agent)

Copy this into the PR body and tick as you go:

- [ ] Phase 0 tests added (failing for missing bridge)
- [ ] Phase 1 updater dual-name + Pester green + BOM
- [ ] Phase 2 bridge builder + pytest green
- [ ] Phase 3 release.yml five assets + attest
- [ ] Phase 4 messaging + DEBUG removed
- [ ] Phase 5 docs restored/updated + CHANGELOG + README + INDEX
- [ ] Phase 6 version 2.4.2 + full altitude gate
- [ ] Phase 7 E2E matrix recorded
- [ ] Phase 8 user-approved tag + post-publish asset verify
- [ ] Plan status → IMPLEMENTED with release link

---

## 19. Appendix A — Live regression command (post-publish)

```powershell
$h = @{ 'User-Agent'='sap-cutover'; 'Accept'='application/vnd.github.v3+json' }
$latest = Invoke-RestMethod 'https://api.github.com/repos/pumni/Sky-Auto-Player/releases/latest' -Headers $h
$ver = $latest.tag_name.TrimStart('v')
$names = $latest.assets | ForEach-Object name
$need = @(
  "Sky-Auto-Player-v$ver.zip",
  "Sky-Auto-Player-v$ver.zip.sha256",
  "Sky-Player-v$ver.zip",
  "Sky-Player-v$ver.zip.sha256",
  "MANIFEST.json"
)
$missing = $need | Where-Object { $_ -notin $names }
if ($missing) { throw "Missing assets: $($missing -join ', ')" }
'OK: dual-publish assets present'
```

## 20. Appendix B — Relationship to prior plans

- Supersedes the CHANGELOG claim that updater migration is impossible (for the 2.4.2
  bridge window only).
- Extends `docs/plan/2026-07-18_distribution-mpv-pattern-plan.md` (notify-only +
  external updater) without reviving in-app apply.
- Complements `docs/plan/2026-07-19_updater-cleanup-and-refinement-plan.md` (orphan
  cleanup already in tree) — do not re-implement orphan logic; only ensure brand
  leftover `Sky-Player.exe` is eligible for orphan removal on later canonical updates.
- Does not reopen winget Phase 7 beyond in-tree manifest version URL hygiene.

---

*End of plan.*
