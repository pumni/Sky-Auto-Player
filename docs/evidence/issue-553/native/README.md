# Native continuous resize measurements

Values are **before / final**. Each row is a four-second native Windows cursor drag in WebView2. Full distributions and artifact identities: [before.json](before.json), [final.json](final.json), [before-artifact.json](before-artifact.json), [final-artifact.json](final-artifact.json). Raw compressed DevTools traces: [trace-manifest.json](trace-manifest.json). See [method and limitations](../native-method.md).

## React and rendering work

Times are milliseconds over the measurement window, including helper setup/settle. Paint totals sum captured Paint event durations, not an exclusive CPU budget.

| Fixture / state / pane  | Root commits | Script ms     | Layout ms     | Style ms     | Paint event ms |
| ----------------------- | ------------ | ------------- | ------------- | ------------ | -------------- |
| 12-idle-inspector       | 288 / 2      | 666.3 / 109.6 | 322.1 / 259.3 | 54.6 / 92.8  | 374.4 / 294.8  |
| 12-idle-navigator       | 277 / 1      | 540.1 / 81.1  | 261.3 / 207.4 | 46.0 / 72.7  | 344.4 / 244.6  |
| 12-idle-clamp           | 208 / 1      | 333.6 / 78.0  | 125.2 / 129.3 | 35.9 / 59.2  | 181.5 / 178.0  |
| 12-playback-inspector   | 341 / 40     | 509.3 / 141.3 | 260.2 / 244.5 | 55.8 / 93.7  | 262.1 / 275.1  |
| 12-playback-navigator   | 315 / 41     | 442.4 / 126.1 | 218.8 / 198.0 | 47.2 / 77.0  | 241.3 / 228.6  |
| 12-playback-clamp       | 235 / 41     | 344.2 / 122.9 | 130.7 / 146.2 | 43.6 / 70.1  | 178.0 / 177.3  |
| 1280-idle-inspector     | 326 / 2      | 619.9 / 109.1 | 388.2 / 358.3 | 52.0 / 91.9  | 465.0 / 406.5  |
| 1280-idle-navigator     | 277 / 1      | 427.0 / 78.2  | 267.7 / 289.5 | 37.8 / 73.9  | 358.9 / 344.0  |
| 1280-idle-clamp         | 202 / 1      | 355.5 / 87.2  | 181.6 / 185.4 | 37.9 / 67.4  | 257.4 / 258.6  |
| 1280-playback-inspector | 358 / 40     | 729.3 / 164.1 | 528.2 / 399.8 | 73.6 / 106.2 | 559.7 / 413.7  |
| 1280-playback-navigator | 310 / 40     | 474.8 / 136.6 | 302.3 / 306.0 | 49.1 / 95.6  | 355.9 / 345.7  |
| 1280-playback-clamp     | 244 / 41     | 366.0 / 131.1 | 176.6 / 188.8 | 45.4 / 74.8  | 258.9 / 263.1  |

## Frame and pointer tracking

Age is a DOM sampling estimate with a 1px matching tolerance, not physical input-to-photon latency. Errors compare the last delivered pointer event with pane geometry at the sampling rAF.

| Fixture / state / pane  | Frame p95 ms | Frame max ms | Gaps >33ms | Pointer error p95 px | Pointer error max px | Matched-event age p95 ms |
| ----------------------- | ------------ | ------------ | ---------- | -------------------- | -------------------- | ------------------------ |
| 12-idle-inspector       | 16.8 / 17.0  | 17.1 / 17.5  | 0 / 0      | 2.0 / 2.8            | 2.8 / 34.2           | 17.8 / 17.4              |
| 12-idle-navigator       | 16.9 / 17.0  | 17.7 / 17.5  | 0 / 0      | 1.8 / 1.8            | 2.4 / 2.8            | 18.1 / 19.3              |
| 12-idle-clamp           | 16.9 / 16.9  | 17.3 / 33.3  | 0 / 1      | 2.4 / 2.4            | 4.4 / 3.0            | 33.0 / 17.4              |
| 12-playback-inspector   | 17.0 / 17.0  | 17.4 / 17.4  | 0 / 0      | 2.4 / 2.4            | 3.6 / 3.2            | 32.9 / 31.9              |
| 12-playback-navigator   | 17.0 / 16.9  | 17.2 / 17.2  | 0 / 0      | 1.6 / 1.8            | 2.0 / 2.8            | 18.0 / 20.9              |
| 12-playback-clamp       | 17.0 / 17.0  | 17.4 / 17.4  | 0 / 0      | 2.4 / 2.4            | 2.8 / 3.6            | 17.8 / 17.7              |
| 1280-idle-inspector     | 16.9 / 16.9  | 17.3 / 17.2  | 0 / 0      | 2.4 / 2.8            | 3.0 / 14.0           | 17.4 / 17.5              |
| 1280-idle-navigator     | 17.0 / 17.0  | 17.3 / 17.3  | 0 / 0      | 1.6 / 1.6            | 2.8 / 2.6            | 17.4 / 33.4              |
| 1280-idle-clamp         | 17.0 / 17.0  | 17.1 / 17.4  | 0 / 0      | 2.4 / 2.0            | 3.0 / 2.8            | 17.4 / 17.5              |
| 1280-playback-inspector | 16.9 / 17.0  | 17.0 / 17.4  | 0 / 0      | 2.4 / 2.4            | 4.0 / 4.0            | 18.4 / 19.0              |
| 1280-playback-navigator | 17.0 / 17.0  | 17.3 / 17.2  | 0 / 0      | 1.6 / 1.6            | 2.0 / 2.0            | 18.0 / 17.9              |
| 1280-playback-clamp     | 17.1 / 16.9  | 17.3 / 17.6  | 0 / 0      | 2.2 / 2.2            | 3.0 / 3.0            | 18.2 / 17.7              |

Frame p50 is approximately 16.7ms throughout; p99/max and all sample counts are retained in JSON. The retained baseline had no >33ms gaps; the final matrix had one isolated 33.3ms gap in the small idle clamp case. Position-error outliers include 34.2px in small idle Inspector; low p95 values do not establish zero lag. The measurements demonstrate reduced React/script work, not improved presentation latency or VS Code parity. Layout/style/paint are reported separately because some costs increased.

## Style-invalidation finding

An intermediate build at `6992f3d0b6d8d436595a96da53db99ce6455e6af` reduced React work but inherited shell properties increased style recalculation. For 1280-idle-inspector, style time was 673.7ms; final was 91.9ms. The final build uses non-inheriting registered properties and explicit inheritance only at immediate panes. The intermediate full matrix is [retained as JSON](intermediate.json), with its [representative raw trace](after-1280-idle-inspector.trace.json.gz).
