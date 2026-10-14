"""Общие вещи: конфиг, база, модель, хранилище векторов, детектор игр."""
import ctypes
import errno
import gc
import json
import logging
import os
import re
import shutil
import sqlite3
import threading
import uuid
from ctypes import wintypes

import numpy as np

log = logging.getLogger("core")

APP_DIR = os.path.dirname(os.path.abspath(__file__))  # папка программы: во время работы только читается
FOLDERID_LOCAL_APP_DATA = "F1B32785-6FBA-4FCF-9D55-7B8E7F157091"


class _GUID(ctypes.Structure):
    _fields_ = [("Data1", wintypes.DWORD), ("Data2", wintypes.WORD), ("Data3", wintypes.WORD),
                ("Data4", ctypes.c_ubyte * 8)]


def known_folder(folder_id):
    """Путь известной папки Windows (SHGetKnownFolderPath, FOLDERID_*) или None, если её нет."""
    path = ctypes.c_wchar_p()
    guid = _GUID.from_buffer_copy(uuid.UUID(folder_id).bytes_le)
    try:
        ok = ctypes.windll.shell32.SHGetKnownFolderPath(ctypes.byref(guid), 0, None, ctypes.byref(path)) == 0
        return path.value if ok else None
    finally:
        ctypes.windll.ole32.CoTaskMemFree(path)  # освобождать нужно и при ошибке


def abs_env(name):
    """Переменная окружения, если в ней абсолютный путь (с диском или \\\\сервер\\папка), иначе None."""
    v = os.environ.get(name, "").strip()
    return v if os.path.splitdrive(v)[0] and os.path.isabs(v) else None


def data_dir():
    """Папка данных: SEMSEARCH_DATA_DIR (только абсолютный путь), иначе %LOCALAPPDATA%\\SemSearch.
    Shell и движок ищут её так же (docs/API.md). Здесь настройки, индексы, превью, журналы и токен."""
    d = abs_env("SEMSEARCH_DATA_DIR")
    if d:
        return os.path.normpath(d)
    base = known_folder(FOLDERID_LOCAL_APP_DATA) or abs_env("LOCALAPPDATA")
    if not base:
        profile = abs_env("USERPROFILE")
        if profile:
            base = os.path.join(profile, "AppData", "Local")
        else:  # в крайнем случае — временная папка, как std::env::temp_dir() в shell и движке
            import tempfile

            base = tempfile.gettempdir()
    return os.path.join(base, "SemSearch")


DATA_DIR = data_dir()
DB_PATH = os.path.join(DATA_DIR, "index.db")
CONFIG_PATH = os.path.join(DATA_DIR, "config.json")
LEGACY_CONFIG_PATH = os.path.join(APP_DIR, "config.json")  # старая раскладка: читаем, пока не перенесён
MODEL_ID = "google/embeddinggemma-2"
DIM = 256  # Matryoshka: 768 -> 256, индекс в 3 раза меньше почти без потери качества

HOME = os.path.expanduser("~")
# Документы, Рабочий стол, Изображения, Видео, Музыка, Загрузки — их часто переносят на другой диск
PROFILE_FOLDERS = ("FDD39AD0-238F-46AF-ADB4-6C85480369C7", "B4BFCC3A-DB2C-424C-B029-7FE99A87C641",
                   "33E28130-4E1E-4676-835A-98395C3BC3BB", "18989B1D-99B5-455B-841C-AB7C74E4DDFC",
                   "4BD8D571-6D19-48D3-BE97-422220080E43", "374DE290-123F-4565-9164-39C4925E467B")

DEFAULT_CONFIG = {
    # Где искать по содержимому (смыслу); load_config добавляет известные папки вне HOME (default_content_roots)
    "content_roots": [HOME],
    # Где искать по имени файла: диски целиком (из них же выводится name_roots); None — все несъёмные диски
    "name_drives": None,
    # Папки с такими именами пропускаются везде (плюс все папки, начинающиеся с точки, в content_roots).
    # Сборки и зависимости (bin/obj, build/dist, Library в Unity, venv…) узнаются по соседним файлам проекта —
    # см. is_build_dir; поэтому обычная папка «Library» с книгами или «Logs» с документами не пропадает.
    "exclude_dirs": ["node_modules", "__pycache__", "site-packages", "AppData", "$Recycle.Bin",
                     "System Volume Information", "WinSxS"],
    # Чужой код (git-подмодули, скачанные архивы исходников, то, что .gitignore проекта считает не исходниками,
    # сгенерированные файлы): skip — не индексировать, last — в последнюю очередь, normal — как обычный
    "foreign_code": "skip",
    "exclude_paths": [],
    "port": 8765,
    "hotkey": "ctrl+alt+s",
    "theme": "system",  # system | light | dark
    "accent": "#005fb8",
    # Пауза индексации: полноэкранное приложение (игра, фильм), запущенный процесс из списка, работа от батареи
    "pause_on_fullscreen": True,
    "pause_processes": [],
    "pause_on_battery": False,
    # Изменения ловятся сразу через уведомления Windows; полный обход — страховка от пропущенных событий
    "rescan_minutes": 360,
    "name_rescan_minutes": 1440,
    "video_frame_every_sec": 10,
    "video_max_frames": 40,
    "audio_segment_sec": 30,
    "audio_max_segments": 10,
    "max_text_mb": 2,
    "max_pdf_pages": 80,
    "max_chunks_per_file": 40,
    "min_image_kb": 20,
    "min_image_side": 200,
}

