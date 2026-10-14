"""SemSearch — сервер: страницы интерфейса, поиск, превью, настройки (контракт: docs/API.md).
Запускается через server.py (pythonw server.py [--no-browser])."""
import ctypes
import io
import logging
import os
import re
import secrets
import socket
import subprocess
import sys
import threading
import time
import webbrowser
from ctypes import wintypes

import core

core.setup_logging()
log = logging.getLogger("server")
cfg = core.load_config()
PORT = cfg["port"]
URL = f"http://127.0.0.1:{PORT}/"
IN_SHELL = os.environ.get("SEMSEARCH_SHELL") == "1"  # запущены из SemSearch.exe: окно, трей и клавишу ведёт он


def already_running():
    with socket.socket() as s:
        return s.connect_ex(("127.0.0.1", PORT)) == 0


if already_running():  # второй запуск просто открывает окно поиска
    if not IN_SHELL:
        webbrowser.open(URL)
    sys.exit(0)

os.environ.setdefault("HF_HUB_DISABLE_SYMLINKS_WARNING", "1")

import mimetypes  # noqa: E402

# Windows берёт типы из реестра и бывает отдаёт .js как text/plain — при nosniff браузер такой скрипт не выполнит
for _ext, _type in ((".js", "text/javascript"), (".css", "text/css"), (".svg", "image/svg+xml")):
    mimetypes.add_type(_type, _ext)

import uvicorn  # noqa: E402
from fastapi import FastAPI, File, HTTPException, Request, UploadFile  # noqa: E402
from fastapi.responses import FileResponse, HTMLResponse, JSONResponse  # noqa: E402
from fastapi.staticfiles import StaticFiles  # noqa: E402
from PIL import Image  # noqa: E402

import engine_ctl  # noqa: E402
import extract  # noqa: E402
import meta  # noqa: E402
import preview  # noqa: E402
import watcher  # noqa: E402
from indexer import Indexer  # noqa: E402
from names import NamesProvider  # noqa: E402

UI_DIR = os.path.join(core.APP_DIR, "ui")
THUMB_DIR = os.path.join(core.DATA_DIR, "thumbs")
os.makedirs(UI_DIR, exist_ok=True)
os.makedirs(THUMB_DIR, exist_ok=True)

# Токен сессии: без него API не отвечает, так что чужая веб-страница не сможет дёргать localhost (CSRF).
# В страницы он подставляется при отдаче, shell читает его из data/token.
TOKEN = os.environ.get("SEMSEARCH_TOKEN") or secrets.token_urlsafe(32)
with open(os.path.join(core.DATA_DIR, "token"), "w", encoding="ascii") as _f:
    _f.write(TOKEN)
HOSTS = (f"127.0.0.1:{PORT}", f"localhost:{PORT}")
ORIGINS = {f"http://{h}" for h in HOSTS}
CSP = ("default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; script-src 'self'; "
       "connect-src 'self'; frame-ancestors 'none'")

STATE = {"ready": False, "error": None}
emb = store = indexer = None
conn = core.connect()
db_lock = threading.Lock()
names = NamesProvider()


def _attach_engine(ix):
    """Движок на Rust отвечает? Тогда список файлов и поток изменений берём у него. -> подключён ли."""
    try:
        mode = engine_ctl.ensure()
    except Exception:
        log.exception("не удалось запустить движок имён")
        mode = None
    client = names._client
    names._checked = 0.0  # движок мог подняться только что — перепроверить, а не верить кешу
    if not mode or client is None or not client.available():  # готовность не нужна: lister её проверяет сам
        return False

    def lister():
        # Пока движок строит индекс, его список неполный — тогда обходим папки сами (см. Indexer._content_files)
        if not client.request("status").get("ready"):
            raise RuntimeError("движок ещё строит индекс")
        r = client.request("list", roots=cfg["content_roots"], exts=sorted(core.CONTENT_EXT),
                           exclude_dirs=cfg["exclude_dirs"], exclude_paths=cfg["exclude_paths"],
                           skip_dot_dirs=True, skip_cloud=True)
        return [(it["path"], os.path.basename(it["path"]), it["size"], it["mtime"]) for it in r["items"]]

    ix.engine_lister = lister
    client.subscribe(cfg["content_roots"], ix.on_engine_event)
    ix.state["watching"] = True
    log.info("движок имён подключён (%s): список файлов и изменения — от него", mode)
    return True


