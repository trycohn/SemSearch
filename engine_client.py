"""Клиент движка имён semsearch-engine (named pipe \\\\.\\pipe\\semsearch-engine-<SessionId>, docs/API.md §4).

Без зависимостей (ctypes + перекрытый ввод-вывод, чтобы работали таймауты).

    from engine_client import EngineClient
    ec = EngineClient()
    if EngineClient.available():
        r = ec.request("search", q="отчёт *.pdf", limit=50)
        sub = ec.subscribe([os.path.expanduser("~")], lambda ev: print(ev))
        ...
        sub.stop()

CLI: python engine_client.py ping | status | search <q> | list <root> [ext ...] | same <name>
                            | exists <path> | subscribe [root ...] | reload | shutdown
"""

import ctypes
import json
import os
import sys
import threading
import time


def _session_id():
    """Номер сеанса Windows этого процесса (ProcessIdToSessionId)."""
    sid = ctypes.c_ulong(0)
    if sys.platform == "win32" and ctypes.WinDLL("kernel32").ProcessIdToSessionId(os.getpid(), ctypes.byref(sid)):
        return sid.value
    return 0


# В каждом сеансе Windows свой канал: движки разных вошедших пользователей не мешают друг другу
PIPE_NAME = rf"\\.\pipe\semsearch-engine-{_session_id()}"

__all__ = ["EngineClient", "EngineError", "EngineUnavailable", "Subscription", "PIPE_NAME"]


class EngineError(Exception):
    """Движок ответил ok=false."""


class EngineUnavailable(EngineError):
    """Канал движка недоступен (не запущен, занят, таймаут)."""


if sys.platform == "win32":
    from ctypes import wintypes as _wt

    _k32 = ctypes.WinDLL("kernel32", use_last_error=True)

    class _OVERLAPPED(ctypes.Structure):
        _fields_ = [("Internal", ctypes.c_size_t), ("InternalHigh", ctypes.c_size_t),
                    ("Offset", _wt.DWORD), ("OffsetHigh", _wt.DWORD), ("hEvent", _wt.HANDLE)]

    _CreateFileW = _k32.CreateFileW
    _CreateFileW.argtypes = [_wt.LPCWSTR, _wt.DWORD, _wt.DWORD, ctypes.c_void_p, _wt.DWORD, _wt.DWORD, _wt.HANDLE]
    _CreateFileW.restype = _wt.HANDLE
    _ReadFile = _k32.ReadFile
    _ReadFile.argtypes = [_wt.HANDLE, ctypes.c_void_p, _wt.DWORD, ctypes.POINTER(_wt.DWORD), ctypes.POINTER(_OVERLAPPED)]
    _ReadFile.restype = _wt.BOOL
    _WriteFile = _k32.WriteFile
    _WriteFile.argtypes = [_wt.HANDLE, ctypes.c_void_p, _wt.DWORD, ctypes.POINTER(_wt.DWORD), ctypes.POINTER(_OVERLAPPED)]
    _WriteFile.restype = _wt.BOOL
    _GetOverlappedResult = _k32.GetOverlappedResult
    _GetOverlappedResult.argtypes = [_wt.HANDLE, ctypes.POINTER(_OVERLAPPED), ctypes.POINTER(_wt.DWORD), _wt.BOOL]
    _GetOverlappedResult.restype = _wt.BOOL
    _CancelIoEx = _k32.CancelIoEx
    _CancelIoEx.argtypes = [_wt.HANDLE, ctypes.POINTER(_OVERLAPPED)]
    _CancelIoEx.restype = _wt.BOOL
    _CreateEventW = _k32.CreateEventW
    _CreateEventW.argtypes = [ctypes.c_void_p, _wt.BOOL, _wt.BOOL, _wt.LPCWSTR]
    _CreateEventW.restype = _wt.HANDLE
    _WaitForSingleObject = _k32.WaitForSingleObject
    _WaitForSingleObject.argtypes = [_wt.HANDLE, _wt.DWORD]
    _WaitForSingleObject.restype = _wt.DWORD
    _CloseHandle = _k32.CloseHandle
    _CloseHandle.argtypes = [_wt.HANDLE]
    _CloseHandle.restype = _wt.BOOL
    _WaitNamedPipeW = _k32.WaitNamedPipeW
    _WaitNamedPipeW.argtypes = [_wt.LPCWSTR, _wt.DWORD]
    _WaitNamedPipeW.restype = _wt.BOOL

_GENERIC_READ = 0x80000000
_GENERIC_WRITE = 0x40000000
_OPEN_EXISTING = 3
_FILE_FLAG_OVERLAPPED = 0x40000000
_INVALID_HANDLE = ctypes.c_void_p(-1).value
_ERROR_FILE_NOT_FOUND = 2
_ERROR_PIPE_BUSY = 231
_ERROR_IO_PENDING = 997
_ERROR_BROKEN_PIPE = 109
_ERROR_OPERATION_ABORTED = 995
_WAIT_OBJECT_0 = 0
_WAIT_TIMEOUT = 0x102