TEXT_EXT = {".txt", ".md", ".markdown", ".rst", ".org", ".tex", ".pdf", ".docx"}
CODE_EXT = {
    ".py", ".js", ".mjs", ".cjs", ".ts", ".tsx", ".jsx", ".java", ".kt", ".kts", ".cs", ".cpp",
    ".cc", ".cxx", ".c", ".h", ".hpp", ".go", ".rs", ".rb", ".php", ".lua", ".sql", ".sh", ".ps1",
    ".bat", ".cmd", ".html", ".htm", ".css", ".scss", ".vue", ".svelte", ".swift", ".dart", ".gd",
    ".shader", ".hlsl", ".glsl", ".yaml", ".yml", ".toml", ".ini", ".cfg", ".gradle",
}
IMAGE_EXT = {".jpg", ".jpeg", ".png", ".webp", ".bmp", ".gif", ".tif", ".tiff"}
AUDIO_EXT = {".mp3", ".wav", ".flac", ".ogg", ".m4a", ".opus", ".aac", ".wma"}
VIDEO_EXT = {".mp4", ".mkv", ".avi", ".mov", ".webm", ".wmv", ".m4v"}
KINDS = ("text", "code", "image", "audio", "video")
CONTENT_EXT = TEXT_EXT | CODE_EXT | IMAGE_EXT | AUDIO_EXT | VIDEO_EXT


def kind_of(ext):
    if ext in TEXT_EXT:
        return "text"
    if ext in CODE_EXT:
        return "code"
    if ext in IMAGE_EXT:
        return "image"
    if ext in AUDIO_EXT:
        return "audio"
    if ext in VIDEO_EXT:
        return "video"
    return None


# Прежний список по умолчанию: исключал папки по одному имени. У кого в config.json он так и записан
# (то есть пользователь его не менял) — подменяем на новый, иначе умные правила не заработают.
OLD_DEFAULT_EXCLUDES = {"node_modules", "__pycache__", "venv", "env", "site-packages", "dist", "build", "obj", "bin",
                        "library", "temp", "target", "packages", "vendor", "appdata", "$recycle.bin",
                        "system volume information", "winsxs", "logs", "cache"}


def default_content_roots():
    """Папка пользователя и известные папки вне её (например, «Документы», перенесённые на D:).
    Вложенные не дублируются: обход корня и так их пройдёт."""
    roots = [HOME]
    for folder_id in PROFILE_FOLDERS:
        p = known_folder(folder_id)
        if p and not under(p, roots):
            roots = [r for r in roots if not under(r, [p])] + [p]
    return roots


def _config_source():
    """Откуда читать настройки: из папки данных, а если там их ещё нет — из старой папки программы (только чтение)."""
    for p in (CONFIG_PATH, LEGACY_CONFIG_PATH):
        if os.path.exists(p):
            return p
    return None


def load_config():
    cfg = dict(DEFAULT_CONFIG, content_roots=default_content_roots())
    src = _config_source()
    if src:
        with open(src, encoding="utf-8-sig") as f:  # -sig: Блокнот и PowerShell пишут BOM
            cfg.update(json.load(f))
        if cfg.get("name_drives") is None and cfg.get("name_roots"):  # конфиг старого формата
            cfg["name_drives"] = [r.rstrip("\\/") for r in cfg["name_roots"]]
    else:
        os.makedirs(DATA_DIR, exist_ok=True)
        with open(CONFIG_PATH, "w", encoding="utf-8") as f:
            json.dump(cfg, f, ensure_ascii=False, indent=2)
    if {d.lower() for d in cfg["exclude_dirs"]} == OLD_DEFAULT_EXCLUDES:
        cfg["exclude_dirs"] = list(DEFAULT_CONFIG["exclude_dirs"])
    # Пропавшие папки (отключённый диск, удалённая папка) пропускаем, но в файле оставляем — вдруг вернутся
    roots = [r for r in cfg["content_roots"] if os.path.isdir(r)]
    for r in cfg["content_roots"]:
        if r not in roots:
            log.warning("папки для поиска нет — пропускаю: %s", r)
    cfg["content_roots"] = roots or [HOME]
    derive_config(cfg)
    return cfg


