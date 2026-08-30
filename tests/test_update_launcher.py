from __future__ import annotations

import hashlib
import json
import os
import subprocess
import time
from pathlib import Path
from typing import Any, cast

import pytest

from sky_music.infrastructure import update_launcher


def _request(root: Path, *, target: str = "2.5.0") -> update_launcher.UpdateLaunchRequest:
    return update_launcher.UpdateLaunchRequest(
        install_root=root,
        current_version="2.4.0",
        target_version=target,
        channel="stable",
        parent_pid=4242,
    )


def _write_install_manifest(root: Path, *, updater: bytes = b"updater") -> None:
    files = []
    for name, data in {
        "Sky-Auto-Player.exe": b"app",
        "native_calibration.exe": b"calibration",
        update_launcher.UPDATER_NAME: updater,
    }.items():
        (root / name).write_bytes(data)
        files.append(
            {
                "path": name,
                "size": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
        )
    (root / "MANIFEST.json").write_text(
        json.dumps(
            {
                "schema_version": 2,
                "app": update_launcher.APP_NAME,
                "version": "2.4.0",
                "executable": "Sky-Auto-Player.exe",
                "files": files,
            }
        ),
        encoding="utf-8",
    )


def _read_manifest(root: Path) -> dict[str, object]:
    return json.loads((root / "MANIFEST.json").read_text(encoding="utf-8"))


def _write_manifest_data(root: Path, data: dict[str, object]) -> None:
    (root / "MANIFEST.json").write_text(json.dumps(data), encoding="utf-8")


def _handoff(run_root: Path, *, state: str = "ready", error_code: str = "", message: str = "") -> dict[str, object]:
    return {
        "schema_version": 1,
        "state": state,
        "run_id": run_root.name,
        "updater_pid": 4711,
        "target_version": "2.5.0",
        "error_code": error_code,
        "message": message,
    }


def test_launch_stages_verified_native_updater(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    _write_install_manifest(root)
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "local"))
    calls: list[tuple[list[str], dict[str, object]]] = []

    class FakeProcess:
        pid = 4711

        def poll(self) -> None:
            return None

        def terminate(self) -> None:
            raise AssertionError("ready handshake should not terminate child")

        def wait(self, *, timeout: float) -> None:
            del timeout

        def kill(self) -> None:
            raise AssertionError("ready handshake should not kill child")

    def fake_popen(arguments: list[str], **kwargs: object) -> FakeProcess:
        calls.append((arguments, kwargs))
        run_root = Path(arguments[0]).parent
        (run_root / "handoff.json").write_text(json.dumps(_handoff(run_root)), encoding="utf-8")
        return FakeProcess()

    monkeypatch.setattr(update_launcher.subprocess, "Popen", fake_popen)
    launched = update_launcher.launch_update(_request(root))

    assert isinstance(launched, update_launcher.UpdateLaunchResult)
    assert launched.status == "ready"
    copied = launched.staged_updater
    assert copied.name == update_launcher.UPDATER_NAME
    assert copied.read_bytes() == b"updater"
    assert launched.updater_pid == 4711
    assert len(calls) == 1
    arguments, kwargs = calls[0]
    assert arguments[0] == str(copied)
    assert arguments[1:] == [
        "--install-root",
        str(root.resolve()),
        "--parent-pid",
        "4242",
        "--current-version",
        "2.4.0",
        "--target-version",
        "2.5.0",
        "--channel",
        "stable",
        "--restart",
    ]
    assert kwargs["shell"] is False


def test_launch_returns_already_running_for_duplicate_updater(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    _write_install_manifest(root)
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "local"))

    class FakeProcess:
        pid = 4711

        def poll(self) -> None:
            return None

        def terminate(self) -> None:
            raise AssertionError("duplicate handoff must not terminate child")

        def wait(self, *, timeout: float) -> None:
            del timeout

        def kill(self) -> None:
            raise AssertionError("duplicate handoff must not kill child")

    def fake_popen(arguments: list[str], **_kwargs: object) -> FakeProcess:
        run_root = Path(arguments[0]).parent
        (run_root / "handoff.json").write_text(
            json.dumps(
                _handoff(
                    run_root,
                    state="rejected",
                    error_code="UPDATE_ALREADY_RUNNING",
                    message="another updater owns the active state",
                )
            ),
            encoding="utf-8",
        )
        return FakeProcess()

    monkeypatch.setattr(update_launcher.subprocess, "Popen", fake_popen)
    launched = update_launcher.launch_update(_request(root))

    assert launched.status == "already_running"


