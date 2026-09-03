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
    (
        crate::ipc_contract::COMMANDS[0].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[1].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[2].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[3].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[4].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[5].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[6].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[7].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[8].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[9].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[10].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[11].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[12].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[13].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[14].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[15].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[16].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[17].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[18].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[19].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[20].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[21].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[22].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[23].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[24].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[25].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[26].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[27].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[28].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[29].method,
        CommandOwner::Native,
    ),
    (
        crate::ipc_contract::COMMANDS[30].method,
        CommandOwner::Native,
    ),
];

/// Native handlers are enumerated separately from policy so the matrix cannot
/// claim a route is native merely because a lifecycle helper happens to call
/// an internal cleanup function.
pub(crate) const NATIVE_HANDLER_METHODS: &[&str] = &[
    crate::ipc_contract::COMMANDS[0].method,
    crate::ipc_contract::COMMANDS[1].method,
    crate::ipc_contract::COMMANDS[2].method,
    crate::ipc_contract::COMMANDS[3].method,
    crate::ipc_contract::COMMANDS[4].method,
    crate::ipc_contract::COMMANDS[5].method,
    crate::ipc_contract::COMMANDS[6].method,
    crate::ipc_contract::COMMANDS[7].method,
    crate::ipc_contract::COMMANDS[8].method,
    crate::ipc_contract::COMMANDS[9].method,
    crate::ipc_contract::COMMANDS[10].method,
    crate::ipc_contract::COMMANDS[11].method,
    crate::ipc_contract::COMMANDS[12].method,
    crate::ipc_contract::COMMANDS[13].method,
    crate::ipc_contract::COMMANDS[14].method,
    crate::ipc_contract::COMMANDS[15].method,
    crate::ipc_contract::COMMANDS[16].method,
    crate::ipc_contract::COMMANDS[17].method,
    crate::ipc_contract::COMMANDS[18].method,
    crate::ipc_contract::COMMANDS[19].method,
    crate::ipc_contract::COMMANDS[20].method,
    crate::ipc_contract::COMMANDS[21].method,
    crate::ipc_contract::COMMANDS[22].method,
    crate::ipc_contract::COMMANDS[23].method,
    crate::ipc_contract::COMMANDS[24].method,
    crate::ipc_contract::COMMANDS[25].method,
    crate::ipc_contract::COMMANDS[26].method,
    crate::ipc_contract::COMMANDS[27].method,
    crate::ipc_contract::COMMANDS[28].method,
    crate::ipc_contract::COMMANDS[29].method,
    crate::ipc_contract::COMMANDS[30].method,
];

const REQUIRED_COMMANDS: [&str; 31] = [
    crate::ipc_contract::COMMANDS[0].method,
    crate::ipc_contract::COMMANDS[1].method,
    crate::ipc_contract::COMMANDS[2].method,
    crate::ipc_contract::COMMANDS[3].method,
    crate::ipc_contract::COMMANDS[4].method,
    crate::ipc_contract::COMMANDS[5].method,
    crate::ipc_contract::COMMANDS[6].method,
    crate::ipc_contract::COMMANDS[7].method,
    crate::ipc_contract::COMMANDS[8].method,
    crate::ipc_contract::COMMANDS[9].method,
    crate::ipc_contract::COMMANDS[10].method,
    crate::ipc_contract::COMMANDS[11].method,
    crate::ipc_contract::COMMANDS[12].method,
    crate::ipc_contract::COMMANDS[13].method,
    crate::ipc_contract::COMMANDS[14].method,
    crate::ipc_contract::COMMANDS[15].method,
    crate::ipc_contract::COMMANDS[16].method,
    crate::ipc_contract::COMMANDS[17].method,
    crate::ipc_contract::COMMANDS[18].method,
    crate::ipc_contract::COMMANDS[19].method,
    crate::ipc_contract::COMMANDS[20].method,
    crate::ipc_contract::COMMANDS[21].method,
    crate::ipc_contract::COMMANDS[22].method,
    crate::ipc_contract::COMMANDS[23].method,
    crate::ipc_contract::COMMANDS[24].method,
    crate::ipc_contract::COMMANDS[25].method,
    crate::ipc_contract::COMMANDS[26].method,
    crate::ipc_contract::COMMANDS[27].method,
    crate::ipc_contract::COMMANDS[28].method,
    crate::ipc_contract::COMMANDS[29].method,
    crate::ipc_contract::COMMANDS[30].method,
];

pub(crate) fn owner_for(method: &str) -> Option<CommandOwner> {
    COMMAND_OWNERS
        .iter()
        .find_map(|(name, owner)| (*name == method).then_some(*owner))
}

pub(crate) fn matrix_is_complete() -> bool {
    // This list is executable evidence that every Native policy entry has a
    // corresponding runtime dispatch branch.
    crate::ipc_contract::is_complete()
        && COMMAND_OWNERS.len() == REQUIRED_COMMANDS.len()
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
        assert_eq!(COMMAND_OWNERS.len(), 31);
        assert_eq!(
            COMMAND_OWNERS
                .iter()
                .filter(|(_, owner)| *owner == CommandOwner::Native)
                .count(),
            31
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
        assert_eq!(NATIVE_HANDLER_METHODS.len(), 31);
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