ALWAYS_SKIP_DIRS = {"node_modules", "bower_components", "jspm_packages", "__pycache__", "site-packages",
                    "dist-packages"}


def is_build_dir(name, siblings, path):
    """Папка сборки или зависимостей — по соседним файлам проекта, а не по одному имени.
    siblings — имена в родительской папке. Те же правила в движке (list с smart_excludes)."""
    n = name.lower()
    if n in ALWAYS_SKIP_DIRS:
        return True
    sib = {s.lower() for s in siblings}
    has = lambda *names: any(x in sib for x in names)  # noqa: E731
    ext = lambda *exts: any(s.endswith(exts) for s in sib)  # noqa: E731
    if n in ("bin", "obj") and ext(".csproj", ".vbproj", ".fsproj", ".sln"):
        return True
    if n == "target" and has("cargo.toml", "pom.xml"):
        return True
    if n in ("build", "dist", "out") and has("package.json", "cmakelists.txt", "pyproject.toml", "setup.py",
                                              "build.gradle", "build.gradle.kts", "meson.build"):
        return True
    if n in ("library", "temp", "logs", "obj", "build", "builds") and has("assets") and has("projectsettings"):
        return True  # Unity
    if n in ("intermediate", "saved", "deriveddatacache", "binaries") and ext(".uproject"):
        return True  # Unreal
    if n == "vendor" and has("composer.json", "go.mod", "gemfile"):
        return True
    if n == "packages" and (ext(".sln") or has("packages.config")):
        return True
    if n == "pods" and has("podfile"):
        return True
    return os.path.exists(os.path.join(path, "pyvenv.cfg"))  # виртуальное окружение Python с любым именем


def derive_config(cfg):
    """Производные ключи — пересчитываются и после изменения настроек из интерфейса."""
    drives = cfg.get("name_drives")
    fixed = [d["drive"] for d in fixed_drives()]
    if not isinstance(drives, list):  # не задано (нет ключа, null) — все несъёмные диски
        drives = fixed
    else:
        drives = list(dict.fromkeys(str(d).upper().rstrip("\\/") for d in drives))
        for d in drives:
            if d not in fixed:
                log.warning("диск %s не найден среди несъёмных — по имени на нём не ищу", d)
        drives = [d for d in drives if d in fixed]
    # В памяти — всегда конкретный список: его показывают интерфейс и /api/status
    cfg["name_drives"] = drives
    cfg["name_roots"] = [d + "\\" for d in drives]
    # Собственные папки не индексируем никогда: иначе превью из thumbs попадут в «фото»
    own = {os.path.normcase(APP_DIR), os.path.normcase(DATA_DIR)}
    cfg["exclude_paths"] = [p for p in cfg["exclude_paths"] if os.path.normcase(p) not in own]
    cfg["exclude_paths"] += [APP_DIR, DATA_DIR]


EDITABLE = {  # ключ -> тип; только их можно менять через POST /api/settings
    "content_roots": list, "name_drives": list, "exclude_dirs": list, "hotkey": str, "theme": str, "accent": str,
    "pause_on_fullscreen": bool, "pause_processes": list, "pause_on_battery": bool,
    "video_frame_every_sec": int, "video_max_frames": int, "audio_segment_sec": int, "audio_max_segments": int,
    "max_text_mb": int, "max_pdf_pages": int, "foreign_code": str,
}


def save_config(changes):
    """Записывает изменения в config.json папки данных, не трогая остальные ключи файла (атомарно)."""
    data = {}
    src = _config_source()  # старый файл из папки программы при первой записи переезжает целиком
    if src:
        with open(src, encoding="utf-8-sig") as f:
            data = json.load(f)
    data.update(changes)
    data.pop("name_roots", None)  # выводится из name_drives
    os.makedirs(DATA_DIR, exist_ok=True)
    tmp = CONFIG_PATH + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
    os.replace(tmp, CONFIG_PATH)


def under(path, roots):
    """Корень из roots, внутри которого лежит path (или None)."""
    p = os.path.normcase(path)
    for r in roots:
        rn = os.path.normcase(r).rstrip("\\/")
        if p == rn or p.startswith(rn + os.sep):
            return r
    return None


