"""Запуск и остановка движка имён (bin/semsearch-engine.exe).

Если пользователь установил задачу Планировщика «SemSearch Engine» (engine/install-task.ps1), движок
стартует через неё с правами администратора и читает MFT/USN — без запроса UAC при каждом запуске.
Иначе backend запускает его сам, без прав: обход диска + ReadDirectoryChangesW."""
import logging
import os
import subprocess
import time

import core

log = logging.getLogger("engine")
EXE = os.path.join(core.APP_DIR, "bin", "semsearch-engine.exe")
TASK = "SemSearch Engine"
NO_WINDOW = 0x08000000  # CREATE_NO_WINDOW

_spawned = None  # процесс, если движок запускали мы сами


def _client():
    try:
        from engine_client import EngineClient

        return EngineClient()
    except Exception:
        return None


def _wait(client, seconds):
    end = time.time() + seconds
    while time.time() < end:
        try:
            if client.available():
                return True
        except Exception:
            pass
        time.sleep(0.3)
    return False


def ensure():
    """Поднимает движок, если он не запущен. -> "running" | "task" | "spawned" | None."""
    global _spawned
    client = _client()
    if client is None or not os.path.exists(EXE):
        return None
    if _wait(client, 0.5):
        return "running"
    r = subprocess.run(["schtasks", "/run", "/tn", TASK], capture_output=True, creationflags=NO_WINDOW)
    if r.returncode == 0 and _wait(client, 10):
        log.info("движок запущен через задачу Планировщика (MFT)")
        return "task"
    _spawned = subprocess.Popen([EXE], cwd=os.path.dirname(EXE), creationflags=NO_WINDOW, close_fds=True)
    if _wait(client, 10):
        log.info("движок запущен без прав администратора (обход диска)")
        return "spawned"
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
