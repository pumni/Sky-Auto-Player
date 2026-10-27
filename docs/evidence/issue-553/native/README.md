# Native continuous resize measurements

Values are **before / final**. Each row is a four-second native WebView2 drag. Full distributions and artifact identities: [before.json](before.json), [final.json](final.json), [before-artifact.json](before-artifact.json), [final-artifact.json](final-artifact.json). Raw compressed DevTools traces: [trace-manifest.json](trace-manifest.json). See [method and limitations](../native-method.md).

## React and rendering work

Times are milliseconds over the measurement window, including helper setup/settle. Paint totals sum captured Paint event durations, not an exclusive CPU budget.

| Fixture / state / pane  | Root commits | Script ms     | Layout ms     | Style ms    | Paint event ms |
| ----------------------- | ------------ | ------------- | ------------- | ----------- | -------------- |
| 12-idle-inspector       | 288 / 2      | 666.3 / 100.3 | 322.1 / 253.5 | 54.6 / 86.1 | 374.4 / 269.5  |
| 12-idle-navigator       | 277 / 1      | 540.1 / 79.0  | 261.3 / 205.7 | 46.0 / 86.4 | 344.4 / 250.8  |
| 12-idle-clamp           | 208 / 1      | 333.6 / 77.7  | 125.2 / 134.0 | 35.9 / 58.0 | 181.5 / 170.7  |
| 12-playback-inspector   | 341 / 41     | 509.3 / 152.2 | 260.2 / 288.8 | 55.8 / 99.6 | 262.1 / 289.4  |
| 12-playback-navigator   | 315 / 41     | 442.4 / 127.6 | 218.8 / 223.1 | 47.2 / 79.4 | 241.3 / 240.9  |
| 12-playback-clamp       | 235 / 40     | 344.2 / 124.4 | 130.7 / 153.9 | 43.6 / 72.4 | 178.0 / 172.1  |
| 1280-idle-inspector     | 326 / 2      | 619.9 / 104.8 | 388.2 / 346.2 | 52.0 / 82.7 | 465.0 / 343.2  |
| 1280-idle-navigator     | 277 / 1      | 427.0 / 84.9  | 267.7 / 273.7 | 37.8 / 68.6 | 358.9 / 331.6  |
| 1280-idle-clamp         | 202 / 1      | 355.5 / 86.1  | 181.6 / 172.2 | 37.9 / 61.8 | 257.4 / 227.5  |
| 1280-playback-inspector | 358 / 40     | 729.3 / 144.0 | 528.2 / 349.4 | 73.6 / 90.3 | 559.7 / 362.2  |
| 1280-playback-navigator | 310 / 41     | 474.8 / 138.4 | 302.3 / 289.1 | 49.1 / 80.6 | 355.9 / 321.0  |
| 1280-playback-clamp     | 244 / 41     | 366.0 / 128.0 | 176.6 / 186.3 | 45.4 / 75.6 | 258.9 / 232.0  |

## Frame and pointer tracking

Age is a DOM sampling estimate with a 1px matching tolerance, not physical input-to-photon latency. Errors compare the last delivered pointer event with pane geometry at the sampling rAF.

| Fixture / state / pane  | Frame p95 ms | Frame max ms | Gaps >33ms | Pointer error p95 px | Matched-event age p95 ms |
| ----------------------- | ------------ | ------------ | ---------- | -------------------- | ------------------------ |
| 12-idle-inspector       | 16.8 / 17.0  | 17.1 / 17.2  | 0 / 0      | 2.0 / 2.4            | 17.8 / 17.5              |
| 12-idle-navigator       | 16.9 / 17.0  | 17.7 / 17.1  | 0 / 0      | 1.8 / 1.4            | 18.1 / 17.5              |
| 12-idle-clamp           | 16.9 / 17.0  | 17.3 / 17.1  | 0 / 0      | 2.4 / 2.4            | 33.0 / 17.5              |
| 12-playback-inspector   | 17.0 / 17.0  | 17.4 / 17.3  | 0 / 0      | 2.4 / 2.0            | 32.9 / 17.8              |
| 12-playback-navigator   | 17.0 / 16.9  | 17.2 / 17.2  | 0 / 0      | 1.6 / 1.4            | 18.0 / 17.4              |
| 12-playback-clamp       | 17.0 / 17.0  | 17.4 / 17.1  | 0 / 0      | 2.4 / 2.4            | 17.8 / 17.8              |
| 1280-idle-inspector     | 16.9 / 17.0  | 17.3 / 17.4  | 0 / 0      | 2.4 / 2.2            | 17.4 / 17.9              |
| 1280-idle-navigator     | 17.0 / 17.0  | 17.3 / 17.7  | 0 / 0      | 1.6 / 1.6            | 17.4 / 17.6              |
| 1280-idle-clamp         | 17.0 / 17.0  | 17.1 / 17.2  | 0 / 0      | 2.4 / 2.2            | 17.4 / 17.5              |
| 1280-playback-inspector | 16.9 / 17.0  | 17.0 / 17.3  | 0 / 0      | 2.4 / 2.8            | 18.4 / 17.9              |
| 1280-playback-navigator | 17.0 / 17.0  | 17.3 / 17.5  | 0 / 0      | 1.6 / 1.6            | 18.0 / 17.4              |
| 1280-playback-clamp     | 17.1 / 17.1  | 17.3 / 17.4  | 0 / 0      | 2.2 / 2.0            | 18.2 / 17.7              |

Frame p50 is approximately 16.7ms throughout; p99/max and all sample counts are retained in JSON. Both retained baseline and final runs had no >33ms gaps. The measurements demonstrate reduced React/script work; they do not establish improved presentation latency or VS Code parity. Layout/style/paint are reported separately because some costs increased.

## Style-invalidation finding

An intermediate build at `6992f3d0b6d8d436595a96da53db99ce6455e6af` reduced React work but inherited shell properties increased style recalculation. For 1280-idle-inspector, style time was 673.7ms; final was 82.7ms. The final build uses non-inheriting registered properties and explicit inheritance only at the immediate panes. The intermediate full matrix is [retained as JSON](intermediate.json), with its [representative raw trace](after-1280-idle-inspector.trace.json.gz).
