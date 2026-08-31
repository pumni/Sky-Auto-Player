fn main() {
    if std::env::var_os("CARGO_FEATURE_TAURI_TEST").is_some() {
        // The compile-only all-features/mock-runtime validation does not run
        // the frontend. Override only that rustc invocation's generated
        // context so it can use noop assets; production desktop-runtime
        // builds still read tauri.conf.json and require frontendDist through
        // tauri/custom-protocol.
        println!("cargo:rustc-env=TAURI_CONFIG={{\"build\":{{\"frontendDist\":null}}}}");
    }
    tauri_build::build();
}
