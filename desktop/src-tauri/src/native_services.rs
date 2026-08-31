//! Composition-root registration for Wave 2 native application adapters.
//!
//! The adapters are intentionally registered before command cutover. Current
//! commands remain Python-owned while Core's cached settings/catalog state is
//! canonical; a later cutover must consume these adapters only after a
//! coherence proof and parity gate.

use crate::event_mux::NativeEventMux;
use sky_app_core::catalog::CatalogIndex;
use sky_app_core::settings::SettingsService;
use sky_native_adapters::{FileCatalogSource, JsonSettingsStore};
use std::path::PathBuf;
use std::sync::Mutex;

pub(crate) struct NativeServices {
    pub(crate) settings: SettingsService<JsonSettingsStore>,
    pub(crate) catalog_source: FileCatalogSource,
    pub(crate) catalog: CatalogIndex,
    pub(crate) event_mux: Mutex<NativeEventMux>,
}

impl NativeServices {
    pub(crate) fn for_current_install() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let settings_store = JsonSettingsStore::new(root.join("config.json"));
        let settings = SettingsService::load(settings_store).unwrap_or_else(|_| {
            SettingsService::load(JsonSettingsStore::new(root.join("config.json")))
                .expect("new settings store must load defaults")
        });
        let songs_dir = root.join(settings.snapshot().songs_dir.clone());
        Self {
            settings,
            catalog_source: FileCatalogSource::new(songs_dir),
            catalog: CatalogIndex::default(),
            event_mux: Mutex::new(NativeEventMux::default()),
        }
    }

    pub(crate) fn assert_composition_contract(&self) {
        let event_mux = self.event_mux.lock().expect("native event mux lock");
        let _ = (
            &self.settings,
            &self.catalog_source,
            &self.catalog,
            event_mux.buffered_len(),
            event_mux.dropped_events(),
        );
    }
}