def test_launch_surfaces_rejected_handoff_code_and_bounded_message(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    _write_install_manifest(root)
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "local"))

    class FakeProcess:
        pid = 4711

        def poll(self) -> None:
            return None

    def fake_popen(arguments: list[str], **_kwargs: object) -> FakeProcess:
        run_root = Path(arguments[0]).parent
        (run_root / "handoff.json").write_text(
            json.dumps(
                _handoff(
                    run_root,
                    state="rejected",
                    error_code="UI_INITIALIZATION_FAILED",
                    message="InitCommonControlsEx failed",
                )
            ),
            encoding="utf-8",
        )
        return FakeProcess()

    monkeypatch.setattr(update_launcher.subprocess, "Popen", fake_popen)
    with pytest.raises(update_launcher.UpdateLaunchError) as caught:
        update_launcher.launch_update(_request(root))
    assert str(caught.value) == (
        "native updater rejected startup [UI_INITIALIZATION_FAILED]: "
        "InitCommonControlsEx failed"
    )


@pytest.mark.parametrize("variant", ["malformed", "pid", "target", "run_id"])
def test_handoff_parser_rejects_malformed_or_mismatched_identity(
    tmp_path: Path, variant: str
) -> None:
    run_root = tmp_path / ("run-" + "a" * 32)
    run_root.mkdir()
    payload = _handoff(run_root)
    if variant == "malformed":
        (run_root / "handoff.json").write_text("{not-json", encoding="utf-8")
    else:
        if variant == "pid":
            payload["updater_pid"] = 9999
        elif variant == "target":
            payload["target_version"] = "9.9.9"
        else:
            payload["run_id"] = "run-" + "b" * 32
        (run_root / "handoff.json").write_text(json.dumps(payload), encoding="utf-8")

    assert (
        update_launcher._parse_handoff(
            run_root / "handoff.json",
            run_root=run_root,
            process_pid=4711,
            target_version="2.5.0",
        )
        is None
    )


def test_wait_for_handoff_rejects_child_exit_before_handoff(tmp_path: Path) -> None:
    run_root = tmp_path / ("run-" + "a" * 32)
    run_root.mkdir()

    class ExitedProcess:
        pid = 4711

        def poll(self) -> int:
            return 1

    with pytest.raises(update_launcher.UpdateLaunchError, match="exited before"):
        update_launcher._wait_for_handoff(
            cast(Any, ExitedProcess()), run_root=run_root, target_version="2.5.0"
        )


def test_wait_for_handoff_timeout_terminates_then_kills_child(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    run_root = tmp_path / ("run-" + "a" * 32)
    run_root.mkdir()
    events: list[str] = []

    class HungProcess:
        pid = 4711

        def poll(self) -> None:
            return None

        def terminate(self) -> None:
            events.append("terminate")

        def wait(self, *, timeout: float) -> None:
            del timeout
            if events == ["terminate"]:
                raise subprocess.TimeoutExpired("updater", 2)
            events.append("wait-after-kill")

        def kill(self) -> None:
            events.append("kill")

    clock = iter((0.0, 6.0))
    monkeypatch.setattr(update_launcher.time, "monotonic", lambda: next(clock))
    with pytest.raises(update_launcher.UpdateLaunchError, match="ready handshake"):
        update_launcher._wait_for_handoff(
            cast(Any, HungProcess()), run_root=run_root, target_version="2.5.0"
        )
    assert events == ["terminate", "kill", "wait-after-kill"]


def test_launch_preflight_failure_does_not_spawn(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    _write_install_manifest(root)
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "local"))
    calls: list[object] = []

    def fake_popen(*_args: object, **_kwargs: object) -> object:
        calls.append(object())
        return object()

    def fail_preflight(_root: Path) -> None:
        raise update_launcher.UpdateLaunchError("install root is not writable: injected")

    monkeypatch.setattr(update_launcher.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(update_launcher, "_preflight_install_root_writable", fail_preflight)
    with pytest.raises(update_launcher.UpdateLaunchError, match="not writable"):
        update_launcher.launch_update(_request(root))
    assert calls == []


@pytest.mark.parametrize(
    ("variant", "message"),
    [
        ("missing_manifest", "installed manifest is missing"),
        ("malformed_manifest", "not valid UTF-8 JSON"),
        ("duplicate_updater", "duplicate paths"),
        ("missing_updater", "exactly one native updater"),
    ],
)
def test_launcher_rejects_manifest_boundary_variants(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    variant: str,
    message: str,
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    _write_install_manifest(root)
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "local"))
    if variant == "missing_manifest":
        (root / "MANIFEST.json").unlink()
    elif variant == "malformed_manifest":
        (root / "MANIFEST.json").write_text("{not-json", encoding="utf-8")
    else:
        data = _read_manifest(root)
        files = data["files"]
        assert isinstance(files, list)
        if variant == "duplicate_updater":
            files.append(dict(files[-1]))
        else:
            data["files"] = [
                entry
                for entry in files
                if isinstance(entry, dict) and entry.get("path") != update_launcher.UPDATER_NAME
            ]
        _write_manifest_data(root, data)
    with pytest.raises(update_launcher.UpdateLaunchError, match=message):
        update_launcher.launch_update(_request(root))


