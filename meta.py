"""Подписи для интерфейса: «крошки» папок и человеческие названия типов файлов."""
import os

HOME = os.path.expanduser("~")
SEP = " › "

# Известные папки профиля — как их называет Проводник
KNOWN_DIRS = {
    "desktop": "Рабочий стол", "documents": "Документы", "downloads": "Загрузки", "pictures": "Изображения",
    "videos": "Видео", "music": "Музыка", "onedrive": "OneDrive", "saved games": "Сохранённые игры",
    "3d objects": "Объёмные объекты", "contacts": "Контакты", "favorites": "Избранное", "links": "Ссылки",
    "searches": "Поиски",
}

TYPES = {
    "pdf": "PDF-документ", "doc": "Документ Word", "docx": "Документ Word", "dotx": "Шаблон Word",
    "rtf": "Документ RTF", "odt": "Документ ODT", "xls": "Таблица Excel", "xlsx": "Таблица Excel",
    "csv": "Таблица CSV", "ppt": "Презентация PowerPoint", "pptx": "Презентация PowerPoint",
    "txt": "Текстовый файл", "md": "Markdown", "markdown": "Markdown", "rst": "reStructuredText",
    "org": "Org-файл", "tex": "LaTeX", "log": "Журнал", "json": "JSON", "xml": "XML", "yaml": "YAML",
    "yml": "YAML", "toml": "TOML", "ini": "Настройки INI", "cfg": "Настройки",
    "py": "Python", "js": "JavaScript", "mjs": "JavaScript", "cjs": "JavaScript", "ts": "TypeScript",
    "tsx": "TypeScript React", "jsx": "JavaScript React", "java": "Java", "kt": "Kotlin", "kts": "Kotlin Script",
    "cs": "C#", "cpp": "C++", "cc": "C++", "cxx": "C++", "c": "C", "h": "Заголовок C/C++", "hpp": "Заголовок C++",
    "go": "Go", "rs": "Rust", "rb": "Ruby", "php": "PHP", "lua": "Lua", "sql": "SQL", "sh": "Скрипт Shell",
    "ps1": "Скрипт PowerShell", "bat": "Пакетный файл", "cmd": "Пакетный файл", "html": "HTML", "htm": "HTML",
    "css": "CSS", "scss": "SCSS", "vue": "Vue", "svelte": "Svelte", "swift": "Swift", "dart": "Dart",
    "gd": "GDScript", "shader": "Шейдер", "hlsl": "Шейдер HLSL", "glsl": "Шейдер GLSL", "gradle": "Gradle",
    "jpg": "JPEG", "jpeg": "JPEG", "png": "PNG", "webp": "WebP", "bmp": "BMP", "gif": "GIF", "tif": "TIFF",
    "tiff": "TIFF", "heic": "HEIC", "svg": "Рисунок SVG", "ico": "Значок",
    "mp4": "Видео MP4", "mkv": "Видео MKV", "avi": "Видео AVI", "mov": "Видео MOV", "webm": "Видео WebM",
    "wmv": "Видео WMV", "m4v": "Видео M4V",
    "mp3": "Аудио MP3", "wav": "Аудио WAV", "flac": "Аудио FLAC", "ogg": "Аудио OGG", "m4a": "Аудио M4A",
    "opus": "Аудио Opus", "aac": "Аудио AAC", "wma": "Аудио WMA",
    "zip": "Архив ZIP", "rar": "Архив RAR", "7z": "Архив 7Z", "tar": "Архив TAR", "gz": "Архив GZ",
    "exe": "Приложение", "msi": "Установщик", "dll": "Библиотека DLL", "lnk": "Ярлык", "iso": "Образ диска",
    "torrent": "Торрент", "asd": "Автокопия Word",
}


def ext_of(name):
    e = os.path.splitext(name)[1]
    return e[1:].upper() if e else ""


def type_name(ext):
    ext = ext.lower()
    return TYPES.get(ext, f"Файл {ext.upper()}" if ext else "Файл")


def crumbs(path):
    """Папка файла для показа: «Документы › Дом › Ремонт крыши», «C: › Program Files › …»."""
    d = os.path.dirname(path)
    nd, home = os.path.normcase(d), os.path.normcase(HOME)
    if nd == home:
        return "Папка пользователя"
    if nd.startswith(home + os.sep):
        parts = d[len(HOME) + 1:].split(os.sep)
        parts[0] = KNOWN_DIRS.get(parts[0].lower(), parts[0])
        return SEP.join(parts)
    drive, rest = os.path.splitdrive(d)
    parts = [p for p in rest.split(os.sep) if p]
    return SEP.join([drive] + parts) if parts else drive + "\\"


def plural(n, forms):
    """plural(5, ("страница", "страницы", "страниц")) -> "страниц"."""
    a, b = n % 10, n % 100
    return forms[0 if a == 1 and b != 11 else 1 if 2 <= a <= 4 and not 12 <= b <= 14 else 2]


def ts(sec):
    sec = int(sec or 0)
    return f"{sec // 3600}:{sec % 3600 // 60:02d}:{sec % 60:02d}" if sec >= 3600 else f"{sec // 60}:{sec % 60:02d}"