def reap_orphans():
    """Рабочие процессы разбора от прошлого запуска, чей backend умер (сбой, kill) — завершить."""
    import psutil

    # Дочерние процессы multiprocessing запускаются базовым интерпретатором, а не из .venv, поэтому
    # свои узнаём по рабочей папке (папка SemSearch) — чужие Python-программы не трогаем.
    home = os.path.normcase(core.APP_DIR)
    for p in psutil.process_iter(["pid", "cmdline"]):
        try:
            cmd = " ".join(p.info["cmdline"] or [])
            m = re.search(r"spawn_main\(parent_pid=(\d+)", cmd)
            if not m or psutil.pid_exists(int(m.group(1))):
                continue
            if os.path.normcase(p.cwd()) == home:
                p.terminate()
                log.info("завершён осиротевший процесс разбора %d", p.pid)
        except (psutil.Error, OSError):
            continue


def boot():
    global emb, store, indexer
    try:
        reap_orphans()
    except Exception:
        log.exception("не удалось проверить осиротевшие процессы")
    try:
        t0 = time.time()
        emb = core.Embedder()
        store = core.VecStore(emb.device)
        n = store.load(conn)
        log.info("модель загружена за %.0f с, векторов: %d", time.time() - t0, n)
        indexer = Indexer(cfg, emb, store)
        indexer.engine_active = lambda: names.current() is not names.local
        indexer.manual_pause = os.environ.get("SEMSEARCH_PAUSED") == "1"  # для отладки: поднять без нагрузки на GPU
        if not _attach_engine(indexer):
            try:
                watcher.start(indexer, cfg)
            except Exception:
                log.exception("не удалось включить отслеживание изменений — остаются периодические обходы")
        indexer.start()
        STATE["ready"] = True
    except Exception as ex:
        log.exception("не удалось запустить")
        STATE["error"] = f"{type(ex).__name__}: {ex}"


app = FastAPI(docs_url=None, redoc_url=None, openapi_url=None)


@app.middleware("http")
async def guard(request: Request, call_next):
    # DNS-rebinding: отвечаем только на запросы к нашему адресу
    if request.headers.get("host") not in HOSTS:
        return JSONResponse({"detail": "forbidden"}, status_code=403)
    path = request.url.path
    if path.startswith("/api/") or path.startswith("/thumb/"):
        token = request.headers.get("x-semsearch-token") or request.query_params.get("t") or ""
        if not secrets.compare_digest(token, TOKEN):
            return JSONResponse({"detail": "нет токена"}, status_code=403)
        origin = request.headers.get("origin")
        if origin and origin not in ORIGINS:
            return JSONResponse({"detail": "чужой источник"}, status_code=403)
    return await call_next(request)


# ------------------------------------------------ страницы

def _page(name):
    p = os.path.join(UI_DIR, name)
    if os.path.exists(p):
        with open(p, encoding="utf-8") as f:
            html = f.read().replace("__SEMSEARCH_TOKEN__", TOKEN)
    else:
        html = "<!doctype html><meta charset=utf-8><title>SemSearch</title><p>Интерфейс ещё собирается.</p>"
    return HTMLResponse(html, headers={"Cache-Control": "no-store", "Content-Security-Policy": CSP,
                                       "X-Content-Type-Options": "nosniff", "Referrer-Policy": "no-referrer"})


@app.get("/")
def page_main():
    return _page("index.html")


@app.get("/settings")
def page_settings():
    return _page("settings.html")


@app.get("/tray")
def page_tray():
    return _page("tray.html")


app.mount("/ui", StaticFiles(directory=UI_DIR, check_dir=False), name="ui")


