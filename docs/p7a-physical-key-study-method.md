# P7a physical-key study

P7a is an evidence-only study. Its optional probe and packet hook compile only
with `test-support`; no production admission, release, timing, or scheduling
policy changes in this phase.

## Current behavior under study

- `TrackedKeyState::ensure_instrument_keys_physically_up` preflights the full
  15-key profile. It checks foreground identity around target-thread keyboard
  layout resolution, maps each scan code with `MapVirtualKeyExW`, and samples
  `GetAsyncKeyState` for each mapped virtual key.
- The current classifier checks only the high-order down bit. If every sample
  is zero, production calls that state `AllUp`; the API also returns zero when
  the query fails, so this report calls the observation `NoHeldObserved`.
- `ensure_preflight_for_target` caches a successful probe by `TargetStamp`.
  Later Down packets for the same stamp reuse that proof. The final packet
  sender does not take another physical-key sample.
- The calibration helper `is_scan_code_physically_down` maps and samples one
  scan code independently. Ownership-derived cleanup remains in the tracked
  release path and is not driven by Down admission evidence.

`GetAsyncKeyState` documents its high-order bit as current down state, zero as
both the up value and a possible failure result, and `VK_SHIFT`, `VK_CONTROL`,
and `VK_MENU` as aggregate left/right queries. See [Microsoft's API
documentation](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getasynckeystate).

## Deterministic cases

The study regression tests characterize these cases without sending input to a
game:

1. A held startup instrument key rejects preflight before any Down is sent.
2. All-zero samples leave the current classifier at `AllUp`, while the study
   records `NoHeldObserved` because zero can also mean query failure.
3. A no-held preflight is cached, then a pending Down key becomes held at the
   final sender boundary. The same-stamp cache does not query again and the
   packet proceeds; a fresh preflight observes and rejects the held mask.
4. A held key outside a packet's pending Down mask is not queried by candidate
   B and is not reported as a same-packet collision.
5. Partial or ambiguous cleanup remains derived from the tracked transport
   ownership path, independently of the Down probe.
6. UpOnly and safety-release packets make zero Down-side queries.

## Candidate definitions

- **A:** current cached preflight only; no final-boundary physical query.
- **B:** query only the mapped VKs for the packet's pending Down mask.
- **C:** B plus five pre-materialized modifier VK queries: `VK_LWIN` (`0x5b`),
  `VK_RWIN` (`0x5c`), aggregate `VK_CONTROL` (`0x11`), `VK_SHIFT` (`0x10`),
  and `VK_MENU` (`0x12`).
- **D:** UpOnly control using C's test hook. The final sender skips the hook
  when the Down mask is empty, so the query count is zero.

Candidate VK mapping and foreground context are prepared before each measured
run. The candidate suffix performs only `GetAsyncKeyState` queries, QPC timing
bracketing, fixed atomic evidence counts, and a fixed-size histogram update. It
does not remap keys, resolve a window/thread/process, allocate, log, or change
the number of sender calls.

The report uses `HeldObserved(mask)`, `NoHeldObserved`, and `Inconclusive`.
`HeldObserved` keeps note and modifier masks separate. No evidence is recorded
for UpOnly packets.

## Modifier and shortcut scope

The receive-only message sink is not sent Windows-key chords in this study.
Microsoft lists Windows+I/K/L/M/P/U and Windows+period/semicolon shortcuts;
some open UI, minimize windows, or lock the workstation. These combinations
are covered only by the deterministic VK/query seam and official shortcut
documentation, with no claim about how Sky handles them. The canonical note VK
mapping is recorded for the active target layout in each benchmark report.
See [Microsoft's Windows shortcut
list](https://support.microsoft.com/en-au/accessibility/windows/keyboard-shortcuts-in-windows).

## Reproducible Windows benchmark

Run `scripts/qualify_p7a_physical_key_study.ps1` from an interactive Windows
session with the benchmark host in the foreground. It builds the optimized
test-support example and runs A/B/C/D in two rotated orders under quiet and
single-process low-priority CPU contention. The default is 300 samples per
workload/run, a 5 ms wait target, and 30 same-key retrigger samples per run.
The reports are written beneath `.benchmarks/p7a-physical-key-study/`; they
record the exact source SHA, mapped VKs, query counts, probe durations, and
timing distributions. Transport is deterministic mock transport. The study
samples physical state but sends no physical input.
