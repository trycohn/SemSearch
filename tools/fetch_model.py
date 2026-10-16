"""Скачивание модели для установщика (docs/INSTALLER.md, шаг model): EmbeddingGemma 2 в <папка данных>\\models.
Запуск: <venv>\\Scripts\\python.exe tools\\fetch_model.py (SEMSEARCH_DATA_DIR — папка данных, как у программы).

В stdout — только JSON-строки (по одной в строке, в ASCII — кодировка консоли не важна):
  {"got": байт в кэше модели, "total": байт всего | null}   не чаще 2 раз в секунду
  {"done": true}                                            модель на месте, код выхода 0
  {"error": "текст", "kind": "network" | "space" | "other"} код выхода 1
Всё остальное (перенос из общего кэша, трассировка ошибки) — в stderr, в UTF-8.

Порядок: core.migrate_model() переносит модель из общего кэша Hugging Face, потом целая модель пропускается,
иначе докачивается то, чего нет: повторный запуск безопасен, целые файлы заново не качаются.

Крупные файлы (веса, ~1,5 ГБ) качаем сами, Range-запросами в <кэш>\\models--...\\blobs\\<sha256>.part: huggingface_hub 1.x
докачивать с середины не умеет (после обрыва начинает файл заново), а обрыв на 1,5 ГБ — обычное дело. Дочитанный файл
проверяется по sha256 из Hugging Face и кладётся в snapshots\\<коммит>\\ — туда, где hub ищет файл (без символических ссылок он
кладёт его так же), refs\\main пишем сами. Остальное (мелкие файлы) докачивает snapshot_download. Оборванный запуск оставляет
.part на месте, следующий продолжает с его конца, в «got» этот файл считается как есть."""
import errno
import hashlib
import json
import os
import re
import socket
import ssl
import sys
import threading
import time
import traceback

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if ROOT not in sys.path:  # скрипт лежит в tools\, а core.py — в корне программы
    sys.path.insert(0, ROOT)
# Полосы tqdm пишут в stderr и установщику не нужны; ставится до первого импорта huggingface_hub
os.environ.setdefault("HF_HUB_DISABLE_PROGRESS_BARS", "1")
os.environ.setdefault("HF_HUB_DISABLE_SYMLINKS_WARNING", "1")

import core  # noqa: E402  — ставит HF_HUB_CACHE = <папка данных>\models, если кэш не выбран пользователем


def _ci(pattern):
    """Шаблон fnmatch без учёта регистра: буквы -> [aA] (core.model_download_total ищет с re.I, а fnmatchcase — нет)."""
    return "".join(f"[{c.lower()}{c.upper()}]" if c.isalpha() else c for c in pattern)


# Не качаем то же, что core.model_download_total не считает в «всего»: варианты ONNX/OpenVINO (приложению нужны
# только веса safetensors), описания и картинки README, .gitattributes
IGNORE = [_ci(p) for p in ("onnx/*", "openvino/*", "*.md", "*.png", "*.jpg", "*.jpeg", "*.gif", ".gitattributes")]
BIG = 100 * 1024 * 1024  # файлы крупнее качаем сами с докачкой (см. выше); мельче — snapshot_download
CHUNK = 1024 * 1024
PROGRESS_EVERY = 0.5  # секунд между строками прогресса: не чаще двух раз в секунду
MSG_LIMIT = 300  # длина текста исключения в сообщении об ошибке
# Паузы между попытками докачки, которые не принесли ни байта: после обрыва Hugging Face бывает минуту-другую
# закрывает соединение сразу (особенно запросы без токена). Все паузы подряд — около двух минут, потом сдаёмся.
RETRY_WAITS = (2, 5, 10, 20, 30, 30, 30)
RETRY_AFTER_MAX = 60  # секунд: дольше по заголовку Retry-After не ждём

_out_lock = threading.Lock()  # строки из потока прогресса и из главного не должны перемешиваться


def _out(obj):
    with _out_lock:
        try:
            if sys.stdout is not None:  # под pythonw stdout нет
                sys.stdout.write(json.dumps(obj) + "\n")
                sys.stdout.flush()
        except (OSError, ValueError):  # установщик закрыл трубу — писать некому
            pass


def _log(text):
    try:
        if sys.stderr is not None:
            sys.stderr.write(text + "\n")
            sys.stderr.flush()
    except (OSError, ValueError):
        pass


# ---------------------------------------------------------------- цела ли модель на диске (без сети)

def _snapshot(cache):
    """Папка снимка модели в кэше `cache` (по refs/main) или None."""
    base = os.path.join(cache, "models--" + core.MODEL_ID.replace("/", "--"))
    try:
        with open(os.path.join(base, "refs", "main"), encoding="ascii") as f:
            rev = f.read().strip()
    except (OSError, ValueError):
        return None
    d = os.path.join(base, "snapshots", rev)
    return d if rev and os.path.isdir(d) else None


