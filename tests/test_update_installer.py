"""Unit tests for ``sky_music.infrastructure.update_installer``.

Covers all pure logic and side-effect-free functions:
  - SHA256 computation / verification
  - SHA256 sidecar parsing (bare hash, Coreutils form, ``Get-FileHash`` form)
  - Zip-slip protection in ``extract_zip``
  - HTTPS-only enforcement in ``download_zip``
  - Apply-batch script content is ASCII-safe and well-formed

No real download or subprocess is exercised — these tests stub network with a
``_StubResponse`` (mirrors urlopen's context-manager contract).
"""

from __future__ import annotations

import hashlib
import io
import zipfile
from pathlib import Path
from typing import Any

import pytest

from sky_music.domain.update_checker import UpdateInfo
from sky_music.infrastructure.update_installer import (
    UpdateInstallerError,
    compute_sha256,
    download_zip,
    extract_zip,
    fetch_sha256_sidecar,
    find_old_backups,
    parse_sha256_sidecar,
    stage_update,
    write_apply_batch,
)


class _StubResponse:
    def __init__(self, body: bytes) -> None:
        self._buf = io.BytesIO(body)
        self.headers: dict[str, str] = {}

    def __enter__(self) -> _StubResponse:
        return self

    def __exit__(self, *_: object) -> None:
        return None

    def read(self, _size: int = -1) -> bytes:
        return self._buf.read(-1 if _size == -1 else _size)


def _stub_opener(body: bytes, *, content_length: str | None = None):
    def opener(url: str, *, timeout: float = 0.0) -> _StubResponse:
        _ = url, timeout
        resp = _StubResponse(body)
        if content_length is not None:
            resp.headers["Content-Length"] = content_length
        return resp

    return opener


def _stub_opener_raises(exc: BaseException):
    def opener(url: str, *, timeout: float = 0.0) -> Any:
        _ = url, timeout
        raise exc

    return opener


# ── compute_sha256 / verify_sha256 ─────────────────────────────────────────────


def test_compute_sha256_matches_hashlib(tmp_path: Path) -> None:
    p = tmp_path / "f.bin"
    p.write_bytes(b"hello world")
    assert compute_sha256(p) == hashlib.sha256(b"hello world").hexdigest()


def test_verify_sha256_case_insensitive(tmp_path: Path) -> None:
    p = tmp_path / "f.bin"
    p.write_bytes(b"x")
    h = compute_sha256(p)
    assert hashlib.sha256(b"x").hexdigest() == h
    assert _verify_case(tmp_path / "f.bin", h.upper())


def test_verify_sha256_missing_file(tmp_path: Path) -> None:
    assert not _verify_case(tmp_path / "missing", "0" * 64)


def test_verify_sha256_empty_expected(tmp_path: Path) -> None:
    p = tmp_path / "f.bin"
    p.write_bytes(b"x")
    assert not _verify_case(p, "")


def _verify_case(file: Path, expected: str) -> bool:
    from sky_music.infrastructure.update_installer import verify_sha256

    return verify_sha256(file, expected)


# ── parse_sha256_sidecar ──────────────────────────────────────────────────────


@pytest.mark.parametrize("text", [
    "a" * 64,
    "A" * 64,
    f"{'a' * 64}  Sky-Player.zip",
    f"{'a' * 64} *Sky-Player.zip",
    f"SHA256\n{'a' * 64}  dist/Sky-Player.zip",
    f"\n  {'a' * 64}  Sky-Player.zip\n",
])
def test_parse_sha256_sidecar_valid_inputs(text: str) -> None:
    assert parse_sha256_sidecar(text) == "a" * 64


@pytest.mark.parametrize("text", ["", "   ", "garbage", "1234", "\n\n"])
def test_parse_sha256_sidecar_invalid_inputs(text: str) -> None:
    assert parse_sha256_sidecar(text) is None


def test_parse_sha256_sidecar_non_string_returns_none() -> None:
    assert parse_sha256_sidecar(None) is None  # type: ignore[arg-type]