def prefix_range(path):
    """Границы для выборки всего, что лежит внутри папки: path\\... (']' идёт сразу после '\\')."""
    p = path.rstrip("\\/")
    return p + "\\", p + "]"


def setup_logging():
    os.makedirs(DATA_DIR, exist_ok=True)
    logging.basicConfig(
        filename=os.path.join(DATA_DIR, "semsearch.log"), level=logging.INFO, encoding="utf-8",
        format="%(asctime)s %(levelname)s %(threadName)s: %(message)s",
    )
    for noisy in ("pypdf", "httpx", "urllib3", "huggingface_hub", "transformers", "sentence_transformers"):
        logging.getLogger(noisy).setLevel(logging.ERROR)


def _move(src, dst):
    """os.replace в пределах тома, на другой том — копированием (shutil.move)."""
    try:
        os.replace(src, dst)
    except OSError as ex:
        if ex.errno != errno.EXDEV:
            raise
        shutil.move(src, dst)


def _remove(path):
    if os.path.isdir(path) and not os.path.islink(path):
        shutil.rmtree(path)
    else:
        os.remove(path)


def migrate_legacy(legacy_root, data_dir):
    """Разовый перенос старой раскладки (legacy_root\\config.json и legacy_root\\data\\) в data_dir — до того,
    как открыты журнал, базы и настройки. Переносится только то, чего в data_dir ещё нет; база — вместе
    со своими -wal/-shm или никак (при сбое возвращается на место). Журналы и токен остаются.
    Никогда не бросает. -> [(уровень logging, сообщение)]: журнал в этот момент ещё не открыт."""
    old = os.path.join(legacy_root, "data")
    groups = ((legacy_root, ("config.json",)), (old, ("index.db", "index.db-wal", "index.db-shm")),
              (old, ("names.db", "names.db-wal", "names.db-shm")), (old, ("thumbs",)))
    notes = []
    for src_dir, names in groups:
        try:
            if (not os.path.exists(os.path.join(src_dir, names[0]))
                    or os.path.normcase(os.path.abspath(src_dir)) == os.path.normcase(os.path.abspath(data_dir))):
                continue
            if any(os.path.lexists(os.path.join(data_dir, n)) for n in names):
                notes.append((logging.WARNING, f"{names[0]} есть и в {data_dir}, и в {src_dir} — используется "
                                               f"{data_dir}, старая копия не переносится"))
                continue
            os.makedirs(data_dir, exist_ok=True)
            present = [n for n in names if os.path.lexists(os.path.join(src_dir, n))]
            done = []
            try:
                for n in present:
                    _move(os.path.join(src_dir, n), os.path.join(data_dir, n))
                    done.append(n)
            except OSError as ex:
                notes.append((logging.WARNING, f"не удалось перенести {', '.join(present)} в {data_dir} ({ex}) — "
                                               f"оставлено в {src_dir}"))
                # Недокопированное на другой том (источник цел) убираем, уже перенесённое — возвращаем
                n = present[len(done)]
                undo = [(os.path.join(data_dir, n), None)] if os.path.lexists(os.path.join(src_dir, n)) else []
                undo += [(os.path.join(data_dir, d), os.path.join(src_dir, d)) for d in reversed(done)]
                for dst, back in undo:
                    try:
                        if not os.path.lexists(dst):
                            continue
                        if back:
                            _move(dst, back)
                        else:
                            _remove(dst)
                    except OSError as ex2:
                        notes.append((logging.ERROR, f"не удалось вернуть {dst} на место: {ex2}"))
                continue
            notes.append((logging.INFO, f"перенесено в {data_dir}: {', '.join(present)}"))
        except Exception as ex:
            notes.append((logging.WARNING, f"перенос {names[0]} в {data_dir}: {type(ex).__name__}: {ex}"))
    return notes


# ---------------------------------------------------------------- база

SCHEMA = """
CREATE TABLE IF NOT EXISTS files(
    id INTEGER PRIMARY KEY, path TEXT UNIQUE, name TEXT, kind TEXT,
    size INTEGER, mtime REAL, status INTEGER DEFAULT 0, err TEXT);
CREATE INDEX IF NOT EXISTS files_status ON files(status, kind);
CREATE TABLE IF NOT EXISTS chunks(
    -- AUTOINCREMENT: id удалённых фрагментов не переиспользуются (VecStore помечает их удалёнными до перезагрузки)
    id INTEGER PRIMARY KEY AUTOINCREMENT, file_id INTEGER, kind TEXT, loc REAL, label TEXT, snippet TEXT, vec BLOB);
CREATE INDEX IF NOT EXISTS chunks_file ON chunks(file_id);
"""
# Имена — в отдельной базе: долгая перестройка не блокирует запись эмбеддингов.
# Два поколения таблиц (a/b): полная пересборка пишет в неактивное и затем переключает meta.active,
# так что поиск по имени работает всё время, а точечные изменения идут в активное.
NAMES_SCHEMA = """
CREATE TABLE IF NOT EXISTS meta(k TEXT PRIMARY KEY, v TEXT);
INSERT OR IGNORE INTO meta(k, v) VALUES ('active', 'a');
"""
NAMES_DB_PATH = os.path.join(DATA_DIR, "names.db")


