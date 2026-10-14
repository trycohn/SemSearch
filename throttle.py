"""Throttling of background (indexing) GPU/CPU work so the desktop, video and windowed games stay smooth.

Why (measured with tools/frameprobe.exe + tools/gpu_bench.py, RTX 4070 SUPER, 240 Hz, HAGS off):
with HAGS off WDDM can only switch the GPU between processes at the end of a submitted chunk of
work, so a 16-chunk sub-batch (~0.3 s of uninterrupted CUDA work) makes every frame of every
other app wait. Lower GPU priority alone does not help (worse, in fact) — the bursts must be short.

What is applied
- GPU priority: the process gets WDDM priority IDLE (gdi32!D3DKMTSetProcessSchedulingPriorityClass) in
  init(), called by core.Embedder before CUDA starts. Search calls run inside foreground(), which
  restores NORMAL for their duration, so a query never queues behind a game.
- Short GPU bursts: attach() puts forward hooks on the transformer layers and their attention/MLP
  blocks. Inside background() (= one indexer sub-batch) the hooks call cuda.synchronize() after every
  layer ("split": 1) or block ("split": 2), so the GPU never holds more than ~2-5 ms of our work and
  graphics gets the GPU between bursts; with "duty" < 1 they also sleep after each burst so that
  busy / (busy + pause) <= duty. Outside background() the hooks cost a thread-local lookup.
  The text tower runs as one CUDA graph per layer (cudagraphs.py): replays fire no hooks, so it calls
  layer_done() after every layer instead, and text sub-batches are sized in tokens ("tokens") — a layer
  of N tokens is one burst whatever the number of texts.
- Profile, re-evaluated before every sub-batch (input/lock checks cached 0.25 s; NVML runs in a 1 Hz
  daemon thread):
    full   - no keyboard/mouse input for IDLE_AFTER s, or the session is locked: maximum throughput;
    gentle - user at the PC (or another process is decoding video);
    strong - other processes keep the GPU busy (NVML per-process SM utilization >= OTHERS_STRONG,
             e.g. a windowed game).
- CPU: background_process() / background_thread() -> IDLE priority + Windows background mode
  (very low I/O and memory priority) for extraction workers / threads.

Every OS/NVML call is wrapped: on failure the module falls back to "gentle" and never raises.
"""
import ctypes
import os
import threading
import time
from contextlib import contextmanager
from ctypes import wintypes

# ------------------------------------------------------------------ tunables

IDLE_AFTER = 60.0     # s without input -> "full"
OTHERS_STRONG = 45.0  # % SM utilization of all other processes (DWM included) -> "strong"
OTHERS_CALM = 35.0    # "strong" ends after this stays below for HOLD s
HOLD = 5.0
# bs: multiplier of the caller's sub-batch (images 8, audio 4); tokens: text sub-batch budget, texts x padded
# length (graph path: one layer ~0.4 ms per 1000 tokens on an RTX 4070 SUPER); duty: max GPU duty cycle;
# split: burst boundary, 1 = transformer layer, 2 = attention / MLP block (graph path: always a layer)
PROFILES = {
    "full": {"bs": 0.5, "tokens": 6144, "duty": 1.0, "split": 1},
    "gentle": {"bs": 0.25, "tokens": 2048, "duty": 1.0, "split": 2},
    "strong": {"bs": 0.125, "tokens": 1024, "duty": 0.5, "split": 2},
}
GPU_PRIORITY = "idle"

_PRIO = {"idle": 0, "below_normal": 1, "normal": 2, "above_normal": 3, "high": 4}

_lock = threading.Lock()
_tls = threading.local()
_torch = None
_forced = None        # benchmarks: profile name or dict
_state = "gentle"
_others = 0.0         # % SM used by other processes (last NVML window)
_strong_until = 0.0   # monotonic deadlines set by the monitor thread
_video_until = 0.0
_cls = ("gentle", 0.0)  # cached classification, monotonic time
_last_end = 0.0       # end of the previous background sub-batch (perf_counter)
_last_gap = 0.0       # pause still owed after it
_gpu_base = None      # priority set by init()
_monitor = None
trace = None          # benchmarks: a list -> (seconds, module class) of every GPU burst
stats = {"busy": 0.0, "bursts": 0}  # benchmarks: total background GPU-burst time / count

