use std::fs::File;
use std::io::Write;
use std::path::Path;

use crate::error::{Result, UpdaterError};

pub const ALLOWED_HOSTS: [&str; 4] = [
    "api.github.com",
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

pub trait HttpClient {
    fn get(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>>;

    fn download_to(&self, url: &str, max_bytes: usize, destination: &Path) -> Result<()> {
        let bytes = self.get(url, max_bytes)?;
        let mut file = File::create(destination)?;
        file.write_all(&bytes)?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WinHttpClient;

pub fn validate_https_url(url: &str) -> Result<()> {
    let (host, _path) = split_https_url(url)?;
    if !ALLOWED_HOSTS.contains(&host) {
        return Err(UpdaterError::RedirectRejected(format!(
            "host is not allow-listed: {host}"
        )));
    }
    Ok(())
}

fn split_https_url(url: &str) -> Result<(&str, &str)> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| UpdaterError::RedirectRejected("HTTPS is required".into()))?;
    let (authority, path) = rest
        .split_once('/')
        .ok_or_else(|| UpdaterError::RedirectRejected("URL path is missing".into()))?;
    if authority.is_empty()
        || authority.contains('@')
        || authority.contains(':')
        || authority.chars().any(char::is_whitespace)
    {
        return Err(UpdaterError::RedirectRejected(
            "URL authority is unsafe".into(),
        ));
    }
    if path.is_empty() || path.contains('#') {
        return Err(UpdaterError::RedirectRejected("URL path is unsafe".into()));
    }
    Ok((authority, path))
}

#[cfg(windows)]
impl HttpClient for WinHttpClient {
    fn get(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>> {
        request(url, max_bytes, None)
    }

    fn download_to(&self, url: &str, max_bytes: usize, destination: &Path) -> Result<()> {
        let mut file = File::create(destination)?;
        request(url, max_bytes, Some(&mut file))?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    }
}

#[cfg(windows)]
fn request(url: &str, max_bytes: usize, mut sink: Option<&mut File>) -> Result<Vec<u8>> {
    use std::ffi::c_void;
    use windows_sys::Win32::Networking::WinHttp::{
        HTTP_STATUS_MOVED, HTTP_STATUS_PERMANENT_REDIRECT, HTTP_STATUS_REDIRECT,
        HTTP_STATUS_REDIRECT_KEEP_VERB, HTTP_STATUS_REDIRECT_METHOD,
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_OPTION_REDIRECT_POLICY,
        WINHTTP_OPTION_REDIRECT_POLICY_NEVER, WINHTTP_QUERY_LOCATION, WinHttpConnect, WinHttpOpen,
        WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpReadData, WinHttpReceiveResponse,
        WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
    };

    let mut current = url.to_string();
    for _ in 0..5 {
        validate_https_url(&current)?;
        let (host, path) = split_https_url(&current)?;
        let agent = wide("Sky-Auto-Player-Updater/1");
        let host_wide = wide(host);
        let path_wide = wide(&format!("/{path}"));
        let verb = wide("GET");
        unsafe {
            let session = WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                std::ptr::null(),
                std::ptr::null(),
                0,
            );
            if session.is_null() {
                return win_error("WinHttpOpen");
            }
            let _session_guard = HandleGuard(session);
            if WinHttpSetTimeouts(session, 10_000, 10_000, 10_000, 10_000) == 0 {
                return win_error("WinHttpSetTimeouts");
            }
            let connection = WinHttpConnect(session, host_wide.as_ptr(), 443, 0);
            if connection.is_null() {
                return win_error("WinHttpConnect");
            }
            let _connection_guard = HandleGuard(connection);
            let request = WinHttpOpenRequest(
                connection,
                verb.as_ptr(),
                path_wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            );
            if request.is_null() {
                return win_error("WinHttpOpenRequest");
            }
            let _request_guard = HandleGuard(request);
            let mut redirect_policy = WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
            if WinHttpSetOption(
                request,
                WINHTTP_OPTION_REDIRECT_POLICY,
                &mut redirect_policy as *mut _ as *mut c_void,
                std::mem::size_of_val(&redirect_policy) as u32,
            ) == 0
            {
                return win_error("WinHttpSetOption");
            }
            if WinHttpSendRequest(request, std::ptr::null(), 0, std::ptr::null_mut(), 0, 0, 0) == 0
            {
                return win_error("WinHttpSendRequest");
            }
            if WinHttpReceiveResponse(request, std::ptr::null_mut()) == 0 {
                return win_error("WinHttpReceiveResponse");
            }
            let status = query_status(request)?;
            if [
                HTTP_STATUS_MOVED,
                HTTP_STATUS_REDIRECT,
                HTTP_STATUS_REDIRECT_METHOD,
                HTTP_STATUS_REDIRECT_KEEP_VERB,
                HTTP_STATUS_PERMANENT_REDIRECT,
            ]
            .contains(&status)
            {
                current = query_header(request, WINHTTP_QUERY_LOCATION)?;
                validate_https_url(&current)?;
                continue;
            }
            if !(200..300).contains(&status) {
                return Err(UpdaterError::NetworkFailure(format!(
                    "HTTP status {status}"
                )));
            }
            let mut output = Vec::new();
            let mut total = 0usize;
            loop {
                let mut available = 0u32;
                if WinHttpQueryDataAvailable(request, &mut available) == 0 {
                    return win_error("WinHttpQueryDataAvailable");
                }
                if available == 0 {
                    break;
                }
                let remaining = max_bytes.saturating_sub(total);
                if remaining == 0 {
                    return Err(UpdaterError::NetworkFailure(
                        "response exceeds size bound".into(),
                    ));
                }
                let requested = (available as usize).min(remaining).min(64 * 1024) as u32;
                let mut buffer = vec![0u8; requested as usize];
                let mut read = 0u32;
                if WinHttpReadData(
                    request,
                    buffer.as_mut_ptr() as *mut c_void,
                    requested,
                    &mut read,
                ) == 0
                {
                    return win_error("WinHttpReadData");
                }
                if read == 0 {
                    break;
                }
                total = total.saturating_add(read as usize);
                if let Some(file) = sink.as_deref_mut() {
                    file.write_all(&buffer[..read as usize])?;
                } else {
                    output.extend_from_slice(&buffer[..read as usize]);
                }
            }
            return Ok(output);
        }
    }
    Err(UpdaterError::RedirectRejected(
        "redirect limit exceeded".into(),
    ))
}

#[cfg(windows)]
unsafe fn query_status(request: *mut std::ffi::c_void) -> Result<u32> {
    use windows_sys::Win32::Networking::WinHttp::{
        WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpQueryHeaders,
    };
    let mut status = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    if unsafe {
        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_FLAG_NUMBER | WINHTTP_QUERY_STATUS_CODE,
            std::ptr::null(),
            &mut status as *mut _ as *mut _,
            &mut size,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return win_error("WinHttpQueryHeaders");
    }
    Ok(status)
}

#[cfg(windows)]
unsafe fn query_header(request: *mut std::ffi::c_void, header: u32) -> Result<String> {
    use windows_sys::Win32::Networking::WinHttp::WinHttpQueryHeaders;
    let mut buffer = [0u16; 2048];
    let mut size = (buffer.len() * std::mem::size_of::<u16>()) as u32;
    if unsafe {
        WinHttpQueryHeaders(
            request,
            header,
            std::ptr::null(),
            buffer.as_mut_ptr() as *mut _,
            &mut size,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return win_error("WinHttpQueryHeaders");
    }
    let length = (size as usize / std::mem::size_of::<u16>()).min(buffer.len());
    let value = String::from_utf16(&buffer[..length])
        .map_err(|_| UpdaterError::RedirectRejected("redirect location is not UTF-16".into()))?
        .trim_matches('\0')
        .trim()
        .to_owned();
    if value.is_empty() {
        return Err(UpdaterError::RedirectRejected(
            "redirect location is empty".into(),
        ));
    }
    Ok(value)
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn win_error<T>(operation: &str) -> Result<T> {
    Err(UpdaterError::NetworkFailure(format!("{operation} failed")))
}

#[cfg(windows)]
struct HandleGuard(*mut std::ffi::c_void);

#[cfg(windows)]
impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Networking::WinHttp::WinHttpCloseHandle(self.0);
        }
    }
}

#[cfg(not(windows))]
impl HttpClient for WinHttpClient {
    fn get(&self, _url: &str, _max_bytes: usize) -> Result<Vec<u8>> {
        Err(UpdaterError::NetworkFailure(
            "native updater requires Windows".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_https_and_untrusted_hosts() {
        assert!(validate_https_url("http://api.github.com/a").is_err());
        assert!(validate_https_url("https://example.com/a").is_err());
        assert!(validate_https_url("https://api.github.com/a").is_ok());
        assert!(validate_https_url("https://api.github.com@evil.example/a").is_err());
    }
}
