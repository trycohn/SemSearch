"""Фоновая индексация: обход папок, отслеживание изменений, извлечение, эмбеддинги, индекс имён."""
import logging
import os
import stat
import threading
import time
from collections import deque
from concurrent.futures import CancelledError, ProcessPoolExecutor, ThreadPoolExecutor
from concurrent.futures import TimeoutError as FuturesTimeout
from concurrent.futures.process import BrokenProcessPool

import numpy as np

import core
import extract
import throttle

log = logging.getLogger("indexer")

# Атрибуты, при которых файл нельзя трогать: ссылки/junction и облачные заглушки OneDrive
# (чтение такой заглушки скачает файл из облака).
_REPARSE = 0x400
_CLOUD = 0x1000 | 0x40000 | 0x400000  # OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS
NAME_EXCLUDE_DIRS = ["$Recycle.Bin", "System Volume Information", "WinSxS"]
QUIET_SEC = 3  # файл должен «успокоиться» (докопироваться/докачаться), прежде чем его индексировать


def walk(roots, exclude_dirs, exclude_paths, skip_dot_dirs, should_stop):
    """Быстрый обход через os.scandir. Отдаёт (path, name, size, mtime)."""
    excl = {d.lower() for d in exclude_dirs}
    excl_paths = tuple(os.path.normcase(p) for p in exclude_paths)
    stack = list(roots)
    while stack:
        if should_stop():
            return
        d = stack.pop()
        try:
            it = os.scandir(d)
        except OSError:
            continue
        with it:
            for e in it:
                try:
                    st = e.stat(follow_symlinks=False)
                except OSError:
                    continue
                attrs = getattr(st, "st_file_attributes", 0)
                if attrs & _REPARSE:
                    continue
                if stat.S_ISDIR(st.st_mode):
                    low = e.name.lower()
                    if low in excl or (skip_dot_dirs and e.name.startswith(".")):
                        continue
                    if excl_paths and os.path.normcase(e.path).startswith(excl_paths):
                        continue
                    stack.append(e.path)
                elif not attrs & _CLOUD:
                    yield e.path, e.name, st.st_size, st.st_mtime


def build_names(gen, roots):
    """Запускается в отдельном процессе: обход всего диска — миллионы итераций Python,
    в потоке они отнимали бы GIL у потока, который кормит видеокарту."""
    conn = core.connect_names()
    sql = f"INSERT OR IGNORE INTO names_t_{gen}(path, name) VALUES (?,?)"
    buf = []
    for path, name, _, _ in walk(roots, NAME_EXCLUDE_DIRS, [], False, lambda: False):
        buf.append((path, name))
        if len(buf) >= 20000:
            with conn:
                conn.executemany(sql, buf)
            buf.clear()
    with conn:
        conn.executemany(sql, buf)
    conn.close()