try:
    _k32 = ctypes.WinDLL("kernel32")
    _u32 = ctypes.WinDLL("user32")
    _gdi = ctypes.WinDLL("gdi32")
    _k32.GetCurrentProcess.restype = wintypes.HANDLE
    _k32.GetCurrentThread.restype = wintypes.HANDLE
    _k32.GetTickCount.restype = wintypes.DWORD
    _k32.SetPriorityClass.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    _k32.SetThreadPriority.argtypes = [wintypes.HANDLE, ctypes.c_int]
    _gdi.D3DKMTSetProcessSchedulingPriorityClass.argtypes = [wintypes.HANDLE, ctypes.c_int]
    _gdi.D3DKMTSetProcessSchedulingPriorityClass.restype = ctypes.c_long
    _gdi.D3DKMTGetProcessSchedulingPriorityClass.argtypes = [wintypes.HANDLE, ctypes.POINTER(ctypes.c_int)]
    _gdi.D3DKMTGetProcessSchedulingPriorityClass.restype = ctypes.c_long
    _u32.OpenInputDesktop.restype = wintypes.HANDLE
    _u32.OpenInputDesktop.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    _u32.GetUserObjectInformationW.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD,
                                              ctypes.POINTER(wintypes.DWORD)]
    _u32.CloseDesktop.argtypes = [wintypes.HANDLE]
except Exception:  # not Windows
    _k32 = _u32 = _gdi = None


class _LASTINPUTINFO(ctypes.Structure):
    _fields_ = [("cbSize", wintypes.UINT), ("dwTime", wintypes.DWORD)]


# ------------------------------------------------------------------ process / thread priorities

def set_gpu_priority(name):
    """WDDM scheduling priority class of this process: 'idle' | 'below_normal' | 'normal' | ... -> bool"""
    try:
        return _gdi.D3DKMTSetProcessSchedulingPriorityClass(_k32.GetCurrentProcess(), _PRIO[name]) == 0
    except Exception:
        return False


def gpu_priority():
    try:
        v = ctypes.c_int(-1)
        if _gdi.D3DKMTGetProcessSchedulingPriorityClass(_k32.GetCurrentProcess(), ctypes.byref(v)) == 0:
            return {p: n for n, p in _PRIO.items()}.get(v.value, str(v.value))
    except Exception:
        pass
    return None


def init():
    """Once per process, before CUDA starts (core.Embedder.__init__)."""
    global _gpu_base
    if _gpu_base is None and set_gpu_priority(GPU_PRIORITY):
        _gpu_base = GPU_PRIORITY


@contextmanager
def foreground():
    """Interactive GPU work (search): NORMAL GPU priority for its duration, no throttling."""
    base = _gpu_base
    if base and base != "normal":
        set_gpu_priority("normal")
    try:
        yield
    finally:
        if base and base != "normal":
            set_gpu_priority(base)


def background_process():
    """Current process -> IDLE CPU priority + background mode (very low I/O and memory priority).
    For worker subprocesses (extraction pool initializer, name scan). Never raises."""
    try:
        h = _k32.GetCurrentProcess()
        _k32.SetPriorityClass(h, 0x00000040)  # IDLE_PRIORITY_CLASS
        _k32.SetPriorityClass(h, 0x00100000)  # PROCESS_MODE_BACKGROUND_BEGIN
    except Exception:
        pass


def background_thread():
    """Current thread -> background mode (lowest CPU, I/O and memory priority).
    Usable as a ThreadPoolExecutor initializer. Never raises."""
    try:
        _k32.SetThreadPriority(_k32.GetCurrentThread(), 0x00010000)  # THREAD_MODE_BACKGROUND_BEGIN
    except Exception:
        pass


