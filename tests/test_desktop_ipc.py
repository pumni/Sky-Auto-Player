from __future__ import annotations

import io
import json
import os
import subprocess
import sys
import threading
import time
from collections.abc import Mapping
from pathlib import Path

import pytest

from sky_music.config import AppConfig
from sky_music.infrastructure import desktop_ipc as desktop_ipc_package
from sky_music.infrastructure.desktop_ipc import protocol
from sky_music.infrastructure.desktop_ipc.server import DesktopCoreServer
from sky_music.orchestration import desktop_playback as playback_module
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


def _request(method: str, params: Mapping[str, object] | None = None, request_id: int = 1) -> dict[str, object]:
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


def test_protocol_reader_progresses_on_an_open_buffered_pipe() -> None:
    """A live inherited pipe must not wait for a full 4 KiB read."""
    read_fd, write_fd = os.pipe()
    reader_stream = io.BufferedReader(os.fdopen(read_fd, "rb", buffering=0))
    writer_stream = os.fdopen(write_fd, "wb", buffering=0)
    try:
        writer_stream.write(protocol.encode_frame(_request("settings.get")))
        writer_stream.flush()
        frames = protocol.iter_bounded_frames(reader_stream)
        assert next(frames) == protocol.encode_frame(_request("settings.get"))[:-1]
    finally:
        reader_stream.close()
        writer_stream.close()


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


