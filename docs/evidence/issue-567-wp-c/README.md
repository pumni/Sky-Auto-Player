# Issue #567 WP-C UI evidence

These captures show the selected-song compatibility flow at a 1366×768 Chromium
viewport using the desktop mock bridge. The mock exercises the UI and consent
state only; these images do not establish native Windows focus, Piano tuning,
or gameplay input behavior.

Captured by:

```powershell
cd desktop
bun run test:e2e -- tests/e2e/sheet-compatibility.spec.ts
```

| Capture | State shown |
| --- | --- |
| `legacy-ready.png` | Flat legacy sheet remains ready and displays its existing timing-risk summary. |
| `multitrack-loss-proposal.png` | Composed v5 losses and explicit conversion consent proposal before a prepared plan exists. |
| `multitrack-loss-accepted.png` | Accepted conversion state and separate offline timing advisory. |
| `unsupported-v99-blocked.png` | Unsupported Composed v99 error with conversion controls hidden. |
| `source-pitch-assumption.png` | Source pitch assumption and in-game Sky Piano tuning warning. |
