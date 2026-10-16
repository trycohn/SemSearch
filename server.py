"""SemSearch — локальный поиск по смыслу и по имени. Запуск: pythonw server.py [--no-browser]"""
# Весь код — в app.py. Сервер здесь запускается только под __main__: процессы разбора документов
# (multiprocessing на Windows) заново импортируют главный модуль и не должны запускать второй сервер.
import os
import sys

for _name in ("stdout", "stderr"):  # pythonw без консоли: tqdm и записи в sys.stderr падали бы на None
    if getattr(sys, _name) is None:
        setattr(sys, _name, open(os.devnull, "w", encoding="utf-8"))


def startup_failed():
    """Запуск оборвался исключением (не загрузилась библиотека, нет Visual C++, нет доступа к папке данных…).
    pythonw молчит, поэтому пишем причину в журнал (core мог не загрузиться — путь к нему выводим сами) и, если
    нас запустил не SemSearch.exe, показываем окно: иначе человек видит только «ничего не запускается»."""
    import time
    import traceback

    text = traceback.format_exc()
    data = getattr(sys.modules.get("core"), "DATA_DIR", None)
    if not data:
        env = os.environ.get("SEMSEARCH_DATA_DIR", "").strip()
        local = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~\\AppData\\Local")
        data = env if os.path.isabs(env) else os.path.join(local, "SemSearch")
    log = os.path.join(data, "semsearch.log")
    try:
        os.makedirs(data, exist_ok=True)
        with open(log, "a", encoding="utf-8") as f:
            f.write(f"{time.strftime('%Y-%m-%d %H:%M:%S')} CRITICAL запуск не удался:\n{text}\n")
    except OSError:
        pass
    if os.environ.get("SEMSEARCH_SHELL") == "1":  # SemSearch.exe сам покажет, что сервер остановился, и укажет журнал
        return
    reason = text.strip().splitlines()[-1][:300] if text.strip() else "?"
    hint = ("\n\nЧаще всего так бывает без «Распространяемого пакета Microsoft Visual C++» (x64):\n"
            "Most often this means Microsoft Visual C++ Redistributable (x64) is missing:\n"
            "https://aka.ms/vs/17/release/vc_redist.x64.exe") if "dll" in text.lower() else ""
    try:
        import ctypes

        ctypes.windll.user32.MessageBoxW(
            None, f"SemSearch не удалось запустить / SemSearch failed to start\n\n{reason}{hint}\n\n{log}",
            "SemSearch", 0x10)  # MB_ICONERROR
    except Exception:
        pass


if __name__ == "__main__":
    try:
        import app

        app.main()
    except Exception:
        startup_failed()
        sys.exit(1)
