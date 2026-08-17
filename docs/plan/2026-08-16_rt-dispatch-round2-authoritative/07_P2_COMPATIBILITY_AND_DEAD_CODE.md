# 07 — P2: Compatibility and Dead-Code Cleanup

This phase happens only after P0/P1 semantics and benchmarks are stable. Its purpose is to remove misleading internal machinery without forcing an unnecessary external schema migration.

## 1. Principle

Compatibility fields are allowed to exist.

Compatibility **control logic** that no longer affects production is not.

The internal architecture should describe what the engine actually does in 2026, not every strategy it tried historically.

---

# 2. Estimator/lead compatibility

Current native/Python interfaces still expose estimator/lead-related fields for older callers even though production lead is non-operative.

Keep external compatibility where needed:

```text
estimator_state_json input accepted but ignored
estimator output = deprecated marker
old applied-lead fields = zero
lead saturation fields = zero/deprecated
```

Remove internal estimator state/types/functions that are unreachable from production and are not needed by migration tests.

Do not add a new estimator implementation.

Update stubs/docs to say explicitly that these fields are compatibility-only.

---

# 3. Retry terminology

Production authored `SendInput` becomes/continues to be single-attempt.

Audit:

```text
PacketRetryReason
zero_progress_retries
recovered_zero_progress_retries
recovered_partial_up_retries
retry_late_abort
STRICT_RETRY_LATE_THRESHOLD_US
```

For each item:

### If needed only by test-support fault matrices

Move behind `#[cfg(any(test, feature = "test-support"))]` or a test-only outcome type.

### If required by stable telemetry schema

Keep the public field but publish a fixed zero/deprecated value in production and remove it from physical authorization/health decisions.

### If used by cleanup FSM

Rename/separate it so cleanup retry evidence is not confused with authored playback retry.

Do not retain a production branch for a retry that production transport cannot perform.

---

# 4. `strict_timing` cleanup

Keep the `strict_timing_diagnostic` profile if current tools/tests rely on it.

Narrow its meaning to diagnostic timing thresholds only.

It must not be the switch for structural correctness rules such as:

- partial insertion;
- impossible retrigger deadline;
- overdue backlog;
- QPC/wait failure;
- coordinator mismatch.

Review names such as `hard_late_abort_threshold` and document that they are diagnostic policy, not the core no-catch-up mechanism.

---

# 5. Timeline-rebase compatibility

This refactor explicitly does not use active-playback rebase.

Audit:

```text
timeline_rebase_count
timeline_rebase_total_ticks
timeline_rebase_max_ticks
timeline_rebase_last_reason
rebase_epoch()
```

If public reports require the metrics, retain compatibility zeros/markers.

Remove them from per-dispatch raw observation payload and healthy producer work.

Keep `PlaybackClockState::rebase_epoch()` only if another active feature/test genuinely calls it. Otherwise deprecate/remove in a separate focused cleanup commit after repository-wide search proves it unused.

Do not delete generic clock functionality blindly because one current product path does not use it.

---

# 6. Wait configuration booleans

Production Python currently hardcodes:

```text
enable_waitable_timer = true
enable_event_wait = true
enable_adaptive_spin = true
```

After P1:

- production waiter/event are mandatory, so the first two are not product choices;
- adaptive production spin is removed.

Simplify `WaitOptions` so shipping configuration does not present impossible/unsupported combinations.

Keep configurable wait modes in test-support/benchmark constructors where needed.

Public Python `SessionConfig` should not gain low-level timing toggles merely to preserve old Rust booleans.

---

# 7. Priority modes

If `PriorityMode::{TimeCritical, Highest, Off}` are used by tests/benchmarks/manual diagnostics, they may remain.

Production `SessionConfig` does not need to expose them.

Do not delete safe diagnostics solely for enum minimalism, but clearly separate:

```text
production default
manual diagnostic modes
```

---

# 8. Rich observation types

After compact raw observation lands, delete the old duplicated rich producer structures if the observer can derive the same report fields.

Avoid keeping parallel structures such as:

```text
DownObservation + DownTraceObservation + pre-derived health fields
UpObservation + UpTraceObservation + pre-derived health fields
```

unless a field genuinely cannot be reconstructed.

One raw transport observation plus observer-side materialization is preferred.

---

# 9. Health structures

If `FrozenDispatchBudget` becomes observer-only or entirely static, simplify/remove:

- plan-side budget creation;
- event-count copies that are derivable from masks;
- producer-side threshold copies.

Keep fixed health windows if they remain useful for user diagnostics. Their location is observer state, not coordinator/physical-plan state.

---

# 10. Error strings vs structured errors

Core/coordinator/platform layers should prefer typed variants carrying masks/ticks/error codes.

String formatting belongs at an outer error/report boundary.

Do not perform a repository-wide error-type rewrite in one PR. Convert only paths touched by P0/P1 where structured data is needed for deterministic tests.

On rare terminal paths, allocating a final descriptive String is acceptable. The goal is not “no strings anywhere”; the goal is no string work on healthy precision paths.

---

# 11. Old tests that encode superseded behavior

A test is not a specification when this plan intentionally changes the behavior.

For every failing old test:

1. identify which decision/invariant it conflicts with;
2. update it to the new semantic contract;
3. preserve useful fault coverage;
4. do not delete tests only to get green.

Examples likely to change:

- tests expecting packet-wide release-floor deferral;
- tests expecting adaptive spin-derived production threshold;
- tests asserting producer exact queue high watermark;
- tests using retry fields as active production semantics.

---

# 12. Documentation consolidation

After implementation:

Update at least:

```text
docs/timing-principles.md
docs/hold-frame-model.md
docs/rt-dispatch-architecture.md
docs/architecture.md (only relevant summary sections)
```

Older dated plans remain historical. Do not rewrite archived plans to pretend they predicted the final design.

Add a short note to any still-prominent older overhaul plan that points readers to this authoritative round-2 plan/implemented normative docs if confusion is likely.

---

# 13. Public schema migration rule

Do not bump/remove external fields unless all of these are true:

- field is actively harmful or impossible to represent honestly;
- Python callers/tests have been migrated in the same phase;
- schema/golden fixtures are updated;
- compatibility benefit is lower than continued ambiguity.

Otherwise prefer:

```text
stable key + clarified/deprecated semantics
```

over a broad unrelated API migration.

---

# 14. Acceptance

P2 cleanup is complete when production source no longer contains dead adaptive/retry/rebase control paths that appear live, while public compatibility remains sufficient for existing callers and all normative docs describe only the actual runtime behavior.