def names_create(conn, g):
    conn.executescript(f"""
        CREATE TABLE IF NOT EXISTS names_t_{g}(id INTEGER PRIMARY KEY, path TEXT UNIQUE, name TEXT);
        CREATE VIRTUAL TABLE IF NOT EXISTS names_{g} USING fts5(
            name, content='names_t_{g}', content_rowid='id', tokenize='trigram');
    """)


def names_drop(conn, g):
    conn.executescript(f"DROP TABLE IF EXISTS names_{g}; DROP TABLE IF EXISTS names_t_{g};")


def names_active(conn):
    return conn.execute("SELECT v FROM meta WHERE k='active'").fetchone()[0]


def names_add(conn, g, path):
    cur = conn.execute(f"INSERT OR IGNORE INTO names_t_{g}(path, name) VALUES (?, ?)", (path, os.path.basename(path)))
    if cur.rowcount:
        conn.execute(f"INSERT INTO names_{g}(rowid, name) VALUES (?, ?)", (cur.lastrowid, os.path.basename(path)))


def names_delete(conn, g, path):
    """Удаляет файл или всё содержимое папки (на Windows по событию удаления не понять, что это было)."""
    lo, hi = prefix_range(path)
    rows = conn.execute(f"SELECT id, name FROM names_t_{g} WHERE path=? OR (path >= ? AND path < ?)",
                        (path, lo, hi)).fetchall()
    for rid, name in rows:
        conn.execute(f"INSERT INTO names_{g}(names_{g}, rowid, name) VALUES ('delete', ?, ?)", (rid, name))
    conn.executemany(f"DELETE FROM names_t_{g} WHERE id=?", [(r[0],) for r in rows])


def _open(path, schema):
    os.makedirs(DATA_DIR, exist_ok=True)
    conn = sqlite3.connect(path, timeout=30, check_same_thread=False)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.executescript(schema)
    return conn


# Статусы файла: 0 — ждёт, 1 — проиндексирован, 2 — не прочитался, 3 — пропущен правилом (чужой или
# сгенерированный код; причина в err), 5 — точная копия проиндексированного файла copy_of (своих векторов нет)
ST_PENDING, ST_DONE, ST_FAILED, ST_SKIPPED, ST_COPY = 0, 1, 2, 3, 5
NEW_COLUMNS = {  # колонки, добавленные после первой версии базы
    "duration": "REAL",          # длительность аудио/видео
    "hash": "TEXT",              # хеш содержимого — для поиска точных копий
    "copy_of": "INTEGER",        # id оригинала для status=5
    "prio": "INTEGER DEFAULT 0",  # 1 — в последнюю очередь (возможно чужой код)
    "tag": "TEXT",               # признак от движка: submodule / gitignored / archive / foreign
}


def connect():
    conn = _open(DB_PATH, SCHEMA)
    cols = {r[1] for r in conn.execute("PRAGMA table_info(files)")}
    for col, ddl in NEW_COLUMNS.items():
        if col not in cols:  # база старой версии
            conn.execute(f"ALTER TABLE files ADD COLUMN {col} {ddl}")
    conn.execute("CREATE INDEX IF NOT EXISTS files_hash ON files(hash)")
    conn.execute("CREATE INDEX IF NOT EXISTS files_copy ON files(copy_of)")
    conn.commit()
    return conn


def connect_names():
    conn = _open(NAMES_DB_PATH, NAMES_SCHEMA)
    names_create(conn, names_active(conn))
    conn.commit()
    return conn


# ---------------------------------------------------------------- модель

# Метки мультимодальных вставок. Буквальный "<|audio|>" в документе процессор модели принимает
# за место для аудио и падает («нет аудиовхода») — в тексте такие метки обезвреживаем.
_MEDIA_MARKERS = re.compile(
    r"<\|([A-Za-z_]{1,30})\|>|<((?:start|end)_of_(?:image|audio|video)|(?:image|audio|video)_soft_token)>")


def neutralize(text):
    return _MEDIA_MARKERS.sub(lambda m: f"<{m.group(1)}>" if m.group(1) else f"< {m.group(2)} >", text)


