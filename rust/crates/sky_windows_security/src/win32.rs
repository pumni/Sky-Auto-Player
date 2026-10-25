//! Audited Win32 FFI for Credential Manager and kill-on-close Job Objects.
//!
//! All pointers passed here are backed by live owned buffers or handles. The
//! Credential Manager blob is copied into zeroizing memory and its OS-owned
//! allocation is released with CredFree. Job handles are RAII-owned; closing
//! a kill-on-close job is the bounded descendant teardown fallback.

#![allow(unsafe_code)]

use crate::NativeBoundaryError;
use crate::credential::MAX_CREDENTIAL_BLOB;
use std::collections::BTreeSet;
use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::io::AsRawHandle;
use std::process::Child;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
#[cfg(test)]
use windows_sys::Win32::Security::Credentials::CRED_PERSIST_LOCAL_MACHINE;
use windows_sys::Win32::Security::Credentials::{
    CRED_PERSIST_SESSION, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW,
    CredWriteW,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WOW64_64KEY, KEY_WRITE, REG_OPTION_NON_VOLATILE,
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW,
    RegEnumValueW, RegOpenKeyExW, RegQueryInfoKeyW, RegSetValueExW,
};
use zeroize::Zeroizing;

const ERROR_NOT_FOUND: u32 = 1168;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_PATH_NOT_FOUND: u32 = 3;
const ERROR_MORE_DATA: u32 = 234;
const MAX_REGISTRY_NAME: u32 = 32_767;
const MAX_REGISTRY_VALUE_BYTES: u32 = 16 * 1024 * 1024;

struct RegistryHandle(HKEY);

impl Drop for RegistryHandle {
    fn drop(&mut self) {
        // SAFETY: this handle is uniquely owned by RegistryHandle and was
        // returned by RegOpenKeyExW or RegCreateKeyExW.
        unsafe { RegCloseKey(self.0) };
    }
}

fn registry_path_wide(path: &str) -> Vec<u16> {
    path.encode_utf16().chain(Some(0)).collect()
}

fn registry_error(operation: &str, code: u32) -> NativeBoundaryError {
    NativeBoundaryError::new(format!("{operation} failed with Windows error {code}"))
}

fn open_registry_key(
    path: &str,
    access: u32,
) -> Result<Option<RegistryHandle>, NativeBoundaryError> {
    let wide = registry_path_wide(path);
    let mut handle = null_mut();
    // SAFETY: `wide` is NUL-terminated and the output slot is valid. The
    // predefined HKCU handle is borrowed and the returned handle is owned.
    let result = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide.as_ptr(),
            0,
            access | KEY_WOW64_64KEY,
            &mut handle,
        )
    };
    if result == ERROR_FILE_NOT_FOUND || result == ERROR_PATH_NOT_FOUND {
        return Ok(None);
    }
    if result != 0 {
        return Err(registry_error("RegOpenKeyExW", result));
    }
    if handle.is_null() {
        return Err(NativeBoundaryError::new(
            "RegOpenKeyExW returned a null registry handle",
        ));
    }
    Ok(Some(RegistryHandle(handle)))
}

fn create_registry_key(path: &str) -> Result<RegistryHandle, NativeBoundaryError> {
    let wide = registry_path_wide(path);
    let mut handle = null_mut();
    let mut disposition = 0;
    // SAFETY: path and output buffers remain live for the synchronous call;
    // security attributes and class are intentionally absent.
    let result = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide.as_ptr(),
            0,
            null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
            null(),
            &mut handle,
            &mut disposition,
        )
    };
    if result != 0 {
        return Err(registry_error("RegCreateKeyExW", result));
    }
    if handle.is_null() {
        return Err(NativeBoundaryError::new(
            "RegCreateKeyExW returned a null registry handle",
        ));
    }
    Ok(RegistryHandle(handle))
}