# ------------------------------------------------ общие помощники

def _rows(sql, args=()):
    with db_lock:
        return conn.execute(sql, args).fetchall()


def _need_ready():
    if not STATE["ready"]:
        raise HTTPException(503, STATE["error"] or "модель ещё загружается")


def _loc(label):
    label = label or ""
    return "фрагм. " + label.split()[-1] if label.startswith("фрагмент") else label


def _hit(kind, cid, loc, label, snippet, duration=None):
    text = snippet or ""
    if kind in ("text", "code"):
        if loc and text:
            text = "…" + text
        if len(snippet or "") >= 299:
            text += "…"
    elif kind == "video":
        text = "кадр совпал с запросом"
    t_end = None
    if kind == "audio":
        t_end = loc + cfg["audio_segment_sec"]
        if duration:
            t_end = min(t_end, duration)
        text = f"отрезок {meta.ts(loc)}–{meta.ts(t_end)}" + (f" из {meta.ts(duration)}" if duration else "")
    return {"chunk": cid, "loc": _loc(label), "t": float(loc) if kind in ("video", "audio") else None,
            "t_end": float(t_end) if t_end is not None else None, "text": text}


def _known(path):
    """Отдаём и открываем только то, что есть в индексах — никаких произвольных путей."""
    if _rows("SELECT 1 FROM files WHERE path=?", (path,)):
        return True
    try:
        return names.exists(path)
    except Exception:
        return False


# ------------------------------------------------ поиск

TABS = ("all", "text", "code", "image", "video", "audio", "name")
CAPS = {"text": 40, "code": 40, "image": 120, "video": 60, "audio": 20}  # карточек на вкладку
ALL_CAPS = {"docs": 30, "media": 24, "audio": 5, "names": 5}  # секции вкладки «Всё»
# Что считать найденным: не дальше REL_DROP от лучшего совпадения своего вида и заметно выше среднего (z-оценка).
# Замер на индексе 67 тыс. фрагментов: у запроса с настоящими совпадениями верх плотный (0,80 → 0,75),
# у запроса «мимо» распределение ровное — относительный порог отсекает хвост и там, и там.
REL_DROP, MIN_Z, MIN_N_FOR_Z = 0.045, 2.5, 50


def _cards(hits):
    """hits: [(chunk_id, cos, kind)], лучшие первыми -> карточки файлов (Item), по одной на файл."""
    if not hits:
        return []
    rows = {}
    ids = [h[0] for h in hits]
    for i in range(0, len(ids), 900):
        part = ids[i:i + 900]
        for r in _rows("SELECT c.id, c.file_id, c.loc, c.label, c.snippet, f.path, f.name, f.size, f.mtime, f.duration "
                       f"FROM chunks c JOIN files f ON f.id=c.file_id WHERE c.id IN ({','.join('?' * len(part))})", part):
            rows[r[0]] = r[1:]
    cards = {}
    for cid, cos, kind in hits:
        r = rows.get(cid)
        if not r:
            continue
        fid, loc, label, snippet, path, name, size, mtime, dur = r
        c = cards.get(fid)
        if c is None:
            c = cards[fid] = {"id": f"f{fid}", "kind": kind, "name": name, "ext": meta.ext_of(name), "path": path,
                              "crumbs": meta.crumbs(path), "size": size, "mtime": mtime, "score": round(float(cos), 4),
                              "hits": [], "thumb": f"/thumb/{cid}" if kind in ("image", "video") else None,
                              "duration": dur}
        if len(c["hits"]) < 3:
            c["hits"].append(_hit(kind, cid, loc, label, snippet, dur))
    return list(cards.values())


def _semantic(v, kinds):
    out = {}
    for k in kinds:
        hits = store.search(k, v, k=CAPS[k] * 4)
        if hits:
            best, strict = hits[0][1], store.count(k) >= MIN_N_FOR_Z
            hits = [h for h in hits if h[1] >= best - REL_DROP and (not strict or h[2] >= MIN_Z)]
        out[k] = _cards([(cid, cos, k) for cid, cos, _ in hits])[:CAPS[k]]
    return out


