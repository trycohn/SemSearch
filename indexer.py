"""Фоновая индексация: обход папок, отслеживание изменений, извлечение, эмбеддинги, индекс имён."""
import logging
import os
import re
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
import winfs

log = logging.getLogger("indexer")

NAME_EXCLUDE_DIRS = ["$Recycle.Bin", "System Volume Information", "WinSxS"]
QUIET_SEC = 3  # файл должен «успокоиться» (докопироваться/докачаться), прежде чем его индексировать


def _moved(old, new):
    """Время изменения файла поменялось? Допуск 1 с (разное разрешение файловых систем); ровно на час — нет: FAT/exFAT
    (флешки, карты памяти) хранят местное время, и при смене летнего времени оно «уезжает» на час у всех файлов."""
    d = abs(old - new)
    return d > 1 and abs(d - 3600) > 2


def walk(roots, exclude_dirs, exclude_paths, skip_dot_dirs, should_stop, smart=False, names=False):
    """Быстрый обход через os.scandir. Отдаёт (path, name, size, mtime).
    smart — пропускать папки сборки и зависимостей, узнавая их по соседним файлам проекта (core.is_build_dir).
    Не трогает ссылки/junction (петли) и облачные заглушки OneDrive без данных на диске (winfs.skip_entry);
    names — обход для индекса имён: файлы-заглушки в нём нужны (читать их не придётся)."""
    excl = {d.lower() for d in exclude_dirs}
    excl_paths = tuple(os.path.normcase(p) for p in exclude_paths)
    stack = list(roots)
    while stack:
        if should_stop():
            return
        d = stack.pop()
        sd = winfs.long_path(d)  # папки глубже 247 знаков иначе не открываются; пути в индексе — без префикса \\?\
        try:
            with os.scandir(sd) as it:
                entries = list(it)
        except OSError:
            continue
        siblings = [e.name for e in entries] if smart else ()
        for e in entries:
            try:
                st = e.stat(follow_symlinks=False)
            except OSError:
                continue
            if winfs.skip_entry(st, keep_cloud_files=names):
                continue
            path = e.path if sd is d else os.path.join(d, e.name)
            if stat.S_ISDIR(st.st_mode):
                low = e.name.lower()
                if low in excl or (skip_dot_dirs and e.name.startswith(".")):
                    continue
                if excl_paths and os.path.normcase(path).startswith(excl_paths):
                    continue
                if smart and core.is_build_dir(e.name, siblings, winfs.long_path(path)):
                    continue
                stack.append(path)
            else:
                yield path, e.name, st.st_size, st.st_mtime


