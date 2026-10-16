# R3 keyboard dispatch and receiver diagnostics

P4 characterizes head-of-line (HOL) delay, prepared startup cost, and a fixed
offline receiver polling model for [ADR-0018](adr/ADR-0018-resumable-release-barrier-and-final-lease-age.md)
and [issue #435](https://github.com/pumni/Sky-Auto-Player/issues/435). It does
not change production scheduling, timing, timer, priority, SendInput, or input
policy. HOL uses the production coordinator and prepared dispatch through
test-support with a deterministic mock sender. Startup measures the production
prepared-stream builder through test-support. Polling is a pure offline model.
No P4 case invokes `SendInput`, a game process, or a game observer.

## HOL characterization

The fixed run contains four sequential dispatch scenarios and 36 maximum-chord
configurations (15 keys; 30/60/120/240 FPS; margins 0/500/3000 µs; hold
multipliers 1/1.25/1.5). These produce 91 packet rows. The production planner
and physical wait target provide readiness; each packet then goes through the
existing prepared dispatch/coordinator admission and deterministic sender.
The diagnostic reports authored target, production ready floor, sender pre-call
and completion, sender duration, blocking packet/keymask, delay attribution,
transport attempts, physical floor, and generation/release accounting.

In the fixed A/B vector, A Down is authored at 100,000 µs and completes at
110,000 µs. A Up is authored at 117,167 µs, becomes physically ready at
127,167 µs, and the configured 400 µs mock send completes at 127,567 µs.
Different-key B Down remains authored at 118,000 µs and is called at
127,567 µs after the A Up sender completes. Its 9,567 µs delay is attributed
to packet 1 and keymask `0x0001`; authored offsets are unchanged.

The mixed Up A / Down B case remains one two-event physical packet and one
transport attempt. The three-Down overdue case shows starts at 130,167,
130,247, and 130,337 µs after a 3,000 µs mock Up sender; those Downs retain
their authored targets and have configured sender durations of 80, 90, and
100 µs. The repeated same-key case keeps the second Down authored at
135,000 µs but waits until 143,884 µs for the production physical release
floor established after the prior Up completes. The maximum-chord matrix also
uses the production physical guard.

Across all 91 rows the characterization found zero physical-floor violations
and zero Down replays. These results retain HOL delay and overdue bursts as
known behavior; they do not propose reordering, packet splitting, catch-up,
replay, or a scheduler redesign. Sender durations are mock-path timing and do
not measure Windows `SendInput` latency.

## Startup and resource characterization

Each fixed schedule contains one feasible Down/Up generation per row, with
`H = 17,167 µs`, a 34,334 µs row interval, and one of 15 canonical scan codes
selected using seed `1,379,160,116`. Input fixture construction is outside the
timed regions. Schedule compile/materialize is separate from coordinator
initialization and prepared-stream build/reset.
The largest fixture has 200,000 actions, below the existing 1,000,000-action
limit.

| Generations | Compile/materialize (µs) | Coordinator init (µs) | Prepared build/reset (µs) | Prepared allocations (calls / requested bytes) | Scoped worker path (µs / 48,000 µs) | Budget outcome |
| ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 100 | 59 | 17 | 226 | 219 / 276,292 | 243 / 48,000 | WOULD_ADMIT |
| 1,000 | 339 | 50 | 1,610 | 2,025 / 2,739,208 | 1,660 / 48,000 | WOULD_ADMIT |
| 10,000 | 2,639 | 274 | 14,900 | 20,033 / 27,982,768 | 15,174 / 48,000 | WOULD_ADMIT |
| 100,000 | 28,490 | 2,298 | 162,697 | 200,039 / 276,682,816 | 164,995 / 48,000 | WOULD_REJECT |

The 48,000 µs comparison is the production 50,000 µs minimum pre-roll less
the 2,000 µs readiness reserve. It includes only coordinator initialization
plus prepared-stream build/reset. Compile/materialize, thread launch, target
preflight, telemetry, and other worker startup work are excluded. Thus the
100,000-generation dataset is labeled `WOULD_REJECT` against this scoped
budget. No production boot was attempted and no product rejection was
observed.

| Generations | Schedule clone (µs; calls / bytes) | Test-support coordinator clone (µs; calls / bytes) | Prepared entries / entry size / array bytes | Coordinator heap payload bytes | Sampled process working-set peak (bytes) |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 8; 6 / 14,482 | 13; 9 / 18,582 | 200 / 16 / 3,200 | 4,100 | 7,204,864 |
| 1,000 | 52; 6 / 144,082 | 83; 9 / 185,082 | 2,000 / 16 / 32,000 | 41,000 | 7,208,960 |
| 10,000 | 270; 6 / 1,440,082 | 372; 9 / 1,850,082 | 20,000 / 16 / 320,000 | 410,000 | 10,063,872 |
| 100,000 | 1,786; 6 / 14,400,082 | 2,569; 9 / 18,500,082 | 200,000 / 16 / 3,200,000 | 4,100,000 | 307,691,520 |

The coordinator clone is test-support-only, is not used by production boot, and
is excluded from the budget comparison. Allocation bytes count successful
allocation and reallocation requests for each named operation; deallocation
is not subtracted, and allocator metadata is not included. Structural byte
counts cover the named vector/entry payloads only. PowerShell sampled child
process working set every 5 ms; this separate sampled value includes shared
resident pages and runtime memory and is not the allocation counter or a
guaranteed peak RSS measurement. The complete per-operation measurements and
provenance are in the linked JSON and CSV evidence.

## Offline receiver frame-polling model

The model has 1,620 primary cases: 30/60/120/240 FPS, `H = T + margin` with
margins 0/500/3000 µs, phases 0/`T/4`/`T/2`/`3T/4`/exact `T`, stalls of 0/1/2
frames, and independent Down and Up delivery skews of 0/500/2000 µs. Events
delivered at a poll timestamp are applied before the sample. A stall skips
polls in `[0, stall duration)`, so a poll at its end is eligible.

Each cell below reports missed Down states out of 135 cases. Every primary
case also reports whether the separating Up state was sampled; all primary
cases sampled that state.

| FPS | Margin 0 µs | Margin 500 µs | Margin 3,000 µs |
| ---: | ---: | ---: | ---: |
| 30 | 83/135 Down missed | 72/135 Down missed | 72/135 Down missed |
| 60 | 83/135 Down missed | 72/135 Down missed | 69/135 Down missed |
| 120 | 83/135 Down missed | 69/135 Down missed | 60/135 Down missed |
| 240 | 82/135 Down missed | 68/135 Down missed | 42/135 Down missed |

Six supplemental 30 FPS cases use a 17,167 µs hold at phases 0, `3T/4`, and
exact `T`, with zero- and one-frame stalls. The model sees the Down only for
phase 0 with no stall; the other five cases miss it. Missing states are
reported outcomes, not failed assertions. This model does not assume a
17,167 µs hold is observed by a 30 FPS receiver, including under a 30 ms
stall.

Evidence remains separated into three layers: this offline model is
**MODELED POLLING**; the published P1/P2 project-owned ReceiveOnly sink runs
are **ACTUAL RECEIVEONLY EVENT OBSERVATION**; Sky/game consumption is
**UNKNOWN**. ReceiveOnly delivery does not prove game polling, rendering, or
audio onset. Monitor refresh rate and DWM behavior are not treated as the
game's polling rate.

## Provenance and immutable prior evidence

The P4 implementation was measured at source head
[`3a96be29eb2e9ceff03db9ab659b073cd7547c17`](https://github.com/pumni/Sky-Auto-Player/tree/3a96be29eb2e9ceff03db9ab659b073cd7547c17),
based on verified P3 merge/base
[`b012b0a1c135a9176cfc7c6aa8696fa0be29a481`](https://github.com/pumni/Sky-Auto-Player/commit/b012b0a1c135a9176cfc7c6aa8696fa0be29a481).
The Windows 11 x64 MSVC run used Rust/Cargo 1.99.1 and Bun 1.4.3. Probe
executable SHA-256:
`35020a202c3f488acaadf96c2f0b91fae506b41923aca9047c4e75dd5b90361d`.

The append-only P4 evidence is committed at
[`5c0445a7d5965fd2320856df60e260372586f341`](https://github.com/pumni/Sky-Auto-Player/tree/5c0445a7d5965fd2320856df60e260372586f341/runs/r3-p4-20261016T145957238Z-be3ec0ab)
for run `r3-p4-20261016T145957238Z-be3ec0ab`. Its
[manifest](https://github.com/pumni/Sky-Auto-Player/blob/5c0445a7d5965fd2320856df60e260372586f341/runs/r3-p4-20261016T145957238Z-be3ec0ab/manifest.json)
SHA-256 is
`b11bbb81732edc6ec6b7c61fe878e7df6621c99c25efcefa2a56435eac11d028`.
The [evidence archive](https://github.com/pumni/Sky-Auto-Player/blob/5c0445a7d5965fd2320856df60e260372586f341/runs/r3-p4-20261016T145957238Z-be3ec0ab/archive/evidence.zip)
contains the JSON, CSV, Markdown, fixed scenario outputs, runner manifest,
and captured child-command logs. Its SHA-256 is
`86b2c2262f4f3233f82b52ce1a347762502227f63b98e3e88362a3fb16e0105f`.
Native full rerun and paired A/B rerun were both false.

P0/P1/P2 evidence remains unchanged and is cross-referenced in
[ADR-0018's published phase evidence](adr/ADR-0018-resumable-release-barrier-and-final-lease-age.md#published-phase-evidence):

- P0 characterization evidence commit
  [`596664e809b69318eb216d25b9bed680446641e3`](https://github.com/pumni/Sky-Auto-Player/tree/596664e809b69318eb216d25b9bed680446641e3/runs/r3-p0-20261005T024258505Z-700aaab7),
  manifest SHA-256 `ca4904e3114d77f2a7abf769b3a3701954297a803629cd16f6768999a4aa5fbb`.
- P1 lifecycle/ReceiveOnly evidence commit
  [`f6c1c9ffb5cf5e0aaceef77b1c06e23efdc562ed`](https://github.com/pumni/Sky-Auto-Player/tree/f6c1c9ffb5cf5e0aaceef77b1c06e23efdc562ed/runs/r3-p1-20261016T111711630Z-df42d2fd),
  manifest SHA-256 `89463e7aaeb2ad13d61eb068c8742aa200fde9e5fe821d70cc8977e042c95fcb`.
- P2 lease and paired/native evidence commit
  [`d9478f2e8e9662a9d1bee2d736326ebdec5f8599`](https://github.com/pumni/Sky-Auto-Player/tree/d9478f2e8e9662a9d1bee2d736326ebdec5f8599/runs/r3-p2-20261016T132824008Z-03e7028d),
  manifest SHA-256 `bf5f662a26610f09b5f90be4e86148c8c60512f9aee0552f1737b32f270f1c19`.

HOL delay, startup cost, and long-hold receiver limitations remain
characterization findings. This phase makes no scheduler or storage redesign
proposal and establishes no new receiver guarantee.
