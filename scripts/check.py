"""Repository-owned verification entry point.

Keep the command surface small: agents, contributors, CI, and release workflows select a verification
group instead of copying command matrices into prompts or YAML. Specialized packaging, Windows
latency, and benchmark evidence stays in its dedicated scripts/workflows.
"""
from __future__ import annotations

import argparse
import os
import subprocess
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


@dataclass(frozen=True, slots=True)
class Check:
    label: str
    command: tuple[str, ...]
    env: tuple[tuple[str, str], ...] = ()
    cwd: Path | None = None


GROUPS: dict[str, tuple[Check, ...]] = {
    "static": (
        Check("ruff", ("ruff", "check", ".")),
        Check("pyright", ("pyright",)),
        Check(
            "agent context",
            (sys.executable, "scripts/audit_agent_context.py"),
        ),
        Check(
            "durable phase-name audit",
            (sys.executable, "scripts/audit_durable_phase_names.py"),
        ),
        Check(
            "security mandates",
            (sys.executable, "scripts/audit_security_mandates.py"),
        ),
        Check(
            "Rust architecture",
            (sys.executable, "scripts/check_rust_architecture.py", "--enforce"),
        ),
    ),
    "tests": (
        Check(
            "Python tests (non-slow)",
            (sys.executable, "-m", "pytest", "-m", "not slow"),
        ),
    ),
    "tests-full": (
        Check(
            "Python tests (full)",
            (sys.executable, "-m", "pytest"),
        ),
    ),
    "rust": (
        Check(
            "Rust format",
            ("cargo", "fmt", "--manifest-path", "rust/Cargo.toml", "--all", "--", "--check"),
        ),
        Check(
            "Rust check",
            (
                "cargo",
                "check",
                "--manifest-path",
                "rust/Cargo.toml",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--locked",
            ),
        ),
        Check(
            "Rust clippy",
            (
                "cargo",
                "clippy",
                "--manifest-path",
                "rust/Cargo.toml",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--locked",
                "--",
                "-D",
                "warnings",
            ),
        ),
        Check(
            "Rust tests",
            (
                "cargo",
                "test",
                "--manifest-path",
                "rust/Cargo.toml",
                "--workspace",
                "--all-features",
                "--locked",
            ),
        ),
    ),
    "desktop": (
        Check(
            "desktop frontend install",
            ("bun", "install", "--frozen-lockfile"),
            cwd=ROOT / "desktop",
        ),
        Check("desktop frontend checks", ("bun", "run", "check"), cwd=ROOT / "desktop"),
        Check(
            "desktop frontend e2e",
            ("bun", "run", "test:e2e"),
            cwd=ROOT / "desktop",
        ),
        Check(
            "desktop Rust check",
            (
                "cargo",
                "check",
                "--manifest-path",
                "rust/Cargo.toml",
                "-p",
                "sky_desktop_shell",
                "--all-features",
                "--locked",
            ),
        ),
        Check(
            "desktop Rust tests and bindings",
            (sys.executable, "scripts/generate_desktop_bindings.py"),
        ),
        Check(
            "desktop Tauri command decoder tests",
            (
                "cargo",
                "test",
                "--manifest-path",
                "rust/Cargo.toml",
                "-p",
                "sky_desktop_shell",
                "--lib",
                "--no-default-features",
                "--features",
                "tauri-test",
                "generated_tauri_handler",
                "--locked",
            ),
        ),
        Check(
            "desktop generated bindings are clean",
            ("git", "diff", "--exit-code", "--", "desktop/src/bridge/generated"),
        ),
    ),
}

DEFAULT_GROUPS: tuple[str, ...] = ("static", "tests", "rust", "desktop")


def _run(check: Check) -> None:
    env = os.environ.copy()
    env.update(check.env)
    rendered = subprocess.list2cmdline(check.command)
    print(f"\n[check] {check.label}\n        {rendered}", flush=True)
    subprocess.run(check.command, cwd=check.cwd or ROOT, env=env, check=True)


def _parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run canonical Sky Auto Player repository verification groups.",
    )
    parser.add_argument(
        "groups",
        nargs="*",
        choices=tuple(GROUPS),
        metavar="GROUP",
        help="verification group(s): static, tests, tests-full, rust, desktop; omit to run normal checks",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = _parse_args(argv)
    groups: tuple[str, ...] = tuple(args.groups) or DEFAULT_GROUPS

    print(f"[check] repository: {ROOT}")
    print(f"[check] groups: {', '.join(groups)}")

    for group in groups:
        print(f"\n[check] === {group} ===", flush=True)
        for check in GROUPS[group]:
            try:
                _run(check)
            except FileNotFoundError as exc:
                print(f"[check] FAIL: required executable not found: {exc.filename}", file=sys.stderr)
                return 127
            except subprocess.CalledProcessError as exc:
                print(
                    f"[check] FAIL: {check.label} exited with {exc.returncode}",
                    file=sys.stderr,
                )
                return exc.returncode or 1

    print("\n[check] PASS", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