# ── download_zip ────────────────────────────────────────────────────────────────


def test_download_zip_refuses_http(tmp_path: Path) -> None:
    with pytest.raises(UpdateInstallerError, match="non-https"):
        download_zip("http://example.com/x.zip", dest_dir=tmp_path)


def test_download_zip_refuses_missing_dir(tmp_path: Path) -> None:
    with pytest.raises(UpdateInstallerError, match="not a directory"):
        download_zip(
            "https://example.com/x.zip",
            dest_dir=tmp_path / "missing",
        )


def test_download_zip_writes_to_dest_dir(tmp_path: Path) -> None:
    body = b"binary content"
    path = download_zip(
        "https://example.com/x.zip",
        dest_dir=tmp_path,
        opener=_stub_opener(body, content_length=str(len(body))),
    )
    assert path.parent == tmp_path
    assert path.read_bytes() == body
    assert path.name.startswith("sky-update-")
    assert path.suffix == ".zip"


def test_download_zip_invokes_progress_callback(tmp_path: Path) -> None:
    body = b"x" * 200
    captured: list[tuple[int, int | None]] = []

    def progress(downloaded: int, total: int | None) -> None:
        captured.append((downloaded, total))

    download_zip(
        "https://example.com/x.zip",
        dest_dir=tmp_path,
        opener=_stub_opener(body, content_length="200"),
        progress=progress,
    )
    assert captured, "progress should have been called at least once"
    assert captured[-1] == (200, 200)


def test_download_zip_failure_wrapped_as_installer_error(tmp_path: Path) -> None:
    with pytest.raises(UpdateInstallerError, match="download failed"):
        download_zip(
            "https://example.com/x.zip",
            dest_dir=tmp_path,
            opener=_stub_opener_raises(OSError("boom")),
        )


# ── extract_zip ─────────────────────────────────────────────────────────────────


def _make_zip(path: Path, entries: dict[str, bytes]) -> None:
    with zipfile.ZipFile(path, "w") as zf:
        for name, content in entries.items():
            zf.writestr(name, content)


def test_extract_zip_round_trips(tmp_path: Path) -> None:
    src = tmp_path / "src.zip"
    _make_zip(src, {"a.txt": b"alpha", "sub/b.txt": b"beta"})
    dest = tmp_path / "out"
    out = extract_zip(src, dest)
    assert out == dest
    assert (dest / "a.txt").read_bytes() == b"alpha"
    assert (dest / "sub" / "b.txt").read_bytes() == b"beta"


def test_extract_zip_blocks_zip_slip(tmp_path: Path) -> None:
    src = tmp_path / "evil.zip"
    # Construct a zip with an absolute-path entry: writes via ZipInfo.filename.
    with zipfile.ZipFile(src, "w") as zf:
        info = zipfile.ZipInfo("/escape.txt")
        zf.writestr(info, b"would slip out")

    with pytest.raises(UpdateInstallerError, match="zip-slip"):
        extract_zip(src, tmp_path / "out")


def test_extract_zip_blocks_dotdot_name(tmp_path: Path) -> None:
    src = tmp_path / "evil.zip"
    with zipfile.ZipFile(src, "w") as zf:
        info = zipfile.ZipInfo("../escape.txt")
        zf.writestr(info, b"would slip out")
    with pytest.raises(UpdateInstallerError, match="zip-slip"):
        extract_zip(src, tmp_path / "out")


def test_extract_zip_missing_zip_raises(tmp_path: Path) -> None:
    with pytest.raises(UpdateInstallerError, match="missing zip"):
        extract_zip(tmp_path / "not-there.zip", tmp_path / "out")


# ── stage_update ───────────────────────────────────────────────────────────────


def _make_zip_bytes(entries: dict[str, bytes]) -> bytes:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        for name, content in entries.items():
            zf.writestr(name, content)
    return buf.getvalue()


