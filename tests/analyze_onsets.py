import sys, csv, statistics

starts = sorted(float(l.split("\t")[0]) for l in open(sys.argv[1], encoding="utf-8") if l.strip())
ioi = [(starts[i+1]-starts[i])*1000 for i in range(len(starts)-1)]   # ms, registered
print(f"[GAME]  onsets={len(starts)}  IOI mean={statistics.mean(ioi):.2f}  "
    f"std={statistics.pstdev(ioi):.2f}  spread={max(ioi)-min(ioi):.2f} ms")

if len(sys.argv) > 2:   # khử jitter phía gửi
    downs = [int(r["actual_us"]) for r in csv.DictReader(open(sys.argv[2], encoding="utf-8"))
            if r["kind"] == "down"]
    sent = [(downs[i+1]-downs[i])/1000 for i in range(len(downs)-1)]
    n = min(len(ioi), len(sent))
    game_only = [ioi[i]-sent[i] for i in range(n)]      # phần jitter THUẦN do game
    print(f"[SENT]  IOI std={statistics.pstdev(sent):.2f} ms")
    print(f"[GAME-only jitter] std={statistics.pstdev(game_only):.2f}  "
        f"spread={max(game_only)-min(game_only):.2f} ms")
    print("residuals(ms):", " ".join(f"{x-statistics.mean(game_only):+.1f}" for x in game_only))