"""Мгновенное отслеживание изменений через уведомления Windows (ReadDirectoryChangesW, библиотека watchdog)."""
import logging
import os

from watchdog.events import FileSystemEventHandler
from watchdog.observers import Observer

import core
import winfs

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
    observer.daemon = True
    observer.start()  # сначала поток обработки, потом корни: так недоступный корень не роняет остальные
    watched = []
    for r in top:
        try:
            # Съёмные носители (флешка, внешний диск по USB) не отслеживаем: пока открыт дескриптор слежения,
            # Windows не даёт «Безопасно извлечь» устройство. Их изменения подхватит периодический обход.
            if winfs.is_removable(r):
                log.info("не слежу за съёмным носителем %s (чтобы его можно было извлечь)", r)
                continue
        except Exception:
            pass
        try:
            observer.schedule(handler, r, recursive=True)
            watched.append(r)
        except OSError as ex:  # нет доступа к корню диска (права, диск пропал) — остальные корни живут
            log.warning("не могу следить за %s: %s", r, ex)
    indexer.state["watching"] = bool(watched)
    log.info("слежу за изменениями: %s", ", ".join(watched))
    return observer
