# -*- mode: python ; coding: utf-8 -*-
"""The single free-threaded Python runtime used by the v4 portable package."""

from importlib.util import find_spec
from pathlib import Path

ROOT = Path(SPECPATH).resolve()
app_name = "Sky-Auto-Player-Core"
entry_point = str(ROOT / "src" / "core_main.py")

datas = []
binaries = []
hiddenimports = [
    "main",
    "sky_music._native_build",
    "sky_music._version",
    "sky_music.platform.win32",
    "sky_music.platform.win32.global_hotkeys",
    "sky_music.orchestration.native_admission",
    "sky_music.orchestration.engine",
    "sky_music.orchestration.calibration",
    "sky_music.orchestration.telemetry",
    "sky_music.infrastructure.background",
    "sky_music.infrastructure.desktop_ipc",
    "sky_music.infrastructure.desktop_ipc.protocol",
    "sky_music.infrastructure.desktop_ipc.server",
    "sky_music.infrastructure.hotkeys",
    "sky_music.infrastructure.doctor",
    "sky_music.infrastructure.focus",
    "sky_music.infrastructure.realtime",
    "sky_music.infrastructure.timing",
    "sky_music.cli.calibration_command",
    "sky_music.cli.console_playback",
    "sky_music.cli.doctor_command",
    "sky_music.ui.hud",
    "sky_music.ui.picker",
    "sky_music.ui.picker_helpers",
    "sky_music.ui.textual_app",
    "sky_music.ui.textual_app.styles",
    "textual.drivers.windows_driver",
    "rich.markdown",
]

stylesheet = ROOT / "src" / "sky_music" / "ui" / "textual_app" / "styles" / "base.tcss"
if not stylesheet.is_file():
    raise RuntimeError(f"required Textual stylesheet is missing: {stylesheet}")
datas.append((str(stylesheet), "sky_music/ui/textual_app/styles"))

native_module = find_spec("sky_player_rs.sky_player_rs")
if native_module is None or native_module.origin is None:
    raise RuntimeError(
        "sky_player_rs is required; run scripts/build_rust_wheel.py before packaging"
    )
binaries.append((native_module.origin, "sky_player_rs"))
hiddenimports.extend(["sky_player_rs", "sky_player_rs.sky_player_rs"])

block_cipher = None

a = Analysis(
    [entry_point],
    pathex=[str(ROOT / "src")],
    binaries=binaries,
    datas=datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
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
    version=str(ROOT / "windows_version_info.txt"),
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
