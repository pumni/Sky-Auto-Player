# Prepare Whole-Stream Timing Advisory

The whole-stream timing advisory is a bounded, read-only estimate computed
during offline plan preparation. It describes how an authored song schedule
interacts with configured $H$ and $G$ timing floors under an illustrative
sender-cost model.

> [!NOTE]
> The advisory is an offline UI feature consumed by the desktop application
> (`apps/desktop/src/components/player/TimingAdvisory.tsx`). It does **not** govern
> or modify realtime playback dispatch. For the normative realtime playback
> architecture and physical execution, see [Playback Engine](playback-engine.md).

## Purpose and Consumers

When a song is prepared for playback, native Prepare computes an advisory for
each playback variant (Proceed, Use Recommended, Dry Run) before publishing:

```text
authored Song + variant timing policy
  -> ScheduleMetadata
  -> canonical schedule compiler
  -> ordered 15-slot model packet stream
  -> bounded sky_player simulator
  -> compact WholeStreamTimingAdvisoryDto
  -> desktop UI renders timing projection
```

The advisory:
- Estimates potential head-of-line (HOL) pressure or floor displacement under
  explicit illustrative sender assumptions.
- Informs users whether their configured tempo, FPS, and hold parameters may
  approach physical frame boundaries.
- **Never** blocks playback admission, modifies authored note timing, or alters
  the realtime dispatch loop.

## Timing Floor Modeling

The model applies the active timing policy:

$$T = \lceil 1{,}000{,}000 / \text{FPS} \rceil\ \mu\text{s}$$
$$H = \lceil \text{hold\_frames} \times T \rceil + \text{timing\_margin\_us}\ \mu\text{s}$$
$$\text{physical } G = T\ \mu\text{s}$$
$$\text{structural same-key cycle floor} = H + G$$

The simulator applies an illustrative sender cost profile (10 µs setup, 3 µs per input,
5 µs post-send) to detect modeled pressure. These numbers are illustrative diagnostics,
not measured hardware promises.

## Boundaries and Fail-Closed Guarantees

- **Bounded Computation:** Capped at 50,000 actions and 64 MiB working memory.
  If bounds are exceeded, the advisory safely returns `unavailable`.
- **Zero Realtime Impact:** Computed entirely off the realtime thread. The realtime
  worker and Win32 `SendInput` paths never call this simulator.
- **Unacknowledged by Game:** The advisory reports `receiver_acknowledged: false`.
  Game input processing and frame scheduling are outside player control.
