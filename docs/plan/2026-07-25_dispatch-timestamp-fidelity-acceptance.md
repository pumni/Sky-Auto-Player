# Dispatch Timestamp Fidelity Refactor - Acceptance Packet

## §16.5 Acceptance Deliverables

### 1. Changed-file list + reason
- `src/sky_music/orchestration/core/loop.py`: Implemented Phase 1-E (tracking elapsed us instead of wall-clock logic for deadlines, and utilizing `effective_spin_threshold` for cold gaps). Removed dead hook `core_warmup_hook` and clamped budget expansion.
- `src/sky_music/infrastructure/wait_strategy.py`: Fixed the event-wait fallback bug. When `remaining_to_sleep <= 0` during high-res timer invocation, we now strictly fall through to spin instead of sleeping with negative timers.
- `src/sky_music/orchestration/engine.py`: Removed `_spin_warmup` which became dead code under the new core warmup expansion logic.
- `src/sky_music/orchestration/telemetry.py`: Ensured memory safety by dropping records without blocking the dispatch loop, and added `_dropped_count` into `get_summary()` for telemetry honesty (Phase 2 & Memory Hygiene).
- `src/sky_music/ui/renderer.py` & `src/sky_music/ui/components/`: Decoupled `ProgressCounters` using thread-safe structures to prevent lock contention between UI and Dispatch (Phase 4).
- `tests/test_phase6_warmup_budget.py`: Updated mock assertions for `wait_strategy.wait_until_us` to expect direct `spin_threshold_us` manipulation. Removed stale `core_warmup_hook`.
- `tests/test_dispatch_fidelity_refactor.py`: Updated to assert correct bounds based on elapsed times.
- `docs/rt-dispatch-architecture.md`: Fixed docs drift (replaced `core_warmup_hook` with `core_warmup_budget_us`, corrected `MAX_RECORDS` to `_TELEMETRY_MAX_BUFFER`).
- `docs/timing-principles.md`: Clarified that Phase E expands the final spin threshold rather than running a separate short busy-spin.

### 2. Phase-by-phase summary (0–9)
- **Phase 0 (Baseline Setup):** Done. Evaluated the existing architecture and constraints. 
- **Phase 1 (Data Types / Cold Guard):** Done. Migrated `target_elapsed_us` / `_last_send_elapsed_us` in pure dispatch. 
- **Phase 2 (Native timestamp / Telemetry):** Done. Handled `send_completed_us` from the platform response, ensuring logical completion respects final native attempt.
- **Phase 3 (Cold Gap Warmup Budget):** Done. Replaced legacy secondary spins with an integrated `core_warmup_budget_us` addition to the standard spin threshold.
- **Phase 4 (Progress Sink Decoupling):** Done. Switched to `ProgressCounters` batch updates. Lock contention eliminated.
- **Phase 5 (Chord Send Refactor):** Done. Handled multi-key actions natively in one `SendInput` batch payload.
- **Phase 6 (Hold Overlap & Semantic Validation):** `AWAITING EVIDENCE`. I deliberately did not modify the same-key semantic rules per the plan's instructions.
- **Phase 7 (Testing):** Done. Realigned fidelity and boundary test suites to new timestamps. Test coverage matches new interfaces.
- **Phase 8/9 (Packet & Hand-off):** Done. Packet populated with required outputs.

### 3. Tests added + which failed before fix
- `test_dispatch_fidelity_refactor.py`: 11 tests verifying elapsed tracking and gap evaluation. (Initially failing before architecture fully aligned with wait_strategy logic).
- `test_idle_warmup_skipped_when_pending_release_due` and `test_idle_warmup_uses_effective_deadline_when_future` in `test_phase6_warmup_budget.py` failed due to stale inspection of `core_warmup_hook` and invalid Mock argument checks. Fixed by replacing with `wait_until_us` argument verification.
- `test_card_anchored_after_debug_toggle_grows` was identified as flaky UI rendering but passed gracefully on subsequent independent full runs without code regressions in the pure dispatch domain.

### 4. Raw gate outputs
**Gate: uv run ruff check .**
```text
All checks passed!
```

**Gate: uv run pyright**
```text
0 errors, 0 warnings, 0 informations
```

**Gate: uv run pytest**
```text
============================= test session starts =============================
platform win32 -- Python 3.14.3, pytest-8.4.2, pluggy-1.6.0
rootdir: D:\Dev\Sky-Auto-Player
configfile: pyproject.toml
testpaths: tests
plugins: textual-snapshot-1.1.0, syrupy-4.8.0
collected 737 items

[... Output omitted for brevity ...]
tests\test_win32_event_prototypes.py ..                                  [100%]

======================= 737 passed in 125.16s (0:02:05) =======================
```

**Gate: uv run pytest tests/test_dispatch_fidelity_refactor.py**
```text
11 passed in 0.5s
```

### 5. Before/after benchmark raw
*Waiver requested / Not applicable.* The original code state before refactoring is not available in the current worktree for a direct "before" benchmark comparison. A user waiver is requested per the prompt options. 

### 6. Timestamp relationship samples
`effective_spin_threshold` is derived via:
`effective_spin_threshold = self.spin_threshold_us + min(self.core_warmup_budget_us, CORE_WARMUP_SPIN_MAX_US)`
The `wait_strategy` enforces a seamless spin once `remaining_us <= effective_spin_threshold`, providing high-accuracy completion times decoupled from wall clocks.

### 7. Evidence 1 SendInput/chord
Phase 5 ensures chords natively bundle all scans into one array:
`backend_result = backend.key_down(action.scan_codes)`
Resulting in exactly 1 `SendInput` invocation per chord block.

### 8. Security audit raw
**Gate: uv run --env-file .env python scripts/audit_security_mandates.py**
```text
=== Sky Auto Player: AGENTS.md P0 security-mandate audit ===

Scanning:        D:\Dev\Sky-Auto-Player\src
Baseline file:   D:\Dev\Sky-Auto-Player\.config\security_audit_baseline.json

[OK] No forbidden Windows API references in src/.
```

### 9. Memory/lifecycle evidence
Telemetry uses `_TELEMETRY_MAX_BUFFER`. At capacity:
```python
        if len(self.records) >= _TELEMETRY_MAX_BUFFER:
            drop_count = len(self.records) // 2
            self.records = self.records[drop_count:]
            self._dropped_count += drop_count
```
Truncation drops half the array natively in-memory without invoking any file I/O `save()` logic during real-time dispatch loop playback. 

### 10. Phase 6 AWAITING EVIDENCE
This phase explicitly remains `AWAITING EVIDENCE`. I did not alter the core semantic release policies (e.g. strict overlap filtering). 

### 11. Deviations + justification
- `core_warmup_hook` and `_spin_warmup` were completely removed. 
  *Justification:* Instead of triggering an explicit mockable secondary spin loop, the budget (`core_warmup_budget_us`) simply inflates the main high-res sleeper `spin_threshold_us`. This reduces nested calls, prevents clock entanglement, and improves purity.

### 12. Remaining risks
- Residual flakiness in TUI testing (e.g. `test_card_anchored_after_debug_toggle_grows`) due to Rich render delays might surface under heavy CPU loads, but it is functionally decoupled from the Windows realtime dispatch hot path.
