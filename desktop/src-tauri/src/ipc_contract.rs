//! The single source of truth for stable request/response command identifiers.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommandSpec {
    pub(crate) invoke_name: &'static str,
    pub(crate) method: &'static str,
}

pub(crate) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        invoke_name: "bootstrap",
        method: "app.bootstrap",
    },
    CommandSpec {
        invoke_name: "shutdown",
        method: "app.shutdown",
    },
    CommandSpec {
        invoke_name: "search_songs",
        method: "catalog.search",
    },
    CommandSpec {
        invoke_name: "get_song_detail",
        method: "catalog.detail",
    },
    CommandSpec {
        invoke_name: "reload_library",
        method: "catalog.reload",
    },
    CommandSpec {
        invoke_name: "set_library_viewport",
        method: "catalog.set_viewport",
    },
    CommandSpec {
        invoke_name: "get_settings",
        method: "settings.get",
    },
    CommandSpec {
        invoke_name: "patch_settings",
        method: "settings.patch",
    },
    CommandSpec {
        invoke_name: "check_for_update",
        method: "update.check",
    },
    CommandSpec {
        invoke_name: "get_update_preferences",
        method: "update.preferences.get",
    },
    CommandSpec {
        invoke_name: "patch_update_preferences",
        method: "update.preferences.patch",
    },
    CommandSpec {
        invoke_name: "begin_update_handoff",
        method: "update.begin_handoff",
    },
    CommandSpec {
        invoke_name: "prepare_playback",
        method: "playback.prepare",
    },
    CommandSpec {
        invoke_name: "start_playback",
        method: "playback.start",
    },
    CommandSpec {
        invoke_name: "stop_playback",
        method: "playback.stop",
    },
    CommandSpec {
        invoke_name: "pause_playback",
        method: "playback.pause",
    },
    CommandSpec {
        invoke_name: "resume_playback",
        method: "playback.resume",
    },
    CommandSpec {
        invoke_name: "skip_playback",
        method: "playback.skip",
    },
    CommandSpec {
        invoke_name: "set_diagnostics_enabled",
        method: "diagnostics.set_enabled",
    },
    CommandSpec {
        invoke_name: "start_calibration",
        method: "calibration.start",
    },
    CommandSpec {
        invoke_name: "cancel_calibration",
        method: "calibration.cancel",
    },
];

pub(crate) const UI_EVENTS_COMMAND: &str = "subscribe_ui_events";

pub(crate) fn is_complete() -> bool {
    COMMANDS.len() == 21
        && COMMANDS.iter().all(|spec| {
            !spec.invoke_name.is_empty()
                && !spec.method.is_empty()
                && COMMANDS
                    .iter()
                    .filter(|other| other.invoke_name == spec.invoke_name)
                    .count()
                    == 1
                && COMMANDS
                    .iter()
                    .filter(|other| other.method == spec.method)
                    .count()
                    == 1
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_command_contract_is_exactly_the_native_set() {
        assert!(is_complete());
        assert_eq!(COMMANDS.len(), 21);
        assert_eq!(
            COMMANDS[0],
            CommandSpec {
                invoke_name: "bootstrap",
                method: "app.bootstrap"
            }
        );
        assert_eq!(
            COMMANDS[20],
            CommandSpec {
                invoke_name: "cancel_calibration",
                method: "calibration.cancel"
            }
        );
        assert_eq!(UI_EVENTS_COMMAND, "subscribe_ui_events");
    }
}
