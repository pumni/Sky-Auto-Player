"""Run schema-compatible native benchmark A/B on one Windows runner.

The candidate checkout supplies the benchmark harness for both legs.  Only the
native wheel changes between legs, so the reports distinguish harness Git SHA
from the native build commit instead of conflating the two.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]


def _run(command: list[str], *, cwd: Path, capture: bool = False) -> subprocess.CompletedProcess[str]:
    print("+", " ".join(command))
    return subprocess.run(
        command,
        cwd=cwd,
        capture_output=capture,
        text=True,
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


def _build_wheel(repo: Path, *, env_file: Path | None) -> Path:
    command = ["uv", "run"]
    if env_file is not None:
        command.extend(["--env-file", str(env_file)])
    command.extend(["python", "scripts/build_rust_wheel.py", "--test-support"])
    result = _run(command, cwd=repo)
    if result.returncode != 0:
        raise RuntimeError(f"native wheel build failed for {repo}: {result.returncode}")
    wheel_dir_candidates = (repo / "target" / "wheels", repo / "rust" / "target" / "wheels")
    wheels = [wheel for directory in wheel_dir_candidates if directory.exists() for wheel in directory.glob("sky_player_rs-*.whl")]
    if not wheels:
        raise RuntimeError(f"native wheel build produced no wheel in {repo}")
    return max(wheels, key=lambda path: path.stat().st_mtime_ns)


def _install_wheel(wheel: Path) -> None:
    result = _run(
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
    parser.add_argument("--game-fps", type=int, default=60)
    parser.add_argument("--lead-mode", choices=("fixed", "adaptive"), required=True)
    parser.add_argument("--fixed-lead-us", type=int, default=0)
    parser.add_argument("--gap-profile", choices=("hot", "cold"), required=True)
    parser.add_argument("--warmup-cycles", type=int, required=True)
    parser.add_argument("--rt-priority-mode", default="off")
    parser.add_argument("--budget-seconds", type=float, default=120.0)
    parser.add_argument("--output-dir", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    if os.name != "nt":
        raise SystemExit("native A/B benchmark requires Windows")
    if args.lead_mode == "adaptive" and args.fixed_lead_us != 0:
        raise SystemExit("--fixed-lead-us must be 0 in adaptive mode")

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
    commands: dict[str, list[str]] = {}
    provenance: dict[str, Any] = {
        "harness_git_sha": candidate_sha,
        "baseline_ref": args.baseline_ref,
        "baseline_sha": baseline_sha,
        "candidate_ref": args.candidate_ref,
        "candidate_sha": candidate_sha,
        "roles": {},
    }

    with tempfile.TemporaryDirectory(prefix="sky-native-ab-") as temp_dir:
        worktree = Path(temp_dir) / "baseline"
        try:
            add = _run(["git", "worktree", "add", "--detach", str(worktree), baseline_sha], cwd=ROOT)
            if add.returncode != 0:
                raise RuntimeError("could not create baseline temporary worktree")
            _assert_clean(worktree, "baseline")
            env_file = ROOT / ".env"
            if env_file.exists():
                shutil.copy2(env_file, worktree / ".env")
            baseline_wheel = _build_wheel(worktree, env_file=Path(".env") if env_file.exists() else None)
            provenance["roles"]["baseline"] = {
                "native_build_commit": baseline_sha,
                "wheel": baseline_wheel.name,
            }

            baseline_report = output_dir / "baseline.json"
            baseline_args = argparse.Namespace(**vars(args), label="baseline")
            commands["baseline"] = _benchmark_command(
                output=baseline_report,
                expected_native_commit=baseline_sha,
                args=baseline_args,
            )
            _install_wheel(baseline_wheel)
            result = _run(commands["baseline"], cwd=ROOT)
            if result.returncode != 0:
                raise RuntimeError("baseline benchmark failed")

            candidate_wheel = _build_wheel(ROOT, env_file=Path(".env") if env_file.exists() else None)
            provenance["roles"]["candidate"] = {
                "native_build_commit": candidate_sha,
                "wheel": candidate_wheel.name,
            }
            candidate_report = output_dir / "candidate.json"
            commands["candidate"] = _benchmark_command(
                output=candidate_report,
                expected_native_commit=candidate_sha,
                args=argparse.Namespace(**vars(args), label="candidate"),
                baseline=baseline_report,
            )
            _install_wheel(candidate_wheel)
            result = _run(commands["candidate"], cwd=ROOT)
            if result.returncode != 0:
                raise RuntimeError("candidate benchmark or A/B regression gate failed")

            summary = {
                "baseline_sha": baseline_sha,
                "candidate_sha": candidate_sha,
                "baseline_report": str(baseline_report),
                "candidate_report": str(candidate_report),
                "benchmark_config": {
                    "actions": args.actions,
                    "dispatch_repeats": args.dispatch_repeats,
                    "command_samples": args.command_samples,
                    "polyphony": args.polyphony,
                    "game_fps": args.game_fps,
                    "lead_mode": args.lead_mode,
                    "fixed_lead_us": args.fixed_lead_us,
                    "gap_profile": args.gap_profile,
                    "warmup_cycles": args.warmup_cycles,
                    "rt_priority_mode": args.rt_priority_mode,
                },
                "commands": commands,
                "statistics_eligible": True,
            }
            (output_dir / "ab-summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
            (output_dir / "build-provenance.json").write_text(
                json.dumps(provenance, indent=2) + "\n", encoding="utf-8"
            )
            print(json.dumps(summary, indent=2))
            return 0
        finally:
            if worktree.exists():
                remove = _run(["git", "worktree", "remove", "--force", str(worktree)], cwd=ROOT)
                if remove.returncode != 0:
                    print("WARNING: failed to remove temporary baseline worktree", file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