def test_stage_update_no_sh256_extracts(tmp_path: Path) -> None:
    body = _make_zip_bytes({"a.txt": b"alpha"})
    release = UpdateInfo(
        latest_version="2.4.0",
        download_url="https://example.com/x.zip",
        release_notes="",
        html_url="",
        published_at="",
        sha256_url="",
    )
    staged = stage_update(
        release,
        staging_parent=tmp_path,
        opener=_stub_opener(body, content_length=str(len(body))),
    )
    assert staged.new_version == "2.4.0"
    assert staged.staging_dir.exists()
    assert (staged.staging_dir / "a.txt").read_bytes() == b"alpha"
    # Zip should have been deleted from the staging dir after extraction.
    assert not any(p.suffix == ".zip" for p in staged.staging_dir.iterdir())


def test_stage_update_sha256_match_succeeds(tmp_path: Path) -> None:
    body = _make_zip_bytes({"a.txt": b"alpha"})
    release = UpdateInfo(
        latest_version="2.4.0",
        download_url="https://example.com/x.zip",
        release_notes="",
        html_url="",
        published_at="",
        sha256_url="",
    )
    expected = hashlib.sha256(body).hexdigest()
    staged = stage_update(
        release,
        staging_parent=tmp_path,
        opener=_stub_opener(body, content_length=str(len(body))),
        sha256_sum=expected,
    )
    assert (staged.staging_dir / "a.txt").exists()


def test_stage_update_sha256_mismatch_cleans_staging(tmp_path: Path) -> None:
    body = _make_zip_bytes({"a.txt": b"alpha"})
    release = UpdateInfo(
        latest_version="2.4.0",
        download_url="https://example.com/x.zip",
        release_notes="",
        html_url="",
        published_at="",
        sha256_url="",
    )
    bad_sum = "0" * 64
    with pytest.raises(UpdateInstallerError, match="sha256 mismatch"):
        stage_update(
            release,
            staging_parent=tmp_path,
            opener=_stub_opener(body, content_length=str(len(body))),
            sha256_sum=bad_sum,
        )
    # Staging dir should have been removed on failure.
    staging_dirs = [p for p in tmp_path.iterdir() if p.is_dir() and p.name.startswith("sky-pending-")]
    assert staging_dirs == []


def test_stage_update_missing_download_url_raises(tmp_path: Path) -> None:
    release = UpdateInfo(
        latest_version="2.4.0",
        download_url="",
        release_notes="",
        html_url="",
        published_at="",
        sha256_url="",
    )
    with pytest.raises(UpdateInstallerError, match="no downloadable asset"):
        stage_update(release, staging_parent=tmp_path)


# ── fetch_sha256_sidecar ──────────────────────────────────────────────────────────


def test_fetch_sha256_sidecar_fetches_and_parses() -> None:
    h = "a" * 64
    result = fetch_sha256_sidecar(
        "https://x/x.sha256",
        opener=_stub_opener(f"{h}  Sky-Player.zip".encode()),
    )
    assert result == h


def test_fetch_sha256_sidecar_handles_empty_url() -> None:
    assert fetch_sha256_sidecar("") is None


def test_fetch_sha256_sidecar_handles_network_error() -> None:
    assert fetch_sha256_sidecar(
        "https://example.com/x.sha256",
        opener=_stub_opener_raises(OSError("boom")),
    ) is None


def test_fetch_sha256_sidecar_handles_garbage_payload() -> None:
    assert fetch_sha256_sidecar(
        "https://example.com/x.sha256",
        opener=_stub_opener(b"not a checksum at all"),
    ) is None


# ── write_apply_batch ──────────────────────────────────────────────────────────