class _Pipe:
    """Одно соединение с каналом (перекрытый ввод-вывод с таймаутами)."""

    def __init__(self, timeout):
        if sys.platform != "win32":
            raise EngineUnavailable("named pipes need Windows")
        deadline = time.monotonic() + timeout
        while True:
            h = _CreateFileW(PIPE_NAME, _GENERIC_READ | _GENERIC_WRITE, 0, None, _OPEN_EXISTING,
                             _FILE_FLAG_OVERLAPPED, None)
            if h and h != _INVALID_HANDLE:
                break
            err = ctypes.get_last_error()
            left = deadline - time.monotonic()
            if err == _ERROR_PIPE_BUSY and left > 0:
                _WaitNamedPipeW(PIPE_NAME, max(1, min(int(left * 1000), 1000)))
                continue
            if err == _ERROR_FILE_NOT_FOUND:
                raise EngineUnavailable("engine is not running")
            raise EngineUnavailable(f"cannot open {PIPE_NAME} (error {err})")
        self.h = h
        self.ev = _CreateEventW(None, True, False, None)
        self.buf = bytearray()
        self.rbuf = ctypes.create_string_buffer(1 << 20)  # одно чтение за раз — буфер переиспользуется
        self.closed = False

    def close(self):
        if not self.closed:
            self.closed = True
            _CloseHandle(self.h)
            _CloseHandle(self.ev)

    def _io(self, write, data_or_size, timeout, stop=None):
        """Одна операция чтения/записи. timeout=None — ждать бесконечно (с проверкой stop)."""
        ov = _OVERLAPPED()
        ov.hEvent = self.ev
        n = _wt.DWORD(0)
        if write:
            cbuf = ctypes.create_string_buffer(bytes(data_or_size), len(data_or_size))
            ok = _WriteFile(self.h, cbuf, len(data_or_size), None, ctypes.byref(ov))
        else:
            cbuf = self.rbuf
            ok = _ReadFile(self.h, cbuf, min(data_or_size, len(cbuf)), None, ctypes.byref(ov))
        if not ok:
            err = ctypes.get_last_error()
            if err != _ERROR_IO_PENDING:
                raise EngineUnavailable(f"pipe {'write' if write else 'read'} failed (error {err})")
        deadline = None if timeout is None else time.monotonic() + timeout
        while True:
            wait_ms = 500 if deadline is None else max(0, min(500, int((deadline - time.monotonic()) * 1000)))
            r = _WaitForSingleObject(self.ev, wait_ms)
            if r == _WAIT_OBJECT_0:
                break
            if (stop is not None and stop.is_set()) or (deadline is not None and time.monotonic() >= deadline):
                _CancelIoEx(self.h, ctypes.byref(ov))
                _GetOverlappedResult(self.h, ctypes.byref(ov), ctypes.byref(n), True)
                raise EngineUnavailable("stopped" if stop is not None and stop.is_set() else "engine timeout")
        if not _GetOverlappedResult(self.h, ctypes.byref(ov), ctypes.byref(n), False):
            err = ctypes.get_last_error()
            raise EngineUnavailable(f"pipe {'write' if write else 'read'} failed (error {err})")
        return n.value if write else ctypes.string_at(cbuf, n.value)

    def send_line(self, obj, timeout):
        data = (json.dumps(obj, ensure_ascii=False) + "\n").encode("utf-8")
        deadline = time.monotonic() + timeout
        while data:
            n = self._io(True, data, max(0.05, deadline - time.monotonic()))
            data = data[n:]

    def read_line(self, timeout=None, stop=None):
        deadline = None if timeout is None else time.monotonic() + timeout
        start = 0
        while True:
            i = self.buf.find(b"\n", start)
            if i >= 0:
                line = bytes(self.buf[:i])
                del self.buf[:i + 1]
                return line
            start = len(self.buf)
            left = None if deadline is None else max(0.05, deadline - time.monotonic())
            chunk = self._io(False, 1 << 20, left, stop)
            if not chunk:
                raise EngineUnavailable("pipe closed")
            self.buf += chunk


