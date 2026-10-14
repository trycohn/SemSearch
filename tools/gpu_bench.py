"""GPU load benchmark that mimics the indexer, measured against the frame-pacing probe.

Drives core.Embedder (documents()/images() with background=True, i.e. the real throttled path) on real
chunks/photos taken from files listed in data/index.db (opened read-only), while tools/frameprobe.exe
renders a vsync'd window and reports frame-time stats.

    .venv\\Scripts\\python.exe tools\\gpu_bench.py [scenario ...] [--secs 20] [--modes light,game]
    .venv\\Scripts\\python.exe tools\\gpu_bench.py --list

A scenario fixes the process GPU priority and a throttle profile (see S); a profile name = throttle.PROFILES,
None = the adaptive policy. Keeps the display awake (asleep monitor -> DWM runs on a 64 Hz timer). Output: one JSON line
per (scenario, probe mode) on stdout and appended to --out.
"""
import argparse
import ctypes
import json
import os
import random
import sqlite3
import subprocess
import sys
import threading
import time

APP_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, APP_DIR)
os.environ.setdefault("HF_HUB_OFFLINE", "1")
os.environ.setdefault("HF_HUB_DISABLE_SYMLINKS_WARNING", "1")

import core  # noqa: E402
import extract  # noqa: E402
import throttle  # noqa: E402

PROBE = os.path.join(APP_DIR, "tools", "frameprobe.exe")
MODES = {"light": 0, "game": 2400}  # PS loop iterations: 0 = desktop-like, 2400 ~ 2.3 ms GPU per 720p frame

S = {  # name: (kind, gpu priority, profile); profile: bs = multiplier of the default sub-batch (text 16,
    # images 8; text runs in "tokens" now, derived as bs x 16 chunks x 512 tokens when absent), duty = max GPU
    # duty cycle, split = GPU burst boundary (0 none, 1 layer, 2 attention/MLP block)
    "base": (None, "normal", None),
    "cur_text": ("text", "normal", {"bs": 1, "duty": 1}),
    "cur_img": ("image", "normal", {"bs": 1, "duty": 1}),
    "idle_text": ("text", "idle", {"bs": 1, "duty": 1}),
    "bs4_text": ("text", "normal", {"bs": 0.25, "duty": 1}),
    "d50_bs4_text": ("text", "normal", {"bs": 0.25, "duty": 0.5}),
    "sl1_bs4_text": ("text", "normal", {"bs": 0.25, "duty": 1, "split": 1}),
    "below_sl1_bs4_text": ("text", "below_normal", {"bs": 0.25, "duty": 1, "split": 1}),
    "idle_s1_text": ("text", "idle", {"bs": 1, "duty": 1, "split": 1}),
    "idle_s1_bs8_text": ("text", "idle", {"bs": 0.5, "duty": 1, "split": 1}),
    "idle_s1_bs4_text": ("text", "idle", {"bs": 0.25, "duty": 1, "split": 1}),
    "idle_s1_bs2_text": ("text", "idle", {"bs": 0.125, "duty": 1, "split": 1}),
    "idle_s1_bs8_img": ("image", "idle", {"bs": 1, "duty": 1, "split": 1}),
    "idle_s1_bs4_img": ("image", "idle", {"bs": 0.5, "duty": 1, "split": 1}),
    "idle_s1_bs2_img": ("image", "idle", {"bs": 0.25, "duty": 1, "split": 1}),
    "idle_s2_text": ("text", "idle", {"bs": 1, "duty": 1, "split": 2}),
    "idle_s2_bs8_text": ("text", "idle", {"bs": 0.5, "duty": 1, "split": 2}),
    "idle_s2_bs4_text": ("text", "idle", {"bs": 0.25, "duty": 1, "split": 2}),
    "idle_s2_bs2_text": ("text", "idle", {"bs": 0.125, "duty": 1, "split": 2}),
    "idle_s2_bs8_img": ("image", "idle", {"bs": 1, "duty": 1, "split": 2}),
    "idle_s2_bs4_img": ("image", "idle", {"bs": 0.5, "duty": 1, "split": 2}),
    "idle_s2_bs2_img": ("image", "idle", {"bs": 0.25, "duty": 1, "split": 2}),
    "idle_s1_d50_bs4_text": ("text", "idle", {"bs": 0.25, "duty": 0.5, "split": 1}),
    "idle_s2_d50_bs8_text": ("text", "idle", {"bs": 0.5, "duty": 0.5, "split": 2}),
    "idle_s2_d50_bs2_text": ("text", "idle", {"bs": 0.125, "duty": 0.5, "split": 2}),
    "idle_s2_d50_bs4_text": ("text", "idle", {"bs": 0.25, "duty": 0.5, "split": 2}),
    "idle_s2_bs1_img": ("image", "idle", {"bs": 0.125, "duty": 1, "split": 2}),
    "idle_s2_d50_bs1_img": ("image", "idle", {"bs": 0.125, "duty": 0.5, "split": 2}),
    "idle_s2_d50_bs2_img": ("image", "idle", {"bs": 0.25, "duty": 0.5, "split": 2}),
    "full_text": ("text", "idle", "full"),
    "gentle_text": ("text", "idle", "gentle"),
    "strong_text": ("text", "idle", "strong"),
    "full_img": ("image", "idle", "full"),
    "gentle_img": ("image", "idle", "gentle"),
    "strong_img": ("image", "idle", "strong"),
    "adaptive_text": ("text", "idle", None),
}


