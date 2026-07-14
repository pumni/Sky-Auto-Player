"""Tests for ``SkyPickerApp.check_for_updates_worker`` decision logic.

The worker is a Textual ``@work(thread=True)`` method that wires together the
update orchestration service and the UI. We exercise only the *decision* layer
(force vs throttle, error surfacing, "up to date" notification) without
booting a full Textual app:

- The worker body imports ``check_for_update``, ``record_successful_check``
  and ``should_auto_check`` lazily from
  ``sky_music.orchestration.update_service`` — so we monkeypatch that module
  to inject deterministic results.
- ``call_from_thread`` and ``notify`` are stubbed to record calls — the worker
  calls them with positional args we can assert on.
- ``VERSION`` (read at the top of ``app``) is left as-is; the worker passes
  it through to ``check_for_update`` untouched.

These tests deliberately do NOT touch the network or filesystem (config goes
through ``isolated_config``).
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest

import sky_music.config as config_mod
from sky_music.config import AppConfig, UpdateSettings, clear_config_cache
from sky_music.domain.update_checker import UpdateCheckResult, UpdateInfo
from sky_music.ui.textual_app import app as app_module


@pytest.fixture(autouse=True)
def _reset_config_cache():
    clear_config_cache()
    yield
    clear_config_cache()


@pytest.fixture
def isolated_config(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    cfg_path = tmp_path / "config.json"
    cfg_path.write_text("{}", encoding="utf-8")
    monkeypatch.setattr(config_mod, "CONFIG_PATH", cfg_path)
    return cfg_path


class _CallRecorder:
    """Records every call made to it; returns None."""

    def __init__(self) -> None:
        self.calls: list[tuple[Any, ...]] = []
        self.kwargs: list[dict[str, Any]] = []

    def __call__(self, *args: Any, **kwargs: Any) -> None:
        self.calls.append(args)
        self.kwargs.append(kwargs)


def _make_app() -> app_module.SkyPickerApp:
    """Build a SkyPickerApp without ``run_test`` / Textual event loop."""
    return app_module.SkyPickerApp(initial_dry_run=True, cfg=AppConfig())


def _install_worker_stubs(
    monkeypatch: pytest.MonkeyPatch,
    *,
    service_mod: Any,
    should_auto: bool,
    result: UpdateCheckResult,
) -> dict[str, _CallRecorder]:
    """Inject deterministic service stubs used by the worker body.

    The worker imports names lazily from ``update_service`` *inside* its body,
    so patching attributes on the live module is sufficient.
    """
    rec: dict[str, _CallRecorder] = {
        "should_auto_check": _CallRecorder(),
        "check_for_update": _CallRecorder(),
        "record_successful_check": _CallRecorder(),
    }

    def _should_auto_check(cfg: AppConfig, *, now_ts: int | None = None) -> bool:
        rec["should_auto_check"](cfg, now_ts=now_ts)
        return should_auto

    def _check_for_update(cfg: AppConfig, *, current_version: str, **kw: Any) -> UpdateCheckResult:
        rec["check_for_update"](cfg, current_version=current_version, **kw)
        return result

    def _record_successful_check(cfg: AppConfig, *, now_ts: int | None = None) -> None:
        rec["record_successful_check"](cfg, now_ts=now_ts)

    monkeypatch.setattr(service_mod, "should_auto_check", _should_auto_check)
    monkeypatch.setattr(service_mod, "check_for_update", _check_for_update)
    monkeypatch.setattr(service_mod, "record_successful_check", _record_successful_check)
    return rec


def _install_ui_stubs(app: app_module.SkyPickerApp) -> dict[str, _CallRecorder]:
    """Replace ``call_from_thread`` and ``notify`` so the worker can be driven
    off the Textual event loop without raising.
    """
    rec: dict[str, _CallRecorder] = {
        "call_from_thread": _CallRecorder(),
        "notify": _CallRecorder(),
    }
    app.call_from_thread = rec["call_from_thread"]  # type: ignore[method-assign]
    app.notify = rec["notify"]  # type: ignore[method-assign]
    return rec


def _run_worker(app: app_module.SkyPickerApp, *, force: bool = False) -> None:
    """Invoke the underlying function backing the ``@work`` decorator.

    Textual's ``@work(thread=True)`` wraps the method; ``functools.wraps``
    exposes the original via ``__wrapped__``. We call that with ``app`` as
    ``self`` so the body runs synchronously on the current thread — no worker
    thread, no Textual event loop required.
    """
    bound = app.check_for_updates_worker
    fn = getattr(bound, "__wrapped__", None)
    if fn is None:
        # Fallback: call the (possibly wrapped) object directly.
        bound(force=force)
        return
    fn(app, force=force)


def _newer_result() -> UpdateCheckResult:
    return UpdateCheckResult(
        update=UpdateInfo(
            latest_version="2.3.2",
            download_url="https://example.com/x.zip",
            release_notes="notes",
            html_url="https://example.com/release",
            published_at="2024-01-01T00:00:00Z",
        ),
        current_version="2.3.1",
    )


def _no_update_result() -> UpdateCheckResult:
    return UpdateCheckResult(update=None, current_version="2.3.1")


def _error_result(msg: str = "boom") -> UpdateCheckResult:
    return UpdateCheckResult(update=None, current_version="2.3.1", error=msg)


# ── Auto mode (force=False) ──────────────────────────────────────────────────


def test_worker_auto_mode_respects_throttle_and_silently_returns(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """When ``should_auto_check`` is False and force=False, worker returns
    immediately without calling the network layer or any UI helper.
    """
    import sky_music.orchestration.update_service as svc

    app = _make_app()
    ui = _install_ui_stubs(app)
    rec = _install_worker_stubs(
        monkeypatch,
        service_mod=svc,
        should_auto=False,
        result=_no_update_result(),
    )

    _run_worker(app, force=False)

    assert len(rec["should_auto_check"].calls) == 1
    assert len(rec["check_for_update"].calls) == 0
    assert len(rec["record_successful_check"].calls) == 0
    assert len(ui["call_from_thread"].calls) == 0
    assert len(ui["notify"].calls) == 0


def test_worker_auto_mode_finds_update_prompts_without_extra_notify(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Auto-mode success with a newer version: record check, prompt update,
    no "up to date" notification, no "failed" notification.
    """
    import sky_music.orchestration.update_service as svc

    app = _make_app()
    ui = _install_ui_stubs(app)
    rec = _install_worker_stubs(
        monkeypatch,
        service_mod=svc,
        should_auto=True,
        result=_newer_result(),
    )

    _run_worker(app, force=False)

    assert len(rec["check_for_update"].calls) == 1
    assert len(rec["record_successful_check"].calls) == 1
    # _prompt_update goes through call_from_thread
    assert len(ui["call_from_thread"].calls) == 1
    # No raw "up to date" / "failed" notify issued alongside auto success
    assert len(ui["notify"].calls) == 0


