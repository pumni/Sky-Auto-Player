use super::{Result, sha256_bytes};
use serde::Serialize;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_HTTP_HEADER_BYTES: usize = 32 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 512 * 1024 * 1024;
const MAX_HTTP_REQUESTS: usize = 4096;
const MAX_HTTP_REQUEST_DURATION: Duration = Duration::from_secs(10);
const MAX_HTTP_RESPONSE_DURATION: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct RequestEvidence {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) status_code: u16,
    pub(super) content_type: String,
    pub(super) content_length: u64,
    pub(super) body_length: u64,
    pub(super) body_sha256: String,
}

#[derive(Clone, Debug)]
struct FixturePayloads {
    manifest: Vec<u8>,
    installer: Vec<u8>,
}

pub(super) struct FixtureServer {
    address: SocketAddr,
    payloads: Arc<Mutex<FixturePayloads>>,
    requests: Arc<Mutex<Vec<RequestEvidence>>>,
    stop: Arc<AtomicBool>,
    active_stream: Arc<Mutex<Option<TcpStream>>>,
    pub(super) thread: Option<JoinHandle<std::io::Result<()>>>,
}

impl FixtureServer {
    pub(super) fn start(manifest: Vec<u8>, installer: Vec<u8>) -> Result<Self> {
        if manifest.is_empty() || installer.is_empty() {
            return Err("updater fixture responses must be non-empty exact byte arrays".into());
        }
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let payloads = Arc::new(Mutex::new(FixturePayloads {
            manifest,
            installer,
        }));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let active_stream = Arc::new(Mutex::new(None));
        let thread_payloads = Arc::clone(&payloads);
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let thread_active_stream = Arc::clone(&active_stream);
        let server_thread = thread::Builder::new()
            .name("tauri-updater-loopback-fixture".to_owned())
            .spawn(move || {
                while !thread_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _peer)) => {
                            stream.set_nonblocking(false)?;
                            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                            let interrupt_stream = stream.try_clone()?;
                            {
                                let mut active = thread_active_stream.lock().map_err(|_| {
                                    std::io::Error::other("active fixture stream lock was poisoned")
                                })?;
                                *active = Some(interrupt_stream);
                                if thread_stop.load(Ordering::Acquire)
                                    && let Some(active) = active.as_ref()
                                {
                                    let _ = active.shutdown(Shutdown::Both);
                                }
                            }
                            let response = handle_fixture_connection(
                                stream,
                                &thread_payloads,
                                &thread_requests,
                            );
                            {
                                let mut active = thread_active_stream.lock().map_err(|_| {
                                    std::io::Error::other("active fixture stream lock was poisoned")
                                })?;
                                active.take();
                            }
                            if response.is_err() && !thread_stop.load(Ordering::Acquire) {
                                response?;
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => return Err(error),
                    }
                }
                Ok(())
            })?;
        Ok(Self {
            address,
            payloads,
            requests,
            stop,
            active_stream,
            thread: Some(server_thread),
        })
    }

    pub(super) fn port(&self) -> u16 {
        self.address.port()
    }

    #[cfg(windows)]
    pub(super) fn candidate_url(&self) -> String {
        format!("http://127.0.0.1:{}/candidate/update.exe", self.port())
    }

    pub(super) fn set_manifest(&self, manifest: Vec<u8>) -> Result<()> {
        if manifest.is_empty() {
            return Err("updater fixture manifest cannot be empty".into());
        }
        self.payloads
            .lock()
            .map_err(|_| "updater fixture payload lock was poisoned")?
            .manifest = manifest;
        Ok(())
    }

    pub(super) fn requests(&self) -> Result<Vec<RequestEvidence>> {
        Ok(self
            .requests
            .lock()
            .map_err(|_| "updater fixture request lock was poisoned")?
            .clone())
    }

    pub(super) fn stop(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        let mut errors = Vec::new();
        match self.active_stream.lock() {
            Ok(active) => {
                if let Some(stream) = active.as_ref()
                    && let Err(error) = stream.shutdown(Shutdown::Both)
                    && error.kind() != std::io::ErrorKind::NotConnected
                {
                    errors.push(format!("interrupt active fixture connection: {error}"));
                }
            }
            Err(_) => errors.push("active fixture stream lock was poisoned".to_owned()),
        }
        if let Some(handle) = self.thread.take() {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("fixture HTTP thread failed: {error}")),
                Err(_) => errors.push("updater fixture HTTP thread panicked".to_owned()),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "updater fixture server shutdown failed: {}",
                errors.join("; ")
            )
            .into())
        }
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn handle_fixture_connection(
    mut stream: TcpStream,
    payloads: &Arc<Mutex<FixturePayloads>>,
    requests: &Arc<Mutex<Vec<RequestEvidence>>>,
) -> std::io::Result<()> {
    let mut request = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 2048];
    let deadline = Instant::now() + MAX_HTTP_REQUEST_DURATION;
    while request.len() < MAX_HTTP_HEADER_BYTES && !request.windows(4).any(|w| w == b"\r\n\r\n") {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "updater fixture request exceeded its absolute read deadline",
            ));
        }
        stream.set_read_timeout(Some(remaining.min(Duration::from_secs(2))))?;
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
    }
    let request_line = request
        .split(|byte| *byte == b'\n')
        .next()
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .unwrap_or_default();
    let fields = request_line
        .split(|byte| *byte == b' ')
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    let method = fields
        .first()
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .unwrap_or_default();
    let raw_path = fields.get(1).copied().unwrap_or_default();
    let path = String::from_utf8_lossy(raw_path)
        .split('?')
        .next()
        .unwrap_or_default()
        .to_owned();
    let (status, status_text, content_type, body) = if method != "GET" {
        (
            405,
            "Method Not Allowed",
            "text/plain",
            b"method not allowed".to_vec(),
        )
    } else if request.len() >= MAX_HTTP_HEADER_BYTES
        && !request.windows(4).any(|w| w == b"\r\n\r\n")
    {
        (
            431,
            "Request Header Fields Too Large",
            "text/plain",
            b"request too large".to_vec(),
        )
    } else if path == "/stable" || path == "/beta" {
        let body = payloads
            .lock()
            .map_err(|_| std::io::Error::other("fixture payload lock was poisoned"))?
            .manifest
            .clone();
        (200, "OK", "application/json", body)
    } else if path == "/candidate/update.exe" {
        let body = payloads
            .lock()
            .map_err(|_| std::io::Error::other("fixture payload lock was poisoned"))?
            .installer
            .clone();
        (200, "OK", "application/octet-stream", body)
    } else {
        (404, "Not Found", "text/plain", b"not found".to_vec())
    };
    let evidence = RequestEvidence {
        method,
        path,
        status_code: status,
        content_type: content_type.to_owned(),
        content_length: body.len() as u64,
        body_length: body.len() as u64,
        body_sha256: sha256_bytes(&body),
    };
    {
        let mut log = requests
            .lock()
            .map_err(|_| std::io::Error::other("fixture request lock was poisoned"))?;
        if log.len() >= MAX_HTTP_REQUESTS {
            return Err(std::io::Error::other(
                "updater fixture request limit exceeded",
            ));
        }
        log.push(evidence);
    }
    let response_header = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    let response_deadline = Instant::now() + MAX_HTTP_RESPONSE_DURATION;
    write_all_before_deadline(
        &mut stream,
        response_header.as_bytes(),
        response_deadline,
        |stream, timeout| stream.set_write_timeout(Some(timeout)),
    )?;
    write_all_before_deadline(&mut stream, &body, response_deadline, |stream, timeout| {
        stream.set_write_timeout(Some(timeout))
    })?;
    let _ = stream.shutdown(Shutdown::Write);
    Ok(())
}

