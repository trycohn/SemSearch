"""Подписи для интерфейса: «крошки» папок и человеческие названия типов файлов (на языке интерфейса, см. i18n.py)."""
import os

import i18n

HOME = os.path.expanduser("~")
SEP = " › "


def ext_of(name):
    e = os.path.splitext(name)[1]
    return e[1:].upper() if e else ""


def type_name(ext, lang="ru"):
    return i18n.type_name(ext, lang)


def crumbs(path, lang="ru"):
    """Папка файла для показа: «Документы › Дом › Ремонт крыши», «C: › Program Files › …»."""
    d = os.path.dirname(path)
    nd, home = os.path.normcase(d), os.path.normcase(HOME)
    if nd == home:
        return i18n.USER_FOLDER.get(lang, i18n.USER_FOLDER["ru"])
    if nd.startswith(home + os.sep):
        parts = d[len(HOME) + 1:].split(os.sep)
        parts[0] = i18n.KNOWN_DIRS.get(lang, i18n.KNOWN_DIRS["ru"]).get(parts[0].lower(), parts[0])
        return SEP.join(parts)
    drive, rest = os.path.splitdrive(d)
    parts = [p for p in rest.split(os.sep) if p]
    return SEP.join([drive] + parts) if parts else drive + "\\"


def plural(n, forms):
    """plural(5, ("страница", "страницы", "страниц")) -> "страниц"."""
    return i18n.plural_ru(n, forms)


def ts(sec):
    sec = int(sec or 0)
    return f"{sec // 3600}:{sec % 3600 // 60:02d}:{sec % 60:02d}" if sec >= 3600 else f"{sec // 60}:{sec % 60:02d}"
