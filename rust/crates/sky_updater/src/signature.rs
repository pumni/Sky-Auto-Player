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
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }
    #[repr(C)]
    struct WinTrustFileInfo {
        cb_struct: u32,
        pcwsz_file_path: *const u16,
        h_file: *mut c_void,
        pg_known_subject: *const Guid,
    }
    #[repr(C)]
    struct WinTrustData {
        cb_struct: u32,
        p_policy_callback_data: *mut c_void,
        p_sip_client_data: *mut c_void,
        dw_ui_choice: u32,
        fdw_revocation_checks: u32,
        dw_union_choice: u32,
        p_file: *mut WinTrustFileInfo,
        dw_state_action: u32,
        h_wvt_state_data: *mut c_void,
        pwsz_url_reference: *mut u16,
        dw_prov_flags: u32,
        dw_ui_context: u32,
    }
    unsafe extern "system" {
        fn WinVerifyTrust(
            hwnd: *mut c_void,
            action_id: *const Guid,
            data: *mut WinTrustData,
        ) -> i32;
    }

    let path_wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let action = Guid {
        data1: 0x00AAC56B,
        data2: 0xCD44,
        data3: 0x11D0,
        data4: [0x8C, 0xC2, 0x00, 0xC0, 0x4F, 0xC2, 0x95, 0xEE],
    };
    let mut file_info = WinTrustFileInfo {
        cb_struct: std::mem::size_of::<WinTrustFileInfo>() as u32,
        pcwsz_file_path: path_wide.as_ptr(),
        h_file: std::ptr::null_mut(),
        pg_known_subject: std::ptr::null(),
    };
    let mut data = WinTrustData {
        cb_struct: std::mem::size_of::<WinTrustData>() as u32,
        p_policy_callback_data: std::ptr::null_mut(),
        p_sip_client_data: std::ptr::null_mut(),
        dw_ui_choice: 2,
        fdw_revocation_checks: 1,
        dw_union_choice: 1,
        p_file: &mut file_info,
        dw_state_action: 1,
        h_wvt_state_data: std::ptr::null_mut(),
        pwsz_url_reference: std::ptr::null_mut(),
        dw_prov_flags: 0,
        dw_ui_context: 0,
    };
    let status = unsafe { WinVerifyTrust(std::ptr::null_mut(), &action, &mut data) };
    data.dw_state_action = 2;
    let _ = unsafe { WinVerifyTrust(std::ptr::null_mut(), &action, &mut data) };
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
        CERT_NAME_RDN_TYPE, CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED,
        CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED, CERT_QUERY_FORMAT_FLAG_ALL,
        CERT_QUERY_OBJECT_FILE, CertCloseStore, CertEnumCertificatesInStore,
        CertFreeCertificateContext, CertGetNameStringW, CryptQueryObject, HCERTSTORE,
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
    let mut message = std::ptr::null_mut();
    let mut context = std::ptr::null_mut();
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
    if queried == 0 || store.is_null() {
        return Err(UpdaterError::SignatureInvalid(format!(
            "signed certificate could not be read: {}",
            path.display()
        )));
    }
    let certificate = unsafe { CertEnumCertificatesInStore(store, std::ptr::null()) };
    if certificate.is_null() {
        unsafe { CertCloseStore(store, 0) };
        return Err(UpdaterError::SignatureInvalid(
            "signed certificate is missing".into(),
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
        CertCloseStore(store, 0);
    }
    if actual != expected {
        return Err(UpdaterError::SignatureInvalid(format!(
            "publisher subject mismatch: expected {expected:?}, got {actual:?}"
        )));
    }
    Ok(())
}
