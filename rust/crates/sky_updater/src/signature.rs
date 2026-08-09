use std::path::{Path, PathBuf};

use crate::error::{Result, UpdaterError};
use crate::manifest::Manifest;
use crate::{PRIMARY_EXE, UPDATER_EXE};

pub fn project_owned_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for required in [PRIMARY_EXE, UPDATER_EXE, "native_calibration.exe"] {
        let path = root.join(required);
        if !path.is_file() {
            return Err(UpdaterError::SignatureInvalid(format!(
                "required PE missing: {required}"
            )));
        }
        files.push(path);
    }
    let internal = root.join("_internal");
    if internal.is_dir() {
        collect_project_pyds(&internal, &mut files)?;
    }
    Ok(files)
}

fn collect_project_pyds(root: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_symlink() {
            return Err(UpdaterError::SignatureInvalid(format!(
                "symlink under _internal: {}",
                path.display()
            )));
        }
        if path.is_dir() {
            collect_project_pyds(&path, output)?;
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("sky_player_rs") && name.ends_with(".pyd"))
        {
            output.push(path);
        }
    }
    Ok(())
}

pub fn verify_manifest_scope(root: &Path, manifest: &Manifest) -> Result<()> {
    let project_files = project_owned_files(root)?;
    for path in project_files {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| UpdaterError::SignatureInvalid("PE escaped staging root".into()))?
            .to_string_lossy()
            .replace('\\', "/");
        if !manifest.files.iter().any(|file| file.path == relative) {
            return Err(UpdaterError::SignatureInvalid(format!(
                "project-owned PE is absent from manifest: {relative}"
            )));
        }
    }
    Ok(())
}

pub fn verify_file(path: &Path) -> Result<()> {
    if !path.is_file() {
        return Err(UpdaterError::SignatureInvalid(format!(
            "missing PE: {}",
            path.display()
        )));
    }
    #[cfg(debug_assertions)]
    return Ok(()); // Development binaries are explicitly unsigned.
    #[cfg(all(not(debug_assertions), windows))]
    return verify_authenticode(path);
    #[cfg(all(not(debug_assertions), not(windows)))]
    Err(UpdaterError::SignatureInvalid(
        "Authenticode requires Windows".into(),
    ))
}

pub fn verify_project_files(root: &Path, manifest: &Manifest) -> Result<()> {
    verify_manifest_scope(root, manifest)?;
    for path in project_owned_files(root)? {
        verify_file(&path)?;
    }
    Ok(())
}

#[cfg(all(windows, not(debug_assertions)))]
fn verify_authenticode(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::Security::WinTrust::{
        WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
        WTD_CHOICE_FILE, WTD_REVOKE_WHOLECHAIN, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY,
        WTD_UI_NONE, WTD_UICONTEXT_EXECUTE, WinVerifyTrust,
    };

    let path_wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: path_wide.as_ptr(),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_WHOLECHAIN,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 {
            pFile: &mut file_info,
        },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwUIContext: WTD_UICONTEXT_EXECUTE,
        ..Default::default()
    };
    let status = unsafe {
        WinVerifyTrust(
            std::ptr::null_mut(),
            &WINTRUST_ACTION_GENERIC_VERIFY_V2 as *const _ as *mut _,
            (&mut data as *mut WINTRUST_DATA).cast::<std::ffi::c_void>(),
        )
    };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    let _ = unsafe {
        WinVerifyTrust(
            std::ptr::null_mut(),
            &WINTRUST_ACTION_GENERIC_VERIFY_V2 as *const _ as *mut _,
            (&mut data as *mut WINTRUST_DATA).cast::<std::ffi::c_void>(),
        )
    };
    if status != ERROR_SUCCESS as i32 {
        return Err(UpdaterError::SignatureInvalid(format!(
            "WinVerifyTrust rejected {}: 0x{status:08x}",
            path.display()
        )));
    }
    verify_publisher_subject(path)?;
    Ok(())
}

