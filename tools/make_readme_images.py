"""Builds docs/images/* — the pictures of README.md and README.en.md: the demo clip at the top, the search-speed chart,
the settings screenshot and the "how it works" diagram, each in Russian and English.

The clip and the screenshot are of the real ui/ pages served by mock_api.py, so they show the mockup's demo files,
never yours (mock_api.py has the same files in English for /dev/lang/en). They are taken in a headless Edge (or Chrome)
over the DevTools protocol and put on a backdrop by tools/readme/frame.html. The clip is the main window typing a few
queries, recorded frame by frame and saved as an animated WebP. The diagram is tools/readme/architecture.html, the
chart is tools/readme/speed.html with the numbers tools/bench_search.py measured (tools/readme/speed.js).
Both themes of everything are written, the READMEs pick one with <picture> and prefers-color-scheme.

Run: .venv\\Scripts\\python.exe tools\\make_readme_images.py [clip settings speed architecture]
(no names: all of them). mock_api.py is started here when nothing answers on 127.0.0.1:8799 (an already running one is reused — its
theme and language get changed). The browser is looked up in the usual places; SEMSEARCH_BROWSER overrides it.
Pillow (in requirements.txt) composes the clip. Rerun it after the interface changes; nothing in the app depends on
these files.
"""
import base64
import io
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "docs" / "images"
TEMPLATES = Path(__file__).resolve().parent / "readme"
MOCK = "http://127.0.0.1:8799"
CDP_PORT = 9341
THEMES = ("light", "dark")
LANGS = ("ru", "en")

# name, page, window size (the shell's own: main 1100x760, settings 880x900), query per language, tab, scroll target,
# backdrop padding and pixel ratio. The settings screenshot sits in the "gives the GPU back" section.
SHOTS = [
    ("settings", "/settings#caps", (1100, 720), None, None, "s-games", 36, 1.0),
]
# The clip at the top of the READMEs: a scene per query — what is typed, the tab opened after it, how long the result
# stays (s). Queries are the start screen's examples (ui/i18n.js start.ex*) and the mockup's "смета" / "estimate".
# It starts and ends on the start screen, so the loop is seamless.
CLIP = {
    "ru": [("договор про крышу", "all", 3.0), ("кот на балконе", "image", 2.6), ("где парсил JSON с ретраями", "all", 2.4),
           ("смета", "name", 2.6)],
    "en": [("roof repair contract", "all", 3.0), ("cat on the balcony", "image", 2.6), ("where I parsed JSON with retries", "all", 2.4),
           ("estimate", "name", 2.6)],
}
CLIP_SIZE, CLIP_PAD, CLIP_DPR = (1100, 720), 36, 1.0
BLANK = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7"  # transparent 1x1
# Runs in the page: types each query letter by letter, as a person would, and opens the tab.
CLIP_JS = """(async (scenes) => {
  const sleep = (ms) => new Promise((ok) => setTimeout(ok, ms));
  const q = document.getElementById('q');
  const tab = (t) => document.querySelector('[data-tab=' + t + ']').click();
  const set = (v) => { q.value = v; q.dispatchEvent(new Event('input', {bubbles: true})); };
  q.focus();
  await sleep(900);
  for (const [text, t, hold] of scenes) {
    if (!document.getElementById('tabs').hidden) tab('all');
    for (let i = 1; i <= text.length; i++) { set(text.slice(0, i)); await sleep(50 + (i * 37) % 45); }
    await sleep(650);
    if (t !== 'all') tab(t);
    await sleep(hold * 1000);
  }
  set('');
  await sleep(900);
  window.clipDone = true;
})"""


