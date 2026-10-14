"""Quick throughput check of core.Embedder on real chunks (no probe): chunks/s per sub-batch size."""
import os
import sys
import time

APP_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, APP_DIR)
sys.path.insert(0, os.path.join(APP_DIR, "tools"))
os.environ.setdefault("HF_HUB_OFFLINE", "1")

import core  # noqa: E402
import gpu_bench  # noqa: E402
import throttle  # noqa: E402

texts = gpu_bench.load_texts(core.load_config(), 400, 1500, 1)
emb = core.Embedder()
m = emb.model
print("params %.0fM" % (sum(p.numel() for p in m.parameters()) / 1e6), "dtype", next(m.parameters()).dtype,
      "max_seq", getattr(m, "max_seq_length", None), "gpu prio", throttle.gpu_priority())
try:
    tok = m.tokenizer
    lens = sorted(len(tok(t)["input_ids"]) for t in texts[:400])
    print("tokens p50", lens[len(lens) // 2], "p90", lens[int(len(lens) * .9)], "max", lens[-1])
except Exception as e:
    print("tok err", e)
emb.documents(texts[:32])
for bs_mult in (float(x) for x in (sys.argv[1:] or ["1"])):
    throttle.force({"bs": bs_mult, "duty": 1})
    t = time.perf_counter()
    n = 0
    while time.perf_counter() - t < 8:
        emb.documents(texts[n % 1400:n % 1400 + 64], background=True)
        n += 64
    dt = time.perf_counter() - t
    print("bs x%.3g: %.1f chunks/s" % (bs_mult, n / dt), flush=True)
