import argparse
import subprocess
import shutil
import sys
import tomllib
from pathlib import Path

def get_project_version() -> str:
    """Read the version from pyproject.toml."""
    try:
        # Use UTF-8 for reading pyproject.toml just in case
        with open("pyproject.toml", "rb") as f:
            data = tomllib.load(f)
            return data.get("project", {}).get("version", "unknown")
    except Exception:
        return "unknown"

def build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Build the Sky Player executable.")
    parser.add_argument(
        "--textual-proof",
        action="store_true",
        help="build the Phase 0 Textual proof app instead of the playback CLI",
    )
    parser.add_argument(
        "--collect-textual",
        action="store_true",
        help="include PyInstaller collect-all flags for textual and rich (default for release builds)",
    )
    parser.add_argument(
        "--no-version-name",
        action="store_true",
        help="do not include the version in the output folder name (defaults to Sky-Player)",
    )
    return parser

def main() -> None:
    if sys.platform == 'win32':
        try:
            sys.stdout.reconfigure(encoding='utf-8')
            sys.stderr.reconfigure(encoding='utf-8')
        except Exception:
            pass

    args = build_arg_parser().parse_args()
    version = get_project_version()
    entrypoint = "src/sky_music/ui/textual_app/app.py" if args.textual_proof else "src/main.py"
    
    # Best practice: versioned output folder for distribution, but stable EXE name inside is usually preferred.
    # However, PyInstaller's --name controls both. We'll use versioned name for the folder by default.
    app_name = f"Sky-Player-{version}" if (version != "unknown" and not args.no_version_name) else "Sky-Player"

    print(f"[+] Bắt đầu đóng gói Sky Player v{version}...")
    
    # Check if pyinstaller is available
    try:
        import PyInstaller
    except ImportError:
        print("[!] LỖI: Không tìm thấy PyInstaller trong môi trường hiện tại.")
        print("    Vui lòng cài đặt bằng: uv add --dev pyinstaller")
        sys.exit(1)

    print("[+] Đang dọn dẹp các thư mục build cũ...")
    for folder in ["build", "dist"]:
        path = Path(folder)
        if path.exists():
            shutil.rmtree(path)
            
    print("[+] Đang chạy PyInstaller để đóng gói ứng dụng...")
    # Khởi tạo lệnh PyInstaller sử dụng sys.executable để đảm bảo dùng đúng venv
    cmd = [
        sys.executable, "-m", "PyInstaller",
        "--noconfirm",
        "--onedir",
        "--console",
        "--name", app_name,
        "--paths", "./src",
        # Tự động gom toàn bộ logic nội bộ của dự án
        "--collect-all", "sky_music",
        # Đảm bảo metadata (version) có sẵn cho importlib.metadata
        "--collect-metadata", "sky-player",
    ]
    
    # Luôn collect textual và rich nếu không phải proof build, hoặc nếu được yêu cầu
    if args.collect_textual or not args.textual_proof:
        cmd.extend(["--collect-all", "textual", "--collect-all", "rich"])

    # Hidden imports dự phòng cho các module lazy-load/dynamic load
    # Mặc dù --collect-all sky_music đã lấy hết file, nhưng hidden-import giúp 
    # PyInstaller hiểu mối liên kết giữa các module khi dùng import động.
    hidden_imports: list[str] = [
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
    for imp in hidden_imports:
        cmd.extend(["--hidden-import", imp])
        
    cmd.append(entrypoint)
    
    print(f"[+] Thực thi: {' '.join(cmd)}")
    subprocess.run(cmd, check=True)
    
    dist_dir = Path("dist") / app_name
    print(f"[+] Đang sao chép tài nguyên vào {dist_dir}...")
    
    songs_dir = Path("songs")
    if songs_dir.exists():
        shutil.copytree(songs_dir, dist_dir / "songs", dirs_exist_ok=True)
        
    readme_file = Path("README.md")
    if readme_file.exists():
        shutil.copy2(readme_file, dist_dir / "README.md")
        
    print("\n===================================================")
    print(f"[v] THÀNH CÔNG: Đã đóng gói xong Sky Player v{version}!")
    print(f"Thư mục ứng dụng: {dist_dir.resolve()}")
    print(f"File thực thi: {app_name}.exe")
    print("===================================================\n")

if __name__ == "__main__":
    main()
