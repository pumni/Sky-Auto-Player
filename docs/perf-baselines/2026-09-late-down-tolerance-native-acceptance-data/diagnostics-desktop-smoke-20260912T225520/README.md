# Desktop physical Diagnostics smoke

This run checked the desktop Diagnostics bridge during real `SendInput`
playback to the project-owned `ReceiveOnly` sink. It did not target Sky.

The live capture showed:

```text
Physical session       Yes
Player attached        Yes
Sender samples         20
Sender backend         Healthy
Max pre-call lateness  96 µs
```

The app was launched while the source worktree was based on
`be9c1d68bf408fb75330c4501691e1834328164e`; the captured application source
was committed unchanged in `bb30148c404146821bbd9ef6b44a55b5cc258051`. The
desktop diagnostics JSON records both identities. The ready record binds the
physical session to the sink PID, HWND, run ID, and event-log ID. The archived
event-log snapshot contains its stream header followed by 20 contiguous input
events. Playback was stopped after the capture and the terminal UI reported no
active playback session.

The screenshot is a rendered capture of the actual desktop Diagnostics panel.
Completion percentiles and release-lateness metrics were unavailable in this
short smoke because that run did not produce samples for those metrics.
