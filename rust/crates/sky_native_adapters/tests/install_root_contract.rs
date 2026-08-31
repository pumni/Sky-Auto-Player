use sky_native_adapters::{FileCatalogSource, JsonSettingsStore};
use std::path::PathBuf;

#[test]
fn packaged_adapters_use_the_explicit_install_root_contract() {
    let install_root = PathBuf::from(r"C:\Program Files\Sky Auto Player");
    let settings = JsonSettingsStore::new(install_root.join("config.json"));
    let catalog = FileCatalogSource::new(install_root.join("songs"));

    assert_eq!(settings.path(), install_root.join("config.json").as_path());
    assert_eq!(catalog.root(), install_root.join("songs").as_path());
}
