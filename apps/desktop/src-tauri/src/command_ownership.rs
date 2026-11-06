//! Explicit ownership matrix for native desktop command handlers.
//!
//! The matrix is executable policy: the selected handler is authoritative and
//! a failure is returned to the caller. There is no implicit fallback, and no
//! command may have two live owners.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandOwner {
    Native,
}

pub(crate) fn owner_for(method: &str) -> Option<CommandOwner> {
    crate::ipc_contract::COMMANDS
        .iter()
        .find(|command| command.method == method)
        .map(|_| CommandOwner::Native)
}

pub(crate) fn matrix_is_complete() -> bool {
    crate::ipc_contract::is_complete()
        && crate::ipc_contract::COMMANDS.len() == 32
        && crate::ipc_contract::COMMANDS
            .iter()
            .all(|command| owner_for(command.method) == Some(CommandOwner::Native))
        && owner_for(crate::ipc_contract::UI_EVENTS_COMMAND).is_none()
        && owner_for(crate::ipc_contract::STARTUP_WINDOW_REVEALED_COMMAND).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_current_desktop_method_has_exactly_one_native_owner() {
        assert_eq!(crate::ipc_contract::COMMANDS.len(), 32);
        for command in crate::ipc_contract::COMMANDS {
            assert_eq!(owner_for(command.method), Some(CommandOwner::Native));
            assert!(
                include_str!("native_runtime.rs").contains(&format!("\"{}\"", command.method)),
                "native ownership has no dispatch branch for {}",
                command.method
            );
        }
        assert_eq!(owner_for("settings.patch"), Some(CommandOwner::Native));
        assert_eq!(
            owner_for("update.preferences.patch"),
            Some(CommandOwner::Native)
        );
        assert_eq!(owner_for("unknown"), None);
        assert!(matrix_is_complete());
    }

    #[test]
    fn tauri_capability_and_handler_are_explicitly_bound_to_the_contract() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("Tauri config JSON");
        assert_eq!(
            config["app"]["security"]["capabilities"],
            serde_json::json!(["main-capability"])
        );

        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/main.json"))
                .expect("main capability JSON");
        assert_eq!(capability["identifier"], "main-capability");
        assert_eq!(capability["windows"], serde_json::json!(["main"]));
        assert_eq!(capability["platforms"], serde_json::json!(["windows"]));

        let permissions = capability["permissions"]
            .as_array()
            .expect("capability permissions");
        for permission in ["core:event:allow-listen", "core:event:allow-unlisten"] {
            assert!(
                permissions.contains(&serde_json::json!(permission)),
                "main capability must explicitly grant {permission}"
            );
        }
        let expected_command_permissions = crate::ipc_contract::TAURI_COMMANDS
            .iter()
            .map(|command| format!("allow-{}", command.replace('_', "-")))
            .collect::<Vec<_>>();
        let actual_command_permissions = permissions
            .iter()
            .filter_map(serde_json::Value::as_str)
            .filter(|permission| permission.starts_with("allow-"))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(actual_command_permissions, expected_command_permissions);
        assert!(permissions.iter().all(|permission| {
            permission
                .as_str()
                .is_some_and(|permission| !permission.contains('*'))
        }));

        let generated_command_names = include_str!("../../src/bridge/generated/command_names.ts");
        for command in crate::ipc_contract::COMMANDS {
            assert!(
                generated_command_names.contains(&format!("'{}'", command.invoke_name)),
                "generated command names missing {}",
                command.invoke_name
            );
        }
        assert!(
            generated_command_names
                .contains(&format!("'{}'", crate::ipc_contract::UI_EVENTS_COMMAND))
        );
        assert!(generated_command_names.contains(&format!(
            "export const STARTUP_WINDOW_REVEALED_COMMAND = '{}' as const",
            crate::ipc_contract::STARTUP_WINDOW_REVEALED_COMMAND
        )));

        let handler_source = include_str!("lib.rs");
        let handler_block = handler_source
            .split_once(".invoke_handler(tauri::generate_handler![")
            .and_then(|(_, source)| source.split_once("])"))
            .map(|(source, _)| source)
            .expect("production Tauri invoke handler");
        let handler_names = handler_block
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("commands::")
                    .and_then(|name| name.strip_suffix(','))
            })
            .collect::<std::collections::BTreeSet<_>>();
        let contract_names = crate::ipc_contract::TAURI_COMMANDS
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(handler_names, contract_names);
        assert_eq!(
            handler_names.len(),
            crate::ipc_contract::TAURI_COMMANDS.len()
        );
    }
}
