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

import contextlib
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


# ── _apply_staged & download_and_apply_update_worker guards ─────────────────


def test_apply_staged_catches_installer_error_and_notifies(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Installer-side error must surface as a user notify, not a crash."""
    from sky_music.infrastructure.update_installer import UpdateInstallerError

    app = _make_app()
    ui = _install_ui_stubs(app)

    def _raise(*args: Any, **kwargs: Any) -> None:
        raise UpdateInstallerError("boom")

    import sky_music.orchestration.update_service as svc
    monkeypatch.setattr(svc, "apply_staged_update", _raise)

    staged = object()  # opaque; _apply_staged forwards it
    app._apply_staged(staged, install_dir=None)

    # notify was called exactly once with severity=error and message contains boom
    assert len(ui["notify"].calls) == 1
    msg = str(ui["notify"].calls[0][0])
    assert "boom" in msg
    assert ui["notify"].kwargs[0].get("severity") == "error"


def test_apply_staged_invokes_service_on_success(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Happy path: service is called with the staged object as documented."""
    app = _make_app()
    _install_ui_stubs(app)
    captured: list[Any] = []

    def _ok(staged: Any, *, install_dir: Any = None) -> None:
        captured.append((staged, install_dir))

    import sky_music.orchestration.update_service as svc
    monkeypatch.setattr(svc, "apply_staged_update", _ok)

    sentinel = object()
    app._apply_staged(sentinel, install_dir=None)
    assert captured == [(sentinel, None)]


def test_download_worker_defers_when_in_playback(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """When ``playback_mode != PICKER``, the worker must NOT download —
    user is asked to exit playback first.
    """
    app = _make_app()
    ui = _install_ui_stubs(app)
    app.playback_mode = app_module.PlaybackMode.PLAYING

    downloaded: list[bool] = []

    import sky_music.orchestration.update_service as svc
    monkeypatch.setattr(
        svc,
        "download_and_verify_update",
        lambda *a, **k: downloaded.append(True) or None,
    )

    release = UpdateInfo(
        latest_version="2.3.2",
        download_url="https://example.com/x.zip",
        release_notes="", html_url="", published_at="",
    )
    bound = app.download_and_apply_update_worker
    fn = getattr(bound, "__wrapped__", bound)
    fn(app, release=release)

    # download was NOT attempted
    assert downloaded == []
    # a warning notify was scheduled via call_from_thread
    assert len(ui["call_from_thread"].calls) == 1
    msg = str(ui["call_from_thread"].calls[0][1])
    assert "exit playback" in msg.lower()
    assert ui["call_from_thread"].kwargs[0].get("severity") == "warning"


def test_download_worker_pushes_modal_and_forwards_progress(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """In PICKER mode, the worker schedules a push of UpdateProgressModal via
    ``call_from_thread`` and forwards progress callbacks to the modal.
    """
    import sky_music.orchestration.update_service as svc
    from sky_music.infrastructure.update_installer import StagedUpdate
    from sky_music.orchestration.update_service import DownloadOutcome
    from sky_music.ui.textual_app.modals import UpdateProgressModal

    app = _make_app()
    app.playback_mode = app_module.PlaybackMode.PICKER

    # Make call_from_thread actually run the synced callable so push_screen is
    # exercised and the progress forwarding path runs end-to-end.
    def _call_from_thread(fn: Any, *args: Any, **kwargs: Any) -> None:
        with contextlib.suppress(Exception):
            fn(*args, **kwargs)

    app.call_from_thread = _call_from_thread  # type: ignore[method-assign]

    push_calls: list[Any] = []
    def _push_screen(modal: Any) -> None:
        push_calls.append(modal)
    app.push_screen = _push_screen  # type: ignore[method-assign]

    progress_seen: list[tuple[int, int | None]] = []

    def _fake_download(release: Any, *, install_dir: Any = None, progress=None):
        if progress is not None:
            progress(1024 * 1024, 10 * 1024 * 1024)
            progress(5 * 1024 * 1024, 10 * 1024 * 1024)
            progress_seen.append((5 * 1024 * 1024, 10 * 1024 * 1024))
        return DownloadOutcome(
            staged=StagedUpdate(staging_dir=Path("/tmp/x"), new_version="2.3.2"),
            error=None,
        )

    monkeypatch.setattr(svc, "download_and_verify_update", _fake_download)

    # Block apply_staged_update from sys.exit-ing the test process.
    def _apply_noop(staged: Any, *, install_dir: Any = None) -> None:
        return None

    monkeypatch.setattr(svc, "apply_staged_update", _apply_noop)

    release = UpdateInfo(
        latest_version="2.3.2",
        download_url="https://example.com/x.zip",
        release_notes="", html_url="", published_at="",
    )
    bound = app.download_and_apply_update_worker
    fn = getattr(bound, "__wrapped__", bound)
    fn(app, release=release)

    assert len(push_calls) == 1
    modal = push_calls[0]
    assert isinstance(modal, UpdateProgressModal)
    assert modal.latest_version == "2.3.2"
    assert progress_seen == [(5 * 1024 * 1024, 10 * 1024 * 1024)]


def test_download_worker_failure_updates_modal_status(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import sky_music.orchestration.update_service as svc
    from sky_music.orchestration.update_service import DownloadOutcome

    app = _make_app()
    app.playback_mode = app_module.PlaybackMode.PICKER

    def _call_from_thread(fn: Any, *args: Any, **kwargs: Any) -> None:
        with contextlib.suppress(Exception):
            fn(*args, **kwargs)

    app.call_from_thread = _call_from_thread  # type: ignore[method-assign]

    modal_seen: list[Any] = []
    def _push_screen(modal: Any) -> None:
        modal_seen.append(modal)
    app.push_screen = _push_screen  # type: ignore[method-assign]

    monkeypatch.setattr(
        svc,
        "download_and_verify_update",
        lambda *a, **k: DownloadOutcome(staged=None, error="checksum missing"),
    )

    release = UpdateInfo(
        latest_version="2.3.2",
        download_url="https://example.com/x.zip",
        release_notes="", html_url="", published_at="",
    )
    fn = getattr(app.download_and_apply_update_worker, "__wrapped__", None) \
        or app.download_and_apply_update_worker
    fn(app, release=release)

    # The failure path chose the modal-status route (modal was pushed) and
    # did not call the previous `self.notify(...)` path. Because notify is
    # stubbed by direct invocation through _call_from_thread, we assert the
    # modal was pushed so the user can read the in-modal error message.
    assert len(modal_seen) == 1


# ── _restore_pending_update_indicator ─────────────────────────────────────────


def test_restore_pending_indicator_skips_when_no_pending_marker(
    isolated_config: Path,
) -> None:
    """No ``pending_update_version`` in config → no indicator restored, the
    app bar stays at the plain ``v{VERSION}``.
    """
    app = _make_app()
    set_version_calls: list[str] = []
    # Build a tiny fake GradientHeader stub recordable as a Static-method-style
    # so we capture what set_version was asked to render.

    class _Header:
        def set_version(self, *args: Any, **kw: Any) -> None:
            set_version_calls.append(str(args[0]))

    # _set_version_indicator + _restore_pending_update_indicator both funnel
    # through self.query_one("#appbar", GradientHeader). Patch query_one.
    def _query(selector: str, _type: Any = None) -> Any:
        return _Header()
    app.query_one = _query  # type: ignore[method-assign]

    app._set_version_indicator()
    app._restore_pending_update_indicator()

    # Only the baseline v{VERSION} call should have run.
    assert set_version_calls == [f"v{app_module.VERSION}"]


def test_restore_pending_indicator_applies_arrow_for_newer_pending(
    isolated_config: Path,
) -> None:
    """When config has a pending version strictly newer than the running one,
    the indicator flips to ``v{VERSION} ↑`` with highlight.
    """
    cfg = AppConfig(update=UpdateSettings(pending_update_version="99.99.99"))
    app = app_module.SkyPickerApp(initial_dry_run=True, cfg=cfg)

    set_version_calls: list[dict[str, Any]] = []

    class _Header:
        def set_version(self, *args: Any, **kw: Any) -> None:
            set_version_calls.append({"label": args[0], **kw})

    def _query(selector: str, _type: Any = None) -> Any:
        return _Header()
    app.query_one = _query  # type: ignore[method-assign]

    app._set_version_indicator()
    app._restore_pending_update_indicator()

    # Two calls total: baseline + highlighted arrow.
    assert len(set_version_calls) == 2
    baseline, arrow = set_version_calls
    assert baseline["label"] == f"v{app_module.VERSION}"
    assert arrow["label"] == f"v{app_module.VERSION} \u2191"
    assert arrow.get("highlight") is True


def test_restore_pending_indicator_clears_stale_pending(
    isolated_config: Path,
) -> None:
    """If the stored pending version is no longer newer (e.g. user manually
    upgraded past it), the marker is cleared from config and no arrow showed.
    """
    cfg = AppConfig(update=UpdateSettings(pending_update_version="0.0.1"))
    app = app_module.SkyPickerApp(initial_dry_run=True, cfg=cfg)

    set_version_calls: list[str] = []

    class _Header:
        def set_version(self, *args: Any, **kw: Any) -> None:
            set_version_calls.append(str(args[0]))

    def _query(selector: str, _type: Any = None) -> Any:
        return _Header()
    app.query_one = _query  # type: ignore[method-assign]

    app._set_version_indicator()
    app._restore_pending_update_indicator()

    # Only baseline; pending marker was cleared (no second call, no arrow).
    assert set_version_calls == [f"v{app_module.VERSION}"]
    # And config now has empty pending_update_version.
    from sky_music.config import load_config
    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.pending_update_version == ""