# ------------------------------------------------------------------ user / GPU activity

def user_idle_seconds():
    try:
        li = _LASTINPUTINFO(ctypes.sizeof(_LASTINPUTINFO), 0)
        if _u32.GetLastInputInfo(ctypes.byref(li)):
            return ((_k32.GetTickCount() - li.dwTime) & 0xFFFFFFFF) / 1000.0
    except Exception:
        pass
    return 0.0


def session_locked():
    """Locked workstation / secure desktop: the input desktop is not 'Default'."""
    try:
        h = _u32.OpenInputDesktop(0, False, 0x0100)  # DESKTOP_SWITCHDESKTOP
        if not h:
            return True
        try:
            buf = ctypes.create_unicode_buffer(64)
            ok = _u32.GetUserObjectInformationW(h, 2, buf, ctypes.sizeof(buf), None)  # UOI_NAME
            return bool(ok) and buf.value.lower() != "default"
        finally:
            _u32.CloseDesktop(h)
    except Exception:
        return False


class Nvml:
    """Per-process GPU utilization from NVML (nvidia-ml-py). Constructor raises if unavailable."""

    def __init__(self):
        import pynvml

        pynvml.nvmlInit()
        self.n = pynvml
        self.h = pynvml.nvmlDeviceGetHandleByIndex(int(os.environ.get("SEMSEARCH_GPU", "0")))
        self.pid = os.getpid()

    def usage(self, window=2.0):
        """-> (others SM %, ours SM %, others video-decoder %) averaged over the last `window` s."""
        since = int((time.time() - window) * 1e6)
        try:
            samples = self.n.nvmlDeviceGetProcessUtilization(self.h, since)
        except self.n.NVMLError_NotFound:
            return 0.0, 0.0, 0.0
        sm, dec = {}, {}
        for s in samples:
            if s.timeStamp >= since:
                sm.setdefault(s.pid, []).append(s.smUtil)
                dec.setdefault(s.pid, []).append(s.decUtil)
        mine = sm.pop(self.pid, [])
        dec.pop(self.pid, None)
        avg = lambda d: sum(sum(v) / len(v) for v in d.values())  # noqa: E731
        return float(min(100.0, avg(sm))), (sum(mine) / len(mine) if mine else 0.0), float(avg(dec))


def _monitor_loop():
    global _others, _strong_until, _video_until
    try:
        nv = Nvml()
    except Exception:
        return  # no NVML: never "strong", video rule off
    while True:
        try:
            others, _, dec = nv.usage()
            _others = others
            now = time.monotonic()
            if others >= OTHERS_STRONG or (others >= OTHERS_CALM and _strong_until > now):
                _strong_until = now + HOLD
            if dec >= 1.0:
                _video_until = now + HOLD
        except Exception:
            pass
        time.sleep(1.0)


def _classify():
    global _cls
    now = time.monotonic()
    if now - _cls[1] < 0.25:
        return _cls[0]
    if now < _strong_until:
        s = "strong"
    elif now >= _video_until and (user_idle_seconds() >= IDLE_AFTER or session_locked()):
        s = "full"
    else:
        s = "gentle"
    _cls = (s, now)
    return s


def _profile():
    global _state, _monitor
    f = _forced
    if isinstance(f, dict):
        _state = f.get("name", "forced")
        return f
    if _monitor is None:
        with _lock:
            if _monitor is None:
                _monitor = threading.Thread(target=_monitor_loop, name="throttle-monitor", daemon=True)
                _monitor.start()
    _state = f or _classify()
    return PROFILES[_state]


# ------------------------------------------------------------------ hooks used by core.Embedder

def attach(model, torch):
    """Hooks on every transformer layer (text, vision, audio towers) and its >=1M-parameter blocks.
    -> number of hooked layers (0 if the model has no recognisable layer stacks)."""
    global _torch
    _torch = torch
    n = 0
    try:
        # the first burst starts when inputs reach the model, not when CPU preprocessing begins
        next(iter(model.children()), model).register_forward_pre_hook(_start_hook)
        for m in model.modules():
            if isinstance(m, torch.nn.ModuleList) and len(m) >= 4 and len({type(c) for c in m}) == 1:
                for layer in m:
                    layer.register_forward_hook(_layer_hook)
                    n += 1
                    for child in layer.children():
                        if sum(p.numel() for p in child.parameters()) >= 1_000_000:
                            child.register_forward_hook(_block_hook)
    except Exception:
        pass
    return n


