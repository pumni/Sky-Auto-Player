from __future__ import annotations

import io
import json
import os
import subprocess
import sys
import threading
from pathlib import Path

import pytest

from sky_music.config import AppConfig
from sky_music.infrastructure import desktop_ipc as desktop_ipc_package
from sky_music.infrastructure.desktop_ipc import protocol
from sky_music.infrastructure.desktop_ipc.server import DesktopCoreServer
from sky_music.orchestration import settings_service as settings_module
from sky_music.orchestration.catalog_service import CatalogService, song_id_for_path
from sky_music.orchestration.native_admission import RustBuildInfo
from sky_music.orchestration.settings_service import SettingsService

NATIVE_INFO = RustBuildInfo(
    native_build_commit="a" * 40,
    schema_version=10,
    native_abi="cp314t-win_amd64",
    native_version="3.5.0",
    rustc_version="1.98.0",
    module_path="sky_player_rs.pyd",
    win32_backend=True,
)


def _request(method: str, params: dict[str, object] | None = None, request_id: int = 1) -> dict[str, object]:
    return {
        "v": 1,
        "id": request_id,
        "type": "request",
        "method": method,
        "params": params or {},
    }


def _server(tmp_path: Path) -> DesktopCoreServer:
    return DesktopCoreServer(
        settings_service=SettingsService(AppConfig(songs_dir=str(tmp_path))),
        catalog_service=CatalogService(tmp_path),
        native_build_info=NATIVE_INFO,
        app_version="3.5.0-test",
    )


def _call(server: DesktopCoreServer, request: dict[str, object]) -> dict[str, object]:
    return server.handle_request(protocol.parse_request_frame(protocol.encode_frame(request)))


def _output_messages(output: io.BytesIO) -> list[dict[str, object]]:
    return [json.loads(line) for line in output.getvalue().splitlines()]


def test_protocol_constants_and_package_exports() -> None:
    assert protocol.DESKTOP_PROTOCOL_VERSION == 1
    assert protocol.MAX_REQUEST_FRAME_BYTES == 64 * 1024
    assert protocol.MAX_RESPONSE_FRAME_BYTES == 1024 * 1024
    assert desktop_ipc_package.MAX_INBOUND_FRAME_BYTES == 64 * 1024


def test_protocol_rejects_bad_version_unknown_fields_duplicates_and_nonfinite() -> None:
    with pytest.raises(protocol.ProtocolError, match="unsupported protocol version"):
        protocol.parse_request_frame(protocol.encode_frame({**_request("settings.get"), "v": 2}))
    with pytest.raises(protocol.ProtocolError, match="unknown fields"):
        protocol.parse_request_frame(protocol.encode_frame({**_request("settings.get"), "extra": True}))
    with pytest.raises(protocol.ProtocolError, match="duplicate JSON key"):
        protocol.parse_request_frame(
            b'{"v":1,"id":1,"type":"request","method":"settings.get","params":{},"params":{}}\n'
        )
    with pytest.raises(protocol.ProtocolError, match="non-finite"):
        protocol.parse_request_frame(
            b'{"v":1,"id":1,"type":"request","method":"settings.patch","params":{"default_tempo_scale":NaN}}\n'
        )


def test_protocol_rejects_oversized_frames_without_readline() -> None:
    class ChunkStream:
        def __init__(self, value: bytes) -> None:
            self._value = value

        def read(self, size: int) -> bytes:
            chunk, self._value = self._value[:size], self._value[size:]
            return chunk

        def readline(self) -> bytes:
            raise AssertionError("bounded reader must not call readline")

    with pytest.raises(protocol.ProtocolError, match="64 KiB"):
        list(protocol.iter_bounded_frames(ChunkStream(b"x" * (64 * 1024 + 1))))

    with pytest.raises(protocol.ProtocolError, match="1 MiB"):
        protocol.encode_frame({"payload": "x" * (1024 * 1024)})


def test_protocol_rejects_bad_envelope_types() -> None:
    bad_requests = (
        {**_request("settings.get"), "id": True},
        {**_request("settings.get"), "type": "event"},
        {**_request("settings.get"), "method": "settings get"},
        {**_request("settings.get"), "params": []},
    )
    for request in bad_requests:
        with pytest.raises(protocol.ProtocolError):
            protocol.parse_request_frame(protocol.encode_frame(request))


