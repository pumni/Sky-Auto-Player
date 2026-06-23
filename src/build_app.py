import argparse
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

def find_project_root(start: Path) -> Path:
    for candidate in [start, *start.parents]:
        if (candidate / "pyproject.toml").exists() and (candidate / "Sky-Player.spec").exists():
            return candidate
    raise RuntimeError("Cannot locate project root. Run build-app from the source checkout.")

PROJECT_ROOT = find_project_root(Path(__file__).resolve())

SPEC_FILE = PROJECT_ROOT / "Sky-Player.spec"
VERSION_FILE = PROJECT_ROOT / "windows_version_info.txt"
DIST_DIR = PROJECT_ROOT / "dist"
BUILD_DIR = PROJECT_ROOT / "build"

APP_NAME = "Sky-Player"
REQUIRED_ASSETS = ("config.json", "songs")
OPTIONAL_ASSETS = ("README.md",)

def get_project_version() -> str:
    path = PROJECT_ROOT / "pyproject.toml"
    try:
        with path.open("rb") as f:
            data = tomllib.load(f)
    except Exception as exc:
        raise RuntimeError(f"Cannot read {path}") from exc

    version = data.get("project", {}).get("version")
    if not isinstance(version, str) or not version.strip():
        raise RuntimeError("Missing or invalid [project].version in pyproject.toml")

    return version.strip()

def windows_version_tuple(version: str) -> tuple[int, int, int, int]:
    parts = [int(x) for x in re.findall(r"\d+", version)[:4]]
    while len(parts) < 4:
        parts.append(0)
    p0, p1, p2, p3 = (parts + [0, 0, 0, 0])[:4]
    return (p0, p1, p2, p3)

def generate_version_info(version: str) -> None:
    v_tuple = windows_version_tuple(version)
    v_str = version

    content = f"""
VSVersionInfo(
  ffi=FixedFileInfo(
    filevers={v_tuple},
    prodvers={v_tuple},
    mask=0x3f,
    flags=0x0,
    OS=0x40004,
    fileType=0x1,
    subtype=0x0,
    date=(0, 0)
  ),
  kids=[
    StringFileInfo([
      StringTable(
        '040904B0',
        [
          StringStruct('CompanyName', 'Sky Player Team'),
          StringStruct('FileDescription', 'Sky Music Player for Windows'),
          StringStruct('FileVersion', '{v_str}'),
          StringStruct('InternalName', '{APP_NAME}'),
          StringStruct('LegalCopyright', 'Copyright (c) 2026'),
          StringStruct('OriginalFilename', '{APP_NAME}.exe'),
          StringStruct('ProductName', 'Sky Player'),
          StringStruct('ProductVersion', '{v_str}')
        ]
      )
    ]),
    VarFileInfo([VarStruct('Translation', [1033, 1200])])
  ]
)
"""
    VERSION_FILE.write_text(content.strip(), encoding="utf-8")

def copy_asset(src: Path, dst: Path) -> None:
    if dst.exists():
        if dst.is_dir():
            shutil.rmtree(dst)
        else:
            dst.unlink()

    if src.is_dir():
        shutil.copytree(src, dst)
    else:
        shutil.copy2(src, dst)

def run_smoke_test(exe_path: Path) -> bool:
    exe_path = exe_path.resolve()
    if not exe_path.exists():
        print(f"[!] Missing executable: {exe_path}")
        return False

    print(f"[+] Đang chạy Smoke Test: {exe_path} --selftest-textual")
    try:
        result = subprocess.run(
            [str(exe_path), "--selftest-textual"],
            capture_output=True,
            text=True,
            timeout=30,
            cwd=str(exe_path.parent),
        )
    except Exception as exc:
        print(f"[!] Lỗi khi chạy Smoke Test: {exc}")
        return False

    if result.returncode == 0:
        print("[v] Smoke Test THÀNH CÔNG.")
        return True

    print("[!] Smoke Test THẤT BẠI!")
    print(result.stderr or result.stdout)
    return False

def kill_hanging_selftest(release_dir: Path) -> None:
    """Attempt to force kill any lingering smoke test processes from our specific release dir."""
    if not release_dir.exists():
        return

    release_root = str(release_dir.resolve()).replace("'", "''")
    ps = f"""
    Get-CimInstance Win32_Process |
      Where-Object {{
        $_.Name -eq '{APP_NAME}.exe' -and
        $_.ExecutablePath -like '{release_root}\\*'
      }} |
      ForEach-Object {{
        Stop-Process -Id $_.ProcessId -Force
      }}
    """
    try:
        subprocess.run(
            ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", ps],
            capture_output=True,
            text=True,
        )
        import time
        time.sleep(1) # Give OS time to release file handles
    except Exception:
        pass

def main() -> None:
    if sys.platform != "win32":
        raise SystemExit("[!] Lỗi: Dự án này chỉ hỗ trợ build trên Windows.")

    parser = argparse.ArgumentParser()
    parser.add_argument("--skip-test", action="store_true")
    args = parser.parse_args()

    version = get_project_version()
    print(f"=== Sky Player Build Pipeline v{version} ===")

    generate_version_info(version)

    release_dir = DIST_DIR / f"{APP_NAME}-v{version}"
    kill_hanging_selftest(release_dir)

    if BUILD_DIR.exists():
        try:
            shutil.rmtree(BUILD_DIR)
        except PermissionError as e:
            print(f"[!] Lỗi dọn dẹp BUILD_DIR: {e}. Đang thử tiếp tục...")
            
    if DIST_DIR.exists():
        try:
            shutil.rmtree(DIST_DIR)
        except PermissionError as e:
            raise SystemExit(f"[!] Lỗi dọn dẹp DIST_DIR: {e}.\nGiải pháp: Đóng mọi cửa sổ CMD/Explorer đang mở thư mục {DIST_DIR} và thử lại.")

    print("[+] Đang khởi động PyInstaller...")
    subprocess.run(
        [sys.executable, "-m", "PyInstaller", "--noconfirm", "--clean", str(SPEC_FILE)],
        check=True,
        cwd=str(PROJECT_ROOT),
    )

    raw_dist = DIST_DIR / APP_NAME

    if not raw_dist.exists():
        raise RuntimeError(f"PyInstaller output missing: {raw_dist}")

    print(f"[+] Chuyển artifact sang {release_dir.name}...")
    shutil.move(str(raw_dist), str(release_dir))

    print("[+] Sao chép assets...")
    for asset in REQUIRED_ASSETS:
        src = PROJECT_ROOT / asset
        if not src.exists():
            raise FileNotFoundError(f"Required asset missing: {src}")
        copy_asset(src, release_dir / asset)

    for asset in OPTIONAL_ASSETS:
        src = PROJECT_ROOT / asset
        if src.exists():
            copy_asset(src, release_dir / asset)

    if not args.skip_test:
        if not run_smoke_test(release_dir / f"{APP_NAME}.exe"):
            raise SystemExit(1)

    print(f"[v] HOÀN TẤT: {release_dir.resolve()}")

if __name__ == "__main__":
    main()