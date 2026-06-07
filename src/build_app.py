import argparse
import subprocess
import shutil
import sys
from pathlib import Path

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
    return parser

def main() -> None:
    if sys.platform == 'win32':
        try:
            sys.stdout.reconfigure(encoding='utf-8')
            sys.stderr.reconfigure(encoding='utf-8')
        except Exception:
            pass

    args = build_arg_parser().parse_args()
    entrypoint = "src/sky_music/ui/textual_app/app.py" if args.textual_proof else "src/main.py"

    print("[+] Đang dọn dẹp các thư mục build cũ...")
    for folder in ["build", "dist"]:
        path = Path(folder)
        if path.exists():
            shutil.rmtree(path)
            
    print("[+] Đang chạy PyInstaller để đóng gói ứng dụng...")
    cmd = [
        "pyinstaller",
        "--noconfirm",
        "--onedir",
        "--console",
        "--name", "Sky-Player",
        "--paths", "./src",
    ]
    if args.collect_textual or not args.textual_proof:
        cmd.extend(["--collect-all", "textual", "--collect-all", "rich"])

    # Hidden imports: modules that PyInstaller cannot auto-detect because they are
    # loaded via lazy/conditional imports (inside if-blocks or function bodies).
    hidden_imports: list[str] = [
        # UI – textual app
        "sky_music.ui.textual_app",
        "sky_music.ui.textual_app.app",
        "sky_music.ui.textual_app.workers",
        # UI – classic prompt-toolkit picker and sub-modules
        "sky_music.ui.picker",
        "sky_music.ui.picker_helpers",
        "sky_music.ui.picker_metadata",
        "sky_music.ui.picker_theme",
        "sky_music.ui.hud",
        "sky_music.ui.text_render",
        # Platform – Win32 inputs (lazy-imported in backend, focus, realtime, engine, picker)
        "sky_music.platform.win32",
        "sky_music.platform.win32.inputs",
        # Orchestration – calibration and telemetry are lazy-imported
        "sky_music.orchestration.engine",
        "sky_music.orchestration.runtime_dispatch",
        "sky_music.orchestration.calibration",
        "sky_music.orchestration.telemetry",
        # Infrastructure – background, hotkeys, doctor use lazy imports or are indirect
        "sky_music.infrastructure.backend",
        "sky_music.infrastructure.background",
        "sky_music.infrastructure.hotkeys",
        "sky_music.infrastructure.doctor",
        "sky_music.infrastructure.focus",
        "sky_music.infrastructure.realtime",
        "sky_music.infrastructure.timing",
        # Domain
        "sky_music.domain.domain",
        "sky_music.domain.analyzer",
        "sky_music.domain.parser",
        "sky_music.domain.scheduler",
        "sky_music.domain.scheduler_types",
        "sky_music.domain.session_context",
        "sky_music.domain.song_repository",
        "sky_music.domain.validation",
        # Config and layouts
        "sky_music.config",
        "sky_music.layouts",
    ]
    for imp in hidden_imports:
        cmd.extend(["--hidden-import", imp])
    cmd.append(entrypoint)
    subprocess.run(cmd, check=True)
    
    print("[+] Đang sao chép thư mục bài hát (songs) và tài liệu hướng dẫn...")
    dist_dir = Path("dist/Sky-Player")
    
    songs_dir = Path("songs")
    if songs_dir.exists():
        shutil.copytree(songs_dir, dist_dir / "songs", dirs_exist_ok=True)
        
    readme_file = Path("README.md")
    if readme_file.exists():
        shutil.copy2(readme_file, dist_dir / "README.md")
        
    print("\n===================================================")
    print("[v] THÀNH CÔNG: Đã đóng gói xong ứng dụng!")
    print(f"Thư mục ứng dụng hoàn chỉnh nằm tại:\n  {dist_dir.resolve()}")
    print("===================================================\n")

if __name__ == "__main__":
    main()
