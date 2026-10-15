"""Данные для панели просмотра: страницы PDF, размеры и дата съёмки фото, строки кода, волна аудио."""
import locale
import logging
import re
from collections import OrderedDict
from datetime import datetime
from threading import Lock

import numpy as np

import winfs

log = logging.getLogger("preview")


class _LRU:
    def __init__(self, size=128):
        self.size, self.data, self.lock = size, OrderedDict(), Lock()

    def get_or(self, key, fn):
        with self.lock:
            if key in self.data:
                self.data.move_to_end(key)
                return self.data[key]
        val = fn()
        with self.lock:
            self.data[key] = val
            while len(self.data) > self.size:
                self.data.popitem(last=False)
        return val


_cache = _LRU(256)


def pdf_pages(path, mtime):
    def calc():
        try:
            from pypdf import PdfReader

            return len(PdfReader(winfs.long_path(path)).pages)
        except Exception:
            return None
    return _cache.get_or(("pages", path, mtime), calc)


def image_info(path, mtime):
    """-> (ширина, высота, дата съёмки epoch|None)."""
    def calc():
        try:
            from PIL import Image

            with Image.open(winfs.long_path(path)) as im:
                w, h = im.size
                exif = im.getexif()
                raw = exif.get_ifd(0x8769).get(36867) or exif.get(306)  # DateTimeOriginal / DateTime
            taken = datetime.strptime(str(raw).strip(), "%Y:%m:%d %H:%M:%S").timestamp() if raw else None
            return w, h, taken
        except Exception:
            return None, None, None
    return _cache.get_or(("img", path, mtime), calc)


def video_info(path, mtime):
    """-> (ширина, высота, длительность)."""
    def calc():
        try:
            import av

            with av.open(winfs.long_path(path)) as c:
                s = c.streams.video[0] if c.streams.video else None
                dur = c.duration / 1e6 if c.duration else None
                return (s.width, s.height, dur) if s else (None, None, dur)
        except Exception:
            return None, None, None
    return _cache.get_or(("vid", path, mtime), calc)


def waveform(path, mtime, bins=44, max_sec=600):
    """44 столбика 0..1 по громкости (первые max_sec секунд)."""
    def calc():
        try:
            import av

            rate, buf, total = 4000, [], 0
            with av.open(winfs.long_path(path)) as c:
                rs = av.AudioResampler(format="flt", layout="mono", rate=rate)
                for fr in c.decode(c.streams.audio[0]):
                    for r in rs.resample(fr):
                        a = r.to_ndarray().reshape(-1)
                        buf.append(a)
                        total += len(a)
                    if total >= max_sec * rate:
                        break
            a = np.concatenate(buf) if buf else np.zeros(bins, np.float32)
            parts = np.array_split(a, bins)
            rms = np.array([float(np.sqrt(np.mean(p * p))) if len(p) else 0.0 for p in parts])
            top = rms.max() or 1.0
            return [round(float(x / top), 3) for x in rms]
        except Exception:
            log.exception("не удалось построить волну для %s", path)
            return None
    return _cache.get_or(("wave", path, mtime), calc)


_ws = re.compile(r"[ \t]+")


def _norm(s):
    return _ws.sub(" ", s).strip()


def code_window(path, snippet, size=8):
    """~8 строк файла вокруг начала фрагмента; hl — строки, с которых фрагмент начинается."""
    try:
        with open(winfs.long_path(path), "rb") as f:
            raw = f.read(2 * 1024 * 1024)
        try:
            text = raw.decode("utf-8-sig")
        except UnicodeDecodeError:
            text = raw.decode(locale.getencoding(), errors="replace")  # кодовая страница ANSI системы
    except OSError:
        return None
    lines = text.splitlines()
    want = [_norm(s) for s in (snippet or "").splitlines() if _norm(s)][:2]
    idx = 0
    if want:
        key = want[0][:80]
        idx = next((i for i, line in enumerate(lines) if key and key in _norm(line)), 0)
    start = max(0, idx - 1)
    window = lines[start:start + size]
    hl = [i for i, line in enumerate(window) if _norm(line) and any(w[:80] in _norm(line) for w in want)]
    return {"lines": [line.expandtabs(4)[:240] for line in window], "hl": hl[:3]}
