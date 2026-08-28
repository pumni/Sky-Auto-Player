# Native dispatch follow-up errata

This note narrows two provenance/qualification statements in
`2026-08-native-dispatch-followup.md`. It does not change runtime timing policy.

## Production waiter wording

The phrase **"production behavior remains frozen"** in the production-equivalent
scheduling section is too broad if read as covering every public diagnostic
profile.

The normal `DispatchProfile::Production` authored timing and safety contract is
unchanged: authored timestamps, the 500 us Down grace, hold/release policy,
focus/target/lease gates, future authorization, one-wait architecture,
SendInput attempt policy, and real-time allocation behavior were not retuned.

`DispatchProfile::StrictTimingDiagnostic` is a production-backend diagnostic
profile. Production backends intentionally use the production waiter constructor
and production startup calibration regardless of whether telemetry is the normal
production profile or strict timing diagnostics. This lets real-SendInput
diagnostics observe the production waiter rather than substituting a test wait
policy. Compared with older baselines where the calibration condition was tied
more narrowly to the production profile, this is therefore an intentional
diagnostic-profile waiter/calibration behavior change, not an authored timing or
safety-policy change.

No calibration constants or runtime adaptation are authorized by this erratum.

## Production-equivalent priority qualification

A production-calibrated test-support run is scheduling-qualified only when all
of the following are true:

- requested priority policy is `auto`;
- the acquired policy is one of the actual production Auto ladder outcomes:
  `mmcss:Games` or `thread:highest`;
- startup calibration provenance is valid;
- sender cutoff, waiter, dispatch and hard correctness dimensions are clean;
- the required physical-boundary sample floor is met.

`Auto -> off` remains **INCONCLUSIVE**. Forced `highest`, `time_critical`, or
`mmcss` test-support runs are not production-equivalent evidence even if they
are active and otherwise clean.

## Historical raw-artifact hashes

The following historical local artifacts were not committed and their SHA256
values were not recorded in the committed report:

- `.benchmarks/production-equivalent-auto-cold-p1-final7c.json`
- `.benchmarks/production-equivalent-auto-hot-p1-final7c.json`
- `.benchmarks/production-equivalent-auto-hot-p15-final7c.json`
- `.benchmarks/production-equivalent-real-wait-core-10k.json`
- associated failed-run JSON artifacts

Their source SHA, commands, host fingerprint and wheel provenance remain useful,
but the raw JSON byte identity is **not independently source-verifiable** from
GitHub. No SHA256 is reconstructed or inferred here.

Any future qualification rerun that keeps benchmark JSON outside Git must record
at minimum the exact artifact path, SHA256, source/native commit, and exact
command line before the evidence is considered provenance-complete.
