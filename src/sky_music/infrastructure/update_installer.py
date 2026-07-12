"""Windows-side self-update installer.

This module performs the concrete steps of downloading, verifying, staging, and
applying an update of Sky Player itself. It isolates all OS-specific code here
behind small, testable functions:

1. ``download_zip`` — stream a release zip into a temp file.
2. ``extract_zip`` — unzip into a staging directory beside the install.
3. ``verify_sha256`` / ``compute_sha256``/ ``parse_sha256_sidecar`` — verify a
   downloaded file's SHA256 against a checksum asset (either bare hash or the
   standard Coreutils ``<hash>  <filename>`` sidecar form).
4. ``write_apply_batch`` / ``apply_update_and_restart`` — emit a detached
   ``.cmd`` script that performs an atomic ``robocopy /MOVE`` of the staged
   tree over the install tree, then relaunches the exe, then deletes itself.

Security notes
--------------
- Only the Sky Player install tree is touched. There is no interaction with the
  Sky game process, game files, game memory, or anti-cheat — the
  ``SECURITY_MANDATES`` in ``AGENTS.md`` forbid those things and this module
  honors them.
- HTTPS is mandatory for downloads; HTTP URLs are rejected to mitigate MITM.
- SHA256 verification guards the downloaded zip against transport corruption
  or donor tampering; if the sidecar is missing or mismatches, the caller is
  expected to refuse the update.
- The apply batch uses ``robocopy`` (Windows native) with ``/MOVE`` to swap the
  install atomically once the current process exits.
"""

from __future__ import annotations

import contextlib
import hashlib
import re
import shutil
import subprocess
import sys
import tempfile
import uuid
import zipfile
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any, NoReturn
from urllib.request import Request, urlopen

from sky_music.domain.update_checker import UpdateInfo

# Default chunk size for streaming downloads. 64 KiB balances syscall overhead
# against memory pressure under free-threaded CPython.
_DOWNLOAD_CHUNK: int = 64 * 1024
_HASH_CHUNK: int = 65536
_BATCH_PING_WAIT_S: int = 2  # `ping -n 3` sleeps ~2s on Windows


class UpdateInstallerError(RuntimeError):
    """Raised for unrecoverable install-side errors."""


@dataclass(frozen=True, slots=True)
class StagedUpdate:
    """Result of a successful download/extract run prior to apply."""

    staging_dir: Path
    new_version: str


def _urlopen_default(url: str | Request, *, timeout: float) -> Any:
    if isinstance(url, str):
        url = Request(url, headers={"User-Agent": "sky-player-update-installer"})
    return urlopen(url, timeout=timeout)


def download_zip(
    url: str,
    *,
    dest_dir: Path,
    timeout: float = 30.0,
    progress: Callable[[int, int | None], None] | None = None,
    opener: Callable[[str], Any] | None = None,
) -> Path:
    """Download ``url`` to ``dest_dir/sky-update-{uuid}.zip`` and return the path.

    Streaming download never holds the whole file in memory. ``progress`` is
    optional and called as ``progress(bytes_downloaded, total_or_None)``; total
    is read from the ``Content-Length`` header when present.

    Validates inputs strictly:
    - ``url`` must be an ``https://`` URL (reject plain HTTP to mitigate MITM).
    - ``dest_dir`` must exist and be a directory.
    """
    if not isinstance(url, str) or not url.lower().startswith("https://"):
        raise UpdateInstallerError(f"refusing non-https url: {url!r}")
    if not dest_dir.exists() or not dest_dir.is_dir():
        raise UpdateInstallerError(f"destination is not a directory: {dest_dir}")

    open_url = opener if opener is not None else _urlopen_default
    try:
        with open_url(url, timeout=timeout) as response:  # type: ignore[arg-type]
            total: int | None = None
            length_raw = response.headers.get("Content-Length")  # type: ignore[union-attr]
            if isinstance(length_raw, str) and length_raw.isdigit():
                total = int(length_raw)
            dest = dest_dir / f"sky-update-{uuid.uuid4().hex}.zip"
            downloaded = 0
            with dest.open("wb") as f:
                while True:
                    chunk = response.read(_DOWNLOAD_CHUNK)  # type: ignore[union-attr]
                    if not chunk:
                        break
                    f.write(chunk)
                    downloaded += len(chunk)
                    if progress is not None:
                        progress(downloaded, total)
            return dest
    except UpdateInstallerError:
        raise
    except Exception as exc:
        raise UpdateInstallerError(f"download failed: {exc}") from exc


