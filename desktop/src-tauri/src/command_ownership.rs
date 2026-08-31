//! Explicit ownership matrix for the strangler boundary.
//!
//! Explicit command ownership for the Wave 3 native strangler boundary.
//!
//! The matrix is executable policy: the selected handler is authoritative and
//! a failure is returned to the caller.  There is no implicit native/Python
//! fallback and no command may have two live owners.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandOwner {
    Python,
    Native,
}

pub(crate) const COMMAND_OWNERS: &[(&str, CommandOwner)] = &[
    ("app.bootstrap", CommandOwner::Native),
    ("app.shutdown", CommandOwner::Native),
    ("catalog.search", CommandOwner::Native),
    ("catalog.detail", CommandOwner::Native),
    ("catalog.reload", CommandOwner::Native),
    ("catalog.set_viewport", CommandOwner::Native),
    // Keep the complete settings family with Core while its process-local
    // AppConfig cache is still live. Native services only read the same
    // atomically persisted file as a shadow during this transition.
    ("settings.get", CommandOwner::Python),
    ("settings.patch", CommandOwner::Python),
    ("update.check", CommandOwner::Python),
    ("update.preferences.get", CommandOwner::Python),
    ("update.preferences.patch", CommandOwner::Python),
    ("update.begin_handoff", CommandOwner::Python),
    ("playback.prepare", CommandOwner::Native),
    ("playback.start", CommandOwner::Native),
    ("playback.stop", CommandOwner::Native),
    ("playback.pause", CommandOwner::Native),
    ("playback.resume", CommandOwner::Native),
    ("playback.skip", CommandOwner::Native),
    ("diagnostics.set_enabled", CommandOwner::Native),
    ("calibration.start", CommandOwner::Python),
    ("calibration.cancel", CommandOwner::Python),
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
    // Keep both ownership states represented in the delivery contract while
    // the remaining update-handoff and calibration routes stay Python-owned.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_current_core_method_has_exactly_one_explicit_owner() {
        assert_eq!(COMMAND_OWNERS.len(), 21);
        assert_eq!(
            COMMAND_OWNERS
                .iter()
                .filter(|(_, owner)| *owner == CommandOwner::Native)
                .count(),
            13
        );
        assert_eq!(
            COMMAND_OWNERS
                .iter()
                .filter(|(_, owner)| *owner == CommandOwner::Python)
                .count(),
            8
        );
        for (method, owner) in COMMAND_OWNERS {
            assert_eq!(owner_for(method), Some(*owner));
        }
        assert_eq!(owner_for("settings.patch"), Some(CommandOwner::Python));
        assert_eq!(
            owner_for("update.preferences.patch"),
            Some(CommandOwner::Python)
        );
        assert_eq!(owner_for("unknown"), None);
        assert!(matrix_is_complete());
    }
}