def build_names(gen, roots):
    """Запускается в отдельном процессе: обход всего диска — миллионы итераций Python,
    в потоке они отнимали бы GIL у потока, который кормит видеокарту."""
    conn = core.connect_names()
    sql = f"INSERT OR IGNORE INTO names_t_{gen}(path, name) VALUES (?,?)"
    buf = []
    for path, name, _, _ in walk(roots, NAME_EXCLUDE_DIRS, [], False, lambda: False, names=True):
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
        self._hashes = {}  # id файла -> хеш содержимого (пока файл считается)
        self._later_copies = {}  # id файла -> id таких же файлов из той же пачки
        self.last_name_scan = 0.0
        # Картинки/видео/аудио декодируются в C-коде без GIL — хватает потоков.
        # PDF и документы разбираются на чистом Python — в отдельных процессах, иначе они душат поток,
        # который кормит видеокарту.
        # Много параллельных чтений ускоряют SSD, но тормозят HDD, флешки и сеть (головки прыгают между файлами)
        try:
            slow = any(winfs.is_slow_volume(r) for r in cfg["content_roots"])
        except Exception:
            slow = False
        if slow:
            log.info("папки поиска на HDD, съёмном или сетевом диске — читаю файлы в меньшее число потоков")
        self.pool = ThreadPoolExecutor(3 if slow else 6, thread_name_prefix="extract",
                                       initializer=throttle.background_thread)
        self.procs = ProcessPoolExecutor(2 if slow else max(2, min(6, (os.cpu_count() or 4) // 3)),
                                         initializer=extract.worker_init)
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
            # Обход и пересборка видеокарту не нагружают — пауза их не держит (пачка будет отброшена, см. _process_kind)
            if self._retag_needed and self.engine_ready():
                self.full_scan_requested = True
            if self.full_scan_requested or self.rebuild_requested:
                return time.time() - t0
            time.sleep(3)
        return time.time() - t0

    # Сколько секунд в среднем уходит на файл, пока своих замеров нет (замер на RTX 4070 SUPER)
    DEFAULT_RATES = {"text": 0.12, "code": 0.025, "image": 0.045, "audio": 1.5, "video": 10.0}

    def eta(self, remaining):
        """remaining: {вид: сколько файлов осталось} -> секунд до конца или None."""
        if not any(remaining.values()):
            return None
        # На другом железе (процессор вместо видеокарты) таблица врёт в разы: виды, до которых дело ещё не дошло,
        # оцениваем по ней, но с поправкой на то, во сколько раз эта машина отличается от неё на уже измеренных
        ratios = [r / self.DEFAULT_RATES[k] for k, r in self.rates.items() if r > 0 and k in self.DEFAULT_RATES]
        scale = sum(ratios) / len(ratios) if ratios else 1.0
        return int(sum(n * self.rates.get(k, self.DEFAULT_RATES[k] * scale) for k, n in remaining.items()))

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
        # Ошибки модели могли быть временными (нехватка памяти, исправленный баг), как и ошибки чтения: файл был
        # занят программой или антивирусом, диск отключён, путь был длиннее 259 знаков — даём таким файлам ещё попытку
        with self.conn:
            retry = self.conn.execute("UPDATE files SET status=0, err=NULL WHERE status=2 AND (err LIKE 'модель:%' "
                                      "OR err LIKE 'PermissionError:%' OR err LIKE 'FileNotFoundError:%' "
                                      "OR err LIKE 'OSError:%')").rowcount
        if retry:
            log.info("повторно в очередь после ошибок модели: %d", retry)
        threading.Thread(target=self.names_loop, name="names", daemon=True).start()
        last_full = 0.0
        while not self.stop_flag:
            try:
                if self.rebuild_requested:
                    self._rebuild()
                if self._retag_needed and self.engine_ready():
                    self.full_scan_requested = True  # движок достроил индекс — теперь обход с признаками чужого кода
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
    engine_ready = staticmethod(lambda: False)  # app подставляет: движок построил индекс?
    _retag_needed = False  # обход прошёл без признаков движка — повторить, как только он будет готов

    def _content_files(self):
        """-> (файлы (path, name, size, mtime, tag), есть ли признаки tag от движка)."""
        if self.engine_lister:
            try:
                files = self.engine_lister()
                self._retag_needed = False
                return files
            except RuntimeError as ex:  # ожидаемо: движок ещё строит индекс
                log.info("%s — обхожу папки сам", ex)
                self._retag_needed = True
            except Exception:
                log.exception("движок не отдал список файлов — обхожу папки сам")
        files = walk(self.cfg["content_roots"], self.cfg["exclude_dirs"], self.cfg["exclude_paths"], True,
                     lambda: self.stop_flag, smart=True)
        return ((p, n, s, m, None) for p, n, s, m in files), False

    # Что делать с файлом: правило «чужого кода» (настройка foreign_code). Касается кода и документации
    # разработчика (.md/.rst — README и CHANGELOG библиотек); ваши PDF, Word и фото не трогает.
    DEV_DOCS = (".md", ".markdown", ".rst")
    GEN_NAME = re.compile(r"(_pb2(_grpc)?\.py|\.pb\.(go|cc|h)|\.g\.cs|\.g\.i\.cs|\.designer\.cs|\.generated\.\w+)$",
                          re.I)
    TAG_REASON = {"submodule": "чужой код: git-подмодуль",
                  "gitignored": "не исходник: так считает .gitignore проекта",
                  "archive": "чужой код: скачанный архив исходников"}

    def _policy(self, kind, name, tag):
        """-> (status, prio, причина) для нового или изменённого файла."""
        mode = self.cfg.get("foreign_code", "skip")
        lname = name.lower()
        if mode == "normal" or not (kind == "code" or (kind == "text" and lname.endswith(self.DEV_DOCS))):
            return core.ST_PENDING, 0, None
        why = self.TAG_REASON.get(tag) or (extract.GENERATED if kind == "code" and self.GEN_NAME.search(name)
                                           else None)
        if why:  # точные признаки: не индексировать (или в конец очереди, если так выбрано)
            return (core.ST_SKIPPED, 0, why) if mode == "skip" else (core.ST_PENDING, 1, None)
        if tag == "foreign" or lname.endswith(".d.ts"):  # неточные признаки — только в конец очереди
            return core.ST_PENDING, 1, None
        return core.ST_PENDING, 0, None

    def _content_verdict(self, kind, old, tag, status, prio, why):
        """Обход видит только имя и признаки, а run_job решает ещё и по шапке файла: сгенерированный — пропустить
        (или в конец очереди), баннер библиотеки — в конец. Для неизменённого файла с теми же признаками такое решение
        остаётся в силе — иначе каждый обход ставил бы файл в очередь заново, а разбор снова его пропускал.
        old — (id, size, mtime, status, prio, tag, err) из базы. -> (status, prio, причина)"""
        mode = self.cfg.get("foreign_code", "skip")
        if kind != "code" or mode == "normal" or tag != old[5] or status == core.ST_SKIPPED:
            return status, prio, why
        if old[3] == core.ST_SKIPPED and old[6] == extract.GENERATED and mode == "skip":
            return core.ST_SKIPPED, old[4] or 0, old[6]
        if (old[4] or 0) == 1 and prio == 0:
            return status, 1, why
        return status, prio, why

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

    def _away_roots(self):
        """Папки поиска, которых сейчас нет: отключённая флешка, недоступный сетевой диск. Пропавшие ещё до запуска
        лежат в cfg["content_roots_missing"] (их заполняет app). Файлы в них не пропали, а недоступны: индекс не стираем
        (иначе после возвращения диска всё считалось бы заново) и читать их не пытаемся."""
        exists = getattr(core, "_dir_exists", os.path.isdir)  # у core — с ограничением по времени для сетевых папок
        missing = list(self.cfg.get("content_roots_missing") or ())
        return [r for r in self.cfg["content_roots"] if not exists(r)] + missing

    def scan_content(self):
        self.full_scan_requested = False  # запрошенный обход — вот он
        self.state.update(phase="scan", current="")
        known = {p: (i, s, m, st, pr, tg, er) for i, p, s, m, st, pr, tg, er in
                 self.conn.execute("SELECT id, path, size, mtime, status, prio, tag, err FROM files")}
        files, tagged = self._content_files()
        seen, new, changed, repol = set(), [], [], []
        for path, name, size, mtime, tag in files:
            kind = self._kind(name, size)
            if not kind:
                continue
            seen.add(path)
            old = known.get(path)
            if old and not tagged:
                tag = old[5]  # движка нет — признаков тоже нет; держимся прежних, а не индексируем чужое заново
            status, prio, why = self._policy(kind, name, tag)
            if old is None:
                new.append((path, name, kind, size, mtime, status, prio, tag, why))
            elif old[1] != size or _moved(old[2], mtime):
                changed.append((old[0], size, mtime, status, prio, tag, why))
            else:
                status, prio, why = self._content_verdict(kind, old, tag, status, prio, why)
                if (status == core.ST_SKIPPED) != (old[3] == core.ST_SKIPPED) or (prio, tag) != (old[4] or 0, old[5]):
                    # файл не менялся, но правило для него поменялось (настройка, новый .gitignore, подмодуль)
                    repol.append((old[0], old[3], status, prio, tag, why))
        if self.stop_flag:
            return
        away = self._away_roots()
        gone = [v[0] for p, v in known.items() if p not in seen and not (away and core.under(p, away))]
        if len(gone) > max(1000, len(known) // 2):
            # Разом «пропало» больше половины индекса — так бывает, если источник списка ответил не полностью
            # (движок ещё строит индекс). Перепроверяем своим обходом, прежде чем стирать посчитанное.
            log.warning("обход: пропало %d из %d файлов — перепроверяю обходом папок", len(gone), len(known))
            really = {path for path, *_ in walk(self.cfg["content_roots"], self.cfg["exclude_dirs"],
                                               self.cfg["exclude_paths"], True, lambda: self.stop_flag, smart=True)}
            if self.stop_flag:
                return
            gone = [v[0] for p, v in known.items() if p not in really and not (away and core.under(p, away))]
            log.warning("после перепроверки пропало %d", len(gone))
        with self.conn:
            self.conn.executemany("INSERT INTO files(path, name, kind, size, mtime, status, prio, tag, err) "
                                  "VALUES (?,?,?,?,?,?,?,?,?)", new)
            for fid, size, mtime, status, prio, tag, why in changed:
                self._drop_chunks(fid)
                self.conn.execute("UPDATE files SET size=?, mtime=?, status=?, prio=?, tag=?, err=?, hash=NULL, "
                                  "copy_of=NULL WHERE id=?", (size, mtime, status, prio, tag, why, fid))
            for fid, old_status, status, prio, tag, why in repol:
                if status == core.ST_SKIPPED:  # стал «чужим» — убрать из индекса
                    self._drop_chunks(fid)
                    self.conn.execute("UPDATE files SET status=?, prio=?, tag=?, err=?, copy_of=NULL WHERE id=?",
                                      (status, prio, tag, why, fid))
                elif old_status == core.ST_SKIPPED:  # снова нужен — в очередь
                    self.conn.execute("UPDATE files SET status=0, prio=?, tag=?, err=NULL WHERE id=?", (prio, tag, fid))
                else:
                    self.conn.execute("UPDATE files SET prio=?, tag=? WHERE id=?", (prio, tag, fid))
            for fid in gone:
                self._drop_chunks(fid)
                self.conn.execute("DELETE FROM files WHERE id=?", (fid,))
        self.state["phase"] = "idle"
        skipped = sum(1 for v in new if v[5] == core.ST_SKIPPED) + sum(1 for v in repol if v[2] == core.ST_SKIPPED)
        log.info("обход: новых %d, изменённых %d, удалённых %d, правило поменялось у %d (пропущено как чужое: %d)",
                 len(new), len(changed), len(gone), len(repol), skipped)

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
        # оригинал удалён или изменился — его копии больше не копии: в очередь, одна из них станет оригиналом
        self.conn.execute("UPDATE files SET status=0, copy_of=NULL WHERE copy_of=?", (fid,))

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
                try:
                    st = os.stat(winfs.long_path(p), follow_symlinks=False)
                except (FileNotFoundError, NotADirectoryError):
                    n += self._delete_under(p)
                    continue
                except OSError:  # занят, нет доступа, том отключён: не значит «удалён» — оставляем, обход разберётся
                    continue
                if winfs.skip_entry(st):  # ссылка, junction, облачная заглушка
                    continue
                if stat.S_ISDIR(st.st_mode):
                    if self._in_scope(p, True):
                        for path, name, size, mtime in walk([p], self.cfg["exclude_dirs"], self.cfg["exclude_paths"],
                                                            True, lambda: self.stop_flag, smart=True):
                            n += self._upsert(path, name, size, mtime)
                elif stat.S_ISREG(st.st_mode):
                    if self._in_scope(p, False):
                        n += self._upsert(p, os.path.basename(p), st.st_size, st.st_mtime)
        if n:
            log.info("изменения на диске: %d", n)
        return n

    def _sibling_tag(self, path):
        """Признак движка для файла из события: у соседей по папке он почти всегда тот же
        (подмодуль, архив, клон — свойства папки). Полный обход потом уточнит."""
        lo, hi = core.prefix_range(os.path.dirname(path))
        row = self.conn.execute("SELECT tag FROM files WHERE path >= ? AND path < ? AND tag IS NOT NULL LIMIT 1",
                                (lo, hi)).fetchone()
        return row[0] if row else None

    def _upsert(self, path, name, size, mtime):
        kind = self._kind(name, size)
        if not kind:
            return 0
        row = self.conn.execute("SELECT id, size, mtime, tag FROM files WHERE path=?", (path,)).fetchone()
        tag = row[3] if row and row[3] else self._sibling_tag(path)
        status, prio, why = self._policy(kind, name, tag)
        if row is None:
            self.conn.execute("INSERT INTO files(path, name, kind, size, mtime, status, prio, tag, err) "
                              "VALUES (?,?,?,?,?,?,?,?,?)", (path, name, kind, size, mtime, status, prio, tag, why))
            return 1
        fid, old_size, old_mtime, _ = row
        if old_size == size and not _moved(old_mtime, mtime):
            return 0
        self._drop_chunks(fid)
        self.conn.execute("UPDATE files SET size=?, mtime=?, status=?, prio=?, tag=?, err=?, hash=NULL, copy_of=NULL "
                          "WHERE id=?", (size, mtime, status, prio, tag, why, fid))
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
            return None, "разбор не уложился в 3 минуты", None
        except (BrokenProcessPool, CancelledError):
            self._reset_procs()
            return None, "сбой разбора файла", None
        except Exception as ex:
            return None, f"{type(ex).__name__}: {ex}"[:200], None

    def _safe_hash(self, row):
        try:
            return extract.file_hash(row[1], row[2])
        except OSError:
            return None

    def _dedupe(self, rows, hashes):
        """Точные копии уже проиндексированного файла не разбираются и не считаются на видеокарте —
        запоминаются копиями (status 5). hashes — хеши rows (_safe_hash). -> строки, которые нужно разбирать."""
        keep, first, copies = [], {}, []
        for r, h in zip(rows, hashes):
            fid = r[0]
            if h is None:
                keep.append(r)
                continue
            self._hashes[fid] = h
            orig = self.conn.execute("SELECT id FROM files WHERE hash=? AND status=1 AND id<>? LIMIT 1",
                                     (h, fid)).fetchone()
            if orig:
                copies.append((orig[0], h, fid))
            elif h in first:  # такой же, как другой файл этой пачки — решится, когда тот посчитается
                self._later_copies.setdefault(first[h], []).append(fid)
            else:
                first[h] = fid
                keep.append(r)
        if copies:
            with self.conn:
                self.conn.executemany("UPDATE files SET status=5, copy_of=?, hash=? WHERE id=? AND status=0", copies)
        return keep

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
            scans = [i for i, p in enumerate(payloads) if isinstance(p, bytes)]  # страницы PDF-сканов (JPEG)
            if not scans:
                return self.emb.documents(payloads, background=True)
            texts = [i for i, p in enumerate(payloads) if not isinstance(p, bytes)]
            vecs = np.zeros((len(payloads), core.DIM), np.float32)
            vecs[texts] = self.emb.documents([payloads[i] for i in texts], background=True)
            vecs[scans] = self.emb.images([payloads[i] for i in scans], background=True)
            return vecs
        if kind in ("image", "video"):
            return self.emb.images(payloads, background=True)
        return self.emb.audio(payloads, background=True)

    def process_pending(self):
        total = self.conn.execute("SELECT count(*) FROM files WHERE status=0").fetchone()[0]
        if not total:
            return
        batch_files = {"text": 32, "code": 32, "image": 32, "audio": 4, "video": 1}
        away = self._away_roots()  # один раз на проход: проверка недоступной сетевой папки занимает секунды
        # Сначала обычные файлы всех видов, потом отложенные (prio 1: возможно чужой код) — они не задерживают ваши
        for prio in (0, 1):
            for kind in self.ORDER:
                if self._process_kind(kind, prio, batch_files[kind], away) is False:
                    return

    def _process_kind(self, kind, prio, n, away=()):
        rows = self.conn.execute("SELECT id, path, kind, mtime FROM files WHERE status=0 AND kind=? AND prio=?",
                                 (kind, prio)).fetchall()
        if away:  # файлы с отключённого диска не пытаемся читать: все «не прочитались» бы навсегда
            rows = [r for r in rows if not core.under(r[1], away)]
        if not rows:
            return True
        self.state["phase"] = kind
        last_end = time.time()
        slices = iter([rows[i:i + n] for i in range(0, len(rows), n)])
        # Пока GPU считает текущую пачку, потоки уже разбирают следующую, а хеши для поиска копий считаются
        # ещё на пачку вперёд: синхронно они занимали ~0,2 с перед каждой пачкой, и видеокарта ждала
        futures, hashing = deque(), deque()
        in_procs = kind in ("text", "code")

        def prefetch():
            sl = next(slices, None)
            if sl is not None:
                hashing.append((sl, [self.pool.submit(self._safe_hash, r) for r in sl]))

        def submit_next():
            while hashing:
                sl, hashes = hashing.popleft()
                prefetch()
                sl = self._dedupe(sl, [f.result() for f in hashes])  # копии уже посчитанного — без разбора и GPU
                if sl:
                    # каждый файл — отдельная задача, файлы пачки разбираются параллельно
                    pool = self.procs if in_procs else self.pool  # пул процессов мог быть перезапущен
                    futures.append((sl, [pool.submit(extract.run_job, r[2], r[1], self.cfg, prio) for r in sl]))
                    return

        prefetch()
        submit_next()
        while futures:
            sl, futs = futures.popleft()
            t0 = time.time()
            results = [(r, *self._result(f, in_procs)) for r, f in zip(sl, futs)]
            if self.stop_flag or self.rebuild_requested:
                return False
            paused = self.wait_if_paused()
            if self.stop_flag or self.full_scan_requested or self.rebuild_requested:
                self.wake.set()  # сначала обход/пересборка; разобранная пачка останется в очереди
                return False
            submit_next()
            t1 = time.time()
            n_items = self._store(kind, results)
            now = time.time()
            log.info("пачка %s%s: файлов %d, фрагментов %d, ждал извлечения %.1f с, модель+запись %.1f с",
                     kind, " (отложенные)" if prio else "", len(sl), n_items, t1 - t0, now - t1)
            # Скорость для оценки «осталось около N минут»: время пачки без пауз, сглаженное
            per_file = max(0.0, now - last_end - paused) / len(sl)
            last_end = now
            old = self.rates.get(kind)
            self.rates[kind] = per_file if old is None else old * 0.8 + per_file * 0.2
            self.state["current"] = os.path.basename(sl[-1][1])
            # Свежие изменения на диске (в т.ч. удаления) не ждут конца долгого прохода
            self.apply_changes()
            if self._retag_needed and self.engine_ready():
                # Очередь собрана без признаков движка; он готов — прервёмся на обход с признаками,
                # чтобы не тратить видеокарту на чужой код
                self.full_scan_requested = True
                self.wake.set()
                return False
        return True

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
        payloads, meta, failed, skipped, deferred = [], [], [], [], []
        for (fid, path, _, _), items, err, flag in results:
            if flag == "skip":  # сгенерированный файл — правило, а не ошибка
                skipped.append((err, fid))
                continue
            if flag == "defer":  # похоже на чужую библиотеку — во второй проход
                deferred.append((fid,))
                continue
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
        mtimes = {fid: mtime for (fid, _, _, mtime), items, err, flag in results if not err and not flag}
        done_fids = set(mtimes) - {f for _, f in failed}
        durations = []
        if kind in ("audio", "video"):  # длительность нужна интерфейсу (таймлайн, «совпал отрезок … из 1:52»)
            paths = {fid: path for (fid, path, _, _), items, err, flag in results}
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
            # хеш оригинала — по нему найдутся его будущие копии
            self.conn.executemany("UPDATE files SET status=1, hash=? WHERE id=?",
                                  [(self._hashes.get(f), f) for f in alive])
            self.conn.executemany("UPDATE files SET status=2, err=? WHERE id=? AND status=0", failed)
            self.conn.executemany("UPDATE files SET status=3, err=? WHERE id=? AND status=0", skipped)
            self.conn.executemany("UPDATE files SET prio=1 WHERE id=? AND status=0", deferred)
            # Такие же файлы из этой пачки: оригинал посчитан — они его копии; пропущен или не прочитался —
            # с одинаковым содержимым будет то же самое
            for f in alive:
                self.conn.executemany("UPDATE files SET status=5, copy_of=?, hash=? WHERE id=? AND status=0",
                                      [(f, self._hashes.get(f), c) for c in self._later_copies.get(f, ())])
            for status, rows in ((2, failed), (3, skipped)):
                for err, f in rows:
                    self.conn.executemany("UPDATE files SET status=?, err=? WHERE id=? AND status=0",
                                          [(status, err, c) for c in self._later_copies.get(f, ())])
        for (fid, *_), *_rest in results:
            self._hashes.pop(fid, None)
            self._later_copies.pop(fid, None)
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
                    continue
                try:
                    st = os.stat(winfs.long_path(p), follow_symlinks=False)
                except OSError:
                    continue
                if winfs.skip_entry(st, keep_cloud_files=True):  # ссылка, junction, папка-заглушка
                    continue
                if stat.S_ISDIR(st.st_mode):
                    for path, _, _, _ in walk([p], NAME_EXCLUDE_DIRS, [], False, lambda: self.stop_flag, names=True):
                        core.names_add(conn, g, path)
                elif stat.S_ISREG(st.st_mode):
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