def _name_items(q, limit):
    try:
        total, rows = names.search(q, limit)
    except Exception:
        log.exception("поиск по имени")
        return 0, []
    items = [{"id": "n:" + r["path"], "kind": "name", "name": r["name"], "ext": meta.ext_of(r["name"]),
              "path": r["path"], "crumbs": meta.crumbs(r["path"]), "size": r.get("size"), "mtime": r.get("mtime"),
              "score": 0.0, "hits": [], "thumb": None, "duration": None}
             for r in rows if not r.get("dir")]
    return total, items


def _indexed_paths(paths):
    out = set()
    paths = list(paths)
    for i in range(0, len(paths), 900):
        part = paths[i:i + 900]
        out |= {r[0] for r in _rows(f"SELECT path FROM files WHERE status=1 AND path IN ({','.join('?' * len(part))})",
                                    part)}
    return out


def _response(q, tab, t0, by_kind, name_total=0, name_items=()):
    by_kind = {k: by_kind.get(k, []) for k in core.KINDS}
    docs = sorted(by_kind["text"] + by_kind["code"], key=lambda c: -c["score"])[:ALL_CAPS["docs"]]
    media = sorted(by_kind["image"] + by_kind["video"], key=lambda c: -c["score"])[:ALL_CAPS["media"]]
    audio = by_kind["audio"][:ALL_CAPS["audio"]]
    indexed = _indexed_paths(n["path"] for n in name_items)
    name_only = [n for n in name_items if n["path"] not in indexed][:ALL_CAPS["names"]]
    all_items = docs + media + audio + name_only
    counts = {k: len(v) for k, v in by_kind.items()}
    counts.update(all=len(all_items), name=name_total)
    items = all_items if tab == "all" else list(name_items) if tab == "name" else by_kind[tab]
    return {"q": q, "tab": tab, "ms": int((time.time() - t0) * 1000), "counts": counts,
            "name_scope": ", ".join(cfg["name_drives"]), "items": items}


@app.get("/api/_scores")
def _scores(q: str, kind: str = "text"):
    """Отладка порогов: косинусы и z-оценки лучших фрагментов."""
    _need_ready()
    hits = store.search(kind, emb.query(q), k=60)
    return {"n": store.count(kind), "top": [(round(c, 3), round(z, 2)) for _, c, z in hits]}


@app.get("/api/search")
def search(q: str = "", tab: str = "all"):
    if tab not in TABS:
        raise HTTPException(400, "неизвестная вкладка")
    t0, q = time.time(), q.strip()[:500]
    if not q:
        return _response(q, tab, t0, {})
    name_total, name_items = _name_items(q, 200 if tab == "name" else 40)
    by_kind = {}
    if STATE["ready"]:
        by_kind = _semantic(emb.query(q), core.KINDS)
    elif tab != "name":  # имена ищутся и без модели
        _need_ready()
    return _response(q, tab, t0, by_kind, name_total, name_items)


def _media_response(v, tab, t0):
    if tab not in ("all", "image", "video"):
        tab = "image"
    return _response("", tab, t0, _semantic(v, ("image", "video")))


@app.get("/api/similar")
def similar(chunk: int, tab: str = "image"):
    _need_ready()
    t0 = time.time()
    v = store.vector(chunk)
    if v is None:
        raise HTTPException(404, "фрагмент не найден")
    return _media_response(v, tab, t0)


@app.post("/api/search_image")
async def search_image(file: UploadFile = File(...), tab: str = "image"):
    _need_ready()
    t0 = time.time()
    data = await file.read(32 * 1024 * 1024)
    try:
        im = Image.open(io.BytesIO(data))
        im.thumbnail((768, 768))
        im = im.convert("RGB")
    except Exception:
        raise HTTPException(400, "это не картинка")
    return _media_response(emb.images([im])[0], tab, t0)


# ------------------------------------------------ превью