def _arch_ok(cap, archs):
    """Есть ли в сборке torch код для карты: sm_XY работает на картах того же поколения X с minor >= Y."""
    for a in archs:
        m = re.fullmatch(r"sm_(\d+)(\d)[a-z]?", a)
        if m and int(m.group(1)) == cap[0] and cap[1] >= int(m.group(2)):
            return True
    return False


def _usable_gpu(torch):
    """-> (название, capability) видеокарты, на которой можно считать, иначе None (причина — в журнале).
    is_available() мало: torch для CUDA 12.8 собран под sm_75+, и на GTX 10xx (sm_61) ядра не запускаются."""
    try:
        if not torch.cuda.is_available():
            log.info("CUDA недоступна (torch %s) — модель считает на процессоре", torch.__version__)
            return None
        name, cap = torch.cuda.get_device_name(0), torch.cuda.get_device_capability(0)
        archs = torch.cuda.get_arch_list()
    except Exception as ex:
        log.warning("CUDA не отвечает (%s: %s) — модель считает на процессоре", type(ex).__name__, ex)
        return None
    if not _arch_ok(cap, archs):
        log.warning("видеокарта %s (sm_%d%d) не поддерживается этой сборкой torch %s (есть: %s) — "
                    "модель считает на процессоре", name, *cap, torch.__version__, " ".join(archs) or "—")
        return None
    return name, cap


def _load_model(device, dtype):
    """Скачанную модель грузим без сети: иначе huggingface_hub при каждом запуске спрашивает Hugging Face,
    не вышла ли новая версия. Нет её в кэше (первый запуск, оборванная загрузка) — тогда качаем."""
    from sentence_transformers import SentenceTransformer

    kw = {"device": device, "model_kwargs": {"torch_dtype": dtype}}
    try:
        return SentenceTransformer(MODEL_ID, local_files_only=True, **kw)
    except OSError:
        return SentenceTransformer(MODEL_ID, **kw)