def _safetensors_ok(path):
    """Файл весов дочитан до конца: размер не меньше того, что обещает заголовок safetensors (8 байт длины +
    JSON с позициями тензоров). Оборванный файл короче."""
    size = os.path.getsize(path)
    with open(path, "rb") as f:
        n = int.from_bytes(f.read(8), "little")
        if not 0 < n <= 64 * 1024 * 1024 or 8 + n > size:
            return False
        header = json.loads(f.read(n))
    end = max((v["data_offsets"][1] for k, v in header.items() if k != "__metadata__"), default=0)
    return size >= 8 + n + end


def _model_ok(cache):
    """Лежит ли в кэше `cache` целая модель — то, что нужно core._load_model (SentenceTransformer с
    local_files_only=True): modules.json и папки его модулей, config.json, токенайзер и дочитанные до конца веса.
    Без сети; докачка (snapshot_download) всё равно сверяет файлы со списком Hugging Face."""
    snap = _snapshot(cache)
    if not snap:
        return False

    def path(name):
        return os.path.join(snap, *name.split("/"))

    try:
        with open(path("modules.json"), encoding="utf-8") as f:
            for m in json.load(f):
                sub = m.get("path") or ""  # модуль Transformer лежит в корне, Pooling и Normalize — в своих папках
                if sub and not any(os.path.isfile(os.path.join(path(sub), n)) for n in os.listdir(path(sub))):
                    return False
        if not os.path.isfile(path("config.json")):
            return False
        if not any(os.path.isfile(path(n)) for n in ("tokenizer.json", "tokenizer.model", "vocab.json")):
            return False
        if os.path.isfile(path("model.safetensors")):
            shards = ["model.safetensors"]
        elif os.path.isfile(path("model.safetensors.index.json")):
            with open(path("model.safetensors.index.json"), encoding="utf-8") as f:
                shards = sorted(set(json.load(f)["weight_map"].values()))
        else:
            return False
        return bool(shards) and all(os.path.isfile(path(s)) and _safetensors_ok(path(s)) for s in shards)
    except (OSError, ValueError, KeyError, TypeError, AttributeError):  # нет файла, битый JSON, битая структура
        return False


# ---------------------------------------------------------------- ошибки

def _chain(ex):
    """Исключение и все его причины (__cause__ / __context__): huggingface_hub оборачивает сетевые сбои."""
    seen = set()
    while ex is not None and id(ex) not in seen:
        seen.add(id(ex))
        yield ex
        ex = ex.__cause__ or ex.__context__


_SPACE_TEXT = re.compile(r"no space left|not enough space|disk (is )?full|\(os error (28|39|112)\)", re.I)


def _is_network(ex):
    if isinstance(ex, (ConnectionError, TimeoutError, socket.timeout, socket.gaierror, ssl.SSLError)):
        return True  # ConnectionError — и OfflineModeIsEnabled
    try:
        import httpx

        if isinstance(ex, httpx.TransportError):  # соединение, таймаут, обрыв чтения, прокси
            return True
    except ImportError:
        pass
    try:
        from huggingface_hub import errors

        # LocalEntryNotFoundError: «не достучался до Hub и в кэше нет» (в том числе в режиме HF_HUB_OFFLINE);
        # XetDownloadError: обрыв загрузки через hf_xet. Какой-то из классов может отсутствовать в старой версии
        known = tuple(getattr(errors, n) for n in ("LocalEntryNotFoundError", "XetDownloadError") if hasattr(errors, n))
        if isinstance(ex, known):
            return True
    except ImportError:
        pass
    status = getattr(getattr(ex, "response", None), "status_code", None)
    return status in (408, 429) or (isinstance(status, int) and status >= 500)  # Hub перегружен или лежит


def _kind(ex):
    """-> "space" | "network" | "other"."""
    chain = list(_chain(ex))
    for e in chain:
        if isinstance(e, OSError) and (e.errno == errno.ENOSPC or getattr(e, "winerror", None) in (39, 112)):
            return "space"
        if _SPACE_TEXT.search(str(e)):  # hf_xet (Rust) отдаёт ошибки диска текстом
            return "space"
    return "network" if any(_is_network(e) for e in chain) else "other"