@app.get("/api/preview")
def preview_ep(path: str, chunk: int | None = None, chunks: str | None = None):
    row = _rows("SELECT id, kind, status, duration, err FROM files WHERE path=?", (path,))
    if not (row or _known(path)) or not os.path.isfile(path):
        raise HTTPException(404, "файл не найден")
    st = os.stat(path)
    name = os.path.basename(path)
    ext = meta.ext_of(name)
    fid, kind, status, dur, err = row[0] if row else (None, None, None, None, None)
    indexed = status == 1
    # Почему файл найден только по имени: ждёт очереди, не прочитался, вне папок поиска или такой тип не индексируем
    state = ("indexed" if indexed else "pending" if status == 0 else "failed" if status == 2
             else "outside" if core.kind_of("." + ext.lower()) else "unsupported")
    k = kind if indexed else "name"
    out = {"path": path, "name": name, "ext": ext, "kind": k, "crumbs": meta.crumbs(path), "size": st.st_size,
           "mtime": st.st_mtime, "ctime": st.st_ctime, "type": meta.type_name(ext), "taken": None, "duration": None,
           "pages": None, "dims": None, "page_label": None, "hit_pos": None, "hits": [], "code": None, "marks": None,
           "wave": None, "seg": None, "dupes": [], "thumb": None, "can_similar": k in ("image", "video"),
           "indexed": indexed, "state": state, "reason": err if status == 2 else None}
    if indexed:
        all_chunks = _rows("SELECT id, loc, label, snippet FROM chunks WHERE file_id=? ORDER BY loc", (fid,))
        by_id = {c[0]: c for c in all_chunks}
        want = [int(x) for x in (chunks or "").split(",") if x.strip().isdigit()] or ([chunk] if chunk else [])
        sel = [by_id[c] for c in want if c in by_id] or all_chunks[:1]
        best = sel[0] if sel else None
        if k in ("audio", "video") and not dur:
            dur = extract.media_duration(path)
        out["hits"] = [_hit(k, c[0], c[1], c[2], c[3], dur) for c in sel[:3]]
        if k == "text" and best:
            if ext == "PDF":
                pages = preview.pdf_pages(path, st.st_mtime)
                page = int(best[2].split()[-1]) if (best[2] or "").startswith("стр.") else 1
                out["pages"] = pages
                if pages:
                    out["type"] += f", {pages} {meta.plural(pages, ('страница', 'страницы', 'страниц'))}"
                    out["page_label"] = f"стр. {page} из {pages}"
                    out["hit_pos"] = round((page - 1) / max(pages - 1, 1), 4)
            else:
                n = len(all_chunks)
                out["page_label"] = f"фрагмент {int(best[1]) + 1} из {n}"
                out["hit_pos"] = round(best[1] / max(n - 1, 1), 4)
        elif k == "code" and best:
            out["code"] = preview.code_window(path, best[3])
        elif k == "image" and best:
            w, h, taken = preview.image_info(path, st.st_mtime)
            if w:
                out["dims"], out["type"] = [w, h], out["type"] + f", {w} × {h}"
            out["taken"], out["thumb"] = taken, f"/thumb/{best[0]}?size=640"
        elif k == "video" and best:
            w, h, vdur = preview.video_info(path, st.st_mtime)
            dur = dur or vdur
            if w:
                out["dims"], out["type"] = [w, h], out["type"] + f", {w} × {h}"
            out["duration"], out["thumb"] = dur, f"/thumb/{best[0]}?size=640"
            if dur:
                out["marks"] = [round(min(max(c[1] / dur, 0.0), 1.0), 4) for c in sel[:3]]
        elif k == "audio" and best:
            out["duration"] = dur
            out["wave"] = preview.waveform(path, st.st_mtime)
            end = best[1] + cfg["audio_segment_sec"]
            out["seg"] = [best[1], min(end, dur) if dur else end]
    try:
        out["dupes"] = [{"path": d["path"], "crumbs": meta.crumbs(d["path"])}
                        for d in names.same_name(name, 8) if d["path"] != path][:5]
    except Exception:
        log.exception("поиск копий %s", name)
    return out