class CDP:
    """The few DevTools calls needed here, over a bare WebSocket (the venv has no websocket package)."""

    def __init__(self, ws_url):
        hostport, path = ws_url.split("://", 1)[1].split("/", 1)
        host, port = hostport.split(":")
        self.sock = socket.create_connection((host, int(port)), timeout=60)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall((f"GET /{path} HTTP/1.1\r\nHost: {hostport}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                           f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n").encode())
        self.buf = bytearray()
        while b"\r\n\r\n" not in self.buf:
            self._fill()
        head, _, rest = bytes(self.buf).partition(b"\r\n\r\n")
        if b" 101 " not in head.split(b"\r\n", 1)[0]:
            raise RuntimeError("DevTools refused the WebSocket: " + head.decode(errors="replace"))
        self.buf = bytearray(rest)
        self.seq = 0

    def _fill(self):
        chunk = self.sock.recv(1 << 20)
        if not chunk:
            raise ConnectionError("DevTools closed the connection")
        self.buf += chunk

    def _take(self, n):
        while len(self.buf) < n:
            self._fill()
        out = bytes(self.buf[:n])
        del self.buf[:n]
        return out

    def _message(self):
        parts = []
        while True:
            b1, b2 = self._take(2)
            n = b2 & 0x7F
            if n == 126:
                n = struct.unpack(">H", self._take(2))[0]
            elif n == 127:
                n = struct.unpack(">Q", self._take(8))[0]
            if b2 & 0x80:
                self._take(4)  # a server never masks, but skip a mask anyway
            data = self._take(n)
            op = b1 & 0x0F
            if op == 8:
                raise ConnectionError("DevTools closed the connection")
            if op in (9, 10):  # ping / pong
                continue
            parts.append(data)
            if b1 & 0x80:
                return b"".join(parts)

    def __call__(self, method, **params):
        self.seq += 1
        payload = json.dumps({"id": self.seq, "method": method, "params": params}).encode()
        n = len(payload)
        head = bytes([0x81]) + (bytes([0x80 | n]) if n < 126 else
                                bytes([0x80 | 126]) + struct.pack(">H", n) if n < 65536 else
                                bytes([0x80 | 127]) + struct.pack(">Q", n))
        mask = os.urandom(4)
        self.sock.sendall(head + mask + bytes(b ^ mask[i & 3] for i, b in enumerate(payload)))
        while True:  # events in between are not needed
            msg = json.loads(self._message())
            if msg.get("id") == self.seq:
                if "error" in msg:
                    raise RuntimeError(f"{method}: {msg['error']}")
                return msg.get("result", {})

    def js(self, expr):
        r = self("Runtime.evaluate", expression=expr, awaitPromise=True, returnByValue=True)
        if "exceptionDetails" in r:
            raise RuntimeError(f"JS failed: {expr[:80]}… {r['exceptionDetails']}")
        return r.get("result", {}).get("value")

    def open(self, url, w, h, dpr, theme):
        self("Emulation.setDeviceMetricsOverride", width=w, height=h, deviceScaleFactor=dpr, mobile=False)
        self("Emulation.setEmulatedMedia", features=[{"name": "prefers-color-scheme", "value": theme}])
        self("Page.navigate", url=url)
        time.sleep(0.3)
        for _ in range(100):
            if self.js("document.readyState") == "complete":
                return
            time.sleep(0.1)
        raise RuntimeError("page did not load: " + url)

    def png(self, path):
        data = self("Page.captureScreenshot", format="png")["data"]
        Path(path).write_bytes(base64.b64decode(data))


def get(url, timeout=2):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return r.read()


def mock_up():
    try:
        get(MOCK + "/ui/icon.svg", 1)
        return True
    except OSError:
        return False


def find_browser():
    env = os.environ
    for p in (env.get("SEMSEARCH_BROWSER"),
              os.path.join(env.get("ProgramFiles(x86)", ""), r"Microsoft\Edge\Application\msedge.exe"),
              os.path.join(env.get("ProgramFiles", ""), r"Microsoft\Edge\Application\msedge.exe"),
              os.path.join(env.get("ProgramFiles", ""), r"Google\Chrome\Application\chrome.exe"),
              os.path.join(env.get("LOCALAPPDATA", ""), r"Google\Chrome\Application\chrome.exe")):
        if p and os.path.isfile(p):
            return p
    sys.exit("Edge or Chrome not found — set SEMSEARCH_BROWSER to the browser's exe")


