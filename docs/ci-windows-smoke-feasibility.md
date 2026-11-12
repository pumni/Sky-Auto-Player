# Future packaged WebView2 smoke feasibility

**Status: proposal only.** This note does not add a workflow job, debug switch,
or production packaging behavior. A later implementation requires a separate
Leader decision and security review.

## Candidate approaches

| Approach | Strengths | Constraints |
| --- | --- | --- |
| Microsoft Edge WebDriver attached to the packaged Tauri WebView2 | W3C WebDriver actions and assertions; Microsoft documents a WebView2 automation path. | The app must start a WebView2 instance with remote debugging enabled and be activated before the driver attaches. The first three version components of Edge and Edge WebDriver must match. Evergreen runtime updates can move that target. |
| Direct WebView2 CDP client | Direct access to the Chromium DevTools Protocol and can use the port reported for the WebView2 instance. | Requires a project-owned CDP client/protocol adapter and tighter protocol-version handling; more custom test infrastructure and less benefit than WebDriver for a short smoke. |

Microsoft documents WebDriver attachment to WebView2 and the need for a
`--remote-debugging-port` parameter. Its Edge WebDriver guidance requires the
first three version components to match the browser. WebView2 Fixed Version
can pin a runtime for reproducible testing, while Evergreen receives automatic
updates; Fixed Version also adds a large runtime payload and does not represent
the normal Evergreen production setup. See [WebView2 WebDriver guidance](https://learn.microsoft.com/microsoft-edge/webview2/how-to/webdriver),
[Edge WebDriver version matching](https://learn.microsoft.com/en-us/microsoft-edge/webdriver-chromium/),
and [Evergreen versus Fixed Version](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/evergreen-vs-fixed-version).

## Bounded implementation proposal

1. Prefer WebDriver for a small packaged smoke that verifies launch, initial
   route, one real bridge call, and clean shutdown. Use CDP directly only if a
   required assertion cannot be expressed through WebDriver.
2. Pin the test runtime and matching WebDriver together, record exact versions
   and SHA-256 values, and verify the downloaded driver before execution. A
   Fixed Version runtime gives a stable pair but is not the production
   Evergreen configuration; a later owner must choose the intended fidelity.
3. Enable remote debugging only for the test process through a test-scoped
   launch environment/configuration and an ephemeral loopback port. Do not add
   the flag to release configuration or ship a debug-enabled binary. Terminate
   the app and driver at test end and keep their profile/port in the runner's
   temporary directory. These are proposed safeguards, not existing controls.
4. First probe the actual `windows-latest` job session by launching the packaged
   candidate and attaching the selected driver. Do not assume the hosted runner
   offers the foreground desktop behavior the test needs. If the session is
   unsuitable, stop and ask for an approved runner strategy instead of silently
   converting the test to browser-only coverage.
5. Budget an initial qualification job at no more than 15 minutes, including
   runtime/driver setup, package install, smoke, and cleanup; record observed
   timings before setting a required status. This is a planning estimate, not a
   measured result.

The browser automation endpoint can expose page control to local processes.
Keeping its port ephemeral, loopback-scoped, and job-lifetime-only is a
security recommendation inferred from that capability; verify the actual
WebView2 host and runner behavior before adoption.

## Immutable updater bridge fixture risks

A future fixture should be immutable and bound to the exact source SHA and
artifact SHA-256, expected bridge protocol/schema, target-version sentinel,
Authenticode publisher identity/thumbprint, and test signer. Reject mutable
branch/tag lookups, missing provenance, unexpected signer identities, and
candidate/bridge mismatches. Do not introduce a production signing key or
production release authority into fixture qualification. This is a proposal;
the current hosted updater fixture result remains the evidence linked from the
CI performance baseline.
