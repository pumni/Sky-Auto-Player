from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).parents[1]
RUST_CACHE_PIN = "6323deb102c322ba6fcbdcafc7e3dddab59af2b6"


def test_ci_fetches_history_for_change_classification_and_retirement_ledger() -> None:
    workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")

    assert workflow.count("fetch-depth: 0") == 4
    assert "fetch-depth: 1" not in workflow
    assert 'printf \'\\n\' | python scripts/classify_ci_changes.py --full' in workflow
    assert 'if [[ "$PACKAGE_REQUIRED" == "true" ]]' in workflow


def test_rust_heavy_ci_jobs_use_the_pinned_workspace_cache() -> None:
    ci_workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    release_workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")

    cache_reference = f"Swatinem/rust-cache@{RUST_CACHE_PIN}"
    assert ci_workflow.count(cache_reference) == 2
    assert cache_reference in release_workflow
    assert ci_workflow.count('workspaces: "rust -> target"') == 2
    assert 'workspaces: "rust -> target"' in release_workflow
    assert ci_workflow.count("cache-on-failure: true") == 2
    assert "-Mode finish -Job validate -CacheHit" in ci_workflow
    assert "-Mode finish -Job packaged -CacheHit" in ci_workflow
    assert "-Mode finish -Job release -CacheHit" in release_workflow


def test_frontend_check_retains_typecheck_without_running_it_twice() -> None:
    package = json.loads((ROOT / "desktop" / "package.json").read_text(encoding="utf-8"))
    scripts = package["scripts"]

    assert scripts["build"] == "bun run typecheck && bun run build:web"
    assert scripts["build:web"] == "vite build"
    assert scripts["check"].startswith("bun run typecheck &&")
    assert scripts["check"].endswith("&& bun run build:web")