@app.get("/thumb/{chunk_id}")
def thumb(chunk_id: int, size: int = 360):
    size = 640 if size > 360 else 360
    out = os.path.join(THUMB_DIR, f"{chunk_id}_{size}.jpg")
    if not os.path.exists(out):
        rows = _rows("SELECT c.kind, c.loc, f.path FROM chunks c JOIN files f ON f.id=c.file_id WHERE c.id=?",
                     (chunk_id,))
        if not rows:
            raise HTTPException(404)
        kind, loc, path = rows[0]
        try:
            im = (extract.load_image(path, size) if kind == "image"
                  else extract.video_frame(path, loc, size) if kind == "video" else None)
        except Exception:
            im = None
        if im is None:
            raise HTTPException(404)
        im.convert("RGB").save(out, "JPEG", quality=84)
    return FileResponse(out, headers={"Cache-Control": "max-age=86400"})


@app.post("/api/open")
async def open_file(request: Request):
    body = await request.json()
    path = str(body.get("path", ""))
    if not _known(path) or not os.path.exists(path):
        raise HTTPException(404, "файл не найден")
    if body.get("reveal"):
        subprocess.Popen(["explorer.exe", "/select,", path])
    else:
        os.startfile(path)
    return {"ok": True}


# ------------------------------------------------ состояние

_size_cache = {"t": 0.0, "v": 0}


def _index_bytes():
    if time.time() - _size_cache["t"] > 30:
        total = 0
        for root, _, files in os.walk(core.DATA_DIR):
            for f in files:
                try:
                    total += os.path.getsize(os.path.join(root, f))
                except OSError:
                    pass
        _size_cache.update(t=time.time(), v=total)
    return _size_cache["v"]


def _throttle_state():
    try:
        import throttle

        return throttle.state()
    except Exception:
        return None


@app.get("/api/status")
def status():
    kinds = {k: {"done": 0, "total": 0, "failed": 0} for k in core.KINDS}
    for k, s, n in _rows("SELECT kind, status, count(*) FROM files GROUP BY kind, status"):
        if k in kinds:
            kinds[k]["total"] += n
            kinds[k]["done"] += n if s in (1, 2) else 0
            kinds[k]["failed"] += n if s == 2 else 0
    remaining = {k: v["total"] - v["done"] for k, v in kinds.items()}
    pending = any(remaining.values())
    ix = indexer.state if indexer else {}
    phase = ix.get("phase")
    paused = None
    if indexer and indexer.manual_pause:
        paused = {"reason": "manual", "detail": None}
    elif pending:
        paused = ix.get("paused")
    if not STATE["ready"]:
        state = "error" if STATE["error"] else "loading"
    elif phase == "error":
        state = "error"
    elif paused:
        state = "paused"
    elif pending or phase == "scan":
        state = "indexing"
    else:
        state = "idle"
    p = names.current()
    n_progress = None
    if p is names.local:
        n_names, n_kind, n_ready, n_updated = ix.get("names", 0), "python", ix.get("names_scan") is None, \
            ix.get("names_updated")
        if ix.get("names_scan") is not None:  # прогресс пересборки — относительно прошлого числа файлов
            n_progress = round(min(ix["names_scan"] / max(n_names, 1), 0.99), 3) if n_names else None
    else:
        try:
            st = p.status()
            n_names, n_kind, n_ready = st.get("files", 0), st.get("mode", "engine"), bool(st.get("ready"))
            n_updated = st["updated"] / 1000 if st.get("updated") else time.time()
        except Exception:
            n_names, n_kind, n_ready, n_updated = 0, "engine", False, None
    return {
        "ready": STATE["ready"], "error": STATE["error"], "device": emb.device if emb else None,
        "state": state, "phase": phase if phase in kinds or phase == "scan" else None, "paused": paused,
        "kinds": kinds,
        "current": {"kind": phase, **{x: kinds[phase][x] for x in ("done", "total")}} if phase in kinds else None,
        "names": {"count": n_names, "drives": cfg["name_drives"], "engine": n_kind, "ready": n_ready,
                  "updated": n_updated, "scan": ix.get("names_scan"), "progress": n_progress},
        "eta_sec": indexer.eta(remaining) if indexer and state == "indexing" else None,
        "watching": bool(ix.get("watching")), "throttle": _throttle_state(), "index_bytes": _index_bytes(),
    }


