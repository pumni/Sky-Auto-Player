//! Explicit ownership matrix for the strangler boundary.
//!
//! Explicit command ownership for the final Native desktop boundary.
//!
//! The matrix is executable policy: the selected handler is authoritative and
//! a failure is returned to the caller.  There is no implicit native/Python
//! fallback and no command may have two live owners.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandOwner {
    Native,
}

pub(crate) const COMMAND_OWNERS: &[(&str, CommandOwner)] = &[
    ("app.bootstrap", CommandOwner::Native),
    ("app.shutdown", CommandOwner::Native),
    ("catalog.search", CommandOwner::Native),
    ("catalog.detail", CommandOwner::Native),
    ("catalog.reload", CommandOwner::Native),
    ("catalog.set_viewport", CommandOwner::Native),
    ("settings.get", CommandOwner::Native),
    ("settings.patch", CommandOwner::Native),
    ("update.check", CommandOwner::Native),
    ("update.preferences.get", CommandOwner::Native),
    ("update.preferences.patch", CommandOwner::Native),
    ("update.begin_handoff", CommandOwner::Native),
    ("playback.prepare", CommandOwner::Native),
    ("playback.start", CommandOwner::Native),
    ("playback.stop", CommandOwner::Native),
    ("playback.pause", CommandOwner::Native),
    ("playback.resume", CommandOwner::Native),
    ("playback.skip", CommandOwner::Native),
    ("diagnostics.set_enabled", CommandOwner::Native),
    ("calibration.start", CommandOwner::Native),
    ("calibration.cancel", CommandOwner::Native),
];

/// Native handlers are enumerated separately from policy so the matrix cannot
/// claim a route is native merely because a lifecycle helper happens to call
/// an internal cleanup function.
pub(crate) const NATIVE_HANDLER_METHODS: &[&str] = &[
    "app.bootstrap",
    "app.shutdown",
    "catalog.search",
    "catalog.detail",
    "catalog.reload",
    "catalog.set_viewport",
    "settings.get",
    "settings.patch",
    "update.check",
    "update.preferences.get",
    "update.preferences.patch",
    "update.begin_handoff",
    "playback.prepare",
    "playback.start",
    "playback.stop",
    "playback.pause",
    "playback.resume",
    "playback.skip",
    "diagnostics.set_enabled",
    "calibration.start",
    "calibration.cancel",
];

const REQUIRED_COMMANDS: [&str; 21] = [
    "app.bootstrap",
    "app.shutdown",
    "catalog.search",
    "catalog.detail",
    "catalog.reload",
    "catalog.set_viewport",
    "settings.get",
    "settings.patch",
    "update.check",
    "update.preferences.get",
    "update.preferences.patch",
    "update.begin_handoff",
    "playback.prepare",
    "playback.start",
    "playback.stop",
    "playback.pause",
    "playback.resume",
    "playback.skip",
    "diagnostics.set_enabled",
    "calibration.start",
    "calibration.cancel",
];

pub(crate) fn owner_for(method: &str) -> Option<CommandOwner> {
    COMMAND_OWNERS
        .iter()
        .find_map(|(name, owner)| (*name == method).then_some(*owner))
}

pub(crate) fn matrix_is_complete() -> bool {
    // This list is executable evidence that every Native policy entry has a
    // corresponding runtime dispatch branch.
    COMMAND_OWNERS.len() == REQUIRED_COMMANDS.len()
        && REQUIRED_COMMANDS.iter().all(|method| {
            COMMAND_OWNERS
                .iter()
                .filter(|(name, _)| name == method)
                .count()
                == 1
        })
        && COMMAND_OWNERS
            .iter()
            .all(|(method, owner)| owner_for(method) == Some(*owner))
        && NATIVE_HANDLER_METHODS.len() == COMMAND_OWNERS.len()
        && NATIVE_HANDLER_METHODS
            .iter()
            .all(|method| owner_for(method) == Some(CommandOwner::Native))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_current_desktop_method_has_exactly_one_native_owner() {
        assert_eq!(COMMAND_OWNERS.len(), 21);
        assert_eq!(
            COMMAND_OWNERS
                .iter()
                .filter(|(_, owner)| *owner == CommandOwner::Native)
                .count(),
            21
        );
        for (method, owner) in COMMAND_OWNERS {
            assert_eq!(owner_for(method), Some(*owner));
        }
        assert_eq!(owner_for("settings.patch"), Some(CommandOwner::Native));
        assert_eq!(
            owner_for("update.preferences.patch"),
            Some(CommandOwner::Native)
        );
        assert_eq!(owner_for("unknown"), None);
        assert!(matrix_is_complete());
        assert_eq!(NATIVE_HANDLER_METHODS.len(), 21);
        for method in NATIVE_HANDLER_METHODS {
            assert_eq!(owner_for(method), Some(CommandOwner::Native));
            assert!(
                include_str!("native_runtime.rs").contains(&format!("\"{method}\"")),
                "native ownership has no dispatch branch for {method}"
            );
        }
        assert_eq!(
            COMMAND_OWNERS
                .iter()
                .filter(|(_, owner)| *owner == CommandOwner::Native)
                .map(|(method, _)| *method)
                .collect::<Vec<_>>(),
            NATIVE_HANDLER_METHODS
        );
    }
}