def _start_hook(module, args):
    st = getattr(_tls, "bg", None)
    if st is not None and st["split"]:
        try:
            _torch.cuda.synchronize()
        except Exception:
            pass
        st["t"] = time.perf_counter()


def _layer_hook(module, args, output):
    st = getattr(_tls, "bg", None)
    if st is not None and st["split"] == 1:
        _sync_point(st, type(module).__name__)


def _block_hook(module, args, output):
    st = getattr(_tls, "bg", None)
    if st is not None and st["split"] == 2:
        _sync_point(st, type(module).__name__)


def layer_done():
    """CUDA-graph path: a replayed layer fires no hooks, so cudagraphs.py calls this after each one."""
    st = getattr(_tls, "bg", None)
    if st is not None and st["split"]:
        _sync_point(st, "graph")


@contextmanager
def suspended():
    """No sync points for the duration (cudagraphs.py: synchronize is illegal while a graph is captured)."""
    st = getattr(_tls, "bg", None)
    _tls.bg = None
    try:
        yield
    finally:
        _tls.bg = st


def _sync_point(st, name):
    try:
        _torch.cuda.synchronize()
        now = time.perf_counter()
        busy = now - st["t"]
        st["busy"] += busy
        st["bursts"] += 1
        if trace is not None:
            trace.append((busy, name))
        if st["duty"] < 1.0:
            time.sleep(busy * (1.0 - st["duty"]) / st["duty"])
        st["t"] = time.perf_counter()
    except Exception:
        st["split"] = 0


def _owed_pause(p):
    if p["duty"] < 1.0:
        gap = _last_gap - (time.perf_counter() - _last_end)
        if gap > 0:
            time.sleep(min(gap, 2.0))


def batch_size(default):
    """Before a background sub-batch (outside the model lock): waits out the pause still owed by the
    previous sub-batch and returns the sub-batch size for the current profile."""
    try:
        p = _profile()
        _owed_pause(p)
        return max(1, int(round(default * p["bs"])))
    except Exception:
        return default


def tokens(default):
    """batch_size() for text: the sub-batch budget in tokens (texts x padded length)."""
    try:
        p = _profile()
        _owed_pause(p)
        return int(p.get("tokens", default))
    except Exception:
        return default


@contextmanager
def background():
    """Wraps one background sub-batch (inside the model lock): enables the burst hooks."""
    global _last_end, _last_gap
    try:
        p = _profile()
        st = {"split": int(p.get("split", 0)), "duty": float(p["duty"]), "busy": 0.0, "bursts": 0,
              "t": time.perf_counter()}
    except Exception:
        st = {"split": 0, "duty": 1.0, "busy": 0.0, "bursts": 0, "t": time.perf_counter()}
    _tls.bg = st
    try:
        yield
    finally:
        _tls.bg = None
        now = time.perf_counter()
        tail = now - st["t"]  # encode() returned numpy, so the GPU is done
        if trace is not None:
            trace.append((tail, "tail"))
        _last_gap = tail * (1.0 - st["duty"]) / st["duty"] if st["duty"] < 1.0 else 0.0
        _last_end = now
        stats["busy"] += st["busy"] + tail
        stats["bursts"] += st["bursts"] + 1


def force(profile=None):
    """Benchmarks: pin a profile name or a dict {"bs", "tokens", "duty", "split"}; None -> adaptive."""
    global _forced
    _forced = profile


def state():
    """Short status for /api/status: 'full' | 'gentle' | 'strong'."""
    return _state


def others_util():
    """% SM utilization of other processes in the last NVML window (0 without NVML)."""
    return _others