pub(super) fn write_all_before_deadline<W: Write>(
    writer: &mut W,
    bytes: &[u8],
    deadline: Instant,
    mut set_timeout: impl FnMut(&mut W, Duration) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "updater fixture response exceeded its absolute write deadline",
            ));
        }
        set_timeout(writer, remaining.min(Duration::from_secs(2)))?;
        match writer.write(&bytes[written..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "updater fixture response writer accepted zero bytes",
                ));
            }
            Ok(count) => {
                written += count;
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "updater fixture response exceeded its absolute write deadline",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn verify_exact_http_source(
    server: &FixtureServer,
    path: &str,
    expected: &[u8],
    content_type: &str,
) -> Result<()> {
    let url = format!("http://127.0.0.1:{}{path}", server.port());
    let response = http_get_loopback(&url)?;
    if response.status != 200
        || response.content_type != content_type
        || response.body != expected
        || response.content_length != expected.len() as u64
        || sha256_bytes(&response.body) != sha256_bytes(expected)
    {
        return Err(
            format!("loopback {path} response differs from exact fixture source bytes").into(),
        );
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct HttpResponse {
    pub(super) status: u16,
    pub(super) content_type: String,
    pub(super) content_length: u64,
    pub(super) body: Vec<u8>,
}

pub(super) fn http_get_loopback(url: &str) -> Result<HttpResponse> {
    let parsed = url::Url::parse(url)?;
    if parsed.scheme() != "http"
        || parsed.host_str() != Some("127.0.0.1")
        || parsed.port().is_none()
    {
        return Err("fixture HTTP client accepts only an explicit 127.0.0.1 endpoint".into());
    }
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), parsed.port().unwrap()),
        Duration::from_secs(2),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    write!(
        stream,
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        parsed.path(),
        parsed.port().unwrap()
    )?;
    stream.flush()?;
    let mut bytes = Vec::with_capacity(4096);
    let mut chunk = [0_u8; 4096];
    let separator = loop {
        if let Some(separator) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break separator;
        }
        if bytes.len() >= MAX_HTTP_HEADER_BYTES {
            return Err("loopback fixture response header exceeds its byte bound".into());
        }
        let remaining = MAX_HTTP_HEADER_BYTES + 4 - bytes.len();
        let read_limit = remaining.min(chunk.len());
        let read = stream.read(&mut chunk[..read_limit])?;
        if read == 0 {
            return Err("loopback fixture returned an incomplete HTTP response header".into());
        }
        bytes.extend_from_slice(&chunk[..read]);
    };
    if separator > MAX_HTTP_HEADER_BYTES {
        return Err("loopback fixture response header exceeds its byte bound".into());
    }
    let header = std::str::from_utf8(&bytes[..separator])?;
    let mut lines = header.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .ok_or("loopback fixture returned an invalid HTTP status line")?
        .parse::<u16>()?;
    let mut content_type = None;
    let mut content_length = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-type") {
                if content_type.is_some() {
                    return Err("fixture response has duplicate Content-Type headers".into());
                }
                content_type = Some(value.trim().to_owned());
            } else if name.eq_ignore_ascii_case("content-length") {
                if content_length.is_some() {
                    return Err("fixture response has duplicate Content-Length headers".into());
                }
                let parsed = value.trim().parse::<u64>()?;
                if parsed > MAX_HTTP_BODY_BYTES as u64 {
                    return Err("fixture response body exceeds its byte bound".into());
                }
                content_length = Some(parsed);
            }
        }
    }
    let content_length = content_length.ok_or("fixture response has no Content-Length")?;
    let body_start = separator + 4;
    let mut body = bytes[body_start..].to_vec();
    if body.len() as u64 > content_length {
        return Err("loopback fixture returned bytes beyond Content-Length".into());
    }
    while (body.len() as u64) < content_length {
        let remaining = (content_length - body.len() as u64) as usize;
        let read_limit = remaining.min(chunk.len());
        let read = stream.read(&mut chunk[..read_limit]).map_err(|error| {
            format!(
                "loopback fixture body length mismatch: header={content_length}, received={}; read failed: {error}",
                body.len()
            )
        })?;
        if read == 0 {
            return Err(format!(
                "loopback fixture body length mismatch: header={content_length}, received={}",
                body.len()
            )
            .into());
        }
        body.extend_from_slice(&chunk[..read]);
    }
    if body.len() as u64 != content_length {
        return Err(format!(
            "loopback fixture body length mismatch: header={content_length}, received={}",
            body.len()
        )
        .into());
    }
    Ok(HttpResponse {
        status,
        content_type: content_type.ok_or("fixture response has no Content-Type")?,
        content_length,
        body,
    })
}