def _fail(ex):
    kind = _kind(ex)
    # huggingface_hub прячет причину (ConnectError и т.п.) в длинную английскую обёртку LocalEntryNotFoundError — берём причину
    cause = next((e for e in _chain(ex) if type(e).__name__ != "LocalEntryNotFoundError"), ex)
    text = " ".join(f"{type(cause).__name__}: {cause}".split())
    if len(text) > MSG_LIMIT:
        text = text[:MSG_LIMIT - 1] + "…"
    if kind == "space":
        msg = f"Не хватает места на диске для модели ({core.MODELS_DIR}). {text}"
    elif kind == "network":
        msg = f"Нет связи с Hugging Face, модель не скачана до конца (повтор продолжит с места обрыва). {text}"
    else:
        msg = f"Не удалось скачать модель. {text}"
    _log(traceback.format_exc())
    _out({"error": msg, "kind": kind})
    return 1


# ---------------------------------------------------------------- крупные файлы с докачкой

def _storage(cache):
    return os.path.join(cache, "models--" + core.MODEL_ID.replace("/", "--"))


def _files(info):
    """Файлы модели из ответа Hugging Face без отбрасываемых (IGNORE) — то же, что считает core.model_download_total."""
    from huggingface_hub.utils import filter_repo_objects

    return list(filter_repo_objects(info.siblings, ignore_patterns=IGNORE, key=lambda s: s.rfilename))


def _retry_after(value):
    """Заголовок Retry-After в секундах (число) -> секунды или None."""
    try:
        return max(0.0, min(float(value), RETRY_AFTER_MAX))
    except (TypeError, ValueError):
        return None


def _download_range(url, part, size):
    """Докачивает url в файл part с его текущего конца (Range). Каждая попытка заново проходит редирект Hub, так что
    адрес на CDN свежий. Обрыв, таймаут, ответ 429 или 5xx — повтор: после попытки с данными сразу, после пустой —
    с паузой из RETRY_WAITS (или по Retry-After). Попытки без единого байта кончились — исключение ConnectionError
    (сетевое: установщик покажет «Нет сети», а повторный запуск продолжит с того же места)."""
    import httpx
    from huggingface_hub.utils import build_hf_headers

    def disk():
        return os.path.getsize(part) if os.path.isfile(part) else 0

    if disk() > size:  # не наш остаток
        os.remove(part)
    fails = 0
    while disk() < size:
        received, err, wait = 0, None, None
        try:
            have = disk()
            headers = {**build_hf_headers(), "Accept-Encoding": "identity", "Range": f"bytes={have}-"}
            with httpx.stream("GET", url, headers=headers, follow_redirects=True, timeout=httpx.Timeout(30.0)) as r:
                if r.status_code == 206:
                    m = re.match(r"bytes (\d+)-\d+/(\d+)$", r.headers.get("content-range", ""))
                    if not m or int(m.group(1)) != have or int(m.group(2)) != size:
                        raise RuntimeError(f"сервер ответил на Range неожиданно: {r.headers.get('content-range')!r}")
                    mode = "ab"
                elif r.status_code == 200:  # Range не поддержан — файл придёт с начала
                    mode = "wb"
                elif r.status_code == 429 or r.status_code >= 500:  # перегрузка или сбой на стороне Hub — подождать
                    mode = None
                    err = RuntimeError(f"HTTP {r.status_code}")
                    wait = _retry_after(r.headers.get("retry-after"))
                else:
                    r.raise_for_status()  # прочие 4xx -> HTTPStatusError: повтор не поможет
                    raise RuntimeError(f"неожиданный ответ HTTP {r.status_code}")
                if mode:
                    with open(part, mode) as f:
                        for chunk in r.iter_bytes(CHUNK):
                            f.write(chunk)
                            received += len(chunk)
        except httpx.TransportError as ex:  # обрыв, таймаут, сброс соединения
            err = ex
        if disk() >= size:
            break
        if received:
            fails = 0
            time.sleep(1)  # оборвалось посреди потока: продолжаем почти сразу
            continue
        fails += 1
        if fails > len(RETRY_WAITS):
            raise ConnectionError(f"Hugging Face не отдаёт данные: {err or 'пустой ответ'}") from err
        time.sleep(wait if wait is not None else RETRY_WAITS[fails - 1])


def _sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            block = f.read(8 * CHUNK)
            if not block:
                return h.hexdigest()
            h.update(block)


