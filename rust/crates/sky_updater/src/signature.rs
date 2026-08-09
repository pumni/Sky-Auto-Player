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
    verify_publisher_subject(path, expected)?;
    Ok(())
}

#[cfg(any(all(windows, not(debug_assertions)), all(test, windows)))]
struct SignedMessageResources {
    store: windows_sys::Win32::Security::Cryptography::HCERTSTORE,
    message: *mut std::ffi::c_void,
    context: *const windows_sys::Win32::Security::Cryptography::CERT_CONTEXT,
}

#[cfg(any(all(windows, not(debug_assertions)), all(test, windows)))]
impl Drop for SignedMessageResources {
    fn drop(&mut self) {
        use windows_sys::Win32::Security::Cryptography::{
            CertCloseStore, CertFreeCertificateContext, CryptMsgClose,
        };

        unsafe {
            if !self.context.is_null() {
                CertFreeCertificateContext(self.context);
            }
            if !self.message.is_null() {
                CryptMsgClose(self.message);
            }
            if !self.store.is_null() {
                CertCloseStore(self.store, 0);
            }
        }
    }
}

#[cfg(any(all(windows, not(debug_assertions)), all(test, windows)))]
struct CertificateResource(*const windows_sys::Win32::Security::Cryptography::CERT_CONTEXT);

#[cfg(any(all(windows, not(debug_assertions)), all(test, windows)))]
impl Drop for CertificateResource {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::Security::Cryptography::CertFreeCertificateContext(self.0);
            }
        }
    }
}

#[cfg(any(all(windows, not(debug_assertions)), all(test, windows)))]
fn verify_publisher_subject(path: &Path, expected: &str) -> Result<()> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::Cryptography::{
        CERT_FIND_SUBJECT_CERT, CERT_INFO, CERT_NAME_RDN_TYPE,
        CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED, CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED,
        CERT_QUERY_FORMAT_FLAG_ALL, CERT_QUERY_OBJECT_FILE, CERT_X500_NAME_STR,
        CMSG_SIGNER_CERT_INFO_PARAM, CertFindCertificateInStore, CertGetNameStringW,
        CryptMsgGetParam, CryptQueryObject, HCERTSTORE, PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    };

    const MAX_SIGNER_CERT_INFO_BYTES: usize = 64 * 1024;

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
    let resources = SignedMessageResources {
        store,
        message,
        context: context.cast(),
    };
    if queried == 0 || resources.store.is_null() || resources.message.is_null() {
        return Err(UpdaterError::SignatureInvalid(format!(
            "signed certificate could not be read: {}",
            path.display()
        )));
    }

    let mut signer_size = 0u32;
    let size_ok = unsafe {
        CryptMsgGetParam(
            resources.message,
            CMSG_SIGNER_CERT_INFO_PARAM,
            0,
            std::ptr::null_mut(),
            &mut signer_size,
        )
    } != 0;
    if !size_ok
        || signer_size == 0
        || signer_size as usize > MAX_SIGNER_CERT_INFO_BYTES
        || (signer_size as usize) < std::mem::size_of::<CERT_INFO>()
    {
        return Err(UpdaterError::SignatureInvalid(
            "signed message signer certificate information is unavailable".into(),
        ));
    }

    // CMSG_SIGNER_CERT_INFO_PARAM returns a variable-length BYTE buffer that
    // contains a CERT_INFO. Vec<usize> provides sufficient alignment for the
    // CERT_INFO view while preserving the exact byte-size contract.
    let word_size = std::mem::size_of::<usize>();
    let word_count = (signer_size as usize).div_ceil(word_size);
    let mut signer_buffer = vec![0usize; word_count];
    let mut signer_capacity = (signer_buffer.len() * word_size) as u32;
    let signer_ok = unsafe {
        CryptMsgGetParam(
            resources.message,
            CMSG_SIGNER_CERT_INFO_PARAM,
            0,
            signer_buffer.as_mut_ptr().cast::<c_void>(),
            &mut signer_capacity,
        )
    } != 0;
    if !signer_ok {
        return Err(UpdaterError::SignatureInvalid(
            "signed message signer certificate information could not be read".into(),
        ));
    }
    if (signer_capacity as usize) < std::mem::size_of::<CERT_INFO>() {
        return Err(UpdaterError::SignatureInvalid(
            "signed message signer certificate information is truncated".into(),
        ));
    }
    let signer_info = unsafe { &*signer_buffer.as_ptr().cast::<CERT_INFO>() };
    let signer_cert_info = CERT_INFO {
        Issuer: signer_info.Issuer,
        SerialNumber: signer_info.SerialNumber,
        ..Default::default()
    };
    let certificate = unsafe {
        CertFindCertificateInStore(
            resources.store,
            X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
            0,
            CERT_FIND_SUBJECT_CERT,
            &signer_cert_info as *const CERT_INFO as *const c_void,
            std::ptr::null(),
        )
    };
    if certificate.is_null() {
        return Err(UpdaterError::SignatureInvalid(
            "actual signed-message signer certificate is missing".into(),
        ));
    }
    let _certificate = CertificateResource(certificate);

    // CERT_NAME_RDN_TYPE requires pvTypePara to point to a DWORD containing
    // the CertNameToStr format. X500 output is deterministic for the policy
    // value supplied by SKY_PUBLISHER_SUBJECT.
    let name_format = CERT_X500_NAME_STR;
    let name_format_ptr = &name_format as *const _ as *const c_void;
    let required = unsafe {
        CertGetNameStringW(
            _certificate.0,
            CERT_NAME_RDN_TYPE,
            0,
            name_format_ptr,
            std::ptr::null_mut(),
            0,
        )
    } as usize;
    if required == 0 {
        return Err(UpdaterError::SignatureInvalid(
            "publisher identity could not be read".into(),
        ));
    }
    let mut subject = vec![0u16; required];
    let length = unsafe {
        CertGetNameStringW(
            _certificate.0,
            CERT_NAME_RDN_TYPE,
            0,
            name_format_ptr,
            subject.as_mut_ptr(),
            subject.len() as u32,
        )
    } as usize;
    if length == 0 || length > subject.len() {
        return Err(UpdaterError::SignatureInvalid(
            "publisher identity could not be decoded".into(),
        ));
    }
    let actual = String::from_utf16_lossy(&subject[..length.saturating_sub(1)]);
    if actual != expected {
        return Err(UpdaterError::SignatureInvalid(format!(
            "publisher subject mismatch: expected {expected:?}, got {actual:?}"
        )));
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use std::path::Path;

    #[test]
    fn publisher_subject_cryptoapi_fixture() {
        let Ok(path) = std::env::var("SKY_AUTHENTICODE_TEST_PE") else {
            return;
        };
        let expected = std::env::var("SKY_AUTHENTICODE_TEST_PUBLISHER")
            .expect("SKY_AUTHENTICODE_TEST_PUBLISHER must accompany the fixture");
        super::verify_publisher_subject(Path::new(&path), &expected)
            .expect("CryptoAPI should resolve the actual Authenticode signer");
    }
}