def test_write_apply_batch_writes_expected_skeleton(tmp_path: Path) -> None:
    staging = tmp_path / "staging"
    install_dir = tmp_path / "install"
    sentinel = install_dir / ".sky-just-updated"
    staging.mkdir()
    install_dir.mkdir()
    # Pre-create a fake exe so the apply batch references it.
    (install_dir / "Sky-Player.exe").write_bytes(b"dummy")
    batch_path = tmp_path / "apply.cmd"

    write_apply_batch(
        staging_dir=staging,
        install_dir=install_dir,
        post_update_flag=sentinel,
        batch_path=batch_path,
    )

    # Read in binary mode so we preserve \r\n line endings written for .cmd.
    raw = batch_path.read_bytes()
    text = raw.decode("ascii")
    assert text.startswith("@echo off")
    assert "Rename-Item" in text
    assert "Sky-Player.exe" in text
    assert "New-Item" in text  # sentinel touch via PowerShell
    assert text.rstrip().endswith('del "%~f0"')  # self-delete
    # The batch uses \r\n line endings consistently.
    assert b"\r\n" in raw
    assert b"\n" not in raw.replace(b"\r\n", b"")


def test_write_apply_batch_omits_start_when_exe_missing(tmp_path: Path) -> None:
    staging = tmp_path / "staging"
    install_dir = tmp_path / "install"
    staging.mkdir()
    install_dir.mkdir()
    batch_path = tmp_path / "apply.cmd"
    write_apply_batch(
        staging_dir=staging,
        install_dir=install_dir,
        post_update_flag=install_dir / ".sky-just-updated",
        batch_path=batch_path,
    )
    text = batch_path.read_text(encoding="ascii")
    assert "start " not in text  # exe missing → no restart line


# ── find_old_backups ──────────────────────────────────────────────────────────


def test_find_old_backups_lists_old_guid_siblings(tmp_path: Path) -> None:
    """A single ``<install>.old.<guid>`` directory should be discovered."""
    install = tmp_path / "Sky-Player"
    install.mkdir()
    backup = tmp_path / "Sky-Player.old.abc123def456"
    backup.mkdir()
    # Noise that must NOT be reported.
    (tmp_path / "Sky-Player.old").mkdir()           # no guid tail
    (tmp_path / "Sky-Player.old.x").mkdir()         # tail < 4 chars
    (tmp_path / "Sky-Player.zip").write_bytes(b"")  # not a directory
    (tmp_path / "unrelated.old.deadbeef").mkdir()
    result = find_old_backups(install)
    assert result == [backup]


def test_find_old_backups_sorted_by_mtime_descending(tmp_path: Path) -> None:
    """Multiple backups → newest first so cleanup can abort on locked dir."""
    import os
    import time

    install = tmp_path / "Sky-Player"
    install.mkdir()
    older = tmp_path / "Sky-Player.old.aaaa1111bbbb"
    newer = tmp_path / "Sky-Player.old.cccc2222dddd"
    older.mkdir()
    # Force distinct mtimes — newer has a higher mtime.
    os.utime(older, (1_000_000, 1_000_000))
    time.sleep(0.01)
    newer.mkdir()
    result = find_old_backups(install)
    assert result[0] == newer
    assert result[1] == older


def test_find_old_backups_empty_when_none(tmp_path: Path) -> None:
    install = tmp_path / "Sky-Player"
    install.mkdir()
    assert find_old_backups(install) == []


def test_find_old_backups_returns_empty_when_install_missing(tmp_path: Path) -> None:
    install = tmp_path / "does-not-exist"
    # Pre-create noise so we know it is not just an empty parent.
    (tmp_path / "Sky-Player.old.1234abcd").mkdir()
    assert find_old_backups(install) == []


def test_find_old_backups_tolerates_unreadable_parent(tmp_path: Path) -> None:
    """If listing the parent directory raises OSError, return [] rather than
    propagating the failure — cleanup is best-effort.
    """
    install = tmp_path / "Sky-Player"
    install.mkdir()

    class _BrokenParent:
        def iterdir(self) -> list[Path]:
            raise OSError("denied")

    class _StubInstall:
        name = "Sky-Player"
        exists = staticmethod(lambda: True)
        parent = _BrokenParent()

    # Boundary check: ``install_dir.parent == install_dir`` is False on the
    # stub, so we reach the iterdir() call, which raises OSError, swallowed.
    assert _StubInstall().parent != _StubInstall()
    assert find_old_backups(_StubInstall()) == []  # type: ignore[arg-type]