fn registry_info(handle: HKEY) -> Result<(u32, u32, u32, u32, u32), NativeBoundaryError> {
    let mut subkey_count = 0;
    let mut max_subkey_name = 0;
    let mut value_count = 0;
    let mut max_value_name = 0;
    let mut max_value_data = 0;
    // SAFETY: optional outputs are null; each supplied output points to a
    // live u32 for the duration of the call.
    let result = unsafe {
        RegQueryInfoKeyW(
            handle,
            null_mut(),
            null_mut(),
            null(),
            &mut subkey_count,
            &mut max_subkey_name,
            null_mut(),
            &mut value_count,
            &mut max_value_name,
            &mut max_value_data,
            null_mut(),
            null_mut(),
        )
    };
    if result != 0 {
        return Err(registry_error("RegQueryInfoKeyW", result));
    }
    if max_subkey_name > MAX_REGISTRY_NAME
        || max_value_name > MAX_REGISTRY_NAME
        || max_value_data > MAX_REGISTRY_VALUE_BYTES
    {
        return Err(NativeBoundaryError::new(
            "registry key contains an unbounded name or value",
        ));
    }
    Ok((
        subkey_count,
        max_subkey_name,
        value_count,
        max_value_name,
        max_value_data,
    ))
}

pub(crate) fn registry_key_exists(path: &str) -> Result<bool, NativeBoundaryError> {
    Ok(open_registry_key(path, KEY_READ)?.is_some())
}

#[cfg(test)]
pub(crate) fn registry_ensure_key(path: &str) -> Result<(), NativeBoundaryError> {
    drop(create_registry_key(path)?);
    Ok(())
}

#[cfg(test)]
pub(crate) fn registry_write_value(
    path: &str,
    name: &str,
    value: &crate::registry::RegistryValue,
) -> Result<(), NativeBoundaryError> {
    if value.bytes.len() > MAX_REGISTRY_VALUE_BYTES as usize {
        return Err(NativeBoundaryError::new(
            "registry value exceeds the bounded snapshot limit",
        ));
    }
    let handle = create_registry_key(path)?;
    let wide_name = registry_path_wide(name);
    // SAFETY: the value buffer and NUL-terminated name remain live for the
    // synchronous write; the handle is an owned writable HKCU key.
    let result = unsafe {
        RegSetValueExW(
            handle.0,
            wide_name.as_ptr(),
            0,
            value.kind,
            if value.bytes.is_empty() {
                null()
            } else {
                value.bytes.as_ptr()
            },
            value.bytes.len() as u32,
        )
    };
    if result != 0 {
        return Err(registry_error("RegSetValueExW", result));
    }
    Ok(())
}