def test_bootstrap_is_lazy_but_publishes_catalog_snapshot(tmp_path: Path) -> None:
    (tmp_path / "Alpha.json").write_text(
        '{"name":"ignored","songNotes":[{"time":0,"key":"Key0"}]}',
        encoding="utf-8",
    )
    server = _server(tmp_path)

    response = _call(server, _request("app.bootstrap"))

    assert response["ok"] is True
    result = response["result"]
    assert isinstance(result, dict)
    assert result["protocol_version"] == 1
    assert result["catalog_generation"] == 1
    assert result["native_build"]["native_build_commit"] == "a" * 40  # type: ignore[index]
    assert result["option_sets"]["fps"]  # type: ignore[index]
    assert str(tmp_path) not in json.dumps(response)


def test_catalog_search_is_path_free_and_supports_offset_limit(tmp_path: Path) -> None:
    for title in ("Bravo", "Alpha", "Charlie"):
        (tmp_path / f"{title}.txt").write_text("", encoding="utf-8")
    server = _server(tmp_path)

    response = _call(server, _request("catalog.search", {"query": "", "offset": 1, "limit": 1}))

    result = response["result"]
    assert isinstance(result, dict)
    assert result["total"] == 3
    assert result["offset"] == 1
    assert result["generation"] == 1
    assert result["items"] == [
        {
            "song_id": song_id_for_path(tmp_path / "Bravo.txt"),
            "title": "Bravo",
            "duration_us": None,
            "note_count": None,
            "risk_level": "unknown",
            "metadata_state": "pending",
        }
    ]
    assert str(tmp_path) not in json.dumps(response)


def test_catalog_detail_is_structured_and_does_not_leak_path(tmp_path: Path) -> None:
    song_path = tmp_path / "Detail.json"
    song_path.write_text(
        '{"name":"ignored","songNotes":[{"time":0,"key":"Key0"},{"time":200,"key":"Key1"}]}',
        encoding="utf-8",
    )
    server = _server(tmp_path)
    _call(server, _request("catalog.search"))

    response = _call(server, _request("catalog.detail", {"song_id": song_id_for_path(song_path)}))

    result = response["result"]
    assert isinstance(result, dict)
    assert result["title"] == "Detail"
    assert result["format_label"] == "JSON"
    assert isinstance(result["risk"], dict)
    assert isinstance(result["recommendation"], dict)
    assert str(tmp_path) not in json.dumps(response)


def test_catalog_reload_returns_response_then_changed_event(tmp_path: Path) -> None:
    server = _server(tmp_path)
    output = io.BytesIO()
    requests = b"".join(
        protocol.encode_frame(request)
        for request in (_request("catalog.reload"), _request("app.shutdown", request_id=2))
    )

    assert server.serve(io.BytesIO(requests), output, stderr=io.StringIO()) == 0
    messages = _output_messages(output)
    assert messages[0]["name"] == "core.ready"
    assert messages[1]["type"] == "response"
    assert messages[2]["name"] == "catalog.changed"
    assert messages[3]["result"] == {"shutdown": True}


def test_catalog_generation_is_checked_for_viewport_and_search(tmp_path: Path) -> None:
    server = _server(tmp_path)
    bootstrap = _call(server, _request("app.bootstrap"))
    generation = bootstrap["result"]["catalog_generation"]  # type: ignore[index]

    accepted = _call(
        server,
        _request(
            "catalog.set_viewport",
            {
                "generation": generation,
                "first_index": 0,
                "last_index": -1,
                "selected_song_id": None,
            },
        ),
    )
    stale = _call(server, _request("catalog.search", {"generation": generation + 1}))

    assert accepted["ok"] is True
    assert stale["ok"] is False
    assert stale["error"]["code"] == "stale_generation"  # type: ignore[index]


def test_catalog_viewport_is_fail_closed_for_empty_and_out_of_bounds_ranges(tmp_path: Path) -> None:
    server = _server(tmp_path)
    bootstrap = _call(server, _request("app.bootstrap"))
    generation = bootstrap["result"]["catalog_generation"]  # type: ignore[index]

    accepted_empty = _call(
        server,
        _request(
            "catalog.set_viewport",
            {"generation": generation, "first_index": 0, "last_index": -1, "selected_song_id": None},
        ),
    )
    invalid_ranges = (
        {"generation": generation, "first_index": 0, "last_index": 0, "selected_song_id": None},
        {"generation": generation, "first_index": 1, "last_index": -1, "selected_song_id": None},
        {"generation": generation, "first_index": 0, "last_index": -2, "selected_song_id": None},
    )
    rejected = [
        _call(server, _request("catalog.set_viewport", params))
        for params in invalid_ranges
    ]

    assert accepted_empty["ok"] is True
    assert all(response["error"]["code"] == "invalid_params" for response in rejected)  # type: ignore[index]


