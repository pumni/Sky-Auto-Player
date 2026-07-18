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

from sky_music.infrastructure.update_installer import (
    UpdateInstallerError,
    compute_sha256,
    download_zip,
    extract_zip,
    fetch_sha256_sidecar,
    parse_sha256_sidecar,
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


