# 13 — Definition of Done

> V3.0.0 status: the Rust-default implementation and explicit Python diagnostic rollback are present in the current code. Keep checklist items pending until the exact-SHA acceptance packet records the corresponding Windows, packaging, and soak evidence; this checklist must not be used to infer a PASS from code inspection alone.

## Build/ABI

- [ ] Rust 1.97.1 pinned and reproducible.
- [ ] Edition 2024.
- [ ] PyO3 0.29.0.
- [ ] windows-sys 0.61.2.
- [ ] CPython 3.14t wheel version-specific; no abi3.
- [ ] Module imports without re-enabling GIL.
- [ ] Maturin wheel and PyInstaller frozen app pass smoke.

## Architecture

- [ ] No Python execution from deadline wake through SendInput/bookkeeping.
- [ ] Rust owns coordinator/generations/pause clock/estimator/wait/backend/telemetry buffer.
- [ ] Worker sole owner of runtime mutable hot state.
- [ ] Python UI only pushes commands/focus and pulls snapshots/results.
- [ ] No Tokio/async runtime.

## Correctness

- [ ] All 36 invariants in `02` covered.
- [ ] Fake-clock differential corpus exact match.
- [ ] Current pytest full suite passes.
- [ ] Rust unit/property/integration suites pass.
- [ ] Partial send/release fault injection passes.
- [ ] Focus/pause/panic/quit/skip paths pass.
- [ ] Generation counts balance.
- [ ] No stuck keys under tested fault cases.

## Timing/performance

- [ ] No p99 sender completion regression beyond agreed threshold.
- [ ] Zero heap allocation per healthy single-note dispatch after prepare.
- [ ] No lock/Python attach/log formatting on hot send path.
- [ ] Event mode idle wake/CPU acceptable.
- [ ] Command/focus latency within current contract.
- [ ] Benchmark report includes machine/config/samples.

## Lifecycle/safety

- [ ] All handles RAII-owned with no double-close.
- [ ] Worker panic boundary performs best-effort release.
- [ ] Join timeout creates POISONED state and skips teardown.
- [ ] External watchdog remains effective.
- [ ] Unsafe only in audited Win32 wrappers.
- [ ] No forbidden game/process APIs.

## Telemetry/UI

- [ ] Existing CSV fields/outcome strings preserved or schema-versioned.
- [ ] Retain-first capacity/counters preserved.
- [ ] `game_observed.available=false` semantics preserved.
- [ ] UI status/health/counters/finish parity.
- [ ] Lead cache version 2 compatibility during migration.

## Rollout

- [ ] Rust implementation behind composition-root flag during soak.
- [ ] Telemetry records implementation/native build version.
- [ ] Rollback tested.
- [ ] Rust default completed.
- [ ] Python production hot path removed only after soak and sign-off.
- [ ] AGENTS/architecture/porting/build docs updated.
