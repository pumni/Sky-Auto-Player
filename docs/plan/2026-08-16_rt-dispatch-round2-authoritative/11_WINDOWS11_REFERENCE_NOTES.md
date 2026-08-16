# 11 — Windows 11 Primary-Source Reference Notes

This document records the Microsoft API constraints that the implementation must respect. It is not a generic Windows tuning guide.

Primary sources are Microsoft Learn pages. Re-check them if a future Windows/API revision changes the documented contract.

---

# 1. QueryPerformanceCounter (QPC)

Microsoft Learn:

- https://learn.microsoft.com/en-us/windows/win32/sysinfo/acquiring-high-resolution-time-stamps
- https://learn.microsoft.com/en-us/windows/win32/api/profileapi/nf-profileapi-queryperformancecounter

Relevant contract:

- QPC is the primary native Windows API for high-resolution interval timestamps.
- Microsoft recommends QPC when resolution of about 1 µs or better is required and UTC synchronization is not required.
- QPC frequency is fixed at system boot and can be cached.
- Calling RDTSC/RDTSCP directly is discouraged for portable/correct user-mode timing.
- Thread affinity to one CPU is not required for QPC and Microsoft describes that practice as neither necessary nor desirable.
- QPC includes time spent in sleep states such as standby/hibernate/connected standby.
- QPC read cost can vary by hardware/platform, so extra reads on the precision path still require justification.

Design consequence:

```text
KEEP QPC
CACHE frequency once
DO NOT pin a core for QPC
DO NOT replace with RDTSC
DO NOT add QPC samples only for cosmetic telemetry
```

---

# 2. High-resolution waitable timer

Microsoft Learn:

- https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createwaitabletimerexw

Relevant contract:

`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` creates a high-resolution timer and is documented for time-critical situations where short expiration delays on the order of a few milliseconds are unacceptable. It is supported from Windows 10 version 1803 onward, therefore available on supported Windows 11 systems.

Design consequence:

```text
production uses CREATE_WAITABLE_TIMER_HIGH_RESOLUTION
failure is explicit/fail-closed
kernel timer wakes early; QPC still decides exact target
```

The timer is not itself proof that the thread will execute at an exact microsecond. Final QPC comparison/spin remains necessary for the chosen design.

---

# 3. `timeBeginPeriod`

Microsoft Learn:

- https://learn.microsoft.com/en-us/windows/win32/api/timeapi/nf-timeapi-timebeginperiod

Relevant Windows 11 points:

- requests a minimum periodic-timer resolution;
- does not improve QPC accuracy;
- high resolution can increase scheduler activity, power use, and reduce overall system performance;
- since Windows 11, Windows does not guarantee the higher resolution for a window-owning process that is fully occluded, minimized, invisible, or inaudible;
- since Windows 10 version 2004 its behavior is no longer the old global-resolution contract for all processes.

Design consequence:

The production path already requires a high-resolution waitable timer, so `timeBeginPeriod` is not a suitable fallback and is removed from production waiter construction.

---

# 4. SendInput

Microsoft Learn:

- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput

Relevant contract:

- takes an array of `INPUT` structures;
- return value is the number of events successfully inserted into the keyboard/mouse input stream;
- events in one `SendInput` array are inserted serially and are not interspersed with other keyboard/mouse events;
- SendInput is subject to UIPI;
- current keyboard state is not reset automatically; already-held keys can interfere, and Microsoft explicitly points to keyboard-state checks such as `GetAsyncKeyState`.

Design consequence:

```text
one authored Down chord -> one INPUT array / one SendInput call
Up entries before Down entries inside a Mixed transaction
partial Down-bearing result -> integrity fault, not a safe retry prefix
preflight existing physical state before precision wait
```

A successful return does not document game polling, frame consumption, rendering, or audio onset. Sender-side completion must not be labeled game-observed timing.

---

# 5. KEYBDINPUT scan-code and timestamp semantics

Microsoft Learn:

- https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-keybdinput

Relevant contract:

- with `KEYEVENTF_SCANCODE`, `wScan` identifies the key and `wVk` is ignored;
- `KEYEVENTF_KEYUP` marks release;
- `time` is a millisecond event timestamp; when zero, the system supplies its own timestamp.

