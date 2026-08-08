from __future__ import annotations

import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace

import pytest

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


def test_benchmark_subprocess_output_uses_utf8_replacement(monkeypatch, tmp_path: Path) -> None:
    captured: dict[str, object] = {}

    def fake_subprocess_run(command, **kwargs):
        captured["command"] = command
        captured.update(kwargs)
        return SimpleNamespace(returncode=0, stdout="", stderr="")

    monkeypatch.setattr(bench_native_ab.subprocess, "run", fake_subprocess_run)

    bench_native_ab._run(["benchmark"], cwd=tmp_path, capture=True)

    assert captured["encoding"] == "utf-8"
    assert captured["errors"] == "replace"


def _ab_args(output_dir: Path) -> SimpleNamespace:
    return SimpleNamespace(
        baseline_ref="baseline",
        candidate_ref="candidate",
        actions=1,
        dispatch_repeats=1,
        command_samples=0,
        polyphony="1",
        backend="mock",
        allow_real_input=False,
        game_fps=60,
        lead_mode="fixed",
        fixed_lead_us=0,
        gap_profile="hot",
        warmup_cycles=0,
        rt_priority_mode="off",
        budget_seconds=1.0,
        output_dir=output_dir,
    )


def test_benchmark_command_propagates_real_backend_explicitly(tmp_path: Path) -> None:
    args = _ab_args(tmp_path)
    args.backend = "sendinput"
    args.allow_real_input = True
    args.label = "candidate"
    command = bench_native_ab._benchmark_command(
        output=tmp_path / "candidate.json",
        expected_native_commit="candidate-sha",
        args=args,
    )
    assert command[command.index("--backend") + 1] == "sendinput"
    assert "--allow-real-input" in command


def _run_ab_scenario(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    scenario: str,
) -> tuple[int, Path]:
    output_dir = tmp_path / scenario
    args = _ab_args(output_dir)
    monkeypatch.setattr(bench_native_ab.os, "name", "nt")
    monkeypatch.setattr(bench_native_ab, "_parse_args", lambda: args)
    monkeypatch.setattr(
        bench_native_ab,
        "_full_sha",
        lambda ref, *, cwd: "baseline-sha" if ref == "baseline" else "candidate-sha",
    )
    monkeypatch.setattr(bench_native_ab, "_assert_clean", lambda cwd, label: None)

    def fake_build(
        repo: Path,
        *,
        env_file: Path | None,
        expected_sha: str,
        runner=None,
    ) -> Path:
        if scenario == "candidate-build" and expected_sha == "candidate-sha":
            raise RuntimeError("synthetic candidate build failure")
        return repo / f"sky_player_rs-{expected_sha}.whl"

    monkeypatch.setattr(bench_native_ab, "_build_wheel", fake_build)
    monkeypatch.setattr(bench_native_ab, "_install_wheel", lambda wheel, *, runner=None: None)

    def fake_run(command, *, cwd, capture=False, env=None):
        if command[:4] == ["git", "worktree", "add", "--detach"]:
            Path(command[4]).mkdir(parents=True)
            return SimpleNamespace(returncode=0, stdout="", stderr="")
        if "scripts/bench_native_acceptance.py" in command:
            role = command[command.index("--label") + 1]
            output = Path(command[command.index("--output") + 1])
            if role == "baseline" or scenario == "regression-gate":
                output.write_text(
                    json.dumps({"statistics_eligible": True}), encoding="utf-8"
                )
            if role == "candidate" and scenario in {"candidate-build", "candidate-benchmark"}:
                return SimpleNamespace(returncode=7, stdout="candidate out", stderr="candidate err")
            if role == "candidate" and scenario == "regression-gate":
                return SimpleNamespace(returncode=1, stdout="gate out", stderr="gate err")
            return SimpleNamespace(returncode=0, stdout="baseline out", stderr="")
        return SimpleNamespace(returncode=0, stdout="", stderr="")

    monkeypatch.setattr(bench_native_ab, "_run", fake_run)
    return bench_native_ab.main(), output_dir


def test_ab_runner_preserves_artifacts_when_candidate_build_fails(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    exit_code, output_dir = _run_ab_scenario(monkeypatch, tmp_path, "candidate-build")

    assert exit_code == 1
    assert (output_dir / "candidate-failure.json").exists()
    assert (output_dir / "ab-summary.json").exists()
    assert (output_dir / "build-provenance.json").exists()
    assert json.loads((output_dir / "failure-summary.json").read_text())["stage"] == "candidate_build"
    assert (output_dir / "candidate-stdout.log").exists()
    assert (output_dir / "candidate-stderr.log").exists()


def test_ab_runner_preserves_artifacts_when_candidate_benchmark_fails(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    exit_code, output_dir = _run_ab_scenario(monkeypatch, tmp_path, "candidate-benchmark")

    assert exit_code == 7
    assert (output_dir / "candidate-failure.json").exists()
    failure = json.loads((output_dir / "failure-summary.json").read_text())
    assert failure["stage"] == "candidate_benchmark"
    assert failure["exit_code"] == 7
    assert failure["candidate_report_exists"] is False
    assert failure["statistics_eligible"] is False


def test_ab_runner_preserves_candidate_report_when_regression_gate_fails(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    exit_code, output_dir = _run_ab_scenario(monkeypatch, tmp_path, "regression-gate")

    assert exit_code == 1
    assert (output_dir / "candidate.json").exists()
    assert not (output_dir / "candidate-failure.json").exists()
    failure = json.loads((output_dir / "failure-summary.json").read_text())
    assert failure["stage"] == "regression_gate"
    assert failure["candidate_report_exists"] is True


def test_ab_summary_marks_failed_run_statistics_ineligible(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    exit_code, output_dir = _run_ab_scenario(monkeypatch, tmp_path, "candidate-benchmark")

    assert exit_code != 0
    summary = json.loads((output_dir / "ab-summary.json").read_text())
    assert summary["statistics_eligible"] is False
    assert summary["failure_stage"] == "candidate_benchmark"