def compute_sha256(path: Path) -> str:
    """Return lowercase-hex SHA256 of ``path``."""
    h = hashlib.sha256()
    with path.open("rb") as f:
        while True:
            buf = f.read(_HASH_CHUNK)
            if not buf:
                break
            h.update(buf)
    return h.hexdigest()


def verify_sha256(file_path: Path, expected_sha256: str) -> bool:
    """True iff SHA256 of ``file_path`` equals ``expected_sha256`` (case-insensitive)."""
    if not file_path.exists():
        return False
    if not isinstance(expected_sha256, str) or not expected_sha256:
        return False
    actual = compute_sha256(file_path)
    return actual.lower() == expected_sha256.strip().lower()


_SHA256_LINE_RE = re.compile(
    r"^\s*([0-9a-fA-F]{64})\s+\*?[^\r\n]*$",
    re.MULTILINE,
)


def parse_sha256_sidecar(text: str) -> str | None:
    """Parse a SHA256 checksum sidecar text blob.

    Accepts both a bare hash on one line and the standard Coreutils form
    ``<hash>  <filename>`` (matching ``.sha256`` files generated by
    ``Get-FileHash`` or ``shasum -a 256``).
    """
    if not isinstance(text, str) or not text.strip():
        return None
    first_line = text.strip().splitlines()[0]
    bare = first_line.strip().split()[0] if first_line.strip() else ""
    if len(bare) == 64 and all(c in "0123456789abcdefABCDEF" for c in bare):
        return bare.lower()
    m = _SHA256_LINE_RE.search(text)
    if m:
        return m.group(1).lower()
    return None


def extract_zip(zip_path: Path, dest_dir: Path) -> Path:
    """Extract ``zip_path`` into ``dest_dir/`` (created if missing) and return ``dest_dir``.

    Safeguards:
    - Refuses entries with absolute paths or ``..`` to avoid zip-slip.
    - Reports a single exception with the offending entry name.
    """
    if not zip_path.exists():
        raise UpdateInstallerError(f"missing zip: {zip_path}")
    dest_dir.mkdir(parents=True, exist_ok=True)
    base = dest_dir.resolve()
    with zipfile.ZipFile(zip_path) as zf:
        for info in zf.infolist():
            if info.is_dir():
                continue
            target = (dest_dir / info.filename).resolve()
            if base != target and base not in target.parents:
                raise UpdateInstallerError(f"zip-slip blocked: {info.filename!r}")
        zf.extractall(dest_dir)
    return dest_dir


def fetch_sha256_sidecar(
    sha256_url: str,
    *,
    timeout: float = 10.0,
    opener: Callable[[str], Any] | None = None,
) -> str | None:
    """Fetch and parse a SHA256 checksum sidecar URL.

    Returns the hex digest (lowercase) or ``None`` if not parseable / missing.
    """
    if not sha256_url:
        return None
    open_url = opener if opener is not None else _urlopen_default
    try:
        with open_url(sha256_url, timeout=timeout) as response:  # type: ignore[arg-type]
            raw = response.read()
    except Exception:
        return None
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        return None
    return parse_sha256_sidecar(text)


def stage_update(
    release: UpdateInfo,
    *,
    staging_parent: Path,
    timeout: float = 30.0,
    opener: Callable[[str], Any] | None = None,
    sha256_sum: str | None = None,
) -> StagedUpdate:
    """Download + (optionally) verify + extract an update to a staging dir.

    Returns ``StagedUpdate`` ready for :func:`apply_update_and_restart`. The
    staging directory is unique per call (uuid suffix) and is cleaned up if any
    step fails before extraction completes successfully.

    ``sha256_sum`` (when provided) is compared against the downloaded zip's
    SHA256; mismatch raises :class:`UpdateInstallerError`.
    """
    if not release.download_url:
        raise UpdateInstallerError("release has no downloadable asset")
    if not staging_parent.exists():
        staging_parent.mkdir(parents=True, exist_ok=True)
    staging_dir = staging_parent / f"sky-pending-{uuid.uuid4().hex}"
    staging_dir.mkdir(parents=True, exist_ok=True)

    try:
        zip_path = download_zip(
            release.download_url,
            dest_dir=staging_dir,
            timeout=timeout,
            opener=opener,
        )
        # SHA256 verification guards against transport corruption and donor
        # tampering; verify_sha256 returns False on missing file or empty sum.
        if sha256_sum and not verify_sha256(zip_path, sha256_sum):
            raise UpdateInstallerError("sha256 mismatch — refusing to stage")
        extract_zip(zip_path, staging_dir)
        with contextlib.suppress(OSError):
            zip_path.unlink()
    except Exception:
        shutil.rmtree(staging_dir, ignore_errors=True)
        raise
    return StagedUpdate(staging_dir=staging_dir, new_version=release.latest_version)


