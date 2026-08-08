"""Run schema-compatible native benchmark A/B on one Windows runner.

The candidate checkout supplies the benchmark harness for both legs.  Only the
native wheel changes between legs, so the reports distinguish harness Git SHA
from the native build commit instead of conflating the two.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
from collections.abc import Callable
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BENCHMARK_BUDGET_SECONDS = 600.0
RunCommand = Callable[..., subprocess.CompletedProcess[str]]


def _run(
    command: list[str],
    *,
    cwd: Path,
    capture: bool = False,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    print("+", " ".join(command))
    return subprocess.run(
        command,
        cwd=cwd,
        env=env,
        capture_output=capture,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )


def _full_sha(ref: str, *, cwd: Path) -> str:
    result = _run(["git", "rev-parse", "--verify", f"{ref}^{{commit}}"], cwd=cwd, capture=True)
    if result.returncode != 0 or not result.stdout.strip():
        raise RuntimeError(f"could not resolve Git ref {ref!r}: {result.stderr.strip()}")
    return result.stdout.strip().lower()


def _assert_clean(cwd: Path, label: str) -> None:
    result = _run(
        ["git", "status", "--porcelain=v1", "--untracked-files=all"],
        cwd=cwd,
        capture=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"could not inspect {label} worktree")
    if result.stdout.strip():
        raise RuntimeError(f"{label} worktree is dirty:\n{result.stdout}")


def _build_wheel(
    repo: Path,
    *,
    env_file: Path | None,
    expected_sha: str,
    runner: RunCommand | None = None,
) -> Path:
    command = ["uv", "run"]
    if env_file is not None:
        command.extend(["--env-file", str(env_file)])
    command.extend(["python", "scripts/build_rust_wheel.py", "--test-support"])
    build_env = os.environ.copy()
    build_env["GITHUB_SHA"] = expected_sha
    run = _run if runner is None else runner
    result = run(command, cwd=repo, env=build_env, capture=True)
    if result.returncode != 0:
        raise RuntimeError(f"native wheel build failed for {repo}: {result.returncode}")
    wheel_dir_candidates = (repo / "target" / "wheels", repo / "rust" / "target" / "wheels")
    wheels = [wheel for directory in wheel_dir_candidates if directory.exists() for wheel in directory.glob("sky_player_rs-*.whl")]
    if not wheels:
        raise RuntimeError(f"native wheel build produced no wheel in {repo}")
    return max(wheels, key=lambda path: path.stat().st_mtime_ns)


def _install_wheel(wheel: Path, *, runner: RunCommand | None = None) -> None:
    run = _run if runner is None else runner
    result = run(
        ["uv", "pip", "install", "--python", sys.executable, "--reinstall", "--no-deps", str(wheel)],
        cwd=ROOT,
    )
    if result.returncode != 0:
        raise RuntimeError(f"wheel install failed: {wheel}")


def _benchmark_command(
    *,
    output: Path,
    expected_native_commit: str,
    args: argparse.Namespace,
    baseline: Path | None = None,
) -> list[str]:
    command = ["uv", "run"]
    env_file = ROOT / ".env"
    if env_file.exists():
        command.extend(["--env-file", ".env"])
    command.extend(
        [
            "python",
            "scripts/bench_native_acceptance.py",
            "--backend",
            args.backend,
            "--actions",
            str(args.actions),
            "--dispatch-repeats",
            str(args.dispatch_repeats),
            "--command-samples",
            str(args.command_samples),
            "--polyphony",
            args.polyphony,
            "--game-fps",
            str(args.game_fps),
            "--lead-mode",
            args.lead_mode,
            "--fixed-lead-us",
            str(args.fixed_lead_us),
            "--gap-profile",
            args.gap_profile,
            "--warmup-cycles",
            str(args.warmup_cycles),
            "--rt-priority-mode",
            args.rt_priority_mode,
            "--budget-seconds",
            str(args.budget_seconds),
            "--expected-native-build-commit",
            expected_native_commit,
            "--label",
            args.label,
            "--output",
            str(output),
        ]
    )
    if args.command_samples == 0:
        command.append("--skip-command-samples")
    if args.allow_real_input:
        command.append("--allow-real-input")
    if baseline is not None:
        command.extend(["--baseline", str(baseline)])
    return command


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-ref", required=True)
    parser.add_argument("--candidate-ref", required=True)
    parser.add_argument("--actions", type=int, required=True)
    parser.add_argument("--dispatch-repeats", type=int, required=True)
    parser.add_argument("--command-samples", type=int, required=True)
    parser.add_argument("--polyphony", default="1,2,3,5,8,15")
    parser.add_argument(
        "--backend",
        choices=("mock", "sendinput"),
        default="mock",
        help="backend used by both A/B legs (default: mock)",
    )
    parser.add_argument(
        "--allow-real-input",
        action="store_true",
        help="required with --backend sendinput; use only on an isolated Windows host",
    )
    parser.add_argument("--game-fps", type=int, default=60)
    parser.add_argument("--lead-mode", choices=("fixed", "adaptive"), required=True)
    parser.add_argument("--fixed-lead-us", type=int, default=0)
    parser.add_argument("--gap-profile", choices=("hot", "cold"), required=True)
    parser.add_argument("--warmup-cycles", type=int, required=True)
    parser.add_argument("--rt-priority-mode", default="off")
    parser.add_argument(
        "--budget-seconds",
        type=float,
        default=DEFAULT_BENCHMARK_BUDGET_SECONDS,
    )
    parser.add_argument("--output-dir", type=Path, required=True)
    return parser.parse_args()


def _append_log(path: Path, value: str | None) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as stream:
        stream.write(value or "")


def _run_logged(
    command: list[str],
    *,
    cwd: Path,
    role: str,
    output_dir: Path,
    env: dict[str, str] | None = None,
    runner: RunCommand | None = None,
) -> subprocess.CompletedProcess[str]:
    run = _run if runner is None else runner
    result = run(command, cwd=cwd, env=env, capture=True)
    _append_log(output_dir / f"{role}-stdout.log", getattr(result, "stdout", ""))
    _append_log(output_dir / f"{role}-stderr.log", getattr(result, "stderr", ""))
    return result


def _host_fingerprint() -> dict[str, str]:
    return {
        "platform": platform.platform(),
        "machine": platform.machine(),
        "processor": platform.processor(),
        "windows_build": platform.version(),
    }


def _benchmark_matrix(args: argparse.Namespace) -> dict[str, Any]:
    return {
        "actions": args.actions,
        "dispatch_repeats": args.dispatch_repeats,
        "command_samples": args.command_samples,
        "polyphony": args.polyphony,
        "backend": args.backend,
        "allow_real_input": args.allow_real_input,
        "game_fps": args.game_fps,
        "lead_mode": args.lead_mode,
        "fixed_lead_us": args.fixed_lead_us,
        "gap_profile": args.gap_profile,
        "warmup_cycles": args.warmup_cycles,
        "rt_priority_mode": args.rt_priority_mode,
        "budget_seconds": args.budget_seconds,
    }


def _ab_provenance(
    *,
    baseline_sha: str,
    candidate_sha: str,
    args: argparse.Namespace,
    dirty_worktree: bool = False,
) -> dict[str, Any]:
    return {
        "harness_git_sha": candidate_sha,
        "native_build_sha": candidate_sha,
        "baseline_sha": baseline_sha,
        "candidate_sha": candidate_sha,
        "dirty_worktree": dirty_worktree,
        "host_fingerprint": _host_fingerprint(),
        "command_line": list(sys.argv),
        "benchmark_matrix": _benchmark_matrix(args),
        "backend": args.backend,
        "real_input_qualification": args.backend == "sendinput" and args.allow_real_input,
    }


def _add_report_provenance(path: Path, provenance: dict[str, Any]) -> bool:
    if not path.exists():
        return False
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return False
    if not isinstance(payload, dict):
        return False
    payload["ab_provenance"] = provenance
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return True


def _write_json(path: Path, payload: dict[str, Any]) -> None:
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


def _failure_report_path(output_dir: Path, role: str) -> Path:
    return output_dir / f"{role}-failure.json"


def main() -> int:
    args = _parse_args()
    if os.name != "nt":
        raise SystemExit("native A/B benchmark requires Windows")
    if args.lead_mode == "adaptive" and args.fixed_lead_us != 0:
        raise SystemExit("--fixed-lead-us must be 0 in adaptive mode")
    if args.backend == "sendinput" and not args.allow_real_input:
        raise SystemExit("--backend sendinput requires --allow-real-input")

    baseline_sha = _full_sha(args.baseline_ref, cwd=ROOT)
    candidate_sha = _full_sha(args.candidate_ref, cwd=ROOT)
    current_sha = _full_sha("HEAD", cwd=ROOT)
    if candidate_sha != current_sha:
        raise RuntimeError(
            "candidate-ref must resolve to the current checkout: "
            f"candidate={candidate_sha} current={current_sha}"
        )
    _assert_clean(ROOT, "candidate")

    output_dir = args.output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    for role in ("baseline", "candidate"):
        (output_dir / f"{role}-stdout.log").write_text("", encoding="utf-8")
        (output_dir / f"{role}-stderr.log").write_text("", encoding="utf-8")

    baseline_report = output_dir / "baseline.json"
    candidate_report = output_dir / "candidate.json"
    commands: dict[str, list[str]] = {}
    executed_commands: dict[str, list[list[str]]] = {"baseline": [], "candidate": []}
    provenance: dict[str, Any] = {
        "harness_git_sha": candidate_sha,
        "baseline_ref": args.baseline_ref,
        "baseline_sha": baseline_sha,
        "candidate_ref": args.candidate_ref,
        "candidate_sha": candidate_sha,
        "dirty_worktree": False,
        "host_fingerprint": _host_fingerprint(),
        "command_line": list(sys.argv),
        "benchmark_matrix": _benchmark_matrix(args),
        "roles": {},
    }
    provenance["ab_provenance"] = _ab_provenance(
        baseline_sha=baseline_sha,
        candidate_sha=candidate_sha,
        args=args,
    )

    def role_runner(role: str) -> RunCommand:
        def run(
            command: list[str],
            *,
            cwd: Path,
            capture: bool = False,
            env: dict[str, str] | None = None,
        ) -> subprocess.CompletedProcess[str]:
            nonlocal observed_exit_code
            executed_commands[role].append(list(command))
            result = _run_logged(
                command,
                cwd=cwd,
                role=role,
                output_dir=output_dir,
                env=env,
            )
            if result.returncode != 0:
                observed_exit_code = result.returncode
            return result

        return run

    stage = "candidate_build"
    failure_role = "candidate"
    failure_summary: dict[str, Any] | None = None
    observed_exit_code = 1
    exit_code = 1
    worktree: Path | None = None
    try:
        with tempfile.TemporaryDirectory(prefix="sky-native-ab-") as temp_dir:
            worktree = Path(temp_dir) / "baseline"
            add = _run(
                ["git", "worktree", "add", "--detach", str(worktree), baseline_sha],
                cwd=ROOT,
            )
            if add.returncode != 0:
                observed_exit_code = add.returncode
                raise RuntimeError("could not create baseline temporary worktree")
            _assert_clean(worktree, "baseline")
            env_file = ROOT / ".env"
            if env_file.exists():
                shutil.copy2(env_file, worktree / ".env")

            stage = "baseline_build"
            failure_role = "baseline"
            baseline_wheel = _build_wheel(
                worktree,
                env_file=Path(".env") if env_file.exists() else None,
                expected_sha=baseline_sha,
                runner=role_runner("baseline"),
            )
            provenance["roles"]["baseline"] = {
                "native_build_commit": baseline_sha,
                "wheel": baseline_wheel.name,
            }

            stage = "baseline_benchmark"
            commands["baseline"] = _benchmark_command(
                output=baseline_report,
                expected_native_commit=baseline_sha,
                args=argparse.Namespace(**vars(args), label="baseline"),
            )
            _install_wheel(baseline_wheel, runner=role_runner("baseline"))
            executed_commands["baseline"].append(list(commands["baseline"]))
            result = _run_logged(
                commands["baseline"],
                cwd=ROOT,
                role="baseline",
                output_dir=output_dir,
            )
            if result.returncode != 0:
                observed_exit_code = result.returncode
            _add_report_provenance(
                baseline_report,
                {**provenance["ab_provenance"], "native_build_sha": baseline_sha},
            )
            if result.returncode != 0:
                raise RuntimeError("baseline benchmark failed")

            stage = "candidate_build"
            failure_role = "candidate"
            candidate_wheel = _build_wheel(
                ROOT,
                env_file=Path(".env") if env_file.exists() else None,
                expected_sha=candidate_sha,
                runner=role_runner("candidate"),
            )
            provenance["roles"]["candidate"] = {
                "native_build_commit": candidate_sha,
                "wheel": candidate_wheel.name,
            }

            stage = "candidate_benchmark"
            commands["candidate"] = _benchmark_command(
                output=candidate_report,
                expected_native_commit=candidate_sha,
                args=argparse.Namespace(**vars(args), label="candidate"),
                baseline=baseline_report,
            )
            _install_wheel(candidate_wheel, runner=role_runner("candidate"))
            executed_commands["candidate"].append(list(commands["candidate"]))
            result = _run_logged(
                commands["candidate"],
                cwd=ROOT,
                role="candidate",
                output_dir=output_dir,
            )
            if result.returncode != 0:
                observed_exit_code = result.returncode
            _add_report_provenance(candidate_report, provenance["ab_provenance"])
            if result.returncode != 0:
                try:
                    candidate_payload = json.loads(candidate_report.read_text(encoding="utf-8"))
                except (OSError, json.JSONDecodeError):
                    candidate_payload = {}
                if isinstance(candidate_payload, dict) and candidate_payload.get(
                    "statistics_eligible"
                ) is True:
                    stage = "regression_gate"
                raise RuntimeError("candidate benchmark or A/B regression gate failed")
            exit_code = 0
    except Exception as exc:
        exit_code = observed_exit_code
        failure_summary = {
            "stage": stage,
            "exit_code": exit_code,
            "baseline_sha": baseline_sha,
            "candidate_sha": candidate_sha,
            "candidate_report_exists": candidate_report.exists(),
            "command": (
                executed_commands.get(failure_role, [])[-1]
                if executed_commands.get(failure_role)
                else commands.get(failure_role, [])
            ),
            "statistics_eligible": False,
            "error": f"{type(exc).__name__}: {exc}",
        }
        report_path = baseline_report if failure_role == "baseline" else candidate_report
        if not report_path.exists():
            _write_json(
                _failure_report_path(output_dir, failure_role),
                {
                    **failure_summary,
                    "report_role": failure_role,
                    "provenance": provenance,
                },
            )
    finally:
        if worktree is not None and worktree.exists():
            remove = _run(["git", "worktree", "remove", "--force", str(worktree)], cwd=ROOT)
            if remove.returncode != 0:
                print("WARNING: failed to remove temporary baseline worktree", file=sys.stderr)

        summary = {
            "baseline_sha": baseline_sha,
            "candidate_sha": candidate_sha,
            "baseline_report": str(baseline_report),
            "candidate_report": str(candidate_report),
            "candidate_failure_report": str(_failure_report_path(output_dir, "candidate")),
            "benchmark_config": _benchmark_matrix(args),
            "commands": commands,
            "executed_commands": executed_commands,
            "statistics_eligible": exit_code == 0,
            "failure_stage": None if failure_summary is None else failure_summary["stage"],
        }
        provenance["commands"] = commands
        provenance["executed_commands"] = executed_commands
        candidate_failure_path = _failure_report_path(output_dir, "candidate")
        if not candidate_report.exists() and not candidate_failure_path.exists():
            _write_json(
                candidate_failure_path,
                {
                    "stage": None if failure_summary is None else failure_summary["stage"],
                    "exit_code": exit_code,
                    "baseline_sha": baseline_sha,
                    "candidate_sha": candidate_sha,
                    "candidate_report_exists": False,
                    "command": [],
                    "statistics_eligible": False,
                    "report_role": "candidate",
                    "provenance": provenance,
                },
            )
        _write_json(output_dir / "build-provenance.json", provenance)
        _write_json(output_dir / "ab-summary.json", summary)
        if failure_summary is not None:
            _write_json(output_dir / "failure-summary.json", failure_summary)

    print(json.dumps(summary, indent=2))
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
