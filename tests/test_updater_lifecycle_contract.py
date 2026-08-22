"""Static guardrails for the native updater's post-READY lifecycle boundary."""

from __future__ import annotations

from pathlib import Path


def test_ready_handoff_has_no_fallible_starting_write_after_it() -> None:
    source = (
        Path(__file__).parents[1] / "rust" / "crates" / "sky_updater" / "src" / "runner.rs"
    ).read_text(encoding="utf-8")

    initial_phase = "last_persisted_phase: Mutex::new(Some(UpdatePhase::Starting))"
    ready = "handoff::write_ready(&run_root, &args.target_version)?;"
    outcome = "let outcome = execute_update(args, source, &progress, &run_root);"
    assert source.index(initial_phase) < source.index(ready)
    ready_end = source.index(ready) + len(ready)
    between_ready_and_outcome = source[ready_end : source.index(outcome)]
    assert "progress.publish" not in between_ready_and_outcome
    assert "?" not in between_ready_and_outcome