def ro_db():
    uri = "file:" + core.DB_PATH.replace("\\", "/")
    try:
        c = sqlite3.connect(uri + "?mode=ro", uri=True)
        c.execute("SELECT 1 FROM files LIMIT 1")
        return c
    except sqlite3.Error:
        return sqlite3.connect(uri + "?immutable=1", uri=True)


def load_texts(cfg, n_files, max_chunks, seed):
    rows = ro_db().execute("SELECT path FROM files WHERE kind IN ('text','code') AND status=1").fetchall()
    random.Random(seed).shuffle(rows)
    out = []
    for (path,) in rows:
        if len(out) >= max_chunks or n_files <= 0:
            break
        try:
            items = extract.extract_text(path, cfg)
        except Exception:
            continue
        out += [it[3] for it in items]
        n_files -= 1
    return out[:max_chunks]


def load_images(n, seed):
    rows = ro_db().execute("SELECT path FROM files WHERE kind='image' AND status<2 AND size>=20480").fetchall()
    random.Random(seed).shuffle(rows)
    out = []
    for (path,) in rows:
        if len(out) >= n:
            break
        try:
            out.append(extract.load_image(path))
        except Exception:
            continue
    return out


class Load(threading.Thread):
    """Calls Embedder like Indexer._encode does: packs of payloads, background=True."""

    def __init__(self, emb, kind, data, pack):
        super().__init__(daemon=True)
        self.emb, self.kind, self.data, self.pack = emb, kind, data, pack
        self.stop = False

    def run(self):
        i = 0
        while not self.stop:
            batch = [self.data[(i + j) % len(self.data)] for j in range(self.pack)]
            i += self.pack
            if self.kind == "text":
                self.emb.documents(batch, background=True)
            else:
                self.emb.images(batch, background=True)