class Indexer(threading.Thread):
    ORDER = ("text", "code", "image", "audio", "video")  # быстрые виды сначала

    def __init__(self, cfg, emb, store):
        super().__init__(name="indexer", daemon=True)
        self.cfg, self.emb, self.store = cfg, emb, store
        self.conn = core.connect()
        self.manual_pause = False
        self.wake = threading.Event()
        self.names_wake = threading.Event()
        self.stop_flag = False
        self.full_scan_requested = True
        self.full_scan_requested_names = False  # «Пересканировать» нажали до того, как поток имён стартовал
        self.rebuild_requested = False
        # phase: start | scan | text | code | image | audio | video | idle | error
        self.state = {"phase": "start", "current": "", "paused": None, "names": 0, "names_scan": None,
                      "names_updated": None, "watching": False}
        self.rates = {}  # вид -> секунд на файл (скользящее среднее) — для оценки оставшегося времени
        self.last_name_scan = 0.0
        # Картинки/видео/аудио декодируются в C-коде без GIL — хватает потоков.
        # PDF и документы разбираются на чистом Python — в отдельных процессах, иначе они душат поток,
        # который кормит видеокарту.
        self.pool = ThreadPoolExecutor(6, thread_name_prefix="extract", initializer=throttle.background_thread)
        self.procs = ProcessPoolExecutor(max(2, min(6, (os.cpu_count() or 4) // 3)), initializer=extract.worker_init)
        self.changes = {}  # path -> время последнего события (для содержимого)
        self.changes_lock = threading.Lock()
        self.name_ops = deque()  # ("add"|"del", path) для индекса имён
        self.excl_dirs = {d.lower() for d in cfg["exclude_dirs"]}
        self.excl_paths = tuple(os.path.normcase(p) for p in cfg["exclude_paths"])

    # ------------------------------------------------ пауза

    def pause_reason(self):
        """-> None или {"reason": manual|fullscreen|process|battery, "detail": ...}."""
        if self.manual_pause:
            return {"reason": "manual", "detail": None}
        if self.cfg["pause_on_fullscreen"] and core.fullscreen_app_active():
            return {"reason": "fullscreen", "detail": None}
        p = core.running_blocker(self.cfg["pause_processes"])
        if p:
            return {"reason": "process", "detail": p}
        if self.cfg.get("pause_on_battery") and core.on_battery():
            return {"reason": "battery", "detail": None}
        return None

    def wait_if_paused(self):
        """Ждёт снятия паузы. -> сколько секунд простояли (не учитывается в скорости индексации)."""
        freed, t0 = False, time.time()
        while not self.stop_flag:
            reason = self.pause_reason()
            self.state["paused"] = reason
            if not reason:
                return time.time() - t0
            if not freed:
                self.emb.free_cache()  # отдаём игре видеопамять кеша
                freed = True
            time.sleep(3)
        return time.time() - t0

    # Сколько секунд в среднем уходит на файл, пока своих замеров нет (RTX 4070 SUPER)
    DEFAULT_RATES = {"text": 0.25, "code": 0.05, "image": 0.08, "audio": 1.5, "video": 10.0}

    def eta(self, remaining):
        """remaining: {вид: сколько файлов осталось} -> секунд до конца или None."""
        if not any(remaining.values()):
            return None
        return int(sum(n * self.rates.get(k, self.DEFAULT_RATES[k]) for k, n in remaining.items()))

    def refresh_config(self):
        """Настройки поменяли из интерфейса: пересчитать производное и перечитать папки."""
        core.derive_config(self.cfg)
        self.excl_dirs = {d.lower() for d in self.cfg["exclude_dirs"]}
        self.excl_paths = tuple(os.path.normcase(p) for p in self.cfg["exclude_paths"])

    def close(self):
        """Выход: остановить рабочие процессы разбора — os._exit их сам не дожидается и не гасит."""
        self.stop_flag = True
        for p in list((getattr(self.procs, "_processes", None) or {}).values()):
            try:
                p.terminate()
            except Exception:
                pass

    def request_rebuild(self):
        self.rebuild_requested = True
        self.wake.set()

    def _rebuild(self):
        """Стирает индекс содержимого и собирает заново (индекс имён не трогаем)."""
        self.rebuild_requested = False
        with self.conn:
            self.conn.execute("DELETE FROM chunks")
            self.conn.execute("DELETE FROM files")
        self.store.load(self.conn)
        thumbs = os.path.join(core.DATA_DIR, "thumbs")
        for f in os.listdir(thumbs) if os.path.isdir(thumbs) else []:
            try:
                os.remove(os.path.join(thumbs, f))
            except OSError:
                pass
        self.rates.clear()
        self.full_scan_requested = True
        log.info("индекс содержимого стёрт, собираю заново")

    # ------------------------------------------------ события файловой системы (из потока watchdog)

    def on_fs_event(self, event_type, src, dst):
        for p, gone in ((src, event_type in ("deleted", "moved")), (dst, False)):
            if not p:
                continue
            if core.under(p, self.cfg["content_roots"]):
                with self.changes_lock:
                    self.changes[p] = time.time()
                self.wake.set()
            if event_type != "modified" and core.under(p, self.cfg["name_roots"]):
                self.name_ops.append(("del" if gone else "add", p))
                self.names_wake.set()

    # ------------------------------------------------ основной цикл

    def run(self):
        core.lower_priority()
        # Ошибки модели могли быть временными (нехватка памяти, исправленный баг) — даём таким файлам ещё попытку
        with self.conn:
            retry = self.conn.execute("UPDATE files SET status=0, err=NULL WHERE status=2 AND err LIKE 'модель:%'").rowcount
        if retry:
            log.info("повторно в очередь после ошибок модели: %d", retry)
        threading.Thread(target=self.names_loop, name="names", daemon=True).start()
        last_full = 0.0
        while not self.stop_flag:
            try:
                if self.rebuild_requested:
                    self._rebuild()
                if self.full_scan_requested or time.time() - last_full > self.cfg["rescan_minutes"] * 60:
                    self.full_scan_requested = False
                    self.scan_content()
                    last_full = time.time()
                self.apply_changes()
                self.process_pending()  # если ждущих файлов нет — это один COUNT
                if not self.rebuild_requested:
                    self.state.update(phase="idle", current="")
            except Exception:
                log.exception("ошибка цикла индексации")
                self.state["phase"] = "error"
            # Пока есть неуспокоившиеся изменения — проверяем часто, иначе спим до события
            with self.changes_lock:
                busy = bool(self.changes)
            self.wake.wait(1 if busy else 60)
            self.wake.clear()

    def request_rescan(self):
        self.full_scan_requested = True
        self.full_scan_requested_names = True
        self.last_name_scan = 0.0
        self.wake.set()
        self.names_wake.set()

    # ------------------------------------------------ содержимое: полный обход

    engine_lister = None  # app подставляет список файлов от движка на Rust (быстрее обхода папок)

    def _content_files(self):
        if self.engine_lister:
            try:
                return self.engine_lister()
            except RuntimeError as ex:  # ожидаемо: движок ещё строит индекс
                log.info("%s — обхожу папки сам", ex)
            except Exception:
                log.exception("движок не отдал список файлов — обхожу папки сам")
        return walk(self.cfg["content_roots"], self.cfg["exclude_dirs"], self.cfg["exclude_paths"], True,
                    lambda: self.stop_flag)

    def on_engine_event(self, ev):
        """Событие из журнала изменений движка (вместо watchdog): created/modified/deleted/renamed/rescan."""
        if ev.get("ev") == "rescan":
            self.full_scan_requested = True
            self.wake.set()
            return
        for p in (ev.get("old"), ev.get("path")):
            if p and core.under(p, self.cfg["content_roots"]):
                with self.changes_lock:
                    self.changes[p] = time.time()
                self.wake.set()

    def scan_content(self):
        self.state.update(phase="scan", current="")
        known = {p: (i, s, m) for i, p, s, m in self.conn.execute("SELECT id, path, size, mtime FROM files")}
        seen, new, changed = set(), [], []
        for path, name, size, mtime in self._content_files():
            kind = self._kind(name, size)
            if not kind:
                continue
            seen.add(path)
            old = known.get(path)
            if old is None:
                new.append((path, name, kind, size, mtime))
            elif old[1] != size or abs(old[2] - mtime) > 1:
                changed.append((old[0], size, mtime))
        if self.stop_flag:
            return
        gone = [i for p, (i, _, _) in known.items() if p not in seen]
        if len(gone) > max(1000, len(known) // 2):
            # Разом «пропало» больше половины индекса — так бывает, если источник списка ответил не полностью
            # (движок ещё строит индекс). Перепроверяем своим обходом, прежде чем стирать посчитанное.
            log.warning("обход: пропало %d из %d файлов — перепроверяю обходом папок", len(gone), len(known))
            really = {path for path, *_ in walk(self.cfg["content_roots"], self.cfg["exclude_dirs"],
                                               self.cfg["exclude_paths"], True, lambda: self.stop_flag)}
            if self.stop_flag:
                return
            gone = [i for p, (i, _, _) in known.items() if p not in really]
            log.warning("после перепроверки пропало %d", len(gone))
        with self.conn:
            self.conn.executemany("INSERT INTO files(path, name, kind, size, mtime) VALUES (?,?,?,?,?)", new)
            for fid, size, mtime in changed:
                self._drop_chunks(fid)
                self.conn.execute("UPDATE files SET size=?, mtime=?, status=0, err=NULL WHERE id=?", (size, mtime, fid))
            for fid in gone:
                self._drop_chunks(fid)
                self.conn.execute("DELETE FROM files WHERE id=?", (fid,))
        self.state["phase"] = "idle"
        log.info("обход: новых %d, изменённых %d, удалённых %d", len(new), len(changed), len(gone))

    def _kind(self, name, size):
        kind = core.kind_of(os.path.splitext(name)[1].lower())
        if kind == "image" and size < self.cfg["min_image_kb"] * 1024:
            return None
        return kind

    def _drop_chunks(self, fid):
        ids = [r[0] for r in self.conn.execute("SELECT id FROM chunks WHERE file_id=?", (fid,))]
        if ids:
            self.store.remove(ids)
            self.conn.execute("DELETE FROM chunks WHERE file_id=?", (fid,))

    # ------------------------------------------------ содержимое: точечные изменения

    def _in_scope(self, path, is_dir):
        root = core.under(path, self.cfg["content_roots"])
        if not root or os.path.normcase(path).startswith(self.excl_paths):
            return False
        parts = os.path.relpath(path, root).split(os.sep)
        dirs = parts if is_dir else parts[:-1]
        return not any(d.lower() in self.excl_dirs or d.startswith(".") for d in dirs)

    def apply_changes(self):
        """Обрабатывает пути, по которым QUIET_SEC не было событий. -> число изменений в базе."""
        now = time.time()
        with self.changes_lock:
            ready = [p for p, t in self.changes.items() if now - t >= QUIET_SEC]
            for p in ready:
                del self.changes[p]
        if not ready:
            return 0
        n = 0
        with self.conn:
            for p in ready:
                if os.path.isdir(p):
                    if self._in_scope(p, True):
                        for path, name, size, mtime in walk([p], self.cfg["exclude_dirs"], self.cfg["exclude_paths"],
                                                            True, lambda: self.stop_flag):
                            n += self._upsert(path, name, size, mtime)
                elif os.path.isfile(p):
                    if not self._in_scope(p, False):
                        continue
                    try:
                        st = os.stat(p, follow_symlinks=False)
                    except OSError:
                        continue
                    if getattr(st, "st_file_attributes", 0) & (_REPARSE | _CLOUD):
                        continue
                    n += self._upsert(p, os.path.basename(p), st.st_size, st.st_mtime)
                else:
                    n += self._delete_under(p)
        if n:
            log.info("изменения на диске: %d", n)
        return n

    def _upsert(self, path, name, size, mtime):
        kind = self._kind(name, size)
        if not kind:
            return 0
        row = self.conn.execute("SELECT id, size, mtime FROM files WHERE path=?", (path,)).fetchone()
        if row is None:
            self.conn.execute("INSERT INTO files(path, name, kind, size, mtime) VALUES (?,?,?,?,?)",
                              (path, name, kind, size, mtime))
            return 1
        fid, old_size, old_mtime = row
        if old_size == size and abs(old_mtime - mtime) <= 1:
            return 0
        self._drop_chunks(fid)
        self.conn.execute("UPDATE files SET size=?, mtime=?, status=0, err=NULL WHERE id=?", (size, mtime, fid))
        return 1

    def _delete_under(self, path):
        lo, hi = core.prefix_range(path)
        fids = [r[0] for r in self.conn.execute(
            "SELECT id FROM files WHERE path=? OR (path >= ? AND path < ?)", (path, lo, hi))]
        for fid in fids:
            self._drop_chunks(fid)
            self.conn.execute("DELETE FROM files WHERE id=?", (fid,))
        return len(fids)

    # ------------------------------------------------ эмбеддинги

    def _result(self, fut, in_procs):
        """Результат извлечения; зависший или упавший разбор файла не останавливает индексацию."""
        try:
            return fut.result(timeout=180)
        except FuturesTimeout:
            if in_procs:
                self._reset_procs()
            return None, "разбор не уложился в 3 минуты"
        except (BrokenProcessPool, CancelledError):
            self._reset_procs()
            return None, "сбой разбора файла"
        except Exception as ex:
            return None, f"{type(ex).__name__}: {ex}"[:200]

    def _reset_procs(self):
        old = self.procs
        if getattr(old, "_reset", False):
            return
        old._reset = True
        for p in list((getattr(old, "_processes", None) or {}).values()):
            try:
                p.terminate()  # зависший разбор иначе так и висел бы
            except Exception:
                pass
        old.shutdown(wait=False, cancel_futures=True)
        self.procs = ProcessPoolExecutor(old._max_workers, initializer=extract.worker_init)
        log.warning("пул разбора документов перезапущен")

    def _encode(self, kind, payloads):
        # background=True: работа режется на кусочки по 1–5 мс и подстраивается под пользователя (throttle.py),
        # чтобы кадры игр и рабочего стола успевали между ними
        if kind in ("text", "code"):
            return self.emb.documents(payloads, background=True)
        if kind in ("image", "video"):
            return self.emb.images(payloads, background=True)
        return self.emb.audio(payloads, background=True)

    def process_pending(self):
        total = self.conn.execute("SELECT count(*) FROM files WHERE status=0").fetchone()[0]
        if not total:
            return
        batch_files = {"text": 32, "code": 32, "image": 32, "audio": 4, "video": 1}
        for kind in self.ORDER:
            rows = self.conn.execute("SELECT id, path, kind, mtime FROM files WHERE status=0 AND kind=?",
                                     (kind,)).fetchall()
            if not rows:
                continue
            self.state["phase"] = kind
            last_end = time.time()
            n = batch_files[kind]
            slices = iter([rows[i:i + n] for i in range(0, len(rows), n)])
            # Пока GPU считает текущую пачку, потоки уже читают следующую
            futures = deque()

            in_procs = kind in ("text", "code")

            def submit_next():
                sl = next(slices, None)
                if sl is not None:  # каждый файл — отдельная задача, файлы пачки разбираются параллельно
                    pool = self.procs if in_procs else self.pool  # пул процессов мог быть перезапущен
                    futures.append((sl, [pool.submit(extract.run_job, r[2], r[1], self.cfg) for r in sl]))

            submit_next()
            while futures:
                sl, futs = futures.popleft()
                t0 = time.time()
                results = [(r, *self._result(f, in_procs)) for r, f in zip(sl, futs)]
                if self.stop_flag or self.rebuild_requested:
                    return
                paused = self.wait_if_paused()
                submit_next()
                t1 = time.time()
                n_items = self._store(kind, results)
                now = time.time()
                log.info("пачка %s: файлов %d, фрагментов %d, ждал извлечения %.1f с, модель+запись %.1f с",
                         kind, len(sl), n_items, t1 - t0, now - t1)
                # Скорость для оценки «осталось около N минут»: время пачки без пауз, сглаженное
                per_file = max(0.0, now - last_end - paused) / len(sl)
                last_end = now
                old = self.rates.get(kind)
                self.rates[kind] = per_file if old is None else old * 0.8 + per_file * 0.2
                self.state["current"] = os.path.basename(sl[-1][1])
                # Свежие изменения на диске (в т.ч. удаления) не ждут конца долгого прохода
                self.apply_changes()

    def _encode_per_file(self, kind, payloads, meta):
        """Пачка упала — считаем по файлам, чтобы ошибка одного файла не губила остальные."""
        by_file = {}
        for i, m in enumerate(meta):
            by_file.setdefault(m[0], []).append(i)
        ok_meta, ok_vecs, failed = [], [], []
        for fid, idx in by_file.items():
            try:
                v = self._encode(kind, [payloads[i] for i in idx])
            except Exception as ex:
                failed.append((f"модель: {type(ex).__name__}: {ex}"[:200], fid))
                continue
            ok_meta += [meta[i] for i in idx]
            ok_vecs.append(v)
        vecs = np.concatenate(ok_vecs) if ok_vecs else np.zeros((0, core.DIM), np.float32)
        return ok_meta, vecs, failed

    def _store(self, kind, results):
        payloads, meta, failed = [], [], []
        for (fid, path, _, _), items, err in results:
            if err:
                failed.append((err, fid))
                continue
            for loc, label, snippet, payload in items:
                payloads.append(payload)
                meta.append((fid, loc, label, snippet))
        vecs = np.zeros((0, core.DIM), np.float32)
        if payloads:
            try:
                vecs = self._encode(kind, payloads)
            except Exception:
                log.exception("ошибка эмбеддинга пачки (%s) — пересчитываю по одному файлу", kind)
                self.emb.free_cache()
                meta, vecs, bad = self._encode_per_file(kind, payloads, meta)
                failed += bad
        mtimes = {fid: mtime for (fid, _, _, mtime), items, err in results if not err}
        done_fids = set(mtimes) - {f for _, f in failed}
        durations = []
        if kind in ("audio", "video"):  # длительность нужна интерфейсу (таймлайн, «совпал отрезок … из 1:52»)
            paths = {fid: path for (fid, path, _, _), items, err in results}
            durations = [(extract.media_duration(paths[f]), f) for f in done_fids]
        with self.conn:
            self.conn.executemany("UPDATE files SET duration=? WHERE id=?", durations)
            # Файл могли удалить или изменить, пока он считался — такие результаты выбрасываем
            # (изменённый останется со status=0 и посчитается заново)
            alive = {fid for fid, mtime in self.conn.execute(
                f"SELECT id, mtime FROM files WHERE status=0 AND id IN ({','.join('?' * len(done_fids))})",
                list(done_fids)) if mtime == mtimes[fid]} if done_fids else set()
            new_ids = []
            for (fid, loc, label, snippet), v in zip(meta, vecs):
                if fid not in alive:
                    continue
                cur = self.conn.execute(
                    "INSERT INTO chunks(file_id, kind, loc, label, snippet, vec) VALUES (?,?,?,?,?,?)",
                    (fid, kind, loc, label, snippet, v.astype(np.float16).tobytes()))
                new_ids.append((cur.lastrowid, v))
            self.conn.executemany("UPDATE files SET status=1 WHERE id=?", [(f,) for f in alive])
            self.conn.executemany("UPDATE files SET status=2, err=? WHERE id=? AND status=0", failed)
        if new_ids:
            self.store.add(kind, [i for i, _ in new_ids], np.stack([v for _, v in new_ids]))
        return len(payloads)

    # ------------------------------------------------ индекс имён (весь диск, отдельный поток и база)

    def names_loop(self):
        core.lower_priority()
        conn = core.connect_names()
        self.state["names"] = conn.execute(f"SELECT count(*) FROM names_t_{core.names_active(conn)}").fetchone()[0]
        # Время прошлой полной сборки переживает перезапуск — иначе каждый старт обходил бы весь диск
        row = conn.execute("SELECT v FROM meta WHERE k='last_scan'").fetchone()
        if row:
            self.state["names_updated"] = float(row[0])
            if not self.full_scan_requested_names:
                self.last_name_scan = float(row[0])
        while not self.stop_flag:
            try:
                if self.engine_active():
                    # Имена ведёт движок на Rust (MFT/USN) — свой индекс не строим и не обновляем
                    self.name_ops.clear()
                else:
                    if time.time() - self.last_name_scan > self.cfg["name_rescan_minutes"] * 60:
                        self.scan_names(conn)
                    self.apply_name_ops(conn)
            except Exception:
                log.exception("ошибка индекса имён")
                self.state["names_scan"] = None
                self.last_name_scan = time.time()
            self.names_wake.wait(2)
            self.names_wake.clear()

    engine_active = staticmethod(lambda: False)  # app подменяет: движок имён на связи?

    def apply_name_ops(self, conn):
        if not self.name_ops:
            return
        g = core.names_active(conn)
        with conn:
            while self.name_ops:
                op, p = self.name_ops.popleft()
                if op == "del":
                    core.names_delete(conn, g, p)
                elif os.path.isdir(p):
                    for path, _, _, _ in walk([p], NAME_EXCLUDE_DIRS, [], False, lambda: self.stop_flag):
                        core.names_add(conn, g, path)
                elif os.path.isfile(p):
                    core.names_add(conn, g, p)

    def scan_names(self, conn):
        """Полная пересборка в неактивное поколение и переключение. События за это время копятся
        в name_ops и применяются уже к новому поколению."""
        self.state["names_scan"] = 0
        t0, n = time.time(), 0
        old = core.names_active(conn)
        new = "b" if old == "a" else "a"
        core.names_drop(conn, new)
        core.names_create(conn, new)
        with ProcessPoolExecutor(1, initializer=throttle.background_process) as ex:
            fut = ex.submit(build_names, new, self.cfg["name_roots"])
            while not fut.done():
                time.sleep(2)
                self.state["names_scan"] = conn.execute(f"SELECT max(id) FROM names_t_{new}").fetchone()[0] or 0
            fut.result()
        if self.stop_flag:
            return
        n = conn.execute(f"SELECT count(*) FROM names_t_{new}").fetchone()[0]
        with conn:
            conn.execute(f"INSERT INTO names_{new}(names_{new}) VALUES ('rebuild')")
            conn.execute("UPDATE meta SET v=? WHERE k='active'", (new,))
            conn.execute("INSERT OR REPLACE INTO meta(k, v) VALUES ('last_scan', ?)", (str(time.time()),))
        core.names_drop(conn, old)
        self.last_name_scan = time.time()
        self.state.update(names=n, names_scan=None, names_updated=self.last_name_scan)
        log.info("индекс имён: %d файлов за %.0f с", n, time.time() - t0)