def open_app(cdp, page, size, dpr, theme):
    """Opens a page of the app with the theme and language that mock_api.py has now: the pages start with the ones
    they cached in localStorage and switch only on the next settings poll, so the cache goes first."""
    get(f"{MOCK}/dev/theme/{theme}")
    cdp.open(MOCK + page, *size, dpr, theme)
    cdp.js("localStorage.clear()")
    cdp("Page.reload")  # not a second navigate: to the same URL with #caps that only scrolls
    time.sleep(0.3)
    while cdp.js("document.readyState") != "complete":
        time.sleep(0.1)


def take_window(cdp, page, size, query, tab, scroll, dpr, theme, path):
    open_app(cdp, page, size, dpr, theme)
    time.sleep(1.2)  # launch animation, first status poll
    if query:
        cdp.js(f"const q = document.getElementById('q'); q.value = {json.dumps(query)};"
               "q.dispatchEvent(new Event('input', {bubbles: true}))")
        time.sleep(1.2)
    if tab:
        cdp.js(f"document.querySelector('[data-tab={tab}]').click()")
        time.sleep(0.4)
    if scroll:
        cdp.js(f"document.getElementById({json.dumps(scroll)}).scrollIntoView({{block: 'start'}});"
               "scrollBy(0, -16)")
    if query or scroll:
        cdp.js("document.activeElement && document.activeElement.blur && document.activeElement.blur()")
    time.sleep(2.6)  # results, thumbnails and the preview fade in
    cdp.png(path)


def frame(cdp, shot, size, pad, dpr, theme, path):
    w, h = size
    url = (TEMPLATES / "frame.html").as_uri() + "?" + urllib.parse.urlencode(
        {"img": shot if str(shot).startswith("data:") else Path(shot).as_uri(), "w": w, "h": h, "pad": pad, "theme": theme})
    cdp.open(url, w + 2 * pad, h + 2 * pad, dpr, theme)
    cdp.js("new Promise((ok) => { const i = document.querySelector('img'); i.complete ? ok() : i.onload = ok; })")
    cdp.png(path)


def take_clip(cdp, scenes, theme, tmp, path):
    """Records the main window while CLIP_JS types the scenes, puts every frame on the frame.html backdrop and writes
    an animated WebP. Frames last as long as they were on screen; repeats are merged."""
    from PIL import Image, ImageDraw

    w, h = round(CLIP_SIZE[0] * CLIP_DPR), round(CLIP_SIZE[1] * CLIP_DPR)
    pad = round(CLIP_PAD * CLIP_DPR)
    back = tmp / f"clip-back-{theme}.png"
    frame(cdp, BLANK, CLIP_SIZE, CLIP_PAD, CLIP_DPR, theme, back)
    backdrop = Image.open(back).convert("RGBA")
    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, w - 1, h - 1), radius=round(8 * CLIP_DPR), fill=255)

    open_app(cdp, "/#caps", CLIP_SIZE, CLIP_DPR, theme)
    time.sleep(1.5)
    cdp("Runtime.evaluate", expression=f"window.clipDone = false; {CLIP_JS}({json.dumps(scenes)})")
    shots, t0 = [], time.time()
    while True:
        data = cdp("Page.captureScreenshot", format="png")["data"]
        now = time.time()
        # the same picture, or one less than 1/15 s after the last: the previous frame just lasts longer
        if not shots or (shots[-1][1] != data and now - shots[-1][0] >= 1 / 15):
            shots.append((now, data))
        if cdp.js("window.clipDone === true") or now - t0 > 90:
            break
    frames, durations = [], []
    for k, (start, data) in enumerate(shots):
        end = shots[k + 1][0] if k + 1 < len(shots) else start + 0.6
        canvas = backdrop.copy()
        canvas.paste(Image.open(io.BytesIO(base64.b64decode(data))).convert("RGB"), (pad, pad), mask)
        frames.append(canvas)
        durations.append(max(20, round((end - start) * 1000)))
    frames[0].save(path, "WEBP", save_all=True, append_images=frames[1:], duration=durations, loop=0,
                   quality=80, method=6, minimize_size=True)
    return len(frames), sum(durations) / 1000