def dwm_cadence_ms(sec=0.3):
    d = ctypes.windll.dwmapi
    ts, end = [], time.perf_counter() + sec
    while time.perf_counter() < end:
        d.DwmFlush()
        ts.append(time.perf_counter())
    iv = sorted((b - a) * 1e3 for a, b in zip(ts, ts[1:]))
    return iv[len(iv) // 2] if iv else -1.0


def run_probe(iters, secs, nv, counter):
    cad = dwm_cadence_ms()
    n0, t0, st0 = counter[0], time.perf_counter(), dict(throttle.stats)
    durs = counter[1]
    d0 = len(durs)
    p = subprocess.Popen([PROBE, "--secs", str(secs), "--iters", str(iters)], stdout=subprocess.PIPE, text=True)
    others, ours = [], []
    while p.poll() is None:
        time.sleep(1.0)
        if nv is not None and time.perf_counter() - t0 > 2.5:
            try:
                o, m, _ = nv.usage(1.0)
                others.append(o)
                ours.append(m)
            except Exception:
                pass
    out = p.stdout.read().strip().splitlines()
    dt = time.perf_counter() - t0
    res = json.loads(out[-1]) if out else {"error": "no probe output"}
    res["items_per_s"] = round((counter[0] - n0) / dt, 1)
    res["dwm_cadence_ms"] = round(cad, 2)
    w = sorted(durs[d0:])
    if w:
        res["subbatch_ms_p50"] = round(w[len(w) // 2] * 1e3, 1)
        res["subbatch_ms_max"] = round(w[-1] * 1e3, 1)
    nb = throttle.stats["bursts"] - st0["bursts"]
    if nb:
        res["burst_ms_avg"] = round((throttle.stats["busy"] - st0["busy"]) / nb * 1e3, 2)
        res["gpu_busy_frac"] = round((throttle.stats["busy"] - st0["busy"]) / dt, 3)
    if others:
        res["nv_others"] = round(sum(others) / len(others), 1)
        res["nv_ours"] = round(sum(ours) / len(ours), 1)
    return res


def query_test(emb, texts, secs=40):
    """Search latency while the game-like probe runs: boosted (default), not boosted, and with a gentle
    indexing load competing for the model lock."""
    p = subprocess.Popen([PROBE, "--secs", str(secs), "--iters", str(MODES["game"])], stdout=subprocess.PIPE)
    time.sleep(3)
    qs = ["отчёт о продажах за квартал", "how to configure the server", "фотографии с моря", "invoice 2023"]

    def lat(n=16):
        out = []
        for i in range(n):
            t = time.perf_counter()
            emb.query(qs[i % len(qs)])
            out.append((time.perf_counter() - t) * 1e3)
            time.sleep(0.25)
        out.sort()
        return {"p50_ms": round(out[len(out) // 2], 1), "max_ms": round(out[-1], 1)}

    res = {"boost": lat()}
    base, throttle._gpu_base = throttle._gpu_base, None  # foreground() becomes a no-op
    res["no_boost_idle_prio"] = lat()
    throttle._gpu_base = base
    throttle.force("gentle")
    load = Load(emb, "text", texts, 32)
    load.start()
    time.sleep(2)
    res["boost_with_gentle_indexing"] = lat()
    load.stop = True
    load.join()
    throttle.force(None)
    p.wait()
    print(json.dumps({"query_latency_under_game_probe": res}), flush=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("scenarios", nargs="*", default=["base", "cur_text", "full_text", "gentle_text", "strong_text", "adaptive_text"])
    ap.add_argument("--secs", type=float, default=20)
    ap.add_argument("--modes", default="light,game")
    ap.add_argument("--pack", type=int, default=32, help="payloads per Embedder call (indexer: all chunks of 32 files)")
    ap.add_argument("--out", default=os.path.join(APP_DIR, "tools", "bench_results.jsonl"))
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--query-test", action="store_true", help="search latency under the game-like probe")
    a = ap.parse_args()
    if a.list:
        for k, v in S.items():
            print(k, v)
        return
    k32 = ctypes.windll.kernel32
    k32.SetThreadExecutionState(0x80000000 | 0x2 | 0x1)  # keep the display on: asleep -> DWM runs at 64 Hz
    cfg = core.load_config()
    kinds = {S[s][0] for s in a.scenarios} | ({"text"} if a.query_test else set())
    t = time.perf_counter()
    texts = load_texts(cfg, 400, 1500, 1) if "text" in kinds else []
    imgs = load_images(96, 1) if "image" in kinds else []
    print(f"# data: {len(texts)} chunks (avg {sum(map(len, texts)) / max(1, len(texts)):.0f} chars), "
          f"{len(imgs)} images in {time.perf_counter() - t:.0f}s", file=sys.stderr)
    emb = core.Embedder()
    hooked = sum(len(m._forward_hooks) for m in emb.model.modules())  # layers + attention/MLP blocks
    print(f"# hooked layers: {hooked}, gpu priority: {throttle.gpu_priority()}", file=sys.stderr)
    core.lower_priority()  # as Indexer.run does
    counter = [0, []]
    fwd = emb._forward

    def counting_forward(payload):  # one sub-batch: text features or a list of images
        t = time.perf_counter()
        r = fwd(payload)
        counter[1].append(time.perf_counter() - t)
        counter[0] += len(r)
        return r

    emb._forward = counting_forward
    if texts:
        emb.documents(texts[:64])
    if imgs:
        emb.images(imgs[:16])
    try:
        nv = throttle.Nvml()
    except Exception:
        nv = None
    if a.query_test:
        query_test(emb, texts)
        return
    modes = [m for m in a.modes.split(",") if m]
    for name in a.scenarios:
        kind, prio, prof = S[name]
        if isinstance(prof, dict) and "tokens" not in prof:
            prof = {**prof, "tokens": int(prof["bs"] * 16 * 512)}
        throttle.set_gpu_priority(prio)
        throttle.force(prof)
        load = None
        if kind:
            load = Load(emb, kind, texts if kind == "text" else imgs, a.pack if kind == "text" else 16)
            load.start()
            time.sleep(3)
        for m in modes:
            res = {"scenario": name, "mode": m, "gpu_prio": throttle.gpu_priority(), "profile": prof,
                   **run_probe(MODES[m], a.secs, nv, counter), "throttle_state": throttle.state()}
            line = json.dumps(res)
            print(line, flush=True)
            with open(a.out, "a", encoding="utf-8") as f:
                f.write(line + "\n")
        if load:
            load.stop = True
            load.join()
    throttle.force(None)


if __name__ == "__main__":
    main()
