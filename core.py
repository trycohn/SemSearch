"""Общие вещи: конфиг, база, модель, хранилище векторов, детектор игр."""
import ctypes
import json
import logging
import os
import re
import sqlite3
import threading
from ctypes import wintypes

import numpy as np

APP_DIR = os.path.dirname(os.path.abspath(__file__))
DATA_DIR = os.path.join(APP_DIR, "data")
DB_PATH = os.path.join(DATA_DIR, "index.db")
CONFIG_PATH = os.path.join(APP_DIR, "config.json")
MODEL_ID = "google/embeddinggemma-2"
DIM = 256  # Matryoshka: 768 -> 256, индекс в 3 раза меньше почти без потери качества

HOME = os.path.expanduser("~")

DEFAULT_CONFIG = {
    # Где искать по содержимому (смыслу)
    "content_roots": [HOME],
    # Где искать по имени файла: диски целиком (из них же выводится name_roots)
    "name_drives": ["C:"],
    # Папки с такими именами пропускаются везде (плюс все папки, начинающиеся с точки, в content_roots)
    "exclude_dirs": [
        "node_modules", "__pycache__", "venv", "env", "site-packages", "dist", "build", "obj",
        "bin", "Library", "Temp", "target", "packages", "vendor", "AppData", "$Recycle.Bin",
        "System Volume Information", "WinSxS", "Logs", "Cache", "cache",
    ],
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


def load_config():
    cfg = dict(DEFAULT_CONFIG)
    if os.path.exists(CONFIG_PATH):
        with open(CONFIG_PATH, encoding="utf-8-sig") as f:  # -sig: Блокнот и PowerShell пишут BOM
            cfg.update(json.load(f))
    else:
        with open(CONFIG_PATH, "w", encoding="utf-8") as f:
            json.dump(DEFAULT_CONFIG, f, ensure_ascii=False, indent=2)
    derive_config(cfg)
    return cfg


def derive_config(cfg):
    """Производные ключи — пересчитываются и после изменения настроек из интерфейса."""
    if "name_roots" in cfg and "name_drives" not in cfg:  # конфиг старого формата
        cfg["name_drives"] = [r.rstrip("\\/") for r in cfg["name_roots"]]
    cfg["name_roots"] = [d.rstrip("\\/") + "\\" for d in cfg["name_drives"]]
    # Собственную папку не индексируем никогда: иначе превью из data/thumbs попадут в «фото»
    cfg["exclude_paths"] = [p for p in cfg["exclude_paths"] if os.path.normcase(p) != os.path.normcase(APP_DIR)]
    cfg["exclude_paths"].append(APP_DIR)


EDITABLE = {  # ключ -> тип; только их можно менять через POST /api/settings
    "content_roots": list, "name_drives": list, "exclude_dirs": list, "hotkey": str, "theme": str, "accent": str,
    "pause_on_fullscreen": bool, "pause_processes": list, "pause_on_battery": bool,
    "video_frame_every_sec": int, "video_max_frames": int, "audio_segment_sec": int, "audio_max_segments": int,
    "max_text_mb": int, "max_pdf_pages": int,
}


def save_config(changes):
    """Записывает изменения в config.json, не трогая остальные ключи файла (атомарно)."""
    data = {}
    if os.path.exists(CONFIG_PATH):
        with open(CONFIG_PATH, encoding="utf-8-sig") as f:
            data = json.load(f)
    data.update(changes)
    data.pop("name_roots", None)  # выводится из name_drives
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


def connect():
    conn = _open(DB_PATH, SCHEMA)
    cols = {r[1] for r in conn.execute("PRAGMA table_info(files)")}
    if "duration" not in cols:  # база старой версии
        conn.execute("ALTER TABLE files ADD COLUMN duration REAL")
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


class Embedder:
    """Одна модель на процесс; индексатор и поиск делят её через lock."""

    def __init__(self):
        import throttle

        throttle.init()  # низкий приоритет GPU для процесса — до создания CUDA-контекста
        import torch
        from sentence_transformers import SentenceTransformer

        self.throttle = throttle
        self.torch = torch
        self.device = "cuda" if torch.cuda.is_available() else "cpu"
        dtype = torch.bfloat16 if self.device == "cuda" and torch.cuda.is_bf16_supported() else torch.float32
        self.model = SentenceTransformer(MODEL_ID, device=self.device, model_kwargs={"torch_dtype": dtype})
        if self.device == "cuda":
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
    """Все векторы лежат на GPU по видам; поиск — одно матричное умножение."""

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
            new_t = self.torch.from_numpy(np.concatenate(p_mats)).to(self.device)
            ids = np.concatenate([ids] + p_ids)
            t = new_t if t is None else self.torch.cat([t, new_t])
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
    """Локальные несъёмные диски: [{"drive": "C:", "label": "...", "fs": "NTFS"}]."""
    k32 = ctypes.windll.kernel32
    mask, out = k32.GetLogicalDrives(), []
    for i in range(26):
        root = f"{chr(65 + i)}:\\"
        if not mask >> i & 1 or k32.GetDriveTypeW(root) != 3:  # 3 = DRIVE_FIXED
            continue
        label, fs = ctypes.create_unicode_buffer(261), ctypes.create_unicode_buffer(261)
        k32.GetVolumeInformationW(root, label, 261, None, None, None, fs, 261)
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
