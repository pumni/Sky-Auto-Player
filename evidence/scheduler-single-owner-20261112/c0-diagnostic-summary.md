# Pinned C0 diagnostic: canonical-max-chord / Quiet

## Existing Compare result

The existing run `dispatch-core-compare-20261112T141248-71a8f15e` stopped at its first native failure. It recorded 49 sessions: 48 PASS and one failure. The failed cell was `canonical-max-chord/Quiet`, pair 01, role `control`, run `native-case-20261112T143020-00e84f79`, using baseline revision `b9ed666e3ddddc4952831626f1654e1cc9071a10` and tree `f554c41b23a3969d02ed29842d4178f6143c9d86`. Expected sink events were 7,890; 1,500 were observed; exit code was 1 with `expected physical evidence did not settle before the drain deadline`. Active keys, possibly active keys, and the final release obligation mask were all zero.

`paired-cells.csv` is copied unchanged with all 72 metric rows. It has no `canonical-max-chord/Quiet` row because the run stopped before that safety cell could form a pair. The existing comparison summary and provenance are included alongside it. No pair, outlier, threshold, or baseline was changed.

## Separate diagnostic

Before considering another Compare, the pinned baseline harness was run once on the failed scenario, quietly, for 15 seconds with a 500 µs timing margin and a 1,000 ms drain deadline. The exact command was:

```powershell
$manifest = Join-Path $env:LOCALAPPDATA 'Temp\sky-scheduler-single-owner\baseline-dist-r1\manifest.json'
$m = Get-Content -Raw $manifest | ConvertFrom-Json
pwsh -NoProfile -File scripts/run_native_acceptance_case.ps1 -QualificationProfile Scheduler -Scenario canonical-max-chord -TimingMarginUs 500 -HarnessPath $m.binary_path -HarnessManifestPath $manifest -OutputRoot (Join-Path $env:LOCALAPPDATA 'Temp\sky-scheduler-single-owner\diagnostic-c0-canonical-max-chord-quiet-20261112') -SampleSeconds 15 -TimingObserverForQualification
```

The command exited 1. Diagnostic run `native-case-20261112T180732-9b6f3730` used source revision `d7e849a6395f8c33f8bafaf5f90214f977f6bdea`, tree `2d7c44cffafbda75c6957b8c7d548d5e87e2b700` (clean), and the pinned baseline harness at `b9ed666e3ddddc4952831626f1654e1cc9071a10` / tree `f554c41b23a3969d02ed29842d4178f6143c9d86`. The harness binary SHA-256 was `4bf477726ad738763c6deef95455812f21759824f260bc33afa127248899a611`; the pinned harness manifest SHA-256 was `0e6bc1bb5ed0beb8f0dc04a7a78842137eacff6208eb322a9ca1fecf0117f7c3`.

The native report expected 7,890 sink events and counted 1,820 in its report window. The independent sink event window contained 1,897 authorized events and did not match the report window; its cursor offset before arm was 281. Qualification failed with `expected physical evidence did not settle before the drain deadline`. The native run otherwise reported zero partial sends, zero zero-progress failures, zero production forensics or structural anomalies, zero active or possibly active keys, and zero stuck keys. This diagnostic does not establish why the sink and report counts diverged.

The raw diagnostic session is preserved in `diagnostic-c0-raw.zip`, including its invocation, run configuration, native report, sink event window, raw sink events, sink readiness data, runner output, and per-file SHA-256 manifest. `compare-c0-failure.csv` contains the original failed Compare row. `compare-summary.md`, `paired-cells.csv`, and `compare-provenance.json` are the existing Compare artifacts copied unchanged.

No Compare rerun was performed.
