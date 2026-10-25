//! Audited Win32 FFI for Credential Manager and kill-on-close Job Objects.
//!
//! All pointers passed here are backed by live owned buffers or handles. The
//! Credential Manager blob is copied into zeroizing memory and its OS-owned
//! allocation is released with CredFree. Job handles are RAII-owned; closing
//! a kill-on-close job is the bounded descendant teardown fallback.

#![allow(unsafe_code)]

use crate::NativeBoundaryError;
use crate::credential::MAX_CREDENTIAL_BLOB;
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
use zeroize::Zeroizing;

const ERROR_NOT_FOUND: u32 = 1168;
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
