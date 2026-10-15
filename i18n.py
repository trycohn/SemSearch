"""Язык интерфейса для подписей, которые собирает backend: «крошки» папок, типы файлов, «стр. 3», причины пропуска,
тексты ошибок API. Интерфейс (ui/i18n.js) переводит свои строки сам; здесь — только то, что приходит из API.

Настройка config.json "lang": "system" | "ru" | "en". "system" — язык интерфейса Windows: русский → ru, иначе en
(так же решает ui/i18n.js по navigator.language и shell — по GetUserDefaultUILanguage)."""
import ctypes
import re

LANGS = ("ru", "en")
_LANG_RUSSIAN = 0x19  # PRIMARYLANGID


def system_lang():
    try:
        return "ru" if ctypes.windll.kernel32.GetUserDefaultUILanguage() & 0x3FF == _LANG_RUSSIAN else "en"
    except Exception:
        return "ru"


def resolve(pref):
    """"system" | "ru" | "en" | None -> "ru" | "en"."""
    return pref if pref in LANGS else system_lang()


# ------------------------------------------------ известные папки профиля (как их называет Проводник)

KNOWN_DIRS = {
    "ru": {"desktop": "Рабочий стол", "documents": "Документы", "downloads": "Загрузки", "pictures": "Изображения",
           "videos": "Видео", "music": "Музыка", "onedrive": "OneDrive", "saved games": "Сохранённые игры",
           "3d objects": "Объёмные объекты", "contacts": "Контакты", "favorites": "Избранное", "links": "Ссылки",
           "searches": "Поиски"},
    "en": {"desktop": "Desktop", "documents": "Documents", "downloads": "Downloads", "pictures": "Pictures",
           "videos": "Videos", "music": "Music", "onedrive": "OneDrive", "saved games": "Saved Games",
           "3d objects": "3D Objects", "contacts": "Contacts", "favorites": "Favorites", "links": "Links",
           "searches": "Searches"},
}
USER_FOLDER = {"ru": "Папка пользователя", "en": "User folder"}

# ------------------------------------------------ типы файлов