#[cfg(all(windows, not(debug_assertions)))]
fn verify_publisher_subject(path: &Path) -> Result<()> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::Cryptography::{
        CERT_FIND_SUBJECT_CERT, CERT_INFO, CERT_NAME_RDN_TYPE,
        CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED, CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED,
        CERT_QUERY_FORMAT_FLAG_ALL, CERT_QUERY_OBJECT_FILE, CMSG_SIGNER_INFO,
        CMSG_SIGNER_INFO_PARAM, CertCloseStore, CertFindCertificateInStore,
        CertFreeCertificateContext, CertGetNameStringW, CryptMsgClose, CryptMsgGetParam,
        CryptQueryObject, HCERTSTORE, PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    };

    let expected = option_env!("SKY_PUBLISHER_SUBJECT").ok_or_else(|| {
        UpdaterError::SignatureInvalid(
            "release updater was built without SKY_PUBLISHER_SUBJECT".into(),
        )
    })?;
    if expected.is_empty() {
        return Err(UpdaterError::SignatureInvalid(
            "SKY_PUBLISHER_SUBJECT is empty".into(),
        ));
    }
    let path_wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut encoding = 0u32;
    let mut content_type = 0u32;
    let mut format_type = 0u32;
    let mut store: HCERTSTORE = std::ptr::null_mut();
    let mut message: *mut c_void = std::ptr::null_mut();
    let mut context: *mut c_void = std::ptr::null_mut();
    let content_flags =
        CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED | CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED;
    let queried = unsafe {
        CryptQueryObject(
            CERT_QUERY_OBJECT_FILE,
            path_wide.as_ptr() as *const c_void,
            content_flags,
            CERT_QUERY_FORMAT_FLAG_ALL,
            0,
            &mut encoding,
            &mut content_type,
            &mut format_type,
            &mut store,
            &mut message,
            &mut context,
        )
    };
    if queried == 0 || store.is_null() || message.is_null() {
        return Err(UpdaterError::SignatureInvalid(format!(
            "signed certificate could not be read: {}",
            path.display()
        )));
    }
    let context = context.cast::<windows_sys::Win32::Security::Cryptography::CERT_CONTEXT>();

    let mut signer_size = 0u32;
    let size_ok = unsafe {
        CryptMsgGetParam(
            message,
            CMSG_SIGNER_INFO_PARAM,
            0,
            std::ptr::null_mut(),
            &mut signer_size,
        )
    } != 0;
    if !size_ok || signer_size as usize != std::mem::size_of::<CMSG_SIGNER_INFO>() {
        unsafe {
            if !context.is_null() {
                CertFreeCertificateContext(context);
            }
            CryptMsgClose(message);
            CertCloseStore(store, 0);
        }
        return Err(UpdaterError::SignatureInvalid(
            "signed message signer information is unavailable".into(),
        ));
    }

    let mut signer_info = std::mem::MaybeUninit::<CMSG_SIGNER_INFO>::zeroed();
    let mut signer_capacity = signer_size;
    let signer_ok = unsafe {
        CryptMsgGetParam(
            message,
            CMSG_SIGNER_INFO_PARAM,
            0,
            signer_info.as_mut_ptr().cast::<c_void>(),
            &mut signer_capacity,
        )
    } != 0;
    if !signer_ok {
        unsafe {
            if !context.is_null() {
                CertFreeCertificateContext(context);
            }
            CryptMsgClose(message);
            CertCloseStore(store, 0);
        }
        return Err(UpdaterError::SignatureInvalid(
            "signed message signer information could not be read".into(),
        ));
    }
    let signer_info = unsafe { signer_info.assume_init() };
    let mut signer_cert_info = CERT_INFO::default();
    signer_cert_info.Issuer = signer_info.Issuer;
    signer_cert_info.SerialNumber = signer_info.SerialNumber;
    let certificate = unsafe {
        CertFindCertificateInStore(
            store,
            X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
            0,
            CERT_FIND_SUBJECT_CERT,
            &signer_cert_info as *const CERT_INFO as *const c_void,
            std::ptr::null(),
        )
    };
    if certificate.is_null() {
        unsafe {
            if !context.is_null() {
                CertFreeCertificateContext(context);
            }
            CryptMsgClose(message);
            CertCloseStore(store, 0);
        }
        return Err(UpdaterError::SignatureInvalid(
            "actual signed-message signer certificate is missing".into(),
        ));
    }

    let mut subject = [0u16; 512];
    let length = unsafe {
        CertGetNameStringW(
            certificate,
            CERT_NAME_RDN_TYPE,
            0,
            std::ptr::null(),
            subject.as_mut_ptr(),
            subject.len() as u32,
        )
    } as usize;
    let actual = if length > 0 && length <= subject.len() {
        String::from_utf16_lossy(&subject[..length.saturating_sub(1)])
    } else {
        String::new()
    };
    unsafe {
        CertFreeCertificateContext(certificate);
        if !context.is_null() {
            CertFreeCertificateContext(context);
        }
        CryptMsgClose(message);
        CertCloseStore(store, 0);
    }
    if actual != expected {
        return Err(UpdaterError::SignatureInvalid(format!(
            "publisher subject mismatch: expected {expected:?}, got {actual:?}"
        )));
    }
    Ok(())
}
