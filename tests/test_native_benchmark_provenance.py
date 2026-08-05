from __future__ import annotations

import importlib.util
from pathlib import Path
from types import SimpleNamespace

_SPEC = importlib.util.spec_from_file_location(
    "bench_native_ab",
    Path(__file__).parents[1] / "scripts" / "bench_native_ab.py",
)
assert _SPEC is not None and _SPEC.loader is not None
bench_native_ab = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(bench_native_ab)


def test_build_wheel_sets_expected_github_sha(monkeypatch, tmp_path: Path) -> None:
    wheel_dir = tmp_path / "target" / "wheels"
    wheel_dir.mkdir(parents=True)
    wheel = wheel_dir / "sky_player_rs-0.0.0-py3-none-any.whl"
    wheel.write_bytes(b"wheel")
    captured: dict[str, object] = {}

    def fake_run(command, *, cwd, env=None, capture=False):
        captured["command"] = command
        captured["cwd"] = cwd
        captured["env"] = env
        return SimpleNamespace(returncode=0)

    monkeypatch.setattr(bench_native_ab, "_run", fake_run)

    result = bench_native_ab._build_wheel(
        tmp_path,
        env_file=None,
        expected_sha="baseline-sha",
    )

    assert result == wheel
    assert captured["env"]["GITHUB_SHA"] == "baseline-sha"  # type: ignore[index]
