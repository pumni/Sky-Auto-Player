//! Exact HKCU tree snapshots used by the isolated Windows installer harness.
//!
//! Callers can only scope the known product/uninstall roots or a uniquely
//! named test subtree. Registry mutation remains inside the Win32 adapter.

use crate::NativeBoundaryError;
use std::collections::{BTreeMap, BTreeSet};

const PRODUCT_KEYS: &[&str] = &[
    "Software\\github\\Sky Auto Player",
    "Software\\pumni\\Sky Auto Player",
    "Software\\Sky Auto Player Team\\Sky Auto Player",
    "Software\\Sky Auto Player",
    "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Sky Auto Player",
    "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\io.github.pumni.skyautoplayer",
];
const TEST_PREFIX: &str = "Software\\SkyAutoPlayerNsisTest-";

const MAX_SNAPSHOT_KEYS: usize = 16_384;
const MAX_SNAPSHOT_VALUES: usize = 65_536;
const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryValue {
    /// Raw Win32 registry value type, preserving unknown types without coercion.
    pub kind: u32,
    /// Exact unexpanded registry bytes, including terminators where present.
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegistryNode {
    pub values: BTreeMap<String, RegistryValue>,
    pub subkeys: BTreeMap<String, RegistryNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySpec {
    key_path: String,
    parent_path: String,
}

impl KeySpec {
    pub fn new(
        key_path: impl Into<String>,
        parent_path: impl Into<String>,
    ) -> Result<Self, NativeBoundaryError> {
        let key_path = key_path.into();
        let parent_path = parent_path.into();
        validate_path(&key_path)?;
        validate_path(&parent_path)?;
        if key_path.rsplit_once('\\').map(|(parent, _)| parent) != Some(parent_path.as_str()) {
            return Err(NativeBoundaryError::new(
                "registry scope parent must be the direct parent of its monitored key",
            ));
        }
        let test_scope = key_path
            .get(..TEST_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(TEST_PREFIX));
        if !PRODUCT_KEYS.contains(&key_path.as_str()) && !test_scope {
            return Err(NativeBoundaryError::new(
                "registry scope is outside the product and isolated test allowlists",
            ));
        }
        Ok(Self {
            key_path,
            parent_path,
        })
    }

    pub fn key_path(&self) -> &str {
        &self.key_path
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct KeySnapshot {
    spec: KeySpec,
    parent_existed: bool,
    node: Option<RegistryNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrySnapshot {
    keys: Vec<KeySnapshot>,
}

pub fn product_scopes() -> Vec<KeySpec> {
    PRODUCT_KEYS
        .iter()
        .map(|key| {
            let (parent, _) = key
                .rsplit_once('\\')
                .expect("product registry key has a parent");
            KeySpec::new(*key, parent).expect("product registry key is allowlisted")
        })
        .collect()
}

pub fn capture(scopes: &[KeySpec]) -> Result<RegistrySnapshot, NativeBoundaryError> {
    let mut seen = BTreeSet::new();
    let mut keys = Vec::with_capacity(scopes.len());
    let mut budget = SnapshotBudget::default();
    for spec in scopes {
        let validated = KeySpec::new(spec.key_path.clone(), spec.parent_path.clone())?;
        let folded = validated.key_path.to_ascii_lowercase();
        if !seen.insert(folded) {
            return Err(NativeBoundaryError::new(
                "registry scope contains a duplicate monitored key",
            ));
        }
        let parent_existed = crate::win32::registry_key_exists(&validated.parent_path)?;
        let node = crate::win32::registry_read_tree(&validated.key_path, &mut budget)?;
        keys.push(KeySnapshot {
            spec: validated,
            parent_existed,
            node,
        });
    }
    Ok(RegistrySnapshot { keys })
}

/// Verifies that every explicitly scoped key is absent from HKCU.
pub fn assert_roots_absent(scopes: &[KeySpec]) -> Result<(), NativeBoundaryError> {
    let mut seen = BTreeSet::new();
    let mut budget = SnapshotBudget::default();
    for spec in scopes {
        let validated = KeySpec::new(spec.key_path.clone(), spec.parent_path.clone())?;
        if !seen.insert(validated.key_path.to_ascii_lowercase()) {
            return Err(NativeBoundaryError::new(
                "registry absence scope contains a duplicate monitored key",
            ));
        }
        if crate::win32::registry_read_tree(&validated.key_path, &mut budget)?.is_some() {
            return Err(NativeBoundaryError::new(format!(
                "registry key remains after uninstall: {}",
                validated.key_path
            )));
        }
    }
    Ok(())
}

impl RegistrySnapshot {
    /// Clears only the explicitly monitored keys. The snapshot remains the
    /// authority for rollback if this operation or later setup fails.
    pub fn clear_monitored(&self) -> Result<(), NativeBoundaryError> {
        let mut errors = Vec::new();
        for key in &self.keys {
            if let Err(error) = crate::win32::registry_delete_tree(&key.spec.key_path) {
                errors.push(format!("{}: {error}", key.spec.key_path));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(NativeBoundaryError::new(format!(
                "failed to clear monitored registry keys: {}",
                errors.join("; ")
            )))
        }
    }

    /// Restores all roots, attempts every root even after an earlier failure,
    /// then verifies exact tree/value-kind/value-byte and parent-presence parity.
    pub fn restore(&self) -> Result<(), NativeBoundaryError> {
        let mut errors = Vec::new();
        for key in &self.keys {
            let result = match &key.node {
                Some(node) => crate::win32::registry_restore_tree(&key.spec.key_path, node),
                None => crate::win32::registry_delete_tree(&key.spec.key_path),
            };
            if let Err(error) = result {
                errors.push(format!("{}: {error}", key.spec.key_path));
            }
        }

        let mut parent_expectations = BTreeMap::<String, bool>::new();
        for key in &self.keys {
            parent_expectations
                .entry(key.spec.parent_path.clone())
                .and_modify(|expected| *expected |= key.parent_existed)
                .or_insert(key.parent_existed);
        }
        for (parent, _) in parent_expectations.iter().filter(|(_, existed)| !**existed) {
            match crate::win32::registry_remove_empty_key(parent) {
                Ok(()) => {}
                Err(error) => errors.push(format!("parent {parent}: {error}")),
            }
        }

        if !errors.is_empty() {
            return Err(NativeBoundaryError::new(format!(
                "registry restore had cleanup errors: {}",
                errors.join("; ")
            )));
        }
        self.assert_equivalent()
    }

    pub fn assert_equivalent(&self) -> Result<(), NativeBoundaryError> {
        let mut budget = SnapshotBudget::default();
        for expected in &self.keys {
            let parent_existed = crate::win32::registry_key_exists(&expected.spec.parent_path)?;
            if parent_existed != expected.parent_existed {
                return Err(NativeBoundaryError::new(format!(
                    "registry parent presence mismatch for {}: expected {}, got {}",
                    expected.spec.parent_path, expected.parent_existed, parent_existed
                )));
            }
            let actual = crate::win32::registry_read_tree(&expected.spec.key_path, &mut budget)?;
            if actual != expected.node {
                return Err(NativeBoundaryError::new(format!(
                    "registry subtree mismatch after smoke scope: {}",
                    expected.spec.key_path
                )));
            }
        }
        Ok(())
    }
}

pub fn read_value(
    key_path: &str,
    name: &str,
) -> Result<Option<RegistryValue>, NativeBoundaryError> {
    let (parent, _) = key_path
        .rsplit_once('\\')
        .ok_or_else(|| NativeBoundaryError::new("registry value path has no parent"))?;
    let spec = KeySpec::new(key_path.to_owned(), parent.to_owned())?;
    let mut budget = SnapshotBudget::default();
    let node = crate::win32::registry_read_tree(spec.key_path(), &mut budget)?;
    Ok(node.and_then(|node| node.values.get(name).cloned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use windows_sys::Win32::System::Registry::{
        REG_BINARY, REG_DWORD, REG_EXPAND_SZ, REG_MULTI_SZ, REG_SZ,
    };

    fn scope() -> (String, String, KeySpec) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let parent = format!(
            "Software\\SkyAutoPlayerNsisTest-{}-{nonce}",
            std::process::id()
        );
        let key = format!("{parent}\\Sky Auto Player");
        let spec = KeySpec::new(key.clone(), parent.clone()).expect("isolated HKCU test scope");
        (parent, key, spec)
    }

    fn string_value(kind: u32, value: &str) -> RegistryValue {
        let mut bytes = value
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        bytes.extend_from_slice(&[0, 0]);
        RegistryValue { kind, bytes }
    }

    fn set(key: &str, name: &str, value: &RegistryValue) {
        crate::win32::registry_write_value(key, name, value).expect("write isolated test value");
    }

    fn cleanup(parent: &str) {
        crate::win32::registry_delete_tree(parent).expect("delete isolated HKCU test scope");
    }

    #[test]
    fn absent_key_and_absent_parent_restore_after_partial_fresh_install_setup() {
        let (parent, key, spec) = scope();
        cleanup(&parent);
        let snapshot = capture(std::slice::from_ref(&spec)).expect("capture absent key");
        snapshot.clear_monitored().expect("clear absent key");
        set(&key, "partial", &string_value(REG_SZ, "failed entry"));
        set(
            &format!("{key}\\Nested"),
            "default",
            &string_value(REG_EXPAND_SZ, "%TEMP%"),
        );
        snapshot.restore().expect("rollback partial setup");
        assert!(!crate::win32::registry_key_exists(&key).unwrap());
        assert!(!crate::win32::registry_key_exists(&parent).unwrap());
        assert_roots_absent(std::slice::from_ref(&spec)).expect("assert root absent");
        cleanup(&parent);
    }

    #[test]
    fn restores_default_named_and_nested_values_with_exact_registry_kinds_and_bytes() {
        let (parent, key, spec) = scope();
        cleanup(&parent);
        crate::win32::registry_ensure_key(&parent).expect("create test parent");
        let expand = string_value(REG_EXPAND_SZ, "%SystemRoot%\\SkyAutoPlayer");
        let binary = RegistryValue {
            kind: REG_BINARY,
            bytes: vec![0, 1, 0xfe, 0xff],
        };
        let dword = RegistryValue {
            kind: REG_DWORD,
            bytes: 0x1234_5678_u32.to_le_bytes().to_vec(),
        };
        let multi = RegistryValue {
            kind: REG_MULTI_SZ,
            bytes: ["first", "second", ""]
                .into_iter()
                .flat_map(|value| {
                    value
                        .encode_utf16()
                        .chain(std::iter::once(0))
                        .flat_map(u16::to_le_bytes)
                })
                .collect(),
        };
        set(&key, "", &expand);
        set(&key, "Binary", &binary);
        set(&key, "Dword", &dword);
        set(&key, "Multi", &multi);
        set(
            &format!("{key}\\Nested"),
            "Sentinel",
            &string_value(REG_SZ, "preserve me"),
        );

        let snapshot = capture(std::slice::from_ref(&spec)).expect("capture present subtree");
        assert!(assert_roots_absent(std::slice::from_ref(&spec)).is_err());
        set(&key, "", &string_value(REG_SZ, "corrupt kind"));
        set(&key, "Added", &string_value(REG_SZ, "remove me"));
        set(
            &format!("{key}\\Nested"),
            "Sentinel",
            &string_value(REG_SZ, "corrupt value"),
        );
        set(
            &format!("{key}\\Unexpected"),
            "Residue",
            &string_value(REG_SZ, "remove subtree"),
        );

        snapshot.restore().expect("restore exact registry subtree");
        assert_eq!(read_value(&key, "").unwrap(), Some(expand));
        assert_eq!(read_value(&key, "Binary").unwrap(), Some(binary));
        assert_eq!(read_value(&key, "Dword").unwrap(), Some(dword));
        assert_eq!(read_value(&key, "Multi").unwrap(), Some(multi));
        assert_eq!(
            read_value(&format!("{key}\\Nested"), "Sentinel").unwrap(),
            Some(string_value(REG_SZ, "preserve me"))
        );
        assert_eq!(read_value(&key, "Added").unwrap(), None);
        assert!(!crate::win32::registry_key_exists(&format!("{key}\\Unexpected")).unwrap());
        cleanup(&parent);
    }

    #[test]
    fn refuses_to_delete_unscoped_sibling_residue_when_parent_was_absent() {
        let (parent, key, spec) = scope();
        cleanup(&parent);
        let snapshot = capture(std::slice::from_ref(&spec)).expect("capture absent parent");
        set(
            &format!("{parent}\\UnscopedSibling"),
            "sentinel",
            &string_value(REG_SZ, "keep"),
        );
        set(&key, "installer", &string_value(REG_SZ, "created"));
        assert!(snapshot.restore().is_err());
        assert!(crate::win32::registry_key_exists(&parent).unwrap());
        assert_eq!(
            read_value(&format!("{parent}\\UnscopedSibling"), "sentinel").unwrap(),
            Some(string_value(REG_SZ, "keep"))
        );
        cleanup(&parent);
    }

    #[test]
    fn preserves_existing_parent_and_sibling_keys() {
        let (parent, key, spec) = scope();
        cleanup(&parent);
        set(
            &format!("{parent}\\PreexistingSibling"),
            "value",
            &string_value(REG_SZ, "untouched"),
        );
        let snapshot = capture(std::slice::from_ref(&spec)).expect("capture existing parent");
        set(&key, "installer", &string_value(REG_SZ, "created"));
        snapshot.restore().expect("restore with existing sibling");
        assert_eq!(
            read_value(&format!("{parent}\\PreexistingSibling"), "value").unwrap(),
            Some(string_value(REG_SZ, "untouched"))
        );
        assert!(crate::win32::registry_key_exists(&parent).unwrap());
        cleanup(&parent);
    }

    #[test]
    fn fresh_install_entry_rolls_back_registry_when_appdata_setup_fails() {
        let (parent, key, spec) = scope();
        cleanup(&parent);
        set(
            &key,
            "Sentinel",
            &string_value(REG_EXPAND_SZ, "%USERPROFILE%\\Songs"),
        );
        let snapshot = capture(std::slice::from_ref(&spec)).expect("capture initial registry");
        snapshot
            .clear_monitored()
            .expect("neutralize monitored key for fresh install");

        let blocker = std::env::temp_dir().join(format!(
            "sky-auto-player-appdata-blocker-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        std::fs::write(&blocker, b"not a directory").expect("create AppData setup blocker");
        let setup = std::fs::create_dir(blocker.join("app-data"));
        assert!(setup.is_err(), "AppData setup must fail below a file");
        snapshot
            .restore()
            .expect("rollback FreshInstall registry change");
        assert_eq!(
            read_value(&key, "Sentinel").unwrap(),
            Some(string_value(REG_EXPAND_SZ, "%USERPROFILE%\\Songs"))
        );
        std::fs::remove_file(blocker).expect("remove AppData setup blocker");
        cleanup(&parent);
    }
}

fn validate_path(path: &str) -> Result<(), NativeBoundaryError> {
    if path.is_empty()
        || path.contains('\0')
        || path.starts_with('\\')
        || path.ends_with('\\')
        || path.contains('/')
        || path
            .split('\\')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || !(path.eq_ignore_ascii_case("Software")
            || path
                .get(.."Software\\".len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Software\\")))
    {
        return Err(NativeBoundaryError::new(
            "registry scope must be a normalized HKCU Software-relative path",
        ));
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct SnapshotBudget {
    keys: usize,
    values: usize,
    bytes: usize,
}

impl SnapshotBudget {
    pub(crate) fn add_key(&mut self) -> Result<(), NativeBoundaryError> {
        self.keys += 1;
        if self.keys > MAX_SNAPSHOT_KEYS {
            return Err(NativeBoundaryError::new(
                "registry snapshot exceeds the key-count limit",
            ));
        }
        Ok(())
    }

    pub(crate) fn add_value(&mut self, bytes: usize) -> Result<(), NativeBoundaryError> {
        self.values += 1;
        self.bytes = self.bytes.saturating_add(bytes);
        if self.values > MAX_SNAPSHOT_VALUES || self.bytes > MAX_SNAPSHOT_BYTES {
            return Err(NativeBoundaryError::new(
                "registry snapshot exceeds the value-count or byte limit",
            ));
        }
        Ok(())
    }
}
