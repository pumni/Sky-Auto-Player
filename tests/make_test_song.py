import json
from pathlib import Path

OUT = Path("songs")
OUT.mkdir(exist_ok=True)

def write(name, notes):
      (OUT / f"{name}.json").write_text(
          json.dumps({"name": name, "songNotes":
              [{"time": t, "key": f"Key{k}"} for t, k in notes]}, indent=2),
          encoding="utf-8")

# --- Song V: visibility — 15 single notes, each key once, 700ms apart (no key repeat) ---
write("TEST_visibility", [(i*700, i) for i in range(15)])

# --- Song R: repeat staircase — 1 key (7), 8 blocks x 8 reps, decreasing intervals ---
def staircase(key=7, reps=8, intervals=(220,180,150,120,100,85,70,55), block_gap=1500):
      notes, t = [], 0
      for interval in intervals:
          for _ in range(reps):
              notes.append((t, key))
              t += interval
          t += block_gap
      return notes
write("TEST_repeat_staircase", staircase())

  # --- Song C (optional): polyphony — chords of 2..6 simultaneous keys, 800ms apart ---
def chords():
      notes, t = [], 0
      for size in (2,3,4,5,6):
          notes.extend((t, k*2) for k in range(size))
          t += 800
      return notes
write("TEST_chords", chords())

def staircase_gap(key=7, reps=10, hold_ms=24,
                    gaps=(50,40,30,24,20,17,14,11,8,5),  # ms, decreasing around 1 frame (16.7)
                    block_gap=2000):
      """Each block: 'reps' presses on the same key; actual_gap = interval - hold_ms.
         Blocks separated by block_gap ms to let the note ring out & ease onset splitting."""
      notes, t = [], 0
      for g in gaps:
          interval = g + hold_ms          # actual_gap = interval - hold = g
          for _ in range(reps):
              notes.append((t, key))
              t += interval
          t += block_gap
      return notes

write("TEST_repeat_gap", staircase_gap())
write(
      "TEST_repeat_gap_30",
      staircase_gap(hold_ms=45, gaps=(70,60,50,45,40,35,30,25,20,15)),
)

# Fine-grained repeat-gap probes. Each variant contains the same gap levels in a
# different order so a result is not confounded with "early vs late in the run".
# Twenty notes per block produce 19 eligible same-key re-trigger transitions.
fine_gap_orders = (
      (0,17,3,14,1,20,5,11,2,8),
      (20,2,11,0,8,17,1,14,5,3),
      (5,0,20,2,14,8,3,17,1,11),
)
for suffix, gaps in zip(("a", "b", "c"), fine_gap_orders, strict=False):
      write(f"TEST_repeat_gap_fine_{suffix}", staircase_gap(reps=20, gaps=gaps))

# Even rhythm, INTERLEAVING 2 different keys -> no same-key repeat pressure,
# so the measured unevenness is PURELY from frame-sampling/lead, not a gap floor.
def metronome_alt(keys=(0, 2), interval_ms=200, count=64):
      return [(i * interval_ms, keys[i % len(keys)]) for i in range(count)]

write("TEST_metro_alt_200", metronome_alt(interval_ms=200))   # 5 notes/s, easy onset separation
write("TEST_metro_alt_120", metronome_alt(interval_ms=120))   # more aggressive

# SAME-key variant as control (hits the gap floor) — used in EXP-4 only if needed
write("TEST_metro_same_200", [(i * 200, 7) for i in range(64)])

# O1: 120 BPM (500 ms) rhythm to match a standard metronome for absolute latency measurement
write("TEST_metro_alt_500", metronome_alt(interval_ms=500, count=40))

# O2/O8: "rolled" chords — keys VERY CLOSE together to probe the chord-merging threshold (chord_merge)
def rolled_chord(keys=(0, 2, 4, 6), spread_ms=18, blocks=8, block_gap=1500):
      notes, t = [], 0
      for _ in range(blocks):
          for i, k in enumerate(keys):
              notes.append((t + i * spread_ms, k))
          t += block_gap
      return notes

write("TEST_rolled_chord_18", rolled_chord(spread_ms=18))

# Floor probe: same-key repeats around the frame-aware min_hold floor. Under the current
# completion-anchor contract, intervals below min_hold are intentionally infeasible; this probe is
# now mainly for synthetic boundary/forensics work. Real-song acceptance should use
# TEST_repeat_clean_* and the corpus gate in tests/acceptance_completion_anchor.py.
#   144fps local_precise: min_hold = ceil(1e6/144) + 500us margin = 7445 us (~7.4 ms)
#   60fps  local_precise: min_hold = ceil(1e6/60)  + 500us margin = 17167 us (~17.2 ms)
# Headroom per 144fps block: 8ms=555us, 9ms=1555us, 10ms=2555us, 12ms=4555us, ...
# (the 7ms block now sits BELOW the margin-included floor and probes the infeasible band).
# READING IT: the 8ms block sits just above the min_hold floor at 144fps. Blocks from 9ms upward
# have increasing headroom and remain useful as stress probes, but production-song gates should
# avoid the synthetic fragile band.
def repeat_floor(key=7, reps=12, intervals=(7, 8, 9, 10, 12, 15, 20), block_gap=1500):
      notes, t = [], 0
      for i in intervals:
          for _ in range(reps):
              notes.append((t, key))
              t += i
          t += block_gap
      return notes

# 84 same-key onsets each; run at the matching --fps so the floor lands where intended.
write("TEST_repeat_floor_144", repeat_floor())
write("TEST_repeat_floor_60", repeat_floor(intervals=(17, 18, 19, 20, 22, 25, 30)))

# Tier-2 GROUND-TRUTH probe: same-key repeats that are BOTH (a) sender-clean — headroom
# (interval - min_hold) far above realistic dispatch jitter so the sender must emit 100% — AND
# (b) above the game's same-key re-trigger wall (Appendix A.4: ~16-17 ms fixed wall at high FPS),
# so the GAME can re-trigger every note. Only then does an audio onset count == intended become a
# valid verdict on the runtime ("did we lose a note in real play?"). Use this for in-game Tier 2;
# use TEST_repeat_floor_* for sender/anchor diagnostics where the game's own wall would confound.
def repeat_clean(key=7, reps=12, intervals=(20, 24, 30, 40, 55, 70), block_gap=1500):
      notes, t = [], 0
      for i in intervals:
          for _ in range(reps):
              notes.append((t, key))
              t += i
          t += block_gap
      return notes

# 72 same-key onsets each.
write("TEST_repeat_clean_144", repeat_clean())
write("TEST_repeat_clean_60", repeat_clean(intervals=(28, 34, 42, 55, 75, 100)))

print("done -> songs/TEST_*.json")