@app.post("/api/pause")
async def pause(request: Request):
    _need_ready()
    indexer.manual_pause = bool((await request.json()).get("paused"))
    if not indexer.manual_pause:
        indexer.state["paused"] = None
    return {"ok": True}


@app.post("/api/rescan")
def rescan():
    _need_ready()
    indexer.request_rescan()
    return {"ok": True}


@app.post("/api/rebuild")
def rebuild():
    _need_ready()
    indexer.request_rebuild()
    return {"ok": True}


# ------------------------------------------------ настройки

@app.get("/api/settings")
def settings_get():
    roots = []
    for r in cfg["content_roots"]:
        lo, hi = core.prefix_range(r)
        n = _rows("SELECT count(*) FROM files WHERE path >= ? AND path < ?", (lo, hi))[0][0]
        roots.append({"path": r, "files": n})
    failed = _rows("SELECT count(*) FROM files WHERE status=2")[0][0]
    keys = ("exclude_dirs", "hotkey", "theme", "accent", "pause_on_fullscreen", "pause_processes", "pause_on_battery",
            "video_frame_every_sec", "video_max_frames", "audio_segment_sec", "audio_max_segments", "max_text_mb",
            "max_pdf_pages")
    return {"content_roots": roots, "drives": {"available": core.fixed_drives(), "selected": cfg["name_drives"]},
            **{k: cfg[k] for k in keys},
            "info": {"model": "EmbeddingGemma 2", "device": emb.device if emb else None,
                     "index_bytes": _index_bytes(), "rescan_minutes": cfg["rescan_minutes"],
                     "name_rescan_minutes": cfg["name_rescan_minutes"], "failed": failed}}


LIMITS = {"video_frame_every_sec": (1, 600), "video_max_frames": (1, 500), "audio_segment_sec": (5, 120),
          "audio_max_segments": (1, 100), "max_text_mb": (1, 100), "max_pdf_pages": (1, 2000)}
_name_re = re.compile(r"^[^\\/:*?\"<>|]{1,128}$")


def _validate(key, value):
    if key == "content_roots":
        roots = [os.path.normpath(str(p)) for p in value]
        bad = [p for p in roots if not os.path.isdir(p)]
        if bad or not roots:
            raise HTTPException(400, f"нет такой папки: {bad[0] if bad else '—'}")
        return roots
    if key == "name_drives":
        allowed = {d["drive"] for d in core.fixed_drives()}
        drives = [str(d).upper().rstrip("\\/") for d in value]
        if not drives or any(d not in allowed for d in drives):
            raise HTTPException(400, "такого диска нет")
        return drives
    if key in ("exclude_dirs", "pause_processes"):
        items = [str(x).strip() for x in value if str(x).strip()]
        if any(not _name_re.match(x) for x in items):
            raise HTTPException(400, "в списке есть недопустимое имя")
        return items
    if key == "theme":
        if value not in ("system", "light", "dark"):
            raise HTTPException(400, "неизвестная тема")
        return value
    if key == "accent":
        if not re.fullmatch(r"#[0-9a-fA-F]{6}", str(value)):
            raise HTTPException(400, "цвет в формате #rrggbb")
        return value
    if key == "hotkey":
        if not re.fullmatch(r"[a-z0-9]+(\+[a-z0-9]+){1,3}", str(value).lower()):
            raise HTTPException(400, "сочетание вида ctrl+alt+s")
        return str(value).lower()
    typ = core.EDITABLE[key]
    if typ is bool:
        return bool(value)
    if typ is int:
        lo, hi = LIMITS[key]
        v = int(value)
        if not lo <= v <= hi:
            raise HTTPException(400, f"{key}: от {lo} до {hi}")
        return v
    return value