_TYPES_RU = {
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
_TYPES_EN = {
    "pdf": "PDF document", "doc": "Word document", "docx": "Word document", "dotx": "Word template",
    "rtf": "RTF document", "odt": "ODT document", "xls": "Excel spreadsheet", "xlsx": "Excel spreadsheet",
    "csv": "CSV spreadsheet", "ppt": "PowerPoint presentation", "pptx": "PowerPoint presentation",
    "txt": "Text file", "org": "Org file", "log": "Log", "ini": "INI settings", "cfg": "Settings",
    "h": "C/C++ header", "hpp": "C++ header", "sh": "Shell script", "ps1": "PowerShell script",
    "bat": "Batch file", "cmd": "Batch file", "shader": "Shader", "hlsl": "HLSL shader", "glsl": "GLSL shader",
    "svg": "SVG image", "ico": "Icon",
    "mp4": "MP4 video", "mkv": "MKV video", "avi": "AVI video", "mov": "MOV video", "webm": "WebM video",
    "wmv": "WMV video", "m4v": "M4V video",
    "mp3": "MP3 audio", "wav": "WAV audio", "flac": "FLAC audio", "ogg": "OGG audio", "m4a": "M4A audio",
    "opus": "Opus audio", "aac": "AAC audio", "wma": "WMA audio",
    "zip": "ZIP archive", "rar": "RAR archive", "7z": "7Z archive", "tar": "TAR archive", "gz": "GZ archive",
    "exe": "Application", "msi": "Installer", "dll": "DLL library", "lnk": "Shortcut", "iso": "Disc image",
    "torrent": "Torrent", "asd": "Word AutoRecover file",
}


def type_name(ext, lang):
    ext = (ext or "").lower()
    if lang == "en":
        name = _TYPES_EN.get(ext) or _TYPES_RU.get(ext)
        if name and re.search("[А-Яа-яЁё]", name):  # русское название без английского — не показываем
            name = None
        return name or (f"{ext.upper()} file" if ext else "File")
    return _TYPES_RU.get(ext, f"Файл {ext.upper()}" if ext else "Файл")


def plural_ru(n, forms):
    """plural_ru(5, ("страница", "страницы", "страниц")) -> "страниц"."""
    a, b = n % 10, n % 100
    return forms[0 if a == 1 and b != 11 else 1 if 2 <= a <= 4 and not 12 <= b <= 14 else 2]


def pages(n, lang):
    """"18 страниц" / "18 pages"."""
    if lang == "en":
        return f"{n} page" + ("" if n == 1 else "s")
    return f"{n} {plural_ru(n, ('страница', 'страницы', 'страниц'))}"


# ------------------------------------------------ место совпадения внутри файла (подписи фрагментов в индексе — по-русски)

_LOC = re.compile(r"^(стр\.|фрагмент|фрагм\.)\s*(\d+)(\s*\(скан\))?$")


def loc(label, lang):
    """Подпись места: «стр. 3» / «фрагм. 2» / «стр. 4 (скан)» -> на языке интерфейса. Время (1:20) — как есть."""
    label = (label or "").strip()
    m = _LOC.match(label)
    if not m:
        return label
    page = m.group(1) == "стр."
    n, scan = m.group(2), bool(m.group(3))
    if lang == "en":
        return (f"p. {n}" if page else f"chunk {n}") + (" (scan)" if scan else "")
    return (f"стр. {n}" if page else f"фрагм. {n}") + (" (скан)" if scan else "")


def page_label(page, total, lang):
    return f"p. {page} of {total}" if lang == "en" else f"стр. {page} из {total}"


def chunk_label(i, total, lang):
    return f"chunk {i} of {total}" if lang == "en" else f"фрагмент {i} из {total}"


def video_hit(lang):
    return "frame matched the query" if lang == "en" else "кадр совпал с запросом"


def audio_hit(start, end, total, lang):
    """"отрезок 0:30–1:00 из 1:52" / "segment 0:30–1:00 of 1:52" (время уже строками)."""
    if lang == "en":
        return f"segment {start}–{end}" + (f" of {total}" if total else "")
    return f"отрезок {start}–{end}" + (f" из {total}" if total else "")


# ------------------------------------------------ причины пропуска и ошибки чтения (хранятся в индексе по-русски)

_REASONS_EN = {
    "слишком большой": "too large",
    "бинарный": "binary file",
    "минифицированный": "minified",
    "нет текста": "no text",
    "маленькая картинка": "image too small",
    "нет видеодорожки": "no video track",
    "неизвестная длительность": "unknown duration",
    "не удалось прочитать кадры": "couldn't read the frames",
    "нет аудио": "no audio",
    "пустое аудио": "empty audio",
    "сгенерированный файл": "generated file",
    "похоже на чужую библиотеку": "looks like a third-party library",
    "чужой код: git-подмодуль": "third-party code: git submodule",
    "не исходник: так считает .gitignore проекта": "not a source file: the project's .gitignore says so",
    "чужой код: скачанный архив исходников": "third-party code: downloaded source archive",
    "разбор не уложился в 3 минуты": "reading took more than 3 minutes",
    "сбой разбора файла": "the file reader crashed",
}


def reason(text, lang):
    """Причина пропуска/ошибки из индекса -> на языке интерфейса. Текст исключения (KeyError: …) не переводится."""
    if not text or lang != "en":
        return text
    t = str(text)
    if t in _REASONS_EN:
        return _REASONS_EN[t]
    if t.startswith("модель: "):
        return "model: " + t[len("модель: "):]
    return t


# ------------------------------------------------ тексты ошибок API (detail) — их показывает интерфейс

_ERRORS = {
    "loading": ("модель ещё загружается", "the model is still loading"),
    "no_token": ("нет токена", "no token"),
    "foreign_origin": ("чужой источник", "foreign origin"),
    "bad_tab": ("неизвестная вкладка", "unknown tab"),
    "no_chunk": ("фрагмент не найден", "chunk not found"),
    "not_image": ("это не картинка", "this is not an image"),
    "no_file": ("файл не найден", "file not found"),
    "no_folder": ("нет такой папки: {0}", "no such folder: {0}"),
    "no_drive": ("такого диска нет", "no such drive"),
    "bad_name": ("в списке есть недопустимое имя", "the list has an invalid name"),
    "bad_theme": ("неизвестная тема", "unknown theme"),
    "bad_lang": ("неизвестный язык", "unknown language"),
    "bad_color": ("цвет в формате #rrggbb", "a color like #rrggbb"),
    "bad_hotkey": ("сочетание вида ctrl+alt+s", "a shortcut like ctrl+alt+s"),
    "range": ("{0}: от {1} до {2}", "{0}: from {1} to {2}"),
    "not_failed": ("запуск не остановлен с ошибкой", "startup has not failed"),
}


def error(key, lang, *args):
    ru, en = _ERRORS[key]
    return (en if lang == "en" else ru).format(*args)
