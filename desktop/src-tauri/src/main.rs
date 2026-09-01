#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if std::env::var_os("SKY_DESKTOP_RESTART_SELFTEST").is_some() {
        std::process::exit(sky_desktop_shell_lib::selftest_packaged_shell());
    }
    if args.iter().any(|arg| arg == "--selftest-build-info") {
        std::process::exit(sky_desktop_shell_lib::selftest_build_info());
    }
    if args.iter().any(|arg| arg == "--selftest-desktop-parent") {
        // This is used only by the exact-package updater harness. The
        // process is still the real packaged Tauri executable, so the native
        // updater verifies the canonical parent image rather than a helper.
        std::thread::sleep(std::time::Duration::from_secs(120));
        return;
    }
    if args.iter().any(|arg| arg == "--selftest-desktop-shell") {
        std::process::exit(sky_desktop_shell_lib::selftest_packaged_shell());
    }
    if args.iter().any(|arg| arg == "--selftest-desktop-gui") {
        sky_desktop_shell_lib::run_gui_smoke();
        return;
    }
    sky_desktop_shell_lib::run();
}