@app.post("/api/settings")
async def settings_post(request: Request):
    body = await request.json()
    if "content_roots" in body and body["content_roots"] and isinstance(body["content_roots"][0], dict):
        body["content_roots"] = [r.get("path") for r in body["content_roots"]]
    if "drives" in body:
        body["name_drives"] = body.pop("drives")
    changes = {k: _validate(k, v) for k, v in body.items() if k in core.EDITABLE}
    if not changes:
        return {"ok": True, "restart": False}
    core.save_config(changes)
    cfg.update(changes)
    if indexer:
        indexer.refresh_config()
    else:
        core.derive_config(cfg)
    restart = any(k in changes for k in ("content_roots", "name_drives")) or ("hotkey" in changes and not IN_SHELL)
    if indexer and any(k in changes for k in ("content_roots", "exclude_dirs", "name_drives")):
        indexer.request_rescan()
    log.info("настройки изменены: %s", ", ".join(changes))
    return {"ok": True, "restart": restart}


@app.get("/api/failed")
def failed(limit: int = 200):
    total = _rows("SELECT count(*) FROM files WHERE status=2")[0][0]
    items = _rows("SELECT path, err FROM files WHERE status=2 ORDER BY path LIMIT ?", (max(1, min(limit, 2000)),))
    return {"total": total, "items": [{"path": p, "err": e} for p, e in items]}


@app.get("/api/processes")
def processes():
    import psutil

    found = set()
    for p in psutil.process_iter(["name"]):
        n = p.info.get("name") or ""
        if n.lower().endswith(".exe"):
            found.add(n)
    return {"items": sorted(found, key=str.lower)}


@app.post("/api/open_log")
def open_log():
    os.startfile(os.path.join(core.DATA_DIR, "semsearch.log"))
    return {"ok": True}


@app.post("/api/shutdown")
async def shutdown(request: Request):
    """Выход из SemSearch. {"engine": false} — оставить движок имён работать (перезапуск backend)."""
    try:
        body = await request.json()
    except Exception:
        body = {}

    def bye():
        if indexer:
            indexer.close()
        if body.get("engine", True):
            engine_ctl.stop()
        os._exit(0)

    threading.Timer(0.5, bye).start()
    return {"ok": True}


# ------------------------------------------------ глобальная горячая клавиша (без SemSearch.exe)

def hotkey_loop(spec):
    mods = {"alt": 0x1, "ctrl": 0x2, "shift": 0x4, "win": 0x8}
    keys = {"space": 0x20, **{chr(c): c for c in range(0x41, 0x5B)}, **{chr(c): c for c in range(0x30, 0x3A)},
            **{f"f{i}": 0x6F + i for i in range(1, 13)}}
    mod, vk = 0x4000, None  # MOD_NOREPEAT
    for part in spec.lower().split("+"):
        if part in mods:
            mod |= mods[part]
        else:
            vk = keys.get(part.upper() if len(part) == 1 else part)
    if not vk or not ctypes.windll.user32.RegisterHotKey(None, 1, mod, vk):
        log.warning("не удалось зарегистрировать горячую клавишу %s (занята?)", spec)
        return
    msg = wintypes.MSG()
    while ctypes.windll.user32.GetMessageW(ctypes.byref(msg), None, 0, 0) != 0:
        if msg.message == 0x0312:  # WM_HOTKEY
            webbrowser.open(URL)


def main():
    threading.Thread(target=boot, name="boot", daemon=True).start()
    if cfg.get("hotkey") and not IN_SHELL:
        threading.Thread(target=hotkey_loop, args=(cfg["hotkey"],), name="hotkey", daemon=True).start()
    if "--no-browser" not in sys.argv and not IN_SHELL:
        threading.Timer(1.5, lambda: webbrowser.open(URL)).start()
    uvicorn.run(app, host="127.0.0.1", port=PORT, log_level="warning", log_config=None)
