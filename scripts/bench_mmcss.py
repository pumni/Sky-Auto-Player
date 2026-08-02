import argparse
import json
import subprocess
import sys
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description="Sweep through MMCSS policy modes and benchmark them.")
    parser.add_argument("--actions", type=int, default=1024)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--output", type=Path, help="Output markdown report file")
    args = parser.parse_args()

    modes = ["off", "highest", "time_critical", "mmcss", "auto"]
    results = {}

    print(f"Starting MMCSS benchmark sweep ({args.actions} actions, {args.repeats} repeats) over modes: {modes}")

    for mode in modes:
        print(f"\nBenchmarking mode: {mode}...")
        cmd = [
            "uv", "run", "python", "scripts/bench_native_acceptance.py",
            "--actions", str(args.actions),
            "--repeats", str(args.repeats),
            "--rt-priority-mode", mode,
            "--label", f"mmcss_{mode}"
        ]
        
        try:
            res = subprocess.run(cmd, capture_output=True, text=True, check=True)
            report = json.loads(res.stdout)
            results[mode] = report
            print(f"[{mode}] completed successfully.")
        except subprocess.CalledProcessError as e:
            print(f"[{mode}] benchmark failed with exit code {e.returncode}:")
            print(e.stderr)
            results[mode] = None
        except json.JSONDecodeError:
            print(f"[{mode}] benchmark returned invalid JSON")
            results[mode] = None

    if args.output:
        print(f"\nWriting report to {args.output}...")
        with args.output.open("w", encoding="utf-8") as f:
            f.write("# MMCSS Policy Benchmark Report\n\n")
            f.write("| Mode | P50 CPU (us) | P99 CPU (us) | Avg RSS (MB) | Keys Dropped | Outcomes |\n")
            f.write("|------|--------------|--------------|--------------|--------------|----------|\n")
            for mode, rep in results.items():
                if not rep:
                    f.write(f"| {mode} | FAILED | FAILED | FAILED | FAILED | FAILED |\n")
                    continue
                cpu = rep.get("spin_cpu_time_us", {})
                rss = rep.get("peak_rss_bytes", {})
                out = rep.get("outcomes", [])
                
                cpu_p50 = cpu.get("p50", 0)
                cpu_p99 = cpu.get("p99", 0)
                rss_mb = rss.get("p50", 0) / (1024 * 1024)
                drops = rep.get("keys_dropped", 0)
                
                f.write(f"| {mode} | {cpu_p50} | {cpu_p99} | {rss_mb:.2f} | {drops} | {','.join(out)} |\n")
        print("Done!")

    return 0

if __name__ == "__main__":
    sys.exit(main())
