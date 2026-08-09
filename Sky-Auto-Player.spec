# -*- mode: python ; coding: utf-8 -*-
from importlib.util import find_spec
from pathlib import Path

ROOT = Path(SPECPATH).resolve()

# --- Configuration ---
app_name = 'Sky-Auto-Player'
entry_point = str(ROOT / 'src' / 'main.py')

# We don't put songs/README in datas here to keep them in the ROOT of dist, not hidden in _internal
datas = []
binaries = []
hiddenimports = [
    "sky_music.platform.win32",
    "sky_music._native_build",
    "sky_music._version",
    "sky_music.orchestration.native_admission",
    "sky_music.orchestration.engine",
    "sky_music.orchestration.calibration",
    "sky_music.orchestration.telemetry",
    "sky_music.infrastructure.background",
    "sky_music.infrastructure.hotkeys",
    "sky_music.infrastructure.doctor",
    "sky_music.infrastructure.focus",
    "sky_music.infrastructure.realtime",
    "sky_music.infrastructure.timing",
    "sky_music.platform.win32.global_hotkeys",
    "textual.drivers.windows_driver",
    "rich.markdown",
]

# Only the stylesheet is a runtime data file owned by the application. All
# Python modules are discovered from the entry point and explicit dynamic
# imports above; broad collection would copy the entire source tree as data.
stylesheet = ROOT / 'src' / 'sky_music' / 'ui' / 'textual_app' / 'styles' / 'base.tcss'
if not stylesheet.is_file():
    raise RuntimeError(f'required Textual stylesheet is missing: {stylesheet}')
datas.append((str(stylesheet), 'sky_music/ui/textual_app/styles'))

# The Rust dispatcher is the sole production release artifact. Collection must
# fail closed if its wheel was not
# built by scripts/build_rust_wheel.py.
native_module = find_spec('sky_player_rs.sky_player_rs')
if native_module is None or native_module.origin is None:
    raise RuntimeError(
        'sky_player_rs is required; run scripts/build_rust_wheel.py before packaging'
    )
binaries.append((native_module.origin, 'sky_player_rs'))
hiddenimports.extend(['sky_player_rs', 'sky_player_rs.sky_player_rs'])

block_cipher = None

a = Analysis(
    [entry_point],
    pathex=[str(ROOT / 'src')],
    binaries=binaries,
    datas=datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    # Exclude dev-only packages that are never needed at runtime.
    # (pyinstaller itself, type checkers, linters, audio-capture dev tool)
    # Do NOT exclude tkinter/numpy without confirming they are unused.
    # Stdlib trims below are verified via `scripts/audit_free_threaded_wheels.py`
    # gate — extend only after grepping `src/` for transitive use.
    excludes=[
        "pyinstaller",
        "pyright",
        "ruff",
        "soundcard",
        "pytest",
        "_pytest",
        "xmlrpc",
        "pydoc",
    ],
    win_no_prefer_redirects=False,
    win_private_assemblies=False,
    cipher=block_cipher,
    noarchive=False,
    # optimize=1: removes docstrings and __debug__-only blocks. NOTE — assert
    # statements are NOT preserved at optimize>=1 (Python compiles them out
    # along with __debug__). The duplicate-release check at
    # ``orchestration/core/loop.py`` is a debug/source invariant; production
    # correctness relies on the coordinator's uniqueness guard
    # (``pending_scan_codes`` / ``pending_by_generation``) at insert time,
    # not on the dispatch-time assertion. ``build_app.run_smoke_test`` invokes
    # ``--selftest-optimize`` after packaging so a regression in this contract
    # cannot ship silently. Add an explicit runtime guard only if P0 mandates
    # it — separate patch + benchmark, never on the frozen-build hot path.
    optimize=1,
)

pyz = PYZ(a.pure, a.zipped_data, cipher=block_cipher)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name=app_name,
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=True,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
    version=str(ROOT / 'windows_version_info.txt'),
    # By NOT setting contents_directory='.', PyInstaller 6 defaults to '_internal'
    # which is the cleanest best practice.
)

coll = COLLECT(
    exe,
    a.binaries,
    a.zipfiles,
    a.datas,
    strip=False,
    upx=False,
    upx_exclude=[],
    name=app_name,
)
