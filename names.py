"""Поиск по имени файла: движок на Rust (MFT/USN), а пока его нет — собственный индекс names.db."""
import logging
import os
import threading
import time

import core

log = logging.getLogger("names")
CAP = 5000  # в запасном индексе больше совпадений не считаем — точное число даёт движок


def _stat(path):
    try:
        st = os.stat(path)
        return st.st_size, st.st_mtime, st.st_ctime
    except OSError:
        return None, None, None


class LocalNames:
    """FTS5-индекс в names.db (папка данных), который строит Indexer.names_loop."""

    kind = "python"

    def __init__(self):
        self.conn = core.connect_names()
        self.lock = threading.Lock()

    def _rows(self, sql, args):
        with self.lock:
            g = core.names_active(self.conn)
            return self.conn.execute(sql.format(g=g), args).fetchall()

    def search(self, q, limit=200):
        q = q.strip()
        if not q:
            return 0, []
        if "*" in q or "?" in q:  # маска: *.pdf, отчёт_??.docx
            like = q.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_").replace("*", "%").replace("?", "_")
            rows = self._rows("SELECT name, path FROM names_t_{g} WHERE name LIKE ? ESCAPE '\\' LIMIT ?", (like, CAP))
            words = []
        else:
            words = [w.lower() for w in q.split()]
            long_words = [w for w in words if len(w) >= 3 and "\\" not in w and "/" not in w]
            if long_words:
                match = " AND ".join('"' + w.replace('"', '""') + '"' for w in long_words)
                rows = self._rows("SELECT t.name, t.path FROM names_{g} f JOIN names_t_{g} t ON t.id = f.rowid "
                                  "WHERE names_{g} MATCH ? LIMIT ?", (match, CAP))
            else:
                rows = self._rows("SELECT name, path FROM names_t_{g} WHERE name LIKE ? LIMIT ?", (f"%{words[0]}%", CAP))
            rows = [r for r in rows
                    if all(w in (r[1].lower() if ("\\" in w or "/" in w) else r[0].lower()) for w in words)]
        first = words[0] if words else ""
        ql = q.lower()
        rows.sort(key=lambda r: (r[0].lower() != ql, not r[0].lower().startswith(first), len(r[0]), len(r[1])))
        items = []
        for name, path in rows[:limit]:
            size, mtime, ctime = _stat(path)
            items.append({"path": path, "name": name, "size": size, "mtime": mtime, "ctime": ctime})
        return len(rows), items

    def same_name(self, name, limit=10):
        words = [w for w in name.lower().split() if len(w) >= 3]
        if words:
            match = " AND ".join('"' + w.replace('"', '""') + '"' for w in words)
            rows = self._rows("SELECT t.name, t.path FROM names_{g} f JOIN names_t_{g} t ON t.id = f.rowid "
                              "WHERE names_{g} MATCH ? LIMIT 2000", (match,))
        else:
            rows = self._rows("SELECT name, path FROM names_t_{g} WHERE name LIKE ? LIMIT 2000", (name,))
        low = name.lower()
        return [{"path": p, "name": n} for n, p in rows if n.lower() == low][:limit]

    def exists(self, path):
        return bool(self._rows("SELECT 1 FROM names_t_{g} WHERE path=?", (path,)))

    def count(self):
        return self._rows("SELECT count(*) FROM names_t_{g}", ())[0][0]


class EngineNames:
    """Клиент движка semsearch-engine (named pipe), см. docs/API.md §4."""

    kind = "engine"

    def __init__(self, client):
        self.client = client

    def search(self, q, limit=200):
        r = self.client.request("search", q=q, mode="auto", limit=limit, files_only=True)
        return r["total"], r["items"]

    def same_name(self, name, limit=10):
        return self.client.request("same_name", name=name, limit=limit)["items"]

    def exists(self, path):
        return bool(self.client.request("exists", path=path).get("exists"))

    def status(self):
        return self.client.request("status")


class NamesProvider:
    """Отдаёт движок, если он отвечает, иначе запасной индекс. Доступность перепроверяется раз в 5 с."""

    def __init__(self):
        self.local = LocalNames()
        self.engine = None
        self._checked = 0.0
        self._lock = threading.Lock()
        try:
            from engine_client import EngineClient

            self._client = EngineClient()
        except Exception:
            self._client = None

    def current(self):
        now = time.time()
        if self._client is not None and now - self._checked > 5:
            with self._lock:
                self._checked = now
                try:
                    # Пока движок строит индекс, его ответы неполные — ищем по запасному индексу
                    ok = self._client.available() and bool(self._client.request("status").get("ready"))
                except Exception:
                    ok = False
                if ok and self.engine is None:
                    log.info("поиск по имени: подключён движок")
                    self.engine = EngineNames(self._client)
                elif not ok and self.engine is not None:
                    log.warning("поиск по имени: движок не отвечает, работаю на запасном индексе")
                    self.engine = None
        return self.engine or self.local

    def reachable(self):
        """Движок на связи, пусть даже ещё строит индекс, — тогда свой индекс имён не строим (он сделает это сам)."""
        try:
            return self._client is not None and self._client.available()
        except Exception:
            return False

    def _call(self, method, *args, **kw):
        p = self.current()
        try:
            return getattr(p, method)(*args, **kw)
        except Exception:
            if p is self.local:
                raise
            log.exception("ошибка движка — повтор на запасном индексе")
            self.engine = None
            return getattr(self.local, method)(*args, **kw)

    def search(self, q, limit=200):
        return self._call("search", q, limit)

    def same_name(self, name, limit=10):
        return self._call("same_name", name, limit)

    def exists(self, path):
        return self._call("exists", path)

    def info(self):
        """-> (count, engine_kind, ready)."""
        p = self.current()
        if p is self.local:
            return self.local.count(), "python", True
        try:
            st = p.status()
            return st.get("files", 0), st.get("mode", "engine"), bool(st.get("ready"))
        except Exception:
            return self.local.count(), "python", True
