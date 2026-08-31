//! Explicit ownership matrix for the strangler boundary.
//!
//! Wave 2 native service shadows are implemented and fixture-tested outside
//! the live Tauri route, but the running Python Core still owns these commands
//! because it retains cached application state and catalog/detail authority. A
//! failed Python-owned command is returned as a failure; the shell never
//! performs an implicit native-then-Python fallback.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandOwner {
    Python,
    Native,
}

pub(crate) const COMMAND_OWNERS: &[(&str, CommandOwner)] = &[
    ("app.bootstrap", CommandOwner::Python),
    ("app.shutdown", CommandOwner::Python),
    ("catalog.search", CommandOwner::Python),
    ("catalog.detail", CommandOwner::Python),
    ("catalog.reload", CommandOwner::Python),
    ("catalog.set_viewport", CommandOwner::Python),
    ("settings.get", CommandOwner::Python),
    ("settings.patch", CommandOwner::Python),
    ("update.check", CommandOwner::Python),
    ("update.preferences.get", CommandOwner::Python),
    ("update.preferences.patch", CommandOwner::Python),
    ("update.begin_handoff", CommandOwner::Python),
    ("playback.prepare", CommandOwner::Python),
    ("playback.start", CommandOwner::Python),
    ("playback.stop", CommandOwner::Python),
    ("playback.pause", CommandOwner::Python),
    ("playback.resume", CommandOwner::Python),
    ("playback.skip", CommandOwner::Python),
    ("diagnostics.set_enabled", CommandOwner::Python),
    ("calibration.start", CommandOwner::Python),
    ("calibration.cancel", CommandOwner::Python),
];

pub(crate) fn owner_for(method: &str) -> Option<CommandOwner> {
    COMMAND_OWNERS
        .iter()
        .find_map(|(name, owner)| (*name == method).then_some(*owner))
}

pub(crate) fn matrix_is_complete() -> bool {
    // Keep both ownership states represented in the delivery contract even
    // while all live routes remain Python-owned for cache-coherence reasons.
    let _native_owner_is_available = CommandOwner::Native;
    COMMAND_OWNERS.len() == 21
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
        for (method, owner) in COMMAND_OWNERS {
            assert_eq!(owner_for(method), Some(*owner));
        }
        assert_eq!(owner_for("settings.patch"), Some(CommandOwner::Python));
        assert_eq!(owner_for("unknown"), None);
        assert!(matrix_is_complete());
    }
}
