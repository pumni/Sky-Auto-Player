#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--selftest-desktop-shell") {
        std::process::exit(sky_desktop_shell_lib::selftest_packaged_shell());
    }
    if args.iter().any(|arg| arg == "--selftest-desktop-gui") {
        sky_desktop_shell_lib::run_gui_smoke();
        return;
    }
    sky_desktop_shell_lib::run();
}
