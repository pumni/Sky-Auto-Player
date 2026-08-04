import re
import sys
from pathlib import Path

SOFT_LIMIT = 700
HARD_LIMIT = 1000
FACADE_SOFT_LIMIT = 250
FACADES = {"engine.rs", "input.rs", "wait.rs", "lib.rs"}

ALLOWED_UNSAFE_MODULES = {
    "crates/sky_dispatch_win32/src/input/raw.rs",
    "crates/sky_dispatch_win32/src/input/physical.rs",
    "crates/sky_dispatch_win32/src/wait/timer.rs",
    # Allow current files before split:
    "crates/sky_dispatch_win32/src/input.rs",
    "crates/sky_dispatch_win32/src/wait.rs",
    # Existing platform seams that own Win32 FFI before the planned split:
    "crates/sky_dispatch_win32/src/calibration.rs",
    "crates/sky_dispatch_win32/src/clock.rs",
    "crates/sky_dispatch_win32/src/cpu.rs",
    "crates/sky_dispatch_win32/src/event.rs",
    "crates/sky_dispatch_win32/src/focus.rs",
    "crates/sky_dispatch_win32/src/mmcss.rs",
    "crates/sky_dispatch_win32/src/power.rs",
    "crates/sky_dispatch_win32/src/timer.rs",
}

def analyze_rust_file(filepath):
    with open(filepath, encoding="utf-8") as f:
        lines = f.readlines()
    
    num_lines = len(lines)
    pub_items = 0
    has_unsafe = False
    has_pyo3 = False
    imports_win32 = False
    imports_player = False
    
    pub_pattern = re.compile(r'^\s*pub\s+(fn|struct|enum|trait|const|type|mod|static|use)\s+')
    
    in_comment = False
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("/*"):
            in_comment = True
        if in_comment:
            if "*/" in stripped:
                in_comment = False
            continue
        if stripped.startswith("//"):
            continue
            
        if pub_pattern.search(line):
            pub_items += 1
            
        if re.search(r"\bunsafe\b", line):
            has_unsafe = True
            
        if "use pyo3" in line or "pyo3::" in line:
            has_pyo3 = True
            
        if "sky_dispatch_win32::" in line or "use sky_dispatch_win32" in line:
            imports_win32 = True
            
        if "sky_player_rs::" in line or "use sky_player_rs" in line:
            imports_player = True

    return {
        "num_lines": num_lines,
        "pub_items": pub_items,
        "has_unsafe": has_unsafe,
        "has_pyo3": has_pyo3,
        "imports_win32": imports_win32,
        "imports_player": imports_player
    }

def main():
    repository_root = Path(__file__).resolve().parents[1]
    workspace_root = repository_root / "rust" / "crates"
    if not workspace_root.exists():
        print(f"Error: {workspace_root} not found.")
        sys.exit(1)
        
    print("--- Rust Architecture Check ---")
    
    warnings = []
    
    for crate in ["sky_dispatch_core", "sky_dispatch_win32", "sky_player_rs"]:
        crate_path = workspace_root / crate / "src"
        if not crate_path.exists():
            continue
            
        for filepath in crate_path.rglob("*.rs"):
            rel_path = filepath.relative_to(workspace_root.parent)
            rel_path_str = str(rel_path).replace("\\", "/")
            
            stats = analyze_rust_file(filepath)
            filename = filepath.name
            
            print(f"{rel_path_str:50} | {stats['num_lines']:4} lines | {stats['pub_items']:3} pub items")
            
            if filename in FACADES:
                if stats["num_lines"] > FACADE_SOFT_LIMIT:
                    warnings.append(f"Soft limit exceeded (facade): {rel_path_str} has {stats['num_lines']} lines (> {FACADE_SOFT_LIMIT})")
            else:
                if stats["num_lines"] > HARD_LIMIT:
                    warnings.append(f"Hard limit exceeded: {rel_path_str} has {stats['num_lines']} lines (> {HARD_LIMIT})")
                elif stats["num_lines"] > SOFT_LIMIT:
                    warnings.append(f"Soft limit exceeded: {rel_path_str} has {stats['num_lines']} lines (> {SOFT_LIMIT})")
                    
            if stats["has_unsafe"] and rel_path_str not in ALLOWED_UNSAFE_MODULES:
                warnings.append(f"Unsafe code outside allowed modules: {rel_path_str}")
                
            if stats["has_pyo3"] and not (rel_path_str.startswith("crates/sky_player_rs/src/python") or rel_path_str == "crates/sky_player_rs/src/lib.rs"):
                warnings.append(f"PyO3 import outside python boundary: {rel_path_str}")
                
            if crate == "sky_dispatch_core":
                if stats["imports_win32"]:
                    warnings.append(f"Layering violation: {rel_path_str} imports sky_dispatch_win32")
                if stats["imports_player"]:
                    warnings.append(f"Layering violation: {rel_path_str} imports sky_player_rs")
            elif crate == "sky_dispatch_win32":
                if stats["imports_player"]:
                    warnings.append(f"Layering violation: {rel_path_str} imports sky_player_rs")
                    
    print("\n--- Warnings ---")
    if not warnings:
        print("None")
    else:
        for w in warnings:
            print("- " + w)
            
    print("\nNote: These are currently reporting-only and will not fail the CI.")

if __name__ == "__main__":
    main()