Design consequence:

Keep current packet template semantics:

```text
wVk = 0
wScan = physical scan code
dwFlags = KEYEVENTF_SCANCODE (+ KEYEVENTF_KEYUP for release)
time = 0
```

Do not attempt to schedule a future key event by putting a future value into `KEYBDINPUT.time`. The production scheduler is QPC wait followed by immediate SendInput.

---

# 6. Multimedia Class Scheduler Service (MMCSS)

Microsoft Learn:

- https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service
- https://learn.microsoft.com/en-us/windows/win32/api/avrt/nf-avrt-avsetmmthreadpriority

Relevant contract:

- MMCSS boosts threads doing time-sensitive multimedia work while participating in Windows scheduling/resource policy;
- `AvSetMmThreadPriority` provides relative levels including NORMAL, HIGH, and CRITICAL for the registered task;
- scheduling still depends on task category, foreground state, CPU use, and MMCSS policy.

Design consequence:

Keep the existing conservative production choice:

```text
AvSetMmThreadCharacteristicsW("Games")
AvSetMmThreadPriority(..., AVRT_PRIORITY_HIGH)
```

with the existing safe fallback if registration fails.

Do not assume `AVRT_PRIORITY_CRITICAL` is automatically lower latency; it needs separate evidence and is not approved by this plan.

---

# 7. General Windows thread priority

Microsoft Learn:

- https://learn.microsoft.com/en-us/windows/win32/procthread/scheduling-priorities
- https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setthreadpriority

Relevant warnings:

- high-priority threads should execute briefly and only when time-critical work is needed;
- Microsoft says `REALTIME_PRIORITY_CLASS` should almost never be used because it can interrupt system threads that manage mouse input, keyboard input, and background disk flushing;
- SetThreadPriority documentation warns that base priority above 11 can interfere with normal OS operation and that realtime priority can cause responsiveness problems.

Design consequence:

```text
DO NOT use REALTIME_PRIORITY_CLASS
DO NOT make THREAD_PRIORITY_TIME_CRITICAL the production default
KEEP dispatch thread blocked outside short deadline regions
```

`THREAD_PRIORITY_HIGHEST` remains an acceptable bounded fallback in the current Auto path when MMCSS registration fails, subject to benchmark/observability.

---

# 8. Thread power throttling / HighQoS

Microsoft Learn:

- https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setthreadinformation
- https://learn.microsoft.com/en-us/windows/win32/procthread/quality-of-service

Relevant contract:

For `ThreadPowerThrottling`:

```text
ControlMask = THREAD_POWER_THROTTLING_EXECUTION_SPEED
StateMask   = 0
```

turns execution-speed throttling off (HighQoS semantics in Microsoft's example). EcoQoS is intended for non-performance-critical work and can reduce CPU frequency/use more efficient cores.

Design consequence:

Keep `PowerThrottlingGuard::disable_current_thread()` behavior and restoration for the dispatch worker.

Do not extend this into process-wide power-plan hacks.

---

# 9. What Microsoft documentation does NOT guarantee

The cited APIs do not provide a contract for:

```text
exact game input sampling timestamp
which game frame consumes a SendInput event
audio onset timestamp
network delivery timestamp
fixed SendInput syscall duration
hard real-time scheduling deadline on Windows
```

Therefore this project must not build a closed-loop onset controller from SendInput completion or Raw Input receipt and claim it is correcting game onset.

---

# 10. Design checklist against Windows contract

A coding review should answer yes to all:

- QPC used for physical timing?
- QPC frequency cached?
- no CPU affinity dependency?
- high-resolution waitable timer required?
- no production `timeBeginPeriod` fallback?
- final deadline checked in QPC domain?
- scan-code SendInput with `time=0`?
- one chord represented in one serialized SendInput array?
- current physical keyboard state considered before playback?
- MMCSS/HighQoS used conservatively?
- no realtime process class/default TimeCritical?
- sender timestamps described only as sender-side evidence?

If any answer becomes no, the implementation must cite a newer primary Windows contract and obtain a new architecture decision rather than silently diverging.
