# Isolated-process offline prepared-storage A/B — #632

This is a TEST-SUPPORT-ONLY measurement tool, not a production sender or
evidence of reduced actual game input latency.

## Design

Earlier #637 proved that per-song retained buffer capacities dropped
149,108,256 B to 95,838,392 B across 110 successfully prepared songs,
but that was not whole-process memory and build_ns was not paired.

The new Windows-only example rt_prepared_process_probe runs ONE variant
(inline OR direct compact) in ONE fresh process. The builder retains
the prepared stream and reset coordinator while collecting Win32 memory
counters. The per-boundary planner still creates one transient INPUT
packet; no entire inline prepared stream coexists with the direct
compact alternative. There is no playback worker, keyboard event, game
window, sender, SendInput or live dispatch.

The PowerShell script scripts/qualify_compact_prepared_ab.ps1 controls
interleaved adjacent AB/BA pairs. Every child trial has a unique
TEMP JSON result. The script validates the verified built-in corpus
(112 total, 110 prepared, 2 unsupported and exact 16-bin histogram)
before starting, and checks every matched pair's entry count, packet
count, metadata count, INPUT histogram/event count and rebuilt
coordinator heap payload. It also checks source Git HEAD, manifest
hash and variant/workload identity.

Smoke workloads: three fixed representative corpus songs plus synthetic
100 and 1,000 generations at width 1 and 15. Full: all 110 prepared
songs plus synthetic 100, 1,000 and 10,000 generations at widths 1 and
15. Each workload runs at least five paired trials per stage (cold
and warm), interleaved AB/BA. Each child holds one prepared owner.

## Metrics and limitations

- Win32 GetProcessMemoryInfo(PROCESS_MEMORY_COUNTERS_EX).PrivateUsage is
  PRIVATE COMMITTED BYTES, not RSS.
- WorkingSetSize includes shared resident pages; it is not private RSS.
- PeakPagefileUsage and PeakWorkingSetSize are whole-process lifetime
  peaks, including source/planner/compilation and prior warmup; they
  cannot be attributed to the prepared build alone.
- Cold means no in-process warmup, not an OS page-cache flush.
- Warm means one unmeasured same-variant preparation in the same fresh
  process. Warm lifetime peaks are explicitly contaminated by warmup.
- A GlobalAlloc wrapper counts Rust-managed allocations during
  preparation and coordinator reset; it excludes Win32/CRT heaps.
  Requested bytes do not equal retained heap or physical memory.
- prepare_reset_ns excludes song source loading, compilation and initial
  coordinator construction. Separate-process pairs reduce process
  contamination but cannot eliminate OS noise or thermal effects.
- The experiment does not profile the real-time prepared sender or
  actual game receipt. A production compact feature flag requires
  separate safety/correctness benchmarks before rollout.

## Validation

Only after PR HEAD and required CI are all verified completed/success,
use the existing clean detached Windows validation worktree. The original
main checkout must not be changed. Run from that worktree in PowerShell 7:

    pwsh -NoProfile -File .\scripts\qualify_compact_prepared_ab.ps1 -ExpectedHead <SHA> -Scope smoke -Pairs 5

Full trial:

    pwsh -NoProfile -File .\scripts\qualify_compact_prepared_ab.ps1 -ExpectedHead <SHA> -Scope full -Pairs 5

The runner builds test-support release examples with Cargo --locked,
stores raw per-trial JSON, CSV pairs/deltas and machine/binary manifest
in TEMP, and uses no real input. Fail closed on any unexplained
difference. Compare matched medians/tails and negative workloads
individually. Do not summarize all song workloads as a single startup
speedup without showing per-song outcomes and variance.

Windows API documentation:
https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex
https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-getprocessmemoryinfo
