# -*- mode: python ; coding: utf-8 -*-
from PyInstaller.utils.hooks import collect_all
from pathlib import Path

ROOT = Path(SPECPATH).resolve()

# --- Configuration ---
package_name = 'sky_music'
app_name = 'Sky-Player'
entry_point = str(ROOT / 'src' / 'main.py')

# We don't put songs/README in datas here to keep them in the ROOT of dist, not hidden in _internal
datas = []
binaries = []
hiddenimports = [
    "sky_music.platform.win32",
    "sky_music.platform.win32.inputs",
    "sky_music.orchestration.engine",
    "sky_music.orchestration.runtime_dispatch",
    "sky_music.orchestration.calibration",
    "sky_music.orchestration.telemetry",
    "sky_music.infrastructure.backend",
    "sky_music.infrastructure.background",
    "sky_music.infrastructure.hotkeys",
    "sky_music.infrastructure.doctor",
    "sky_music.infrastructure.focus",
    "sky_music.infrastructure.realtime",
    "sky_music.infrastructure.timing",
]

# Collect all from main package and key dependencies
tmp_ret = collect_all(package_name)
datas += tmp_ret[0]
binaries += tmp_ret[1]
hiddenimports += tmp_ret[2]

tmp_ret = collect_all('textual')
datas += tmp_ret[0]
binaries += tmp_ret[1]
hiddenimports += tmp_ret[2]

tmp_ret = collect_all('rich')
datas += tmp_ret[0]
binaries += tmp_ret[1]
hiddenimports += tmp_ret[2]

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
    excludes=[],
    win_no_prefer_redirects=False,
    win_private_assemblies=False,
    cipher=block_cipher,
    noarchive=False,
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