def main():
    only = set(sys.argv[1:])
    OUT.mkdir(parents=True, exist_ok=True)
    tmp = Path(tempfile.mkdtemp(prefix="semsearch-readme-"))
    mock = None
    if not mock_up():
        mock = subprocess.Popen([sys.executable, str(ROOT / "mock_api.py")], cwd=ROOT)
        for _ in range(100):
            if mock_up():
                break
            time.sleep(0.2)
        else:
            sys.exit("mock_api.py did not start on port 8799")
    browser = subprocess.Popen([find_browser(), "--headless=new", f"--remote-debugging-port={CDP_PORT}",
                                f"--user-data-dir={tmp / 'profile'}", "--no-first-run", "--no-default-browser-check",
                                "--hide-scrollbars", "--mute-audio", "--allow-file-access-from-files",
                                "about:blank"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(100):
            try:
                pages = [t for t in json.loads(get(f"http://127.0.0.1:{CDP_PORT}/json/list")) if t["type"] == "page"]
                if pages:
                    break
            except OSError:
                pass
            time.sleep(0.2)
        else:
            sys.exit("the browser did not open its DevTools port")
        cdp = CDP(pages[0]["webSocketDebuggerUrl"])
        cdp("Page.enable")
        cdp("Emulation.setDefaultBackgroundColorOverride", color={"r": 0, "g": 0, "b": 0, "a": 0})
        get(f"{MOCK}/dev/scenario/video")
        for lang in LANGS:
            get(f"{MOCK}/dev/lang/{lang}")
            for name, page, size, query, tab, scroll, pad, dpr in SHOTS:
                if only and name not in only:
                    continue
                for theme in THEMES:
                    raw = tmp / f"{name}-{lang}-{theme}.png"
                    take_window(cdp, page, size, query and query[lang], tab, scroll, dpr, theme, raw)
                    out = OUT / f"{name}-{lang}-{theme}.png"
                    frame(cdp, raw, size, pad, dpr, theme, out)
                    print(f"{out.relative_to(ROOT)}  {out.stat().st_size // 1024} KB")
            for theme in THEMES if not only or "clip" in only else ():
                out = OUT / f"clip-{lang}-{theme}.webp"
                n, sec = take_clip(cdp, CLIP[lang], theme, tmp, out)
                print(f"{out.relative_to(ROOT)}  {out.stat().st_size // 1024} KB, {n} frames, {sec:.1f} s")
        get(f"{MOCK}/dev/lang/system")
        for lang in LANGS if (not only or "speed" in only) and (TEMPLATES / "speed.js").exists() else ():
            for theme in THEMES:
                out = OUT / f"speed-{lang}-{theme}.png"
                cdp.open((TEMPLATES / "speed.html").as_uri() + f"?{lang}-{theme}", 860, 900, 2.0, theme)
                time.sleep(0.3)
                h = cdp.js("Math.ceil(document.getElementById('root').getBoundingClientRect().height)")
                cdp("Emulation.setDeviceMetricsOverride", width=860, height=h, deviceScaleFactor=2.0, mobile=False)
                time.sleep(0.2)
                cdp.png(out)
                print(f"{out.relative_to(ROOT)}  {out.stat().st_size // 1024} KB")
        for lang in LANGS if not only or "architecture" in only else ():
            for theme in THEMES:
                out = OUT / f"architecture-{lang}-{theme}.png"
                cdp.open((TEMPLATES / "architecture.html").as_uri() + f"?{lang}-{theme}", 1200, 690, 1.5, theme)
                time.sleep(0.3)
                cdp.png(out)
                print(f"{out.relative_to(ROOT)}  {out.stat().st_size // 1024} KB")
        get(f"{MOCK}/dev/theme/system")
    finally:
        browser.terminate()
        if mock:
            mock.terminate()
        try:
            browser.wait(10)
        except subprocess.TimeoutExpired:
            pass
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
