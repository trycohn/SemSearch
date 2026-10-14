"""Compact table from gpu_bench.py JSON lines: python tools/bench_table.py results.jsonl [...]"""
import json
import sys

cols = [("scenario", 24, "scenario"), ("mode", 5, "mode"), ("fps", 6, "fps"), ("p99_ms", 7, "p99"),
        ("max_ms", 7, "max"), ("hitch_pct", 6, "hitch%"), ("items_per_s", 6, "it/s"),
        ("subbatch_ms_p50", 7, "sub_ms"), ("burst_ms_avg", 7, "burst"), ("gpu_busy_frac", 5, "busy"),
        ("nv_ours", 5, "ours"), ("nv_others", 5, "oth"), ("throttle_state", 7, "state")]
print(" ".join(h.ljust(w) for _, w, h in cols))
for path in sys.argv[1:]:
    for line in open(path, encoding="utf-8"):
        try:
            j = json.loads(line)
        except ValueError:
            continue
        print(" ".join(str(j.get(k, "-")).ljust(w)[:max(w, len(str(j.get(k, "-"))))] for k, w, _ in cols))
