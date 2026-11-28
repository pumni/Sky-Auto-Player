use crate::app_state::AppState;
use crate::ui_events::UiEvent;
#[cfg(test)]
use crate::ui_events::UpdateChannel;
pub use crate::ui_events::{UpdateCheckAckDto, UpdateCheckRequest, UpdateInstallAckDto};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;

mod dto;
pub use dto::*;

async fn blocking_request<P, R>(
    state: State<'_, AppState>,
    method: &'static str,
    params: P,
) -> Result<R, String>
where
    P: Serialize + Send + 'static,
    R: DeserializeOwned + Send + 'static,
{
    let app_state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _coherence_guard = (method == "playback.start").then(|| app_state.lock_coherence());
        if crate::command_ownership::owner_for(method)
            != Some(crate::command_ownership::CommandOwner::Native)
        {
            return Err(format!("unowned non-Native desktop command: {method}"));
        }
        let runtime = app_state.ensure_native_blocking()?;
        let params = serde_json::to_value(params).map_err(|error| error.to_string())?;
        let value = runtime.dispatch(method, params)?;
        serde_json::from_value(value)
            .map_err(|error| format!("invalid native {method} response: {error}"))
    })
    .await
    .map_err(|error| format!("Native command worker failed: {error}"))?
}

async fn blocking_settings_request<P, R>(
    state: State<'_, AppState>,
    method: &'static str,
    params: P,
) -> Result<R, String>
where
    P: Serialize + Send + 'static,
    R: DeserializeOwned + Send + 'static,
{
    let app_state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Tauri may run multiple async commands concurrently. Keep the
        // persistence boundary serialized/exclusive even if callers bypass the
        // frontend queue. The frontend Promise tail owns user-intent order;
        // this guard only prevents overlapping persistence operations.
        let _write_guard = app_state.lock_settings_writes();
        let _coherence_guard = (method == "settings.patch").then(|| app_state.lock_coherence());
        if crate::command_ownership::owner_for(method)
            != Some(crate::command_ownership::CommandOwner::Native)
        {
            return Err(format!("unowned non-Native desktop command: {method}"));
        }
        let runtime = app_state.ensure_native_blocking()?;
        let params = serde_json::to_value(params).map_err(|error| error.to_string())?;
        let value = runtime.dispatch(method, params)?;
        serde_json::from_value(value)
            .map_err(|error| format!("invalid native {method} response: {error}"))
    })
    .await
    .map_err(|error| format!("Native settings worker failed: {error}"))?
}

#[tauri::command]
pub async fn bootstrap(state: State<'_, AppState>) -> Result<BootstrapDto, String> {
    let app_state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let runtime = app_state.ensure_native_blocking()?;
        let value = runtime.dispatch("app.bootstrap", serde_json::json!({}))?;
        let mut result: BootstrapDto = serde_json::from_value(value)
            .map_err(|error| format!("invalid native app.bootstrap response: {error}"))?;
        runtime.start_catalog_load()?;
        let (catalog_state, catalog_generation) = runtime.catalog_readiness()?;
        result.catalog_state = catalog_state;
        result.catalog_generation = catalog_generation;
        result.gui_smoke = app_state.should_exit_after_close();
        Ok(result)
    })
    .await
    .map_err(|error| format!("Native bootstrap worker failed: {error}"))?
}

#[tauri::command]
pub async fn search_songs(
    state: State<'_, AppState>,
    params: CatalogSearchRequest,
) -> Result<CatalogSearchDto, String> {
    blocking_request(state, "catalog.search", params).await
}

#[tauri::command]
pub async fn get_song_detail(
    state: State<'_, AppState>,
    params: CatalogDetailRequest,
) -> Result<SongDetailDto, String> {
    blocking_request(state, "catalog.detail", params).await
}

#[tauri::command]
pub async fn reload_library(state: State<'_, AppState>) -> Result<CatalogReloadDto, String> {
    blocking_request(state, "catalog.reload", serde_json::json!({})).await
}

#[tauri::command]
pub async fn set_library_viewport(
    state: State<'_, AppState>,
    params: CatalogViewportRequest,
) -> Result<CatalogViewportDto, String> {
    blocking_request(state, "catalog.set_viewport", params).await
}

#[tauri::command]
pub async fn set_song_liked(
    state: State<'_, AppState>,
    params: CatalogSetLikedRequest,
) -> Result<CatalogSetLikedDto, String> {
    blocking_request(state, "catalog.set_liked", params).await
}

