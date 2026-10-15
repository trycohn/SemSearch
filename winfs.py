"""Мелочи Windows-файловой системы: длинные пути (\\\\?\\) и тип тома (HDD, сеть, съёмный).
Без зависимостей от остального кода: его импортируют и процессы разбора, и обход, и наблюдатель."""
import ctypes
import functools
import os
import struct
from ctypes import wintypes

LONG_AT = 240  # с какой длины путь переводим в форму \\?\: MAX_PATH — 259 знаков для файла и 247 для папки

_REPARSE = 0x400
_CLOUD = 0x1000 | 0x40000 | 0x400000  # OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS: данных нет на диске
_TAG_CLOUD = 0x9000001A  # IO_REPARSE_TAG_CLOUD_*: нижние 12 бит — тип, биты 12–15 — номер варианта (OneDrive и др.)

DRIVE_REMOVABLE, DRIVE_FIXED, DRIVE_REMOTE, DRIVE_CDROM = 2, 3, 4, 5


def long_path(p):
    """Длинный путь в форме \\\\?\\C:\\… (или \\\\?\\UNC\\…). Windows с настройкой по умолчанию (LongPathsEnabled=0)
    отвечает «файл не найден» на путь длиннее 259 знаков, и Python здесь не помогает. Короткие и относительные
    пути возвращает как есть (тот же объект)."""
    if len(p) < LONG_AT or p.startswith("\\\\?\\") or p.startswith("\\\\.\\"):
        return p
    if p.startswith("\\\\"):
        return "\\\\?\\UNC\\" + os.path.normpath(p)[2:]
    if len(p) > 2 and p[1] == ":" and p[2] in "\\/":
        return "\\\\?\\" + os.path.normpath(p)  # после префикса Windows сама ничего не нормализует
    return p


def plain_path(p):
    """Обратное long_path: убирает \\\\?\\ (для показа и записи в индекс)."""
    if p.startswith("\\\\?\\UNC\\"):
        return "\\\\" + p[8:]
    if p.startswith("\\\\?\\") and len(p) > 6 and p[5] == ":":
        return p[4:]
    return p


def is_cloud_placeholder(st):
    """Облачная заглушка без данных на диске (OneDrive и др.): чтение скачает файл из сети. st — os.stat_result."""
    return bool(getattr(st, "st_file_attributes", 0) & _CLOUD)


def skip_entry(st, keep_cloud_files=False):
    """Запись обхода, которую не трогаем: облачная заглушка без данных; символическая ссылка или junction (петли,
    чужие места); любая другая точка повторной обработки. Скачанные файлы OneDrive («всегда хранить на устройстве»,
    уже открывавшиеся) остаются точкой повторной обработки с облачным тегом, но данные у них на диске — их берём.
    keep_cloud_files — для индекса имён: имена файлов-заглушек известны без скачивания, они нужны (папки-заглушки
    всё равно пропускаются: чтобы их перечислить, OneDrive подтянул бы их содержимое из сети)."""
    attrs = getattr(st, "st_file_attributes", 0)
    if attrs & _CLOUD and (not keep_cloud_files or attrs & 0x10):  # 0x10 — папка
        return True
    if attrs & _REPARSE:
        tag = getattr(st, "st_reparse_tag", 0)
        return (tag & 0xFFFF0FFF) != _TAG_CLOUD
    return False


# ------------------------------------------------ тип тома

_k32 = ctypes.WinDLL("kernel32", use_last_error=True)
_k32.GetDriveTypeW.argtypes = [wintypes.LPCWSTR]
_k32.GetDriveTypeW.restype = wintypes.UINT
_k32.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD,
                             wintypes.DWORD, wintypes.HANDLE]
_k32.CreateFileW.restype = wintypes.HANDLE
_k32.DeviceIoControl.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p,
                                 wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p]
_k32.DeviceIoControl.restype = wintypes.BOOL
_k32.CloseHandle.argtypes = [wintypes.HANDLE]
_k32.CloseHandle.restype = wintypes.BOOL
_INVALID = ctypes.c_void_p(-1).value
_IOCTL_STORAGE_QUERY_PROPERTY = 0x002D1400
_BUS_USB = 7


def _drive_of(path):
    """"C:" (заглавными) или "\\\\сервер\\папка" для сетевого пути."""
    return os.path.splitdrive(os.path.abspath(path))[0].upper()


def drive_type(path):
    """GetDriveTypeW для диска, на котором лежит path: DRIVE_FIXED, DRIVE_REMOVABLE, DRIVE_REMOTE… (1 — диска нет)."""
    d = _drive_of(path)
    if d.startswith("\\\\"):
        return DRIVE_REMOTE
    return _k32.GetDriveTypeW(d + "\\") if d else 0


@functools.lru_cache(maxsize=None)
def _storage_query(drive, prop_id, out_size):
    """IOCTL_STORAGE_QUERY_PROPERTY к тому \\\\.\\X: с нулевыми правами доступа — хватает обычного пользователя,
    и дескриптор тут же закрывается (иначе «Безопасное извлечение» упёрлось бы в него). -> bytes | None."""
    h = _k32.CreateFileW("\\\\.\\" + drive, 0, 7, None, 3, 0, None)  # доступ 0; делим на чтение/запись/удаление
    if h is None or h == _INVALID:
        return None
    try:
        query = struct.pack("<IIB3x", prop_id, 0, 0)  # STORAGE_PROPERTY_QUERY: PropertyStandardQuery
        out, n = ctypes.create_string_buffer(out_size), wintypes.DWORD(0)
        if not _k32.DeviceIoControl(h, _IOCTL_STORAGE_QUERY_PROPERTY, query, len(query), out, out_size,
                                    ctypes.byref(n), None):
            return None
        return out.raw[:n.value]
    finally:
        _k32.CloseHandle(h)


def is_slow_volume(path):
    """Вращающийся диск (HDD), съёмный носитель или сеть: много потоков чтения разом на них не ускоряют, а тормозят
    (головки прыгают между файлами). Не удалось выяснить (старый драйвер, массив) — считаем, что нет."""
    t = drive_type(path)
    if t in (DRIVE_REMOTE, DRIVE_REMOVABLE, DRIVE_CDROM):
        return True
    if t != DRIVE_FIXED:
        return False
    data = _storage_query(_drive_of(path), 7, 12)  # StorageDeviceSeekPenaltyProperty: Version, Size, IncursSeekPenalty
    return bool(data and len(data) >= 9 and data[8])


def is_removable(path):
    """Съёмный носитель или внешний диск по USB (его покажут в «Безопасном извлечении»): на нём не держим
    открытых дескрипторов слежения, иначе Windows не даст его извлечь. Не удалось выяснить — False."""
    t = drive_type(path)
    if t in (DRIVE_REMOVABLE, DRIVE_CDROM):
        return True
    if t != DRIVE_FIXED:
        return False
    data = _storage_query(_drive_of(path), 0, 1024)  # StorageDeviceProperty: STORAGE_DEVICE_DESCRIPTOR
    return bool(data and len(data) >= 32 and (data[10] or struct.unpack_from("<I", data, 28)[0] == _BUS_USB))
