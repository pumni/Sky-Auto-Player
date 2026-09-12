# Release-gap stress result

## Review correction (2026-09-13)

The original interpretation below that the extra event was a production
KeyDown is withdrawn. The full raw sink log contains 1,031 records; sequences
1–4 are `0x4B` extended Left Arrow activity before the apparent stress stream.
The report's 1,027-event window begins at sequence 5, and its extra `0x4B`
record at sequence 7 is a KeyUp. This archive did not preserve the sink cursor
captured before arm, so event provenance is inconclusive. Do not attribute that
event to the production engine.

The physical forensics remain a confirmed qualification failure: the raw report
has 4 of 512 release gaps below the one-frame floor, and 1 of 513 completion
holds below the floor (minimum `16.3889 ms` versus `16.667 ms`). Those findings
do not depend on the disputed input-event attribution.

This Windows interactive run used the production `sky_player` dispatch path
against the project-owned `ReceiveOnly` sink. It is bound to implementation
commit `bb30148c404146821bbd9ef6b44a55b5cc258051` and the sink identity in
`sink-ready.json`.

Configuration:

```text
FPS                    60
Base Hold              1.0 frame
Timing Margin          800 µs
Late Down tolerance    500 µs
Expected key events    1,026 (513 Down + 513 Up)
Required gap samples   512
QPC frequency          10,000,000 Hz
```

The run completed all 512 gap observations but did **not** qualify:

```text
production_release_gap_below_policy_count = 4
production_forensics_anomaly_count        = 4
minimum release gap                      = 162,742 ticks = 16.2742 ms
fixed one-frame floor                    = 16.667 ms
observed below-floor fraction             = 4 / 512 = 0.78125%
```

The captured report's reason string says one extra KeyDown on scan `0x4B`
(extended) was rejected in its 1,027-event window instead of 1,026 expected.
The independent raw-log review above contradicts the reported direction and
does not establish the event source. There were no partial or zero-progress
SendInput calls, dropped keys, stuck keys, or focus losses. The harness result
is `FAIL` because its production forensics include below-floor observations;
the disputed input-event attribution is not part of that conclusion.

This run used only the 500 µs cutoff and 800 µs margin. It does not establish
that another cutoff would change the release-gap behavior. No production
SendInput batching or keyboard encoding change was made.
