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
        cmd.extend(
            [
                "--hidden-import",
                "sky_music.ui.textual_app",
                "--hidden-import",
                "sky_music.ui.textual_app.app",
                "--hidden-import",
                "sky_music.ui.textual_app.workers",
            ]
        )
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
