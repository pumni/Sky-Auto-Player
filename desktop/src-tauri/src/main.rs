#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--selftest-desktop-shell")
    {
        std::process::exit(sky_desktop_shell_lib::selftest_packaged_shell());
    }
    sky_desktop_shell_lib::run();
}
