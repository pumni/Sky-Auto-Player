# Hotkey and Window-Focus Hardening

## Goal

Remove unnecessary high-frequency global keyboard-state polling and aggressive automatic foreground manipulation without changing core playback input semantics.

Do not modify the Rust `SendInput` dispatcher for this work.

## Hotkey target

Current pattern:

```text
timer tick
→ GetAsyncKeyState(modifiers)
→ GetAsyncKeyState(action keys)
→ edge detection
```

Target:

```text
RegisterHotKey
→ WM_HOTKEY
→ thread-safe action queue
→ PlaybackControls consumes action
```

## Platform boundary

Add low-level Windows module, e.g.:

```text
src/sky_music/platform/win32/global_hotkeys.py
```

All ctypes bindings live there.

Potential APIs:

```text
RegisterHotKey
UnregisterHotKey
GetMessageW
PostThreadMessageW
GetCurrentThreadId
WM_HOTKEY
WM_QUIT
MOD_ALT
MOD_CONTROL
MOD_SHIFT
MOD_NOREPEAT
```

Forbidden:

```text
SetWindowsHookEx
WH_KEYBOARD
WH_KEYBOARD_LL
```

No keyboard hooks.

## Infrastructure behavior

Keep binding/action semantics in:

```text
src/sky_music/infrastructure/hotkeys.py
```

Use one dedicated message-loop thread.

Thread:

1. records Windows thread ID;
2. registers configured hotkeys;
3. reports startup result;
4. blocks in `GetMessageW`;
5. maps `WM_HOTKEY` IDs to actions;
6. pushes actions to thread-safe queue;
7. exits on `WM_QUIT`;
8. unregisters in `finally`.

No busy loop.

## Required actions

Preserve current controls:

```text
pause
skip
quit
refocus
panic
```

Also migrate any debug-toggle or secondary consumer that continuously polls key state.

Search full repo for:

```text
is_hotkey_down
is_virtual_key_down
GetAsyncKeyState
```

Do not leave a second general polling path.

## Registration behavior

Use `MOD_NOREPEAT` where supported.

Registration must be atomic.

If one binding fails:

1. unregister all registrations made in that start attempt;
2. return typed conflict/startup error;
3. do not start real playback controls partially configured.

Never silently disable panic/quit.

Never fallback to polling.

## Lifecycle

`close()` must be idempotent.

Repeated playback sessions must register/unregister cleanly.

No leaked global hotkeys after normal exit, exception, stop, or UI teardown.

## Focus architecture

Separate read-only target discovery from focus mutation.

### Read-only allowed

```text
EnumWindows
GetWindowThreadProcessId
PROCESS_QUERY_LIMITED_INFORMATION
QueryFullProcessImageNameW
GetForegroundWindow
IsWindow
```

### Explicit user-requested focus

Minimal preferred attempt:

```text
ShowWindow(SW_RESTORE)
SetForegroundWindow
```

If Windows refuses, return false and tell user to click/focus Sky manually.

## Remove aggressive automatic focus reclaim

Remove automatic use of:

```text
AttachThreadInput
SetWindowPos(HWND_TOP)
BringWindowToTop
SetActiveWindow
```

Do not replace these with a more aggressive trick.

Focus loss during playback should follow existing safe focus-loss handling, not steal focus back automatically.

## Startup behavior

Target exists + foreground:

```text
start playback
```

Target exists + not foreground:

```text
ask user to focus Sky
```

Target missing:

```text
show target-not-found
```

Do not force foreground merely to start.

## Explicit refocus

User presses configured `refocus` action:

```text
minimal refocus attempt
```

If it fails:

```text
show instruction to focus Sky manually
```

## Tests

Hotkey:

- modifier mapping;
- `MOD_NOREPEAT`;
- ID/action mapping;
- queue ordering;
- conflict failure;
- atomic rollback;
- close idempotency;
- repeated lifecycle;
- disabled controls register nothing;
- no polling fallback.

Focus:

- discovery has no focus side effect;
- process mismatch rejected;
- invalid HWND rejected;
- startup does not steal focus;
- explicit refocus uses only minimal path;
- refocus failure handled;
- focus loss does not invoke aggressive APIs.

## Done criteria

Production Python playback hotkey path has no continuous `GetAsyncKeyState` loop.

Remaining `GetAsyncKeyState` must be bounded/safety-specific, not a general global poller.

Rust physical-state checks used for instrument cleanup/safety may remain.
