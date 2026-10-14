"""GPU burst lengths of background sub-batches per hook location (no probe):
    python tools/diag_bursts.py [text|image] [split] [bs-multiplier]"""
import os
import sys
import time
from collections import defaultdict

APP_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, APP_DIR)
sys.path.insert(0, os.path.join(APP_DIR, "tools"))
os.environ.setdefault("HF_HUB_OFFLINE", "1")

import core  # noqa: E402
import gpu_bench  # noqa: E402
import throttle  # noqa: E402

kind = sys.argv[1] if len(sys.argv) > 1 else "image"
split = int(sys.argv[2]) if len(sys.argv) > 2 else 2
bsm = float(sys.argv[3]) if len(sys.argv) > 3 else 0.25
data = gpu_bench.load_images(24, 1) if kind == "image" else gpu_bench.load_texts(core.load_config(), 60, 200, 1)
emb = core.Embedder()
torch = emb.torch
for name, m in emb.model.named_modules():
    if isinstance(m, torch.nn.ModuleList) and len(m) >= 4 and len({type(c) for c in m}) == 1:
        print("layers:", name, len(m), type(m[0]).__name__)
run = (lambda x, **k: emb.images(x, **k)) if kind == "image" else (lambda x, **k: emb.documents(x, **k))
run(data[:8])
throttle.force({"bs": bsm, "duty": 1, "split": split})
throttle.trace = []
t = time.perf_counter()
run(data, background=True)
dt = time.perf_counter() - t
by = defaultdict(list)
for d, n in throttle.trace:
    by[n].append(d * 1e3)
print(f"{kind}: {len(data)} items in {dt:.2f}s ({len(data) / dt:.1f}/s), split={split} bs x{bsm}")
for n, v in sorted(by.items(), key=lambda kv: -max(kv[1])):
    v.sort()
    print(f"  {n:28s} n={len(v):5d} p50={v[len(v) // 2]:7.2f} ms  max={v[-1]:7.2f} ms  sum={sum(v):8.1f} ms")
if os.environ.get("SEQ"):
    print("first sub-batch sequence:", [(n[:14], round(d * 1e3, 2)) for d, n in throttle.trace[:60]])
