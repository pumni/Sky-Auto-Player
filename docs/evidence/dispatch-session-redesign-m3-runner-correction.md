# Dispatch Session Redesign M3 runner correction

Status: runner correction for Leader review. The prior one-shot native
authorization was consumed by a runner failure. This correction does not
authorize or perform a native run.

## Failed attempt and root cause

The preserved attempt at runner SHA
`d76d4fb7cdcc49fcd0f0a7d80c1f691578c86fed` exited 2 before invoking a case.
Its `summary.json` reports `run_count: 0` and `runner failed before completing
protocol`; the single `RUNNER_FAILURE` row records the StrictMode exception.
Therefore the canary, all seven safety cases, and all 24 paired cases were
`NotRun` (0/32). There are no native timing, CPU/spin, release, sink-receipt,
or gameplay observations from that attempt.

In the failed runner, the `if` expression assigning
`$performanceWorkloads` returned `@($dispatchSessionRedesignM3.PerformanceCase.Workload)`
for the single `mixed-up-down` workload. PowerShell pipeline enumeration
unwrapped that one-item result to `System.String`. StrictMode Latest then raised
`System.Management.Automation.PropertyNotFoundException` when the Final plan
read `$performanceWorkloads.Count`, before case invocation. This is an
orchestration failure, not a dispatch-engine or playback failure.

The original console log preserved the error message but did not emit an
ErrorRecord stack. An offline reproduction used the exact old source lines
1131–1143 in a line-numbered temporary script under StrictMode Latest. It
reported:

```text
exception_type: System.Management.Automation.PropertyNotFoundException
message: The property 'Count' cannot be found on this object. Verify that the property exists.
line_number: 1142
workload_type: System.String
script_stack_trace: at <ScriptBlock>, <temp reproduction path>: line 1142
```

The full temporary reproduction output and script are outside the checkout at
`%TEMP%\sky-auto-player-dispatch-session-redesign-m3-count-repro-b5addb4bc6134f2e8ecbadd77d0bbfa7`.
Their SHA-256 values are recorded in the Leader handoff. The temporary
reproduction invoked no sink, native acceptance, or input API.

## Preserved original artifacts

No original attempt file was edited, regenerated, or replaced. SHA-256 before
and after correction:

| Original file | SHA-256 |
|---|---|
| `native-evidence/summary.json` | `f4877781ff2626a5f9c07f85b82cf5e6bc554a43be92c3c51cdf6a9f30d2d1e0` |
| `native-evidence/failures.json` | `ef88dc337a6740c84df8f3a51abc8db44a6eb108d4ff639b98b12f3a50e952e5f` |
| `native-evidence/SHA256SUMS.txt` | `a631f8c955a1dfea3bd7ae667e4dcc0a0702b727155b2ddca2c3ad8b191804dd` |
| `native-evidence.zip` | `399642171183a6ec1a741758061193410df79d8fa7348d4d0486850dd4760f85` |
| `native-final-console-0f9916b1c99142afa7af2368e39d3d65.log` | `16c1fc60f9aae78c2786cf192d47e5a2b06ce1554dc59c816e9177038ea278a1` |

The original files are in
`%TEMP%\sky-auto-player-dispatch-session-redesign-m3-d76d4fb7`. A separate
`failure-erratum.md` in the correction evidence directory records the
orchestration error and 0/32 `NotRun` disposition; it does not alter the
original archive.

## Correction and compatibility

| Path | Correction |
|---|---|
| `scripts/qualify_dispatch_core.ps1` | M3 workload/scenario locals are explicit `string[]`; array cardinality uses `.Length`. M3 Final calls the shared pre-case control path. Catch output now records script path, line, stack, and exception text in a separate `runner-error.json`; existing summary/failure JSON fields are unchanged. `summary.md` is printed only when it exists and zero-attempt runner failure states `NotRun`. |
| `scripts/dispatch_qualification_orchestration.psm1` | Normalizes null/scalar/array inputs into explicit arrays; constructs and checks the approved 32-case plan; runs the same synchronous fail-stop control path used by production with an injected offline case callback in contracts. |
| `scripts/dispatch_qualification_profiles.psm1` | Adds the orchestration module and this correction record to M3 runner provenance. |
| `scripts/test_qualify_dispatch_core_contracts.ps1` | Adds StrictMode 0/1/N cardinality, exact 32/20/12 plan, canonical canary, ordered safety cases, two six-pair AB/BA cells, offline pass/fail-stop, zero-attempt error artifacts, and schema compatibility assertions. |

The Final order remains one Candidate `canonical-single` canary, seven
Candidate safety cases, then two `mixed-up-down` cells with six AB/BA pairs
each. The settings remain Frozen waiter, 60 FPS for performance, 15 seconds,
and 500 µs margin; H/G and timing provenance continue through the existing
plan and report contracts. Fail-stop remains at the first non-PASS and never
retries.

## StrictMode collection audit

The former failing workload local was the only M3 Final plan cardinality read
from a pipeline-unwrapped scalar. Its replacement is `[string[]]` and uses
`.Length`. The remaining `.Count` uses in the dispatch runner path are over
explicit `@(...)` arrays, strongly typed `string[]` report parameters,
`Generic.List` instances, or `Measure-Object` results. The orchestration module
uses `.Length` for array inputs and plan cardinalities; its one `.Count` read is
on a `Generic.List[object]`. The legacy DispatchCleanupM3 report `.Count` reads
take strongly typed array parameters or local `@(...)` arrays and remain
unchanged. Its historic profiles, readers, output schema, and artifact pins
were not changed.

## Evidence limits

The PowerShell regression invokes the same M3 Final control-path function that
the runner calls, with synthetic validated Candidate/Baseline identities and
fake case rows. It verifies dispatch ordering and fail-stop behavior without
starting a sink, launching `run_native_acceptance_case.ps1`, calling
`SendInput`, or claiming native acceptance. Rust checks are software tests and
build/lint evidence only. Windows input delivery, ReceiveOnly evidence,
hardware timing, and Sky gameplay reception remain unmeasured. A new express
Leader approval is required before any replacement native run.