pub(crate) fn registry_read_tree(
    path: &str,
    budget: &mut crate::registry::SnapshotBudget,
) -> Result<Option<crate::registry::RegistryNode>, NativeBoundaryError> {
    let Some(handle) = open_registry_key(path, KEY_READ)? else {
        return Ok(None);
    };
    budget.add_key()?;
    let (subkey_count, max_subkey_name, value_count, max_value_name, max_value_data) =
        registry_info(handle.0)?;
    let mut node = crate::registry::RegistryNode::default();

    for index in 0..value_count {
        let mut name = vec![0_u16; (max_value_name.max(1) + 2) as usize];
        let mut name_length = name.len() as u32;
        let mut kind = 0;
        let mut data = vec![0_u8; max_value_data as usize];
        let mut data_length = data.len() as u32;
        // SAFETY: all buffers and output lengths are live for this call. The
        // key handle was opened with KEY_QUERY_VALUE by KEY_READ.
        let result = unsafe {
            RegEnumValueW(
                handle.0,
                index,
                name.as_mut_ptr(),
                &mut name_length,
                null(),
                &mut kind,
                if data.is_empty() {
                    null_mut()
                } else {
                    data.as_mut_ptr()
                },
                &mut data_length,
            )
        };
        if result == ERROR_MORE_DATA || data_length > MAX_REGISTRY_VALUE_BYTES {
            return Err(NativeBoundaryError::new(
                "registry value changed or exceeded the snapshot limit during capture",
            ));
        }
        if result != 0 {
            return Err(registry_error("RegEnumValueW", result));
        }
        let value_name = String::from_utf16(&name[..name_length as usize])
            .map_err(|_| NativeBoundaryError::new("registry value name is not valid UTF-16"))?;
        data.truncate(data_length as usize);
        budget.add_value(data.len())?;
        if node
            .values
            .insert(
                value_name,
                crate::registry::RegistryValue { kind, bytes: data },
            )
            .is_some()
        {
            return Err(NativeBoundaryError::new(
                "registry returned duplicate value names during capture",
            ));
        }
    }

    for index in 0..subkey_count {
        let mut name = vec![0_u16; (max_subkey_name + 2) as usize];
        let mut name_length = name.len() as u32;
        // SAFETY: the name buffer and length output remain live. Other
        // optional metadata outputs are not requested.
        let result = unsafe {
            RegEnumKeyExW(
                handle.0,
                index,
                name.as_mut_ptr(),
                &mut name_length,
                null(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if result != 0 {
            return Err(registry_error("RegEnumKeyExW", result));
        }
        let subkey = String::from_utf16(&name[..name_length as usize])
            .map_err(|_| NativeBoundaryError::new("registry subkey name is not valid UTF-16"))?;
        let child_path = format!("{path}\\{subkey}");
        let child = registry_read_tree(&child_path, budget)?.ok_or_else(|| {
            NativeBoundaryError::new("registry subkey disappeared during snapshot capture")
        })?;
        if node.subkeys.insert(subkey, child).is_some() {
            return Err(NativeBoundaryError::new(
                "registry returned duplicate subkey names during capture",
            ));
        }
    }
    Ok(Some(node))
}

pub(crate) fn registry_delete_tree(path: &str) -> Result<(), NativeBoundaryError> {
    if open_registry_key(path, KEY_READ)?.is_none() {
        return Ok(());
    }
    let wide = registry_path_wide(path);
    // SAFETY: `wide` is NUL-terminated and HKCU is the predefined root handle.
    let tree_result = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wide.as_ptr()) };
    if tree_result != 0 {
        return Err(registry_error("RegDeleteTreeW", tree_result));
    }
    // SAFETY: the target is a validated, explicitly scoped key and is now empty.
    let key_result = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, wide.as_ptr()) };
    if key_result != 0 && key_result != ERROR_FILE_NOT_FOUND && key_result != ERROR_PATH_NOT_FOUND {
        return Err(registry_error("RegDeleteKeyW", key_result));
    }
    Ok(())
}

pub(crate) fn registry_remove_empty_key(path: &str) -> Result<(), NativeBoundaryError> {
    let Some(handle) = open_registry_key(path, KEY_READ)? else {
        return Ok(());
    };
    let (subkey_count, _, value_count, _, _) = registry_info(handle.0)?;
    drop(handle);
    if subkey_count != 0 || value_count != 0 || path.eq_ignore_ascii_case("Software") {
        return Ok(());
    }
    let wide = registry_path_wide(path);
    // SAFETY: only a previously absent direct parent in the monitored scope is
    // offered here, and the key was verified empty before deletion.
    let result = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, wide.as_ptr()) };
    if result == 0 || result == ERROR_FILE_NOT_FOUND || result == ERROR_PATH_NOT_FOUND {
        Ok(())
    } else {
        Err(registry_error("RegDeleteKeyW for empty parent", result))
    }
}

