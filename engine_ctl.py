"""Запуск и остановка движка имён (bin/semsearch-engine.exe).

Если пользователь установил задачу Планировщика «SemSearch Engine» (engine/install-task.ps1), движок
стартует через неё с правами администратора и читает MFT/USN — без запроса UAC при каждом запуске.
Иначе backend запускает его сам, без прав: обход диска + ReadDirectoryChangesW."""
import logging
import os
import subprocess
import threading
import time

import core

log = logging.getLogger("engine")
EXE = os.path.join(core.APP_DIR, "bin", "semsearch-engine.exe")
TASK = "SemSearch Engine"
# Полный путь: PATH у процесса может быть урезан, а Windows стоять не на C:
SCHTASKS = os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32", "schtasks.exe")
NO_WINDOW = 0x08000000  # CREATE_NO_WINDOW

_spawned = None  # процесс, если движок запускали мы сами
_lock = threading.Lock()  # ensure() зовут и на старте запуска, и при подключении индексатора


def _client():
    try:
        from engine_client import EngineClient

        return EngineClient()
    except Exception:
        return None


def _wait(client, seconds, proc=None):
    """Ждёт, пока движок ответит. proc — запущенный нами процесс: если он уже завершился, ждать нечего."""
    end = time.time() + seconds
    while time.time() < end:
        try:
            if client.available():
                return True
        except Exception:
            pass
        if proc is not None and proc.poll() is not None:
            return False
        time.sleep(0.3)
    return False


def ensure():
    """Поднимает движок, если он не запущен. -> "running" | "task" | "spawned" | None."""
    with _lock:
        return _ensure()


def _ensure():
    global _spawned
    client = _client()
    if client is None or not os.path.exists(EXE):
        return None
    if _wait(client, 0.5):
        return "running"
    # SEMSEARCH_ENGINE_PIPE — свой канал для разработки: установленный движок (задача Планировщика) не трогаем
    dev_pipe = os.environ.get("SEMSEARCH_ENGINE_PIPE")
    if not dev_pipe:
        try:  # вывод не разбираем (язык Windows любой) — смотрим только на код возврата
            r = subprocess.run([SCHTASKS, "/run", "/tn", TASK], capture_output=True, creationflags=NO_WINDOW,
                               timeout=20)
            if r.returncode == 0 and _wait(client, 10):
                log.info("движок запущен через задачу Планировщика (MFT)")
                return "task"
        except (OSError, subprocess.SubprocessError):  # нет schtasks.exe, служба Планировщика не отвечает
            pass
    args = [EXE] + (["--pipe", dev_pipe] if dev_pipe else [])
    try:
        _spawned = subprocess.Popen(args, cwd=os.path.dirname(EXE), creationflags=NO_WINDOW, close_fds=True)
    except OSError as ex:  # антивирус заблокировал exe, нет прав на запуск из этой папки
        log.warning("не удалось запустить движок %s: %s", EXE, ex)
        return None
    if _wait(client, 10, _spawned):
        log.info("движок запущен без прав администратора (обход диска)")
        return "spawned"
    code = _spawned.poll()
    if code is not None:  # завершился сразу: чаще всего не нашлась DLL (0xC0000135) или exe заблокирован
        log.warning("движок завершился сразу после запуска, код 0x%08X", code & 0xFFFFFFFF)
    else:
        log.warning("движок не ответил после запуска")
    return None


def stop():
    """Останавливает движок при выходе из SemSearch."""
    client = _client()
    try:
        if client is not None and client.available():
            client.request("shutdown")
    except Exception:
        pass
    if _spawned is not None and _spawned.poll() is None:
        try:
            _spawned.wait(3)
        except subprocess.TimeoutExpired:
            _spawned.kill()