@pytest.mark.parametrize(
    ("variant", "message"),
    [
        ("invalid_channel", "channel must be stable or beta"),
        ("invalid_current", "valid PEP 440"),
        ("invalid_target", "valid PEP 440"),
        ("downgrade", "target version must be newer"),
        ("same_version", "target version must be newer"),
    ],
)
def test_launcher_request_validation_boundary_variants(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    variant: str,
    message: str,
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    _write_install_manifest(root)
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "local"))
    request = _request(root)
    if variant == "invalid_channel":
        request = update_launcher.UpdateLaunchRequest(
            install_root=root,
            current_version=request.current_version,
            target_version=request.target_version,
            channel="nightly",
            parent_pid=request.parent_pid,
        )
    elif variant == "invalid_current":
        request = update_launcher.UpdateLaunchRequest(
            install_root=root,
            current_version="not-a-version",
            target_version=request.target_version,
            channel=request.channel,
            parent_pid=request.parent_pid,
        )
    elif variant == "invalid_target":
        request = update_launcher.UpdateLaunchRequest(
            install_root=root,
            current_version=request.current_version,
            target_version="not-a-version",
            channel=request.channel,
            parent_pid=request.parent_pid,
        )
    elif variant == "downgrade":
        request = _request(root, target="2.3.0")
    else:
        request = _request(root, target="2.4.0")
    with pytest.raises(update_launcher.UpdateLaunchError, match=message):
        update_launcher.launch_update(request)


@pytest.mark.parametrize("parent_pid", [0, -1, 0x1_0000_0000, True, None])
def test_launcher_rejects_missing_or_malformed_parent_pid(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path, parent_pid: object
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    _write_install_manifest(root)
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "local"))
    request = update_launcher.UpdateLaunchRequest(
        install_root=root,
        current_version="2.4.0",
        target_version="2.5.0",
        channel="stable",
        parent_pid=parent_pid,  # type: ignore[arg-type]
    )
    with pytest.raises(update_launcher.UpdateLaunchError, match="parent PID"):
        update_launcher.launch_update(request)


def test_launcher_rejects_missing_local_app_data(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    _write_install_manifest(root)
    monkeypatch.delenv("LOCALAPPDATA", raising=False)
    with pytest.raises(update_launcher.UpdateLaunchError, match="LOCALAPPDATA"):
        update_launcher.launch_update(_request(root))


def test_launcher_rejects_copy_hash_mismatch_after_staging(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    _write_install_manifest(root)
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "local"))
    original_copy2 = update_launcher.shutil.copy2

    def tampering_copy2(source: Path, destination: Path) -> str:
        copied = original_copy2(source, destination)
        destination.write_bytes(b"tampered after copy")
        return str(copied)

    monkeypatch.setattr(update_launcher.shutil, "copy2", tampering_copy2)
    with pytest.raises(update_launcher.UpdateLaunchError, match="staged native updater"):
        update_launcher.launch_update(_request(root))
    runs = tmp_path / "local" / update_launcher.APP_NAME / "update-runs"
    assert list(runs.iterdir()) == []


def test_launch_rejects_tampered_installed_updater(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    _write_install_manifest(root)
    (root / update_launcher.UPDATER_NAME).write_bytes(b"tampered")
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "local"))
    with pytest.raises(update_launcher.UpdateLaunchError, match="does not match"):
        update_launcher.launch_update(_request(root))


def test_launch_failure_cleans_created_run(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    _write_install_manifest(root)
    local = tmp_path / "local"
    monkeypatch.setenv("LOCALAPPDATA", str(local))

    def fail_popen(*_args: object, **_kwargs: object) -> object:
        raise OSError("spawn denied")

    monkeypatch.setattr(update_launcher.subprocess, "Popen", fail_popen)
    with pytest.raises(update_launcher.UpdateLaunchError, match="could not launch"):
        update_launcher.launch_update(_request(root))
    assert list((local / update_launcher.APP_NAME / "update-runs").iterdir()) == []


def test_launcher_request_validation_remains_strict(tmp_path: Path) -> None:
    root = tmp_path / "install"
    root.mkdir()
    (root / "Sky-Auto-Player.exe").write_bytes(b"app")

    with pytest.raises(update_launcher.UpdateLaunchError, match="stable channel"):
        update_launcher._validate_request(_request(root, target="2.5.0rc1"))


def test_cleanup_only_removes_old_owned_run_directories(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path))
    runs = tmp_path / update_launcher.APP_NAME / "update-runs"
    old_run = runs / ("run-" + "a" * 32)
    recent_run = runs / ("run-" + "b" * 32)
    unknown = runs / "run-not-owned"
    old_run.mkdir(parents=True)
    recent_run.mkdir()
    unknown.mkdir()
    (old_run / "payload").write_text("old", encoding="utf-8")
    old_timestamp = time.time() - 120
    os.utime(old_run, (old_timestamp, old_timestamp))

    assert update_launcher.cleanup_stale_update_runs(max_age_s=60) == 1
    assert not old_run.exists()
    assert recent_run.exists()
    assert unknown.exists()