def _quote(p: Path) -> str:
    """Windows-safe double-quote for paths in .cmd scripts."""
    s = str(p)
    if '"' in s:
        raise UpdateInstallerError(f"refusing path with quote: {s!r}")
    return f'"{s}"'


def write_apply_batch(
    *,
    staging_dir: Path,
    install_dir: Path,
    post_update_flag: Path,
    batch_path: Path,
) -> Path:
    """Emit a detached ``.cmd`` that performs the atomic swap + restart.

    The script:
      1. ``ping 127.0.0.1 -n 3`` — wait ~2s for the current exe to exit.
      2. ``robocopy <staging> <install> /E /MOVE /R:2 /W:1 > NUL`` — move new
         files over old, retrying twice with 1s on transient locks.
      3. Touch ``post_update_flag`` so the next launch shows a success toast.
      4. ``start "" "<install-dir>/Sky-Player.exe"`` — relaunch detached.
      5. ``(goto) 2>nul & del <self>`` — the batch deletes itself.
    """
    exe_path = install_dir / "Sky-Player.exe"
    batch_lines = [
        "@echo off",
        f"ping 127.0.0.1 -n {_BATCH_PING_WAIT_S + 1} > NUL",
        f"robocopy {_quote(staging_dir)} {_quote(install_dir)} /E /MOVE /R:2 /W:1 > NUL",
        f"if not exist {_quote(post_update_flag)} type nul > {_quote(post_update_flag)}",
    ]
    if exe_path.exists():
        batch_lines.append(f"start \"\" {_quote(exe_path)}")
    batch_lines.append("(goto) 2>nul & del \"%~f0\"")
    # Write in binary mode so we control the exact line endings (\r\n) without
    # Python's text-mode newline translation doubling ``\r`` on Windows.
    batch_path.write_bytes(("\r\n".join(batch_lines) + "\r\n").encode("ascii"))
    return batch_path


def apply_update_and_restart(
    *,
    staging_dir: Path,
    install_dir: Path,
    post_update_flag: Path,
) -> NoReturn:
    """Write the apply batch, launch it detached, and exit the current process.

    IMPORTANT: This call does not return. Invoke it as the final step after
    Textual UI teardown and config flush; once it runs, ``sys.exit(0)`` is
    invoked and the batch takes over the install swap.
    """
    if sys.platform != "win32":
        raise UpdateInstallerError("apply_update_and_restart is Windows-only")
    if not install_dir.exists():
        raise UpdateInstallerError(f"install dir missing: {install_dir}")

    batch_dir = Path(tempfile.gettempdir())
    batch_name = f"sky-apply-{uuid.uuid4().hex}.cmd"
    batch_path = batch_dir / batch_name
    write_apply_batch(
        staging_dir=staging_dir,
        install_dir=install_dir,
        post_update_flag=post_update_flag,
        batch_path=batch_path,
    )
    # CREATE_NEW_PROCESS_GROUP so Ctrl+C in the parent terminal doesn't kill
    # the application batch; DETACHED_PROCESS so it survives the parent exit.
    DETACHED_PROCESS = 0x00000008
    CREATE_NEW_PROCESS_GROUP = 0x00000200
    try:
        subprocess.Popen(
            ["cmd.exe", "/c", str(batch_path)],
            cwd=str(install_dir),
            creationflags=DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP,
            close_fds=True,
        )
    except Exception as exc:
        raise UpdateInstallerError(f"failed to launch apply batch: {exc}") from exc
    sys.exit(0)


def install_dir_for_frozen() -> Path:
    """The Sky Player install root: parent of the running exe when frozen.

    Under PyInstaller ``--onedir`` (used here, see ``Sky-Player.spec:97-99``),
    ``sys.executable`` is the launcher exe directly inside the install root.
    Fails with a helpful error if invoked outside a frozen build.
    """
    if not getattr(sys, "frozen", False):
        raise UpdateInstallerError("install_dir_for_frozen only meaningful in frozen builds")
    return Path(sys.executable).resolve().parent


def post_update_flag_path(install_dir: Path) -> Path:
    """Path to a flag file consumed by the next launch's success toast."""
    return install_dir / ".sky-just-updated"
