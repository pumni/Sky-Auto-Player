import json
from pathlib import Path

OUT = Path("songs"); OUT.mkdir(exist_ok=True)

def write(name, notes):
      (OUT / f"{name}.json").write_text(
          json.dumps({"name": name, "songNotes":
              [{"time": t, "key": f"Key{k}"} for t, k in notes]}, indent=2),
          encoding="utf-8")

# --- Bài V: visibility — 15 note đơn, mỗi phím 1 lần, cách 700ms (không lặp phím) ---
write("TEST_visibility", [(i*700, i) for i in range(15)])

# --- Bài R: repeat staircase — 1 phím (Key7), 8 block x 8 lần, interval giảm dần ---
def staircase(key=7, reps=8, intervals=(220,180,150,120,100,85,70,55), block_gap=1500):
      notes, t = [], 0
      for I in intervals:
          for _ in range(reps):
              notes.append((t, key)); t += I
          t += block_gap            # 1.5s im lặng giữa các block
      return notes
write("TEST_repeat_staircase", staircase())

  # --- Bài C (tùy chọn): polyphony — hợp âm 2..6 phím đồng thời, cách 800ms ---
def chords():
      notes, t = [], 0
      for size in (2,3,4,5,6):
          for k in range(size): notes.append((t, k*2))  # các phím rời nhau
          t += 800
      return notes
write("TEST_chords", chords())

def staircase_gap(key=7, reps=10, hold_ms=24,
                    gaps=(50,40,30,24,20,17,14,11,8,5),  # ms, giảm dần quanh 1 frame(16.7)
                    block_gap=2000):
      """Mỗi block 'reps' lần bấm cùng phím; gap_thực = interval - hold_ms.
         Block cách nhau block_gap ms để tiếng ngân tắt hẳn & dễ tách onset."""
      notes, t = [], 0
      for g in gaps:
          interval = g + hold_ms          # gap_thực = interval - hold = g
          for _ in range(reps):
              notes.append((t, key)); t += interval
          t += block_gap
      return notes

write("TEST_repeat_gap", staircase_gap())

print("done -> songs/TEST_*.json")