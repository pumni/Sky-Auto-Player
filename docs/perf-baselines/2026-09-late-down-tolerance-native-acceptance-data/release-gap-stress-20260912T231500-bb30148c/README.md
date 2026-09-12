# Release-gap stress result

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

The harness also rejected one extra KeyDown on scan `0x4B` (extended) in the
authorized event window: 1,027 events were observed where 1,026 were expected.
There were no partial or zero-progress SendInput calls, dropped keys, stuck
keys, or focus losses. The harness result is `FAIL`, not a clean pass. The full
JSONL report and event log are retained here so both findings can be reviewed.

This run used only the 500 µs cutoff and 800 µs margin. It does not establish
that another cutoff would change the release-gap behavior. No production
SendInput batching or keyboard encoding change was made.