pub(crate) fn registry_restore_tree(
    path: &str,
    expected: &crate::registry::RegistryNode,
) -> Result<(), NativeBoundaryError> {
    let handle = create_registry_key(path)?;
    let (subkey_count, max_subkey_name, value_count, max_value_name, _) = registry_info(handle.0)?;
    let mut current_values = BTreeSet::new();
    for index in 0..value_count {
        let mut name = vec![0_u16; (max_value_name.max(1) + 2) as usize];
        let mut name_length = name.len() as u32;
        let mut kind = 0;
        let mut data_length = 0;
        // SAFETY: the value-name buffer and type/length outputs remain live;
        // null data is a supported size-query form of RegEnumValueW.
        let result = unsafe {
            RegEnumValueW(
                handle.0,
                index,
                name.as_mut_ptr(),
                &mut name_length,
                null(),
                &mut kind,
                null_mut(),
                &mut data_length,
            )
        };
        if result != 0 {
            return Err(registry_error("RegEnumValueW during restore", result));
        }
        current_values.insert(
            String::from_utf16(&name[..name_length as usize])
                .map_err(|_| NativeBoundaryError::new("registry value name is not valid UTF-16"))?,
        );
    }
    for name in current_values.difference(&expected.values.keys().cloned().collect()) {
        let wide = registry_path_wide(name);
        // SAFETY: name is an enumerated value name on the opened key.
        let result = unsafe { RegDeleteValueW(handle.0, wide.as_ptr()) };
        if result != 0 {
            return Err(registry_error("RegDeleteValueW", result));
        }
    }
    for (name, value) in &expected.values {
        let wide = registry_path_wide(name);
        // SAFETY: value bytes are stable during the call; empty values pass a
        // null data pointer and a zero byte count as documented by Win32.
        let result = unsafe {
            RegSetValueExW(
                handle.0,
                wide.as_ptr(),
                0,
                value.kind,
                if value.bytes.is_empty() {
                    null()
                } else {
                    value.bytes.as_ptr()
                },
                value.bytes.len() as u32,
            )
        };
        if result != 0 {
            return Err(registry_error("RegSetValueExW", result));
        }
    }

    let mut current_subkeys = BTreeSet::new();
    for index in 0..subkey_count {
        let mut name = vec![0_u16; (max_subkey_name + 2) as usize];
        let mut name_length = name.len() as u32;
        // SAFETY: the name buffer and length output remain live.
        let result = unsafe {
            RegEnumKeyExW(
                handle.0,
                index,
                name.as_mut_ptr(),
                &mut name_length,
                null(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if result != 0 {
            return Err(registry_error("RegEnumKeyExW during restore", result));
        }
        current_subkeys.insert(
            String::from_utf16(&name[..name_length as usize]).map_err(|_| {
                NativeBoundaryError::new("registry subkey name is not valid UTF-16")
            })?,
        );
    }
    for name in current_subkeys.difference(&expected.subkeys.keys().cloned().collect()) {
        let child = format!("{path}\\{name}");
        registry_delete_tree(&child)?;
    }
    drop(handle);
    for (name, child) in &expected.subkeys {
        registry_restore_tree(&format!("{path}\\{name}"), child)?;
    }
    Ok(())
}
pub(crate) struct JobHandle(HANDLE);

impl JobHandle {
    pub(crate) fn for_child(child: &Child) -> Result<Self, NativeBoundaryError> {
        // SAFETY: CreateJobObjectW accepts null attributes/name and returns an
        // owned handle or null. The handle is immediately wrapped for RAII.
        let handle = unsafe { CreateJobObjectW(null(), null()) };
        if handle.is_null() {
            return Err(last_error("CreateJobObjectW failed"));
        }
        let owner = Self(handle);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` is initialized, correctly sized, and remains live
        // for the synchronous SetInformationJobObject call.
        let configured = unsafe {
            SetInformationJobObject(
                owner.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast::<c_void>(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            return Err(last_error("SetInformationJobObject failed"));
        }
        let process: HANDLE = child.as_raw_handle();
        // SAFETY: Child owns a live process handle and `owner` owns a valid job.
        let assigned = unsafe { AssignProcessToJobObject(owner.0, process) };
        if assigned == 0 {
            return Err(last_error("AssignProcessToJobObject failed"));
        }
        Ok(owner)
    }

    pub(crate) fn terminate(&self) -> Result<(), NativeBoundaryError> {
        // SAFETY: `self.0` is a live handle owned by this object.
        if unsafe { TerminateJobObject(self.0, 1) } == 0 {
            Err(last_error("TerminateJobObject failed"))
        } else {
            Ok(())
        }
    }
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        // SAFETY: This object uniquely owns the handle returned by
        // CreateJobObjectW. Closing it enforces JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub(crate) fn credential_has_nonempty(target: &str) -> Result<bool, NativeBoundaryError> {
    match read_credential(target) {
        Ok(secret) => Ok(!secret.is_empty()),
        Err(CredentialReadError::Absent) => Ok(false),
        Err(CredentialReadError::Failed(error)) => Err(error),
    }
}

pub(crate) fn credential_is_session_persistent(target: &str) -> Result<bool, NativeBoundaryError> {
    let target_wide: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    let mut pointer = null_mut();
    // SAFETY: target_wide is NUL-terminated and pointer is a valid out slot.
    if unsafe { CredReadW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) } == 0 {
        let code = unsafe { GetLastError() };
        if code == ERROR_NOT_FOUND {
            return Err(NativeBoundaryError::new(
                "Windows Credential Manager credential is absent",
            ));
        }
        return Err(last_error("CredReadW failed"));
    }
    if pointer.is_null() {
        return Err(NativeBoundaryError::new(
            "CredReadW returned an empty allocation",
        ));
    }
    // SAFETY: Successful CredReadW returns a live CredFree-owned structure.
    let credential = unsafe { &*pointer };
    let persistence = credential.Persist;
    if !credential.CredentialBlob.is_null()
        && (credential.CredentialBlobSize as usize) <= MAX_CREDENTIAL_BLOB
    {
        // SAFETY: The blob pointer and bounded size came from this live
        // CredReadW allocation. Clear the transient copy before freeing it.
        let blob = unsafe {
            std::slice::from_raw_parts_mut(
                credential.CredentialBlob,
                credential.CredentialBlobSize as usize,
            )
        };
        zeroize::Zeroize::zeroize(blob);
    }
    // SAFETY: Pointer is the CredReadW allocation and is released once.
    unsafe { CredFree(pointer.cast_const().cast::<c_void>()) };
    Ok(persistence == CRED_PERSIST_SESSION)
}

pub(crate) fn credential_read(target: &str) -> Result<Zeroizing<String>, NativeBoundaryError> {
    match read_credential(target) {
        Ok(secret) if !secret.is_empty() => Ok(secret),
        Ok(_) => Err(NativeBoundaryError::new(
            "Windows Credential Manager credential blob is empty",
        )),
        Err(CredentialReadError::Absent) => Err(NativeBoundaryError::new(
            "Windows Credential Manager credential is absent",
        )),
        Err(CredentialReadError::Failed(error)) => Err(error),
    }
}

enum CredentialReadError {
    Absent,
    Failed(NativeBoundaryError),
}

fn read_credential(target: &str) -> Result<Zeroizing<String>, CredentialReadError> {
    let target_wide: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    let mut pointer = null_mut();
    // SAFETY: target_wide is NUL-terminated and pointer is a valid out slot.
    if unsafe { CredReadW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) } == 0 {
        let code = unsafe { GetLastError() };
        if code == ERROR_NOT_FOUND {
            return Err(CredentialReadError::Absent);
        }
        return Err(CredentialReadError::Failed(last_error("CredReadW failed")));
    }
    if pointer.is_null() {
        return Err(CredentialReadError::Failed(NativeBoundaryError::new(
            "CredReadW returned an empty allocation",
        )));
    }
    // SAFETY: Successful CredReadW returns a CredFree-owned CREDENTIALW.
    let credential = unsafe { &*pointer };
    let result = if credential.Persist != CRED_PERSIST_SESSION {
        Err(CredentialReadError::Failed(NativeBoundaryError::new(
            "Windows Credential Manager credential is not session-persistent",
        )))
    } else if credential.CredentialBlobSize == 0 {
        Ok(Zeroizing::new(String::new()))
    } else if credential.CredentialBlob.is_null()
        || credential.CredentialBlobSize as usize > MAX_CREDENTIAL_BLOB
    {
        Err(CredentialReadError::Failed(NativeBoundaryError::new(
            "Windows Credential Manager credential blob has an invalid length",
        )))
    } else {
        // SAFETY: The OS-owned credential blob is live until CredFree; its
        // size was bounded to the documented generic credential maximum.
        let source = unsafe {
            std::slice::from_raw_parts(
                credential.CredentialBlob,
                credential.CredentialBlobSize as usize,
            )
        };
        let bytes = Zeroizing::new(source.to_vec());
        match String::from_utf8(bytes.to_vec()) {
            Ok(value) => Ok(Zeroizing::new(value)),
            Err(error) => {
                let mut invalid_bytes = error.into_bytes();
                zeroize::Zeroize::zeroize(&mut invalid_bytes);
                Err(CredentialReadError::Failed(NativeBoundaryError::new(
                    "Windows Credential Manager credential is not valid UTF-8",
                )))
            }
        }
    };
    if !credential.CredentialBlob.is_null()
        && (credential.CredentialBlobSize as usize) <= MAX_CREDENTIAL_BLOB
    {
        // SAFETY: The blob pointer and bounded length came from this live
        // CredReadW allocation. Clear the transient OS-owned copy before free.
        let borrowed = unsafe {
            std::slice::from_raw_parts_mut(
                credential.CredentialBlob,
                credential.CredentialBlobSize as usize,
            )
        };
        zeroize::Zeroize::zeroize(borrowed);
    }
    // SAFETY: pointer was returned by CredReadW and is released exactly once.
    unsafe { CredFree(pointer.cast_const().cast::<c_void>()) };
    result
}

pub(crate) fn credential_write(target: &str, secret: &str) -> Result<(), NativeBoundaryError> {
    credential_write_with_persistence(target, secret, CRED_PERSIST_SESSION)
}

#[cfg(test)]
pub(crate) fn credential_write_machine_for_test(
    target: &str,
    secret: &str,
) -> Result<(), NativeBoundaryError> {
    credential_write_with_persistence(target, secret, CRED_PERSIST_LOCAL_MACHINE)
}

fn credential_write_with_persistence(
    target: &str,
    secret: &str,
    persistence: u32,
) -> Result<(), NativeBoundaryError> {
    let mut target_wide: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    let mut comment_wide: Vec<u16> = "Sky Auto Player V4 Updater Production Session Credential"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut user_wide: Vec<u16> = "SkyAutoPlayerOperator"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut blob = Zeroizing::new(secret.as_bytes().to_vec());
    if blob.is_empty() {
        return Err(NativeBoundaryError::new(
            "credential secret cannot be empty",
        ));
    }
    if blob.len() > MAX_CREDENTIAL_BLOB {
        return Err(NativeBoundaryError::new(
            "credential secret exceeds the Windows Credential Manager blob limit",
        ));
    }
    let credential = CREDENTIALW {
        Flags: 0,
        Type: CRED_TYPE_GENERIC,
        TargetName: target_wide.as_mut_ptr(),
        Comment: comment_wide.as_mut_ptr(),
        LastWritten: Default::default(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: persistence,
        AttributeCount: 0,
        Attributes: null_mut(),
        TargetAlias: null_mut(),
        UserName: user_wide.as_mut_ptr(),
    };
    // SAFETY: All pointers in `credential` refer to live writable buffers for
    // this synchronous call. Windows copies the blob into session storage.
    if unsafe { CredWriteW(&credential, 0) } == 0 {
        return Err(last_error("CredWriteW failed"));
    }
    Ok(())
}

pub(crate) fn credential_delete(target: &str) -> Result<bool, NativeBoundaryError> {
    let target_wide: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    // SAFETY: target_wide is NUL-terminated and remains live for the call.
    if unsafe { CredDeleteW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
        return Ok(true);
    }
    if unsafe { GetLastError() } == ERROR_NOT_FOUND {
        Ok(false)
    } else {
        Err(last_error("CredDeleteW failed"))
    }
}

fn last_error(operation: &str) -> NativeBoundaryError {
    // SAFETY: GetLastError is thread-local and has no pointer preconditions.
    let code = unsafe { GetLastError() };
    NativeBoundaryError::new(format!("{operation} (Windows error {code})"))
}