def test_exact_core_main_entrypoint_smoke_with_real_admission() -> None:
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
    song_path = repository_root / "songs" / "blue.json"
    assert song_path.is_file()
    song_id = song_id_for_path(song_path)
    env = os.environ.copy()
    env["PYTHONPATH"] = os.pathsep.join(filter(None, [str(source_root), env.get("PYTHONPATH", "")]))
    requests = b"".join(
        protocol.encode_frame(request)
        for request in (
            _request("app.bootstrap", request_id=1),
            _request("catalog.search", {"query": "blue", "offset": 0, "limit": 1}, request_id=2),
            _request("catalog.detail", {"song_id": song_id, "generation": 1}, request_id=3),
            _request("settings.get", request_id=4),
            _request("app.shutdown", request_id=5),
        )
    )

    completed = subprocess.run(
        [sys.executable, str(source_root / "core_main.py"), "--desktop-worker", "--install-root", str(repository_root)],
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
    assert messages[2]["result"]["items"][0]["song_id"] == song_id  # type: ignore[index]
    assert messages[3]["id"] == 3
    assert messages[3]["result"]["song_id"] == song_id  # type: ignore[index]
    assert messages[4]["id"] == 4
    assert messages[-1] == {
        "v": 1,
        "id": 5,
        "type": "response",
        "ok": True,
        "result": {"shutdown": True},
    }
    assert all(message.get("v") == 1 for message in messages)


def test_exact_core_main_entrypoint_runs_dry_run_playback_lifecycle() -> None:
    """Exercise core_main.py through prepare/start/events without physical input."""
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
    song_path = repository_root / "songs" / "blue.json"
    song_id = song_id_for_path(song_path)
    env = os.environ.copy()
    env["PYTHONPATH"] = os.pathsep.join(filter(None, [str(source_root), env.get("PYTHONPATH", "")]))
    process = subprocess.Popen(
        [sys.executable, str(source_root / "core_main.py"), "--desktop-worker", "--install-root", str(repository_root)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
        cwd=repository_root,
        bufsize=0,
    )
    messages: list[dict[str, object]] = []

    def read_message() -> dict[str, object]:
        assert process.stdout is not None
        line = process.stdout.readline()
        assert line, "Core closed stdout before completing the dry-run lifecycle"
        message = json.loads(line)
        assert message.get("v") == 1
        assert message.get("type") in {"event", "response"}
        messages.append(message)
        return message

    def send(request: dict[str, object]) -> None:
        assert process.stdin is not None
        process.stdin.write(protocol.encode_frame(request))
        process.stdin.flush()

    try:
        ready = read_message()
        assert ready["name"] == "core.ready"
        send(_request("app.bootstrap", request_id=1))
        bootstrap = read_message()
        assert bootstrap["id"] == 1
        generation = bootstrap["result"]["catalog_generation"]  # type: ignore[index]

        send(_request("playback.prepare", {
            "song_id": song_id,
            "generation": generation,
            "config": {"hold_frames": 1, "tempo_scale": 1, "fps": 60, "dry_run": True},
        }, request_id=2))
        prepared_response = read_message()
        assert prepared_response["id"] == 2
        prepared = prepared_response["result"]
        assert isinstance(prepared, dict)
        assert prepared["admission"] in {"ready", "confirmation_required"}
        decisions = (
            [{"decision": prepared["decisions"][0]["decision"], "accepted": True}]  # type: ignore[index]
            if prepared["admission"] == "confirmation_required"
            else []
        )

        send(_request("playback.start", {"prepared_id": prepared["prepared_id"], "decisions": decisions}, request_id=3))  # type: ignore[index]
        start_response = read_message()
        assert start_response["id"] == 3
        assert start_response["result"]["state"] == "starting"  # type: ignore[index]

        finished = False
        while not finished:
            message = read_message()
            finished = message.get("name") == "playback.finished"

        send(_request("app.shutdown", request_id=4))
        shutdown = read_message()
        assert shutdown["id"] == 4
        assert shutdown["result"] == {"shutdown": True}
        # The production supervisor closes its inherited stdin while reaping
        # the child. Mirror that final pipe lifecycle so the reader thread can
        # observe EOF before this standalone smoke waits for exit.
        assert process.stdin is not None
        process.stdin.close()
        return_code = process.wait(timeout=10)
        stderr_output = process.stderr.read().decode(errors="replace") if process.stderr is not None else ""
        assert return_code == 0, stderr_output
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        if process.stdin is not None:
            process.stdin.close()
        if process.stdout is not None:
            process.stdout.close()
        if process.stderr is not None:
            process.stderr.close()
    assert all(message.get("v") == 1 for message in messages)


def _playback_server(tmp_path: Path, *, repeat: bool = False) -> tuple[DesktopCoreServer, str]:
    notes = (
        [{"time": 0, "key": "Key0"}, {"time": 1, "key": "Key0"}]
        if repeat
        else [{"time": 0, "key": "Key0"}, {"time": 100, "key": "Key1"}]
    )
    song_path = tmp_path / "Playback.json"
    song_path.write_text(json.dumps({"name": "Playback", "songNotes": notes}), encoding="utf-8")
    server = _server(tmp_path)
    bootstrap = _call(server, _request("app.bootstrap"))
    assert bootstrap["ok"] is True
    return server, song_id_for_path(song_path)


def _wait_for_playback_event(
    server: DesktopCoreServer,
    name: str,
    *,
    timeout: float = 2.0,
) -> dict[str, object]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        for message in server.drain_events():
            if message.get("name") == name:
                return message
        time.sleep(0.01)
    raise AssertionError(f"did not receive {name} within {timeout}s")


def _wait_for_playback_state(
    server: DesktopCoreServer,
    state: str,
    *,
    timeout: float = 2.0,
) -> dict[str, object]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        for message in server.drain_events():
            if (
                message.get("name") == "playback.state_changed"
                and isinstance(message.get("payload"), dict)
                and message["payload"].get("state") == state  # type: ignore[index]
            ):
                return message
        time.sleep(0.01)
    raise AssertionError(f"did not receive playback state {state} within {timeout}s")


def test_playback_prepare_start_and_stop_use_opaque_identity_and_dry_run(tmp_path: Path) -> None:
    server, song_id = _playback_server(tmp_path)
    config = {"hold_frames": 1, "tempo_scale": 1, "fps": 60, "dry_run": True}

    stale = _call(
        server,
        _request("playback.prepare", {"song_id": song_id, "generation": 2, "config": config}),
    )
    assert stale["ok"] is False
    assert stale["error"]["code"] == "stale_generation"  # type: ignore[index]

    invalid = server.handle_request(
        _request(
            "playback.prepare",
            {"song_id": song_id, "generation": 1, "config": {**config, "tempo_scale": float("nan")}},
        )
    )
    assert invalid["ok"] is False
    assert invalid["error"]["code"] == "invalid_params"  # type: ignore[index]

    prepared_response = _call(
        server,
        _request("playback.prepare", {"song_id": song_id, "generation": 1, "config": config}),
    )
    prepared = prepared_response["result"]
    assert prepared_response["ok"] is True
    assert isinstance(prepared, dict)
    prepared_id = prepared["prepared_id"]
    assert isinstance(prepared_id, str) and len(prepared_id) == 32
    assert str(tmp_path) not in json.dumps(prepared_response)

    tampered = _call(
        server,
        _request("playback.start", {"prepared_id": "0" * 32, "decisions": []}),
    )
    assert tampered["error"]["code"] == "prepared_not_found"  # type: ignore[index]

    started = _call(
        server,
        _request("playback.start", {"prepared_id": prepared_id, "decisions": []}),
    )
    assert started["ok"] is True
    session = started["result"]
    assert isinstance(session, dict)
    session_id = session["session_id"]
    assert isinstance(session_id, str) and len(session_id) == 32
    _wait_for_playback_event(server, "playback.finished")

    # Stop remains idempotent for the just-finished session, while a foreign
    # session ID remains fail-closed.
    repeated_stop = _call(server, _request("playback.stop", {"session_id": session_id}))
    assert repeated_stop["ok"] is True
    foreign_stop = _call(server, _request("playback.stop", {"session_id": "f" * 32}))
    assert foreign_stop["ok"] is False
    assert foreign_stop["error"]["code"] == "no_active_session"  # type: ignore[index]


def test_blocked_prepare_never_creates_a_startable_plan(tmp_path: Path) -> None:
    server, song_id = _playback_server(tmp_path, repeat=True)
    prepared = _call(
        server,
        _request(
            "playback.prepare",
            {
                "song_id": song_id,
                "generation": 1,
                "config": {"hold_frames": 1.5, "tempo_scale": 1, "fps": 60, "dry_run": False},
            },
        ),
    )
    result = prepared["result"]
    assert isinstance(result, dict)
    assert result["admission"] == "blocked"
    assert result["prepared_id"] is None


def test_confirmation_required_uses_one_exact_typed_decision_and_is_retryable(tmp_path: Path) -> None:
    server, song_id = _playback_server(tmp_path, repeat=True)
    request = {
        "song_id": song_id,
        "generation": 1,
        "config": {"hold_frames": 1.5, "tempo_scale": 1, "fps": 60, "dry_run": True},
    }
    prepared = _call(server, _request("playback.prepare", request))["result"]
    assert isinstance(prepared, dict)
    prepared_id = prepared["prepared_id"]
    assert prepared["admission"] == "confirmation_required"

    missing = _call(server, _request("playback.start", {"prepared_id": prepared_id, "decisions": []}))
    assert missing["error"]["code"] == "confirmation_required"  # type: ignore[index]

    unknown = _call(
        server,
        _request(
            "playback.start",
            {"prepared_id": prepared_id, "decisions": [{"decision": "confirmed", "accepted": True}]},
        ),
    )
    assert unknown["error"]["code"] == "confirmation_required"  # type: ignore[index]

    started = _call(
        server,
        _request(
            "playback.start",
            {"prepared_id": prepared_id, "decisions": [{"decision": "proceed", "accepted": True}]},
        ),
    )
    assert started["ok"] is True
    _wait_for_playback_event(server, "playback.finished")


def test_supported_physical_controls_use_native_engine_boundary_without_input_in_test(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Exercise the desktop physical-session state machine with a test engine."""

    class FakeEngine:
        def __init__(self, *, controls: object, renderer: object, **_kwargs: object) -> None:
            self.controls = controls
            self.renderer = renderer

        def prepare_focus_for_playback(self) -> bool:
            return True

        def play(self) -> str:
            paused = False
            self.renderer.render(0.0, 0.2, "Playback", status="playing")  # type: ignore[attr-defined]
            while True:
                command = self.controls.poll()  # type: ignore[attr-defined]
                if command == "pause":
                    paused = not paused
                    self.renderer.render(  # type: ignore[attr-defined]
                        0.0,
                        0.2,
                        "Playback",
                        status="paused" if paused else "playing",
                    )
                elif command == "skip":
                    return "skipped"
                elif command == "quit":
                    return "quit"
                time.sleep(0.005)

    monkeypatch.setattr(playback_module, "PlaybackEngine", FakeEngine)
    server, song_id = _playback_server(tmp_path)
    prepared = _call(
        server,
        _request(
            "playback.prepare",
            {
                "song_id": song_id,
                "generation": 1,
                "config": {"hold_frames": 1, "tempo_scale": 1, "fps": 60, "dry_run": False},
            },
        ),
    )["result"]
    assert isinstance(prepared, dict)
    decisions = (
        [{"decision": prepared["decisions"][0]["decision"], "accepted": True}]  # type: ignore[index]
        if prepared["admission"] == "confirmation_required"
        else []
    )
    started = _call(
        server,
        _request(
            "playback.start",
            {"prepared_id": prepared["prepared_id"], "decisions": decisions},  # type: ignore[index]
        ),
    )
    assert started["ok"] is True
    session = started["result"]
    assert isinstance(session, dict)
    session_id = session["session_id"]

    _wait_for_playback_state(server, "playing")
    paused = _call(server, _request("playback.pause", {"session_id": session_id}))
    assert paused["ok"] is True
    _wait_for_playback_state(server, "paused")
    resumed = _call(server, _request("playback.resume", {"session_id": session_id}))
    assert resumed["ok"] is True
    _wait_for_playback_state(server, "playing")
    skipped = _call(server, _request("playback.skip", {"session_id": session_id}))
    assert skipped["ok"] is True
    _wait_for_playback_state(server, "finished")


def test_playback_snapshot_buffer_is_latest_wins_and_bounded(tmp_path: Path) -> None:
    server = _server(tmp_path)
    for seq in range(1000):
        server._publish_event(
            "playback.snapshot",
            {"session_id": "a" * 32, "seq": seq},
        )
    events = server.drain_events()
    assert len(events) == 1
    assert events[0]["payload"]["seq"] == 999  # type: ignore[index]