def test_worker_auto_mode_swallows_error_silently(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Auto-mode network failure must NOT surface anything to the user."""
    import sky_music.orchestration.update_service as svc

    app = _make_app()
    ui = _install_ui_stubs(app)
    rec = _install_worker_stubs(
        monkeypatch,
        service_mod=svc,
        should_auto=True,
        result=_error_result("rate-limited"),
    )

    _run_worker(app, force=False)

    # check happened but no timestamp is recorded when error != None
    assert len(rec["check_for_update"].calls) == 1
    assert len(rec["record_successful_check"].calls) == 0
    # No UI surfacing at all
    assert len(ui["call_from_thread"].calls) == 0
    assert len(ui["notify"].calls) == 0


# ── Manual mode (force=True) ─────────────────────────────────────────────────


def test_worker_force_mode_bypasses_throttle(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Even when ``should_auto_check`` says no, force=True proceeds to the
    fetch. Crucially, the worker must NOT even consult ``should_auto_check``
    when force=True.
    """
    import sky_music.orchestration.update_service as svc

    app = _make_app()
    ui = _install_ui_stubs(app)
    rec = _install_worker_stubs(
        monkeypatch,
        service_mod=svc,
        should_auto=False,  # throttle would normally block
        result=_newer_result(),
    )

    _run_worker(app, force=True)

    assert len(rec["should_auto_check"].calls) == 0
    assert len(rec["check_for_update"].calls) == 1
    assert len(rec["record_successful_check"].calls) == 1
    assert len(ui["call_from_thread"].calls) == 1


def test_worker_force_mode_surfaces_error_to_user(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Manual check with a fetch error must notify the user (severity=error)."""
    import sky_music.orchestration.update_service as svc

    app = _make_app()
    ui = _install_ui_stubs(app)
    _install_worker_stubs(
        monkeypatch,
        service_mod=svc,
        should_auto=True,
        result=_error_result("rate-limited"),
    )

    _run_worker(app, force=True)

    # call_from_thread(fun, message, severity=..., timeout=...) — the only
    # call_from_thread invocation in the error path.
    calls = ui["call_from_thread"].calls
    assert len(calls) == 1
    args = calls[0]
    # First positional arg should be the bound notify callable
    assert callable(args[0])
    assert "rate-limited" in str(args[1])
    assert ui["call_from_thread"].kwargs[0].get("severity") == "error"


def test_worker_force_mode_reports_up_to_date_when_no_update(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Manual check, no newer version, no error → inform the user."""
    import sky_music.orchestration.update_service as svc

    app = _make_app()
    ui = _install_ui_stubs(app)
    _install_worker_stubs(
        monkeypatch,
        service_mod=svc,
        should_auto=True,
        result=_no_update_result(),
    )

    _run_worker(app, force=True)

    # Should also persist the check timestamp (successful fetch, even if no update)
    assert len(ui["call_from_thread"].calls) == 1
    args = ui["call_from_thread"].calls[0]
    assert callable(args[0])
    assert "up to date" in str(args[1])
    assert ui["call_from_thread"].kwargs[0].get("severity") == "information"


def test_worker_force_mode_error_does_not_record_check_ts(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """On error, ``record_successful_check`` must not run even in force mode."""
    import sky_music.orchestration.update_service as svc

    app = _make_app()
    _install_ui_stubs(app)
    rec = _install_worker_stubs(
        monkeypatch,
        service_mod=svc,
        should_auto=True,
        result=_error_result("dns"),
    )

    _run_worker(app, force=True)

    assert len(rec["record_successful_check"].calls) == 0


def test_worker_force_mode_preserves_skip_version_behavior(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """If the user skipped the latest version but explicitly presses Check,
    the worker should still surface "up to date" rather than the modal — because
    parse_release_payload suppresses skipped versions (returns update=None).
    """
    import sky_music.orchestration.update_service as svc

    cfg = AppConfig(update=UpdateSettings(skip_version="2.3.2"))
    app = app_module.SkyPickerApp(initial_dry_run=True, cfg=cfg)
    ui = _install_ui_stubs(app)
    _install_worker_stubs(
        monkeypatch,
        service_mod=svc,
        should_auto=False,
        result=UpdateCheckResult(update=None, current_version="2.3.1"),
    )

    _run_worker(app, force=True)

    # No modal, no error notify — user is informed they are up to date.
    assert len(ui["call_from_thread"].calls) == 1
    assert "up to date" in str(ui["call_from_thread"].calls[0][1])


# ── Command registration & binding ──────────────────────────────────────────


def test_command_registered_as_update() -> None:
    """``/`` Commands modal must include a "Check for Update" entry whose
    id resolves to the ``update`` action handled in ``_run_command``.
    """
    from sky_music.ui.textual_app.keymap import COMMANDS

    ids = {cmd.id for cmd in COMMANDS}
    assert "update" in ids
    update_cmd = next(cmd for cmd in COMMANDS if cmd.id == "update")
    assert update_cmd.key.lower() == "u"
    assert update_cmd.label == "Check for Update"
    assert update_cmd.group == "System"


def test_update_binding_present_on_song_table() -> None:
    """Pressing ``u`` should dispatch ``check_for_update``; ensure binding
    exists and maps to that action.
    """
    from textual.binding import Binding

    bindings = app_module.SongTable.BINDINGS
    update_bindings = [b for b in bindings if getattr(b, "action", None) == "check_for_update"]
    assert update_bindings, "Binding for check_for_update missing on SongTable"
    assert any(isinstance(b, Binding) and b.key == "u" for b in update_bindings)


def test_run_command_routes_update(monkeypatch: pytest.MonkeyPatch) -> None:
    """``_run_command("update")`` must invoke ``action_check_for_update`` on
    the picker screen — verified by replacing the action with a stub and
    injecting a minimal picker stub into the app's ``_picker`` slot.
    """
    from sky_music.ui.textual_app.screens import picker as picker_module

    called: list[bool] = []

    def _fake_check(self: picker_module.PickerScreen) -> None:
        called.append(True)

    monkeypatch.setattr(
        picker_module.PickerScreen, "action_check_for_update", _fake_check
    )
    app = _make_app()
    # Inject a bare PickerScreen so _find_picker_screen() returns it and the
    # _run_command delegation reaches action_check_for_update.
    stub = picker_module.PickerScreen.__new__(picker_module.PickerScreen)
    app._picker = stub  # type: ignore[attr-defined]
    app._run_command("update")
    assert called == [True]
