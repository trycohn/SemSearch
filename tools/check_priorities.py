"""Sanity checks: extraction worker priority (extract.worker_init), GPU priority switching, and the
per-sub-batch overhead of throttle's adaptive policy."""
import os
import sys
import time
from concurrent.futures import ProcessPoolExecutor

APP_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, APP_DIR)

import extract  # noqa: E402
import throttle  # noqa: E402


def probe():
    import psutil

    p = psutil.Process()
    return {"pid": p.pid, "nice": str(p.nice()), "ionice": str(p.ionice())}


if __name__ == "__main__":
    with ProcessPoolExecutor(1, initializer=extract.worker_init) as ex:
        print("extraction worker:", ex.submit(probe).result())
    print("this process:", probe())
    throttle.init()
    print("gpu priority after init:", throttle.gpu_priority())
    with throttle.foreground():
        print("inside foreground():", throttle.gpu_priority())
    print("after foreground():", throttle.gpu_priority())
    throttle.force(None)
    n, t = 2000, time.perf_counter()
    for _ in range(n):
        throttle.batch_size(16)
        with throttle.background():
            pass
    print(f"adaptive policy overhead: {(time.perf_counter() - t) / n * 1e6:.1f} us per sub-batch, state={throttle.state()}")
    t = time.perf_counter()
    for _ in range(200):
        throttle.session_locked(), throttle.user_idle_seconds()
    print(f"uncached input/lock check: {(time.perf_counter() - t) / 200 * 1e6:.1f} us")
    time.sleep(2.5)
    print("NVML others %:", throttle.others_util())