#[tauri::command]
pub async fn library_list_playlists(
    state: State<'_, AppState>,
) -> Result<LibraryNavigationDto, String> {
    blocking_request(state, "library.list_playlists", serde_json::json!({})).await
}

#[tauri::command]
pub async fn library_create_playlist(
    state: State<'_, AppState>,
    params: LibraryCreatePlaylistRequest,
) -> Result<LibraryPlaylistSummaryDto, String> {
    blocking_request(state, "library.create_playlist", params).await
}

#[tauri::command]
pub async fn library_rename_playlist(
    state: State<'_, AppState>,
    params: LibraryRenamePlaylistRequest,
) -> Result<LibraryPlaylistSummaryDto, String> {
    blocking_request(state, "library.rename_playlist", params).await
}

#[tauri::command]
pub async fn library_delete_playlist(
    state: State<'_, AppState>,
    params: LibraryPlaylistIdRequest,
) -> Result<bool, String> {
    blocking_request(state, "library.delete_playlist", params).await
}

#[tauri::command]
pub async fn library_add_songs(
    state: State<'_, AppState>,
    params: LibraryPlaylistSongsRequest,
) -> Result<LibraryPlaylistSummaryDto, String> {
    blocking_request(state, "library.add_songs", params).await
}

#[tauri::command]
pub async fn library_remove_songs(
    state: State<'_, AppState>,
    params: LibraryPlaylistSongsRequest,
) -> Result<LibraryPlaylistSummaryDto, String> {
    blocking_request(state, "library.remove_songs", params).await
}

