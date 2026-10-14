"""Мгновенное отслеживание изменений через уведомления Windows (ReadDirectoryChangesW, библиотека watchdog)."""
import logging
import os

from watchdog.events import FileSystemEventHandler
from watchdog.observers import Observer

import core

log = logging.getLogger("watcher")


class _Handler(FileSystemEventHandler):
    def __init__(self, indexer):
        self.indexer = indexer
        # Свою базу и превью не отслеживаем — иначе каждая запись в индекс порождала бы новое событие
        self.ignore = os.path.normcase(core.DATA_DIR)

    def on_any_event(self, event):
        if event.event_type not in ("created", "modified", "deleted", "moved"):
            return
        if event.is_directory and event.event_type == "modified":
            return  # «в папке что-то поменялось» — сами файлы придут отдельными событиями
        src = event.src_path
        dst = getattr(event, "dest_path", "") or ""
        if os.path.normcase(src).startswith(self.ignore):
            return
        try:
            self.indexer.on_fs_event(event.event_type, src, dst)
        except Exception:
            log.exception("ошибка обработки события %s", src)


def start(indexer, cfg):
    """Один рекурсивный наблюдатель на каждый корень (вложенные корни не дублируются)."""
    # Пути в событиях начинаются ровно с того, что передали в schedule — поэтому регистр сохраняем
    # как есть (иначе одни и те же файлы попали бы в базу в двух написаниях)
    roots = sorted({r.rstrip("\\/") + "\\" for r in cfg["content_roots"] + cfg["name_roots"]}, key=len)
    top = []
    for r in roots:
        if not any(os.path.normcase(r).startswith(os.path.normcase(t)) for t in top) and os.path.isdir(r):
            top.append(r)
    observer = Observer()
    handler = _Handler(indexer)
    for r in top:
        observer.schedule(handler, r, recursive=True)
    observer.daemon = True
    observer.start()
    indexer.state["watching"] = True
    log.info("слежу за изменениями: %s", ", ".join(top))
    return observer