def test_catalog_viewport_rejects_unknown_selection_and_overscan(tmp_path: Path) -> None:
    song = tmp_path / "Alpha.txt"
    song.write_text("", encoding="utf-8")
    server = _server(tmp_path)
    bootstrap = _call(server, _request("app.bootstrap"))
    generation = bootstrap["result"]["catalog_generation"]  # type: ignore[index]
    song_id = song_id_for_path(song)

    accepted = _call(
        server,
        _request(
            "catalog.set_viewport",
            {"generation": generation, "first_index": 0, "last_index": 0, "selected_song_id": song_id},
        ),
    )
    unknown_selection = _call(
        server,
        _request(
            "catalog.set_viewport",
            {
                "generation": generation,
                "first_index": 0,
                "last_index": 0,
                "selected_song_id": "f" * 32,
            },
        ),
    )
    overscan = _call(
        server,
        _request(
            "catalog.set_viewport",
            {"generation": generation, "first_index": 0, "last_index": 1, "selected_song_id": None},
        ),
    )

    assert accepted["ok"] is True
    assert unknown_selection["error"]["code"] == "invalid_params"  # type: ignore[index]
    assert overscan["error"]["code"] == "invalid_params"  # type: ignore[index]


def test_settings_patch_uses_service_and_is_atomic(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    writes: list[AppConfig] = []
    monkeypatch.setattr(settings_module, "save_config", lambda cfg: writes.append(cfg))
    server = _server(tmp_path)

    response = _call(
        server,
        _request(
            "settings.patch",
            {
                "theme": "slate",
                "playback_defaults": {"tempo_scale": 0.95, "fps": 120},
                "telemetry_enabled": True,
            },
        ),
    )
    result = response["result"]
    assert isinstance(result, dict)
    assert result["theme"] == "slate"
    assert result["playback_defaults"]["tempo_scale"] == 0.95  # type: ignore[index]
    assert writes

    before = server.settings_service.snapshot()
    invalid = _call(
        server,
        _request("settings.patch", {"playback_defaults": {"tempo_scale": 0.9, "fps": True}}),
    )

    assert invalid["ok"] is False
    assert invalid["error"]["code"] == "invalid_params"  # type: ignore[index]
    assert server.settings_service.snapshot() == before
    assert len(writes) == 1


def test_unknown_method_and_invalid_params_are_responses() -> None:
    server = _server(Path("songs"))
    unknown = _call(server, _request("catalog.nope"))
    invalid = _call(server, _request("catalog.search", {"limit": 201}))

    assert unknown["error"]["code"] == "unknown_method"  # type: ignore[index]
    assert invalid["error"]["code"] == "invalid_params"  # type: ignore[index]


def test_server_eof_is_graceful_and_stdout_contains_protocol_only(tmp_path: Path) -> None:
    server = _server(tmp_path)
    output = io.BytesIO()
    errors = io.StringIO()

    assert server.serve(io.BytesIO(), output, stderr=errors) == 0
    messages = _output_messages(output)
    assert messages[0]["name"] == "core.ready"
    assert errors.getvalue() == ""


def test_server_malformed_frame_emits_bounded_fatal_on_stdout(tmp_path: Path) -> None:
    server = _server(tmp_path)
    output = io.BytesIO()
    errors = io.StringIO()

    assert server.serve(io.BytesIO(b"not-json\n"), output, stderr=errors) == 2
    messages = _output_messages(output)
    assert messages[0]["name"] == "core.ready"
    assert messages[1]["name"] == "core.fatal"
    assert len(json.dumps(messages[1]["payload"]["message"]).encode()) <= 4096  # type: ignore[index]
    assert errors.getvalue()


def test_parent_loss_stops_reader_without_waiting_for_stdin(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    class BlockingStream:
        def read(self, _size: int) -> bytes:
            threading.Event().wait(10)
            return b""

    monkeypatch.setattr("sky_music.infrastructure.desktop_ipc.server.parent_process_alive", lambda _pid: False)
    server = DesktopCoreServer(
        settings_service=SettingsService(AppConfig(songs_dir=str(tmp_path))),
        catalog_service=CatalogService(tmp_path),
        native_build_info=NATIVE_INFO,
        parent_pid=12345,
    )
    output = io.BytesIO()

    assert server.serve(BlockingStream(), output, stderr=io.StringIO()) == 0
    assert _output_messages(output)[0]["name"] == "core.ready"


def test_run_desktop_core_admits_runtime_before_services(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    from sky_music.cli import desktop_core

    calls: list[str] = []
    monkeypatch.setattr(desktop_core, "load_config", lambda: AppConfig(songs_dir=str(tmp_path)))

    def runtime_guard() -> None:
        calls.append("runtime")

    def native_admission() -> RustBuildInfo:
        calls.append("native")
        return NATIVE_INFO

    output = io.BytesIO()
    request = protocol.encode_frame(_request("app.shutdown"))
    result = desktop_core.run_desktop_core(
        ["--desktop-worker"],
        stdin=io.BytesIO(request),
        stdout=output,
        stderr=io.StringIO(),
        runtime_guard=runtime_guard,
        native_admission=native_admission,
    )

    assert result == 0
    assert calls == ["runtime", "native"]
    assert _output_messages(output)[-1]["result"] == {"shutdown": True}


def test_run_desktop_core_startup_failure_is_protocol_fatal(tmp_path: Path) -> None:
    from sky_music.cli.desktop_core import run_desktop_core

    output = io.BytesIO()
    errors = io.StringIO()

    result = run_desktop_core(
        ["--desktop-worker"],
        stdin=io.BytesIO(),
        stdout=output,
        stderr=errors,
        runtime_guard=lambda: (_ for _ in ()).throw(RuntimeError("runtime rejected")),
    )

    assert result == 2
    assert _output_messages(output)[0]["name"] == "core.fatal"
    assert errors.getvalue()


def test_desktop_core_subprocess_round_trip(tmp_path: Path) -> None:
    source_root = Path(__file__).parents[1] / "src"
    script = """
import sys
from sky_music.config import AppConfig
from sky_music.infrastructure.desktop_ipc.server import DesktopCoreServer
from sky_music.orchestration.catalog_service import CatalogService
from sky_music.orchestration.native_admission import RustBuildInfo
from sky_music.orchestration.settings_service import SettingsService

root = sys.argv[1]
info = RustBuildInfo(
    native_build_commit="a" * 40,
    schema_version=10,
    native_abi="cp314t-win_amd64",
    native_version="3.5.0",
    rustc_version="1.98.0",
    module_path="sky_player_rs.pyd",
    win32_backend=True,
)
server = DesktopCoreServer(
    settings_service=SettingsService(AppConfig(songs_dir=root)),
    catalog_service=CatalogService(root),
    native_build_info=info,
)
raise SystemExit(server.serve(sys.stdin.buffer, sys.stdout.buffer, stderr=sys.stderr))
"""
    env = os.environ.copy()
    env["PYTHONPATH"] = os.pathsep.join(filter(None, [str(source_root), env.get("PYTHONPATH", "")]))
    input_data = protocol.encode_frame(_request("app.shutdown"))

    completed = subprocess.run(
        [sys.executable, "-c", script, str(tmp_path)],
        input=input_data,
        capture_output=True,
        env=env,
        cwd=source_root.parent,
        timeout=30,
        check=False,
    )

    assert completed.returncode == 0, completed.stderr.decode(errors="replace")
    messages = [json.loads(line) for line in completed.stdout.splitlines()]
    assert messages[0]["name"] == "core.ready"
    assert messages[-1]["result"] == {"shutdown": True}
    assert completed.stderr == b""


def test_exact_core_main_entrypoint_smoke_with_real_admission(tmp_path: Path) -> None:
    """Exercise the production source path, not only DesktopCoreServer in isolation."""
    from sky_music.orchestration.native_admission import (
        NativeAdmissionError,
        require_rust_core,
    )

    try:
        require_rust_core()
    except NativeAdmissionError as exc:
        pytest.skip(f"native free-threaded test wheel is unavailable: {exc}")

    repository_root = Path(__file__).parents[1]
    source_root = repository_root / "src"
    env = os.environ.copy()
    env["PYTHONPATH"] = os.pathsep.join(filter(None, [str(source_root), env.get("PYTHONPATH", "")]))
    requests = b"".join(
        protocol.encode_frame(request)
        for request in (
            _request("app.bootstrap", request_id=1),
            _request("catalog.search", {"query": "", "offset": 0, "limit": 1}, request_id=2),
            _request("app.shutdown", request_id=3),
        )
    )

    completed = subprocess.run(
        [sys.executable, str(source_root / "core_main.py"), "--desktop-worker", "--install-root", str(tmp_path)],
        input=requests,
        capture_output=True,
        env=env,
        cwd=repository_root,
        timeout=30,
        check=False,
    )

    assert completed.returncode == 0, completed.stderr.decode(errors="replace")
    lines = completed.stdout.splitlines()
    assert lines, "exact core entrypoint emitted no protocol frames"
    messages = [json.loads(line) for line in lines]
    assert messages[0]["name"] == "core.ready"
    assert messages[1]["id"] == 1
    assert messages[2]["id"] == 2
    assert messages[-1] == {
        "v": 1,
        "id": 3,
        "type": "response",
        "ok": True,
        "result": {"shutdown": True},
    }
    assert all(message.get("v") == 1 for message in messages)