class Embedder:
    """Одна модель на процесс; индексатор и поиск делят её через lock."""

    def __init__(self):
        import throttle

        throttle.init()  # низкий приоритет GPU для процесса — до создания CUDA-контекста
        import torch

        self.throttle = throttle
        self.torch = torch
        self.device, self.model = "cpu", None
        gpu = _usable_gpu(torch)
        if gpu:
            name, cap = gpu
            # bfloat16 — с Ampere (8.0); fp16 Gemma не держит, поэтому на более старых картах — float32
            dtype = torch.bfloat16 if cap >= (8, 0) else torch.float32
            try:
                self.model = _load_model("cuda", dtype)
                self.model.encode(["проверка"], show_progress_bar=False)  # ядра под эту карту действительно есть
                self.device = "cuda"
                log.info("модель на видеокарте %s (sm_%d%d), %s", name, *cap, str(dtype).replace("torch.", ""))
            except Exception as ex:
                log.warning("видеокарта %s (sm_%d%d): модель не запустилась (%s: %s) — считаю на процессоре",
                            name, *cap, type(ex).__name__, str(ex)[:300])
                self.model = None
        if self.device == "cpu":
            if gpu:  # отдать видеопамять, занятую неудачной попыткой
                gc.collect()
                try:
                    torch.cuda.empty_cache()
                except Exception:
                    pass
            # Не больше половины логических ядер: фоновая индексация не должна занимать процессор целиком
            torch.set_num_threads(max(1, (os.cpu_count() or 2) // 2))
            self.model = _load_model("cpu", torch.float32)
            log.info("модель на процессоре, потоков: %d", torch.get_num_threads())
        else:
            throttle.attach(self.model, torch)  # дробит фоновые порции на короткие всплески по слоям
        self.lock = threading.Lock()

    def _enc(self, items, bs, background=False, **kw):
        """background=True — работа индексатора: размер порции и паузы между порциями задаёт throttle,
        чтобы видеокарта не душила рабочий стол, видео и игры. False (поиск) — без пауз и с обычным
        приоритетом GPU на время расчёта."""
        if not items:
            return np.zeros((0, DIM), np.float32)
        # Тексты сортируем по длине: в порции короткие дополняются до самого длинного,
        # и на смешанных длинах половина работы видеокарты уходит в пустоту
        order = sorted(range(len(items)), key=lambda i: len(items[i]) if isinstance(items[i], str) else 0)
        ordered = [items[i] for i in order]
        th, parts, i = self.throttle, [], 0
        while i < len(ordered):  # lock на каждую порцию — поиск не ждёт всю пачку индексатора
            n = th.batch_size(bs) if background else bs  # тут же выдерживается пауза после прошлой порции
            with self.lock, self.torch.inference_mode(), (th.background() if background else th.foreground()):
                parts.append(self.model.encode(ordered[i:i + n], batch_size=n, convert_to_numpy=True,
                                               show_progress_bar=False, **kw))
            i += n
        e = np.empty((len(items), DIM), np.float32)
        e[order] = np.concatenate([np.asarray(p, np.float32) for p in parts])[:, :DIM]
        return e / np.maximum(np.linalg.norm(e, axis=1, keepdims=True), 1e-9)

    def query(self, q, code=False):
        """Вектор запроса. Промпт CodeRetrieval лучше для вкладки «Код», но завышает косинусы,
        поэтому в смешанной выдаче код сравнивается с текстом на общем промпте."""
        return self._enc([neutralize(q)], 1, prompt_name="CodeRetrieval" if code else "SearchQuery")[0]

    # background=True передаёт только индексатор; поиск (в т.ч. по картинке) зовёт без него
    def documents(self, texts, background=False):
        # тексты уже в формате "title: ... | text: ..."
        return self._enc([neutralize(t) for t in texts], 16, background)

    def images(self, pil_images, background=False):
        return self._enc(pil_images, 8, background)

    def audio(self, arrays, background=False):
        items = [{"text": "<|audio|>", "audio": [{"array": a, "sampling_rate": 16000}]} for a in arrays]
        return self._enc(items, 4, background)

    def free_cache(self):
        if self.device == "cuda":
            self.torch.cuda.empty_cache()


# ---------------------------------------------------------------- векторы в памяти

class VecStore:
    """Все векторы лежат на GPU (без него или если не влезли — в памяти процессора) по видам;
    поиск — одно матричное умножение."""

    def __init__(self, device):
        import torch

        self.torch = torch
        self.device = device
        self.lock = threading.Lock()
        self.pending = {k: ([], []) for k in KINDS}  # ids, mats — ещё не склеены
        self.tensors = {k: (np.zeros(0, np.int64), None) for k in KINDS}
        self.removed = set()

    def load(self, conn):
        rows = conn.execute("SELECT id, kind, vec FROM chunks WHERE vec IS NOT NULL").fetchall()
        groups = {k: ([], []) for k in KINDS}
        for cid, kind, blob in rows:
            if kind in groups:
                groups[kind][0].append(cid)
                groups[kind][1].append(np.frombuffer(blob, np.float16))
        with self.lock:
            self.removed.clear()
            self.pending = {k: ([], []) for k in KINDS}
            for k, (ids, vecs) in groups.items():
                self.tensors[k] = (np.zeros(0, np.int64), None)
                if ids:
                    self.pending[k] = ([np.array(ids, np.int64)], [np.stack(vecs)])
        return len(rows)

    def add(self, kind, ids, mat):
        with self.lock:
            self.pending[kind][0].append(np.asarray(ids, np.int64))
            self.pending[kind][1].append(np.asarray(mat, np.float16))

    def remove(self, chunk_ids):
        with self.lock:
            self.removed.update(chunk_ids)

    def _tensor(self, kind):
        ids, t = self.tensors[kind]
        p_ids, p_mats = self.pending[kind]
        if p_ids:
            mat = self.torch.from_numpy(np.concatenate(p_mats))
            try:
                new_t = mat.to(self.device)
                t = new_t if t is None else self.torch.cat([t, new_t])
            except RuntimeError as ex:  # не хватило видеопамяти — весь индекс переезжает в обычную память
                if self.device == "cpu":
                    raise
                log.warning("векторы не помещаются на видеокарту (%s) — индекс остаётся в памяти процессора",
                            str(ex)[:200])
                self.device = "cpu"
                for k, (k_ids, k_t) in self.tensors.items():
                    if k_t is not None:
                        self.tensors[k] = (k_ids, k_t.cpu())
                t = self.tensors[kind][1]
                t = mat if t is None else self.torch.cat([t, mat])
                try:
                    self.torch.cuda.empty_cache()
                except Exception:
                    pass
            ids = np.concatenate([ids] + p_ids)
            self.tensors[kind] = (ids, t)
            self.pending[kind] = ([], [])
        return ids, t

    def count(self, kind):
        with self.lock:
            return len(self._tensor(kind)[0])

    def search(self, kind, qvec, k=300):
        """-> [(chunk_id, cosine, z)] — z нормирует оценки, чтобы сравнивать фото с текстом."""
        with self.lock:
            ids, t = self._tensor(kind)
            if t is None or len(ids) == 0:
                return []
            q = self.torch.from_numpy(np.asarray(qvec, np.float16)).to(self.device)
            sims = (t @ q).float()
            mean, std = sims.mean().item(), max(sims.std().item() if len(ids) > 1 else 1.0, 1e-3)
            top = self.torch.topk(sims, min(k + len(self.removed), len(ids)))
            out = []
            for s, i in zip(top.values.tolist(), top.indices.tolist()):
                cid = int(ids[i])
                if cid not in self.removed:
                    out.append((cid, s, (s - mean) / std))
                if len(out) >= k:
                    break
            return out

    def vector(self, chunk_id):
        with self.lock:
            for k in KINDS:
                ids, t = self._tensor(k)
                hit = np.nonzero(ids == chunk_id)[0]
                if len(hit):
                    return t[int(hit[0])].float().cpu().numpy()
        return None


# ---------------------------------------------------------------- игры / полноэкранные приложения

class _MONITORINFO(ctypes.Structure):
    _fields_ = [("cbSize", wintypes.DWORD), ("rcMonitor", wintypes.RECT),
                ("rcWork", wintypes.RECT), ("dwFlags", wintypes.DWORD)]


_user32 = ctypes.windll.user32
_user32.GetForegroundWindow.restype = wintypes.HWND
_user32.MonitorFromWindow.argtypes = [wintypes.HWND, wintypes.DWORD]
_user32.MonitorFromWindow.restype = wintypes.HANDLE
_user32.GetMonitorInfoW.argtypes = [wintypes.HANDLE, ctypes.POINTER(_MONITORINFO)]
_user32.GetWindowRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
_user32.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]


def fullscreen_app_active():
    # Windows сам сообщает про D3D-полноэкранный режим, презентации и т.п.
    state = ctypes.c_int()
    if ctypes.windll.shell32.SHQueryUserNotificationState(ctypes.byref(state)) == 0 and state.value in (2, 3, 4):
        return True
    # Borderless-окна игр Windows не помечает — проверяем, что окно закрывает весь монитор
    hwnd = _user32.GetForegroundWindow()
    if not hwnd:
        return False
    cls = ctypes.create_unicode_buffer(256)
    _user32.GetClassNameW(hwnd, cls, 256)
    if cls.value in ("Progman", "WorkerW", "Shell_TrayWnd"):
        return False
    r = wintypes.RECT()
    _user32.GetWindowRect(hwnd, ctypes.byref(r))
    mi = _MONITORINFO()
    mi.cbSize = ctypes.sizeof(mi)
    if not _user32.GetMonitorInfoW(_user32.MonitorFromWindow(hwnd, 2), ctypes.byref(mi)):
        return False
    m = mi.rcMonitor
    return r.left <= m.left and r.top <= m.top and r.right >= m.right and r.bottom >= m.bottom


class _POWER_STATUS(ctypes.Structure):
    _fields_ = [("ACLineStatus", ctypes.c_ubyte), ("BatteryFlag", ctypes.c_ubyte),
                ("BatteryLifePercent", ctypes.c_ubyte), ("SystemStatusFlag", ctypes.c_ubyte),
                ("BatteryLifeTime", wintypes.DWORD), ("BatteryFullLifeTime", wintypes.DWORD)]


def on_battery():
    s = _POWER_STATUS()
    return bool(ctypes.windll.kernel32.GetSystemPowerStatus(ctypes.byref(s))) and s.ACLineStatus == 0


def fixed_drives():
    """Локальные несъёмные диски с файловой системой: [{"drive": "C:", "label": "...", "fs": "NTFS"}]."""
    k32 = ctypes.windll.kernel32
    mask, out = k32.GetLogicalDrives(), []
    for i in range(26):
        root = f"{chr(65 + i)}:\\"
        if not mask >> i & 1 or k32.GetDriveTypeW(root) != 3:  # 3 = DRIVE_FIXED
            continue
        label, fs = ctypes.create_unicode_buffer(261), ctypes.create_unicode_buffer(261)
        if not k32.GetVolumeInformationW(root, label, 261, None, None, None, fs, 261):
            continue  # том не размечен или заблокирован (BitLocker) — искать на нём нечего
        out.append({"drive": root[:2], "label": label.value, "fs": fs.value})
    return out


def running_blocker(names):
    if not names:
        return None
    import psutil

    wanted = {n.lower() for n in names}
    for p in psutil.process_iter(["name"]):
        n = (p.info.get("name") or "").lower()
        if n in wanted:
            return n
    return None


def lower_priority():
    BELOW_NORMAL_PRIORITY_CLASS = 0x4000
    ctypes.windll.kernel32.SetPriorityClass(ctypes.windll.kernel32.GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS)