#[tauri::command]
pub async fn library_import_local_files_to_playlist(
    app: tauri::AppHandle<super::ShellRuntime>,
    state: State<'_, AppState>,
    params: LibraryPlaylistIdRequest,
) -> Result<LibraryPlaylistImportResultDto, String> {
    use tauri_plugin_dialog::DialogExt;

    let playlist_id = params.playlist_id;
    let paths = tauri::async_runtime::spawn_blocking(move || {
        let selected = app
            .dialog()
            .file()
            .add_filter("Sky sheets", &["json", "skysheet", "txt"])
            .blocking_pick_files();
        selected
            .unwrap_or_default()
            .into_iter()
            .map(|path| {
                path.into_path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .map_err(|error| format!("selected import path is invalid: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()
    })
    .await
    .map_err(|error| format!("Native dialog worker failed: {error}"))??;
    blocking_request(
        state,
        "library.import_local_files_to_playlist",
        serde_json::json!({ "playlistId": playlist_id, "paths": paths }),
    )
    .await
}

#[tauri::command]
pub async fn library_import_local_folder_to_playlist(
    app: tauri::AppHandle<super::ShellRuntime>,
    state: State<'_, AppState>,
    params: LibraryPlaylistIdRequest,
) -> Result<LibraryPlaylistImportResultDto, String> {
    use tauri_plugin_dialog::DialogExt;

    let playlist_id = params.playlist_id;
    let path = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .map(|path| {
                path.into_path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .map_err(|error| format!("selected import folder is invalid: {error}"))
            })
            .transpose()
    })
    .await
    .map_err(|error| format!("Native dialog worker failed: {error}"))??;
    blocking_request(
        state,
        "library.import_local_folder_to_playlist",
        serde_json::json!({
            "playlistId": playlist_id,
            "paths": path.into_iter().collect::<Vec<_>>(),
        }),
    )
    .await
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<SettingsDto, String> {
    blocking_request(state, "settings.get", serde_json::json!({})).await
}

#[tauri::command]
pub async fn patch_settings(
    state: State<'_, AppState>,
    params: SettingsPatch,
) -> Result<SettingsDto, String> {
    blocking_settings_request(state, "settings.patch", params).await
}

#[tauri::command]
pub async fn check_for_update(
    state: State<'_, AppState>,
    params: UpdateCheckRequest,
) -> Result<UpdateCheckAckDto, String> {
    blocking_request(state, "update.check", params).await
}

#[tauri::command]
pub async fn get_update_preferences(
    state: State<'_, AppState>,
) -> Result<UpdatePreferencesDto, String> {
    blocking_request(state, "update.preferences.get", serde_json::json!({})).await
}

#[tauri::command]
pub async fn patch_update_preferences(
    state: State<'_, AppState>,
    params: UpdatePreferencesPatch,
) -> Result<UpdatePreferencesDto, String> {
    blocking_settings_request(state, "update.preferences.patch", params).await
}

#[tauri::command]
pub async fn begin_update_handoff(
    state: State<'_, AppState>,
    params: UpdateBeginHandoffRequest,
) -> Result<UpdateInstallAckDto, String> {
    blocking_request(state, "update.begin_handoff", params).await
}

#[tauri::command]
pub async fn set_diagnostics_enabled(
    state: State<'_, AppState>,
    params: DiagnosticsSetEnabledRequest,
) -> Result<DiagnosticsEnabledDto, String> {
    blocking_request(state, "diagnostics.set_enabled", params).await
}

#[tauri::command]
pub async fn prepare_playback(
    state: State<'_, AppState>,
    params: PlaybackPrepareRequest,
) -> Result<PreparedPlaybackDto, String> {
    blocking_request(state, "playback.prepare", params).await
}

#[tauri::command]
pub async fn start_playback(
    state: State<'_, AppState>,
    params: PlaybackStartRequest,
) -> Result<PlaybackSessionDto, String> {
    blocking_request(state, "playback.start", params).await
}

#[tauri::command]
pub async fn get_playback_status(state: State<'_, AppState>) -> Result<PlaybackStatusDto, String> {
    blocking_request(state, "playback.get_status", serde_json::json!({})).await
}

async fn playback_session_command(
    state: State<'_, AppState>,
    control: PlaybackControl,
    params: PlaybackSessionCommandRequest,
) -> Result<PlaybackCommandAckDto, String> {
    blocking_request(state, control.method(), params).await
}

#[tauri::command]
pub async fn stop_playback(
    state: State<'_, AppState>,
    params: PlaybackSessionCommandRequest,
) -> Result<PlaybackCommandAckDto, String> {
    playback_session_command(state, PlaybackControl::Stop, params).await
}

#[tauri::command]
pub async fn pause_playback(
    state: State<'_, AppState>,
    params: PlaybackSessionCommandRequest,
) -> Result<PlaybackCommandAckDto, String> {
    playback_session_command(state, PlaybackControl::Pause, params).await
}

#[tauri::command]
pub async fn resume_playback(
    state: State<'_, AppState>,
    params: PlaybackSessionCommandRequest,
) -> Result<PlaybackCommandAckDto, String> {
    playback_session_command(state, PlaybackControl::Resume, params).await
}

#[tauri::command]
pub async fn skip_playback(
    state: State<'_, AppState>,
    params: PlaybackSessionCommandRequest,
) -> Result<PlaybackCommandAckDto, String> {
    playback_session_command(state, PlaybackControl::Skip, params).await
}

#[tauri::command]
pub async fn export_sender_trace(state: State<'_, AppState>) -> Result<String, String> {
    blocking_request(state, "playback.export_sender_trace", serde_json::json!({})).await
}

#[tauri::command]
pub async fn subscribe_ui_events(
    state: State<'_, AppState>,
    channel: Channel<UiEvent>,
    params: Option<StartupTelemetryRequest>,
) -> Result<(), String> {
    if let Some(params) = params {
        return crate::startup_telemetry::record_frontend_marker(
            &params.marker,
            params.frontend_elapsed_us,
        );
    }
    let _command_name = crate::ipc_contract::UI_EVENTS_COMMAND;
    crate::startup_telemetry::record("events.subscribe.start");
    let app_state = state.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || app_state.subscribe_events(channel))
        .await
        .map_err(|error| format!("Native event worker failed: {error}"))?;
    if result.is_ok() {
        crate::startup_telemetry::record("events.subscribe.end");
    }
    result
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupTelemetryRequest {
    pub marker: String,
    pub frontend_elapsed_us: u64,
}

#[tauri::command]
pub async fn shutdown(
    window: tauri::WebviewWindow<super::ShellRuntime>,
    state: State<'_, AppState>,
    params: Option<ShutdownRequest>,
) -> Result<(), String> {
    // This command is used only after an authoritative update handoff. Keep
    // shell exit under the same prevent-close -> bounded Native cleanup ->
    // destroy lifecycle as a user-initiated close; React never destroys the
    // native window directly.
    if params.is_some_and(|request| request.failed) {
        state.inner().set_gui_smoke_failed();
    }
    crate::lifecycle::close_window(window.as_ref().window());
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShutdownRequest {
    pub failed: bool,
}

#[tauri::command]
pub fn startup_window_revealed(
    window: tauri::WebviewWindow<super::ShellRuntime>,
    state: State<'_, crate::startup_window::StartupWindowFailsafe>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("startup reveal acknowledgment is only available to the main window".into());
    }

    state.inner().acknowledge_frontend_reveal(|| {
        window
            .is_visible()
            .map_err(|error| format!("could not inspect main window visibility: {error}"))
    })
}

#[cfg(test)]
#[path = "commands/tests.rs"]
mod tests;