def _fetch_big(files, commit, cache):
    """Крупные файлы снимка (с sha256 в LFS-данных Hub) -> snapshots\\<коммит>\\<путь>. Уже на месте — пропуск."""
    from huggingface_hub import hf_hub_url

    storage = _storage(cache)
    blobs = os.path.join(storage, "blobs")
    big = [s for s in files if s.size and s.size >= BIG and s.lfs and s.lfs.sha256]
    wanted = {s.lfs.sha256 + ".part" for s in big}
    if os.path.isdir(blobs):  # остатки другой версии файла уже никогда не понадобятся
        for name in os.listdir(blobs):
            if name.endswith(".part") and name not in wanted:
                try:
                    os.remove(os.path.join(blobs, name))
                except OSError:
                    pass
    for s in big:
        dst = os.path.join(storage, "snapshots", commit, *s.rfilename.split("/"))
        if os.path.isfile(dst):
            if os.path.getsize(dst) == s.size:
                continue
            os.remove(dst)  # размер не тот — файл битый
        os.makedirs(blobs, exist_ok=True)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        part = os.path.join(blobs, s.lfs.sha256 + ".part")  # имя стабильное: следующий запуск найдёт и продолжит
        url = hf_hub_url(core.MODEL_ID, s.rfilename, revision=commit)
        for attempt in (1, 2):
            _download_range(url, part, s.size)
            if _sha256(part) == s.lfs.sha256:
                break
            os.remove(part)  # на конце остатка мог остаться мусор (сбой питания): один раз заново с нуля
            if attempt == 2:
                raise RuntimeError(f"{s.rfilename}: sha256 не совпал и после повторной загрузки")
        os.replace(part, dst)


def _write_ref(cache, commit):
    """refs\\main = коммит (40 символов без перевода строки, как пишет huggingface_hub): по нему SentenceTransformer
    находит снимок. Атомарно."""
    refs = os.path.join(_storage(cache), "refs")
    os.makedirs(refs, exist_ok=True)
    tmp = os.path.join(refs, "main.tmp")
    with open(tmp, "w", encoding="ascii") as f:
        f.write(commit)
    os.replace(tmp, os.path.join(refs, "main"))


# ---------------------------------------------------------------- прогресс

def _progress(stop, state):
    """Раз в PROGRESS_EVERY секунд — сколько байт модели уже в кэше (вместе с недокачанным .part). «Всего» приходит из
    главного потока, когда Hugging Face ответит (state["total"]): до этого, а без сети всегда, total — null."""
    try:
        _out({"got": core.model_cache_bytes()[0], "total": None})
        while not stop.wait(PROGRESS_EVERY):
            total = state["total"]
            got = core.model_cache_bytes()[0]
            _out({"got": min(got, total) if total else got, "total": total})  # в кэше ещё refs/main и пр.: чуть больше
    except Exception:  # прогресс не должен ронять загрузку
        _log(traceback.format_exc())


# ---------------------------------------------------------------- главное

def main():
    for level, msg in core.migrate_model():  # до первого импорта huggingface_hub: он читает HF_HUB_CACHE при импорте
        _log(msg)
    if core.OWN_HF_CACHE and os.environ.get("HF_HUB_CACHE") != core.MODELS_DIR:
        # Перенести из общего кэша не удалось, и core оставил модель на старом месте (программа повторит перенос при
        # запуске). Цела там — этого достаточно; нет — качаем сразу на место, а не в общий кэш
        if _model_ok(os.environ["HF_HUB_CACHE"]):
            _out({"done": True})
            return 0
        os.environ["HF_HUB_CACHE"] = core.MODELS_DIR
    try:
        from huggingface_hub import HfApi, constants, snapshot_download

        cache = constants.HF_HUB_CACHE
        # Остатки оборванных попыток snapshot_download (отмена в установщике = kill): они не докачиваются и занимают место
        freed = core.drop_stale_downloads()
        if freed:
            _log(f"удалены остатки оборванной загрузки: {freed / 1e6:.0f} МБ")
        if _model_ok(cache):
            _out({"done": True})
            return 0
        state = {"total": None}
        stop = threading.Event()
        watcher = threading.Thread(target=_progress, args=(stop, state), name="progress", daemon=True)
        watcher.start()
        try:
            info = HfApi().model_info(core.MODEL_ID, files_metadata=True)
            if not info.sha:
                raise RuntimeError("Hugging Face не вернул номер коммита модели")
            files = _files(info)
            state["total"] = sum(s.size or 0 for s in files) or None
            _fetch_big(files, info.sha, cache)
            _write_ref(cache, info.sha)
            snapshot_download(core.MODEL_ID, revision=info.sha, ignore_patterns=IGNORE)  # мелкие файлы; крупные уже на месте
        finally:
            stop.set()
            watcher.join(2)
        if not _model_ok(cache):
            raise RuntimeError(f"после загрузки в {cache} нет целой модели {core.MODEL_ID}")
    except KeyboardInterrupt:
        _out({"error": "Загрузка модели прервана.", "kind": "other"})
        return 1
    except Exception as ex:
        return _fail(ex)
    _out({"done": True})
    return 0


if __name__ == "__main__":
    try:
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")  # журнал установщика читает UTF-8, не кодовую страницу
    except (AttributeError, OSError, ValueError):
        pass
    sys.exit(main())