class Subscription:
    """Фоновый поток подписки; переподключается сам. После переподключения вызывает
    on_event({"ev": "rescan", ...}) — события за время разрыва могли потеряться."""

    def __init__(self, client, roots, on_event):
        self.client, self.roots, self.on_event = client, list(roots or []), on_event
        self._stop = threading.Event()
        self.connected = threading.Event()
        self._pipe = None
        self.thread = threading.Thread(target=self._run, name="engine-subscribe", daemon=True)
        self.thread.start()

    def stop(self, wait=True):
        """Остановить подписку (поток сам отменит ожидание чтения в течение ~0,5 с)."""
        self._stop.set()
        if wait and self.thread is not threading.current_thread():
            self.thread.join(2.0)

    def _run(self):
        backoff, first = 0.5, True
        while not self._stop.is_set():
            try:
                p = _Pipe(self.client.timeout)
                self._pipe = p
                try:
                    p.send_line({"id": 1, "op": "subscribe", "roots": self.roots}, self.client.timeout)
                    ack = json.loads(p.read_line(self.client.timeout))
                    if not ack.get("ok"):
                        raise EngineError(ack.get("error") or "subscribe failed")
                    self.connected.set()
                    if not first:
                        for r in (self.roots or [""]):
                            self._deliver({"ev": "rescan", "path": r, "old": None, "dir": True})
                    first, backoff = False, 0.5
                    while not self._stop.is_set():
                        line = p.read_line(None, self._stop)
                        if line.strip():
                            self._deliver(json.loads(line))
                finally:
                    self.connected.clear()
                    self._pipe = None
                    p.close()
            except Exception:
                if self._stop.is_set():
                    break
                self._stop.wait(backoff)
                backoff = min(backoff * 2, 10.0)

    def _deliver(self, ev):
        try:
            self.on_event(ev)
        except Exception:
            pass


class EngineClient:
    """Потокобезопасный клиент запрос/ответ с автопереподключением."""

    def __init__(self, timeout=10.0):
        self.timeout = timeout
        self._lock = threading.Lock()
        self._pipe = None
        self._id = 0

    @staticmethod
    def available(timeout=0.5):
        """Запущен ли движок (канал существует и отвечает на ping)."""
        try:
            p = _Pipe(timeout)
        except EngineError:
            return False
        try:
            p.send_line({"id": 0, "op": "ping"}, timeout)
            return bool(json.loads(p.read_line(max(timeout, 2.0))).get("ok"))
        except Exception:
            return False
        finally:
            p.close()

    def close(self):
        with self._lock:
            if self._pipe is not None:
                self._pipe.close()
                self._pipe = None

    def request(self, op, timeout=None, **kw):
        """Отправить запрос и вернуть ответ (dict без "id"/"ok"). EngineError при ok=false,
        EngineUnavailable — движок недоступен. Одна повторная попытка при обрыве."""
        timeout = self.timeout if timeout is None else timeout
        with self._lock:
            for attempt in (0, 1):
                fresh = self._pipe is None
                try:
                    if self._pipe is None:
                        self._pipe = _Pipe(timeout)
                    self._id += 1
                    rid = self._id
                    self._pipe.send_line(dict(kw, id=rid, op=op), timeout)
                    while True:
                        resp = json.loads(self._pipe.read_line(timeout))
                        if resp.get("id") == rid:
                            break
                except EngineUnavailable as e:
                    if self._pipe is not None:
                        self._pipe.close()
                        self._pipe = None
                    if attempt == 0 and not fresh and "timeout" not in str(e):
                        continue
                    raise
                if not resp.pop("ok", False):
                    raise EngineError(resp.get("error") or "engine error")
                resp.pop("id", None)
                return resp

    def subscribe(self, roots, on_event):
        """Поток событий для путей под roots: on_event(dict) вызывается из фонового потока."""
        return Subscription(self, roots, on_event)


def _main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    cmd, args = argv[1], argv[2:]
    try:
        sys.stdout.reconfigure(errors="replace")
    except Exception:
        pass
    ec = EngineClient(timeout=30)
    try:
        if cmd == "ping":
            r = ec.request("ping")
        elif cmd == "status":
            r = ec.request("status")
        elif cmd == "search":
            r = ec.request("search", q=" ".join(args), limit=20)
            print(f"total={r['total']} ms={r['ms']} search_ms={r.get('search_ms')}")
            for it in r["items"]:
                print(f"  {it['path']}  size={it['size']} dir={it['dir']}")
            return 0
        elif cmd == "list":
            root = args[0] if args else os.environ.get("SystemDrive", "C:") + "\\"
            exts = args[1:] or [".pdf", ".docx", ".py", ".jpg"]
            t = time.perf_counter()
            r = ec.request("list", roots=[root], exts=exts, exclude_dirs=[], exclude_paths=[],
                           skip_dot_dirs=True, skip_cloud=True)
            dt = (time.perf_counter() - t) * 1000
            print(f"items={len(r['items'])} engine_ms={r.get('ms')} roundtrip_ms={dt:.0f}")
            for it in r["items"][:10]:
                print(f"  {it['path']}  {it['size']}")
            return 0
        elif cmd == "same":
            r = ec.request("same_name", name=args[0] if args else "", limit=10)
        elif cmd == "exists":
            r = ec.request("exists", path=args[0] if args else "")
        elif cmd == "reload":
            r = ec.request("reload_config")
        elif cmd == "shutdown":
            r = ec.request("shutdown")
        elif cmd == "subscribe":
            sub = ec.subscribe(args, lambda ev: print(json.dumps(ev, ensure_ascii=False), flush=True))
            try:
                while True:
                    time.sleep(1)
            except KeyboardInterrupt:
                sub.stop()
            return 0
        else:
            print(__doc__)
            return 2
    except EngineError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    print(json.dumps(r, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(_main(sys.argv))
