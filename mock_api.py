"""SemSearch — mock API для разработки UI (данные из design/*.dc.html).

Запуск:  .venv\\Scripts\\python.exe mock_api.py   →  http://127.0.0.1:8799/
Сценарии статуса: /dev/scenario/<first|video|idle|paused|fullscreen|process|battery|loading>
Тема: /dev/theme/<system|light|dark>   (токен для /dev не нужен)
Язык: /dev/lang/<system|ru|en> — при en и демо-файлы английские (картинки README.en.md)
"""
import math
import mimetypes
import os
import re
import time

import uvicorn
from fastapi import FastAPI, File, Request, UploadFile
from fastapi.responses import HTMLResponse, JSONResponse, Response

import i18n

import functools

print = functools.partial(print, flush=True)  # noqa: A001 — журнал сразу виден в фоне
mimetypes.add_type("text/javascript", ".js")
mimetypes.add_type("text/css", ".css")
mimetypes.add_type("image/svg+xml", ".svg")

ROOT = os.path.dirname(os.path.abspath(__file__))
UI = os.path.join(ROOT, "ui")
TOKEN = "dev-token"
CSP = "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'"

KB, MB, GB = 1024, 1024 * 1024, 1024 ** 3
H = "C:\\Users\\Anna"
ROOF = H + "\\Documents\\Дом\\Ремонт крыши"


def ts(y, mo, d, hh=12, mi=0):
    return time.mktime((y, mo, d, hh, mi, 0, 0, 0, -1))


def hit(chunk, loc="", text="", t=None):
    return {"chunk": chunk, "loc": loc, "t": t, "text": text}


INFO = {}  # path -> дополнительные поля для /api/preview


def item(kind, name, folder, crumbs, size, mtime, hits=(), thumb=None, duration=None, **info):
    path = folder + "\\" + name
    ext = os.path.splitext(name)[1][1:].upper()
    first = hits[0]["chunk"] if hits else abs(hash(path)) % 100000
    it = {"id": ("n:" + path) if kind == "name" else "f%d" % first, "kind": kind, "name": name, "ext": ext,
          "path": path, "crumbs": crumbs, "size": int(size), "mtime": mtime, "score": 0.5,
          "hits": list(hits), "thumb": thumb, "duration": duration}
    info.setdefault("ctime", mtime - 3600)
    INFO[path] = dict(info, kind=kind, item=it)
    return it


# ---------- «договор про крышу» (Main.dc.html) ----------
CODE = ["import re", "", "def parse_contract(path):", '    """Стороны, сумма и сроки из договора подряда."""', "    text = read_pdf(path)",
        "    parties = re.findall(PARTY_RE, text)", '    total = find_amount(text, "Цена договора")', "    return Contract(parties, total, find_terms(text))"]
d1 = item("text", "Договор подряда №14 — кровля.pdf", ROOF, "Документы › Дом › Ремонт крыши", 2.4 * MB, ts(2026, 3, 12, 18, 40),
          [hit(101, "стр. 3", "…Подрядчик обязуется выполнить работы по замене кровельного покрытия жилого дома, включая демонтаж старого шифера и монтаж металлочерепицы…"),
           hit(102, "стр. 7", "…Гарантийный срок на кровельные работы — 5 (пять) лет с даты подписания акта приёмки…")],
          type="PDF-документ, 18 страниц", pages=18, page_label="стр. 3 из 18", hit_pos=0.4)
d2 = item("text", "Смета_кровля_v3.docx", ROOF, "Документы › Дом › Ремонт крыши", 86 * KB, ts(2026, 3, 9, 11, 2),
          [hit(111, "фрагм. 2", "Металлочерепица 0,5 мм — 160 м² · водосточная система · монтаж кровли и снегозадержателей по договору…")],
          type="Документ Word", page_label="фрагмент 2 из 4", hit_pos=0.7, ctime=ts(2026, 2, 27, 19, 15))
d3 = item("text", "Переписка с прорабом.txt", H + "\\Desktop", "Рабочий стол", 4 * KB, ts(2026, 4, 2, 21, 15),
          [hit(121, "фрагм. 1", "…по договору гарантия на крышу пять лет, если потечёт — сразу писать Андрею, он приедет и посмотрит…")],
          type="Текстовый файл", page_label="фрагмент 1 из 1", hit_pos=0.1)
c1 = item("code", "contract_parser.py", H + "\\Projects\\docflow", "Projects › docflow", 12 * KB, ts(2026, 2, 28, 2, 37),
          [hit(131, "фрагм. 3", 'def parse_contract(path):\n    """Стороны, сумма и сроки из договора подряда."""')],
          type="Python", code={"lines": CODE, "hl": [2, 3]})
i1 = item("image", "IMG_4512.jpg", H + "\\Pictures\\Крыша 2026", "Изображения › Крыша 2026", 3.8 * MB, ts(2026, 3, 15, 14, 6),
          [hit(201)], "/thumb/201", type="JPEG, 4032 × 3024", taken=ts(2026, 3, 15, 14, 6), dims=[4032, 3024])
v1 = item("video", "Обход крыши.mp4", H + "\\Videos", "Видео", 412 * MB, ts(2026, 3, 15, 14, 20),
          [hit(202, "1:20", "кадр совпал с запросом", 80), hit(203, "2:40", "кадр совпал с запросом", 160), hit(204, "3:50", "кадр совпал с запросом", 230)],
          "/thumb/202", 252, type="Видео MP4, 1920 × 1080", marks=[0.317, 0.635, 0.913], dims=[1920, 1080])
i2 = item("image", "Скан договора, стр. 1.jpg", ROOF, "Документы › Дом › Ремонт крыши", 1.1 * MB, ts(2026, 3, 12, 18, 52),
          [hit(205)], "/thumb/205", type="JPEG, 2480 × 3508", dims=[2480, 3508])
i3 = item("image", "IMG_4513.jpg", H + "\\Pictures\\Крыша 2026", "Изображения › Крыша 2026", 4.0 * MB, ts(2026, 3, 15, 14, 7),
          [hit(206)], "/thumb/206", type="JPEG, 4032 × 3024", taken=ts(2026, 3, 15, 14, 7))
i4 = item("image", "IMG_4520.jpg", H + "\\Pictures\\Крыша 2026", "Изображения › Крыша 2026", 3.5 * MB, ts(2026, 3, 15, 18, 31),
          [hit(207)], "/thumb/207", type="JPEG, 4032 × 3024", taken=ts(2026, 3, 15, 18, 31))
a1 = item("audio", "Голосовая заметка 14 марта.m4a", H + "\\Documents\\Звукозаписи", "Документы › Звукозаписи", 2.1 * MB, ts(2026, 3, 14, 9, 12),
          [dict(hit(301, "0:30", "отрезок 0:30–1:00 из 1:52", 30), t_end=60)], None, 112, type="Аудио M4A", seg=[30, 60])
n1 = item("name", "Договор про крышу — сканы.zip", H + "\\Downloads", "Загрузки", 18.6 * MB, ts(2026, 3, 16, 10, 44), type="Архив ZIP", indexed=False)
n2 = item("name", "договор_про_крышу_старый.xlsx", H + "\\Documents\\Архив 2024", "Документы › Архив 2024", 31 * KB, ts(2024, 10, 3, 16, 20),
          type="Таблица Excel", indexed=False)
MAIN = [d1, d2, d3, c1, i1, v1, i2, i3, i4, a1, n1, n2]

# ---------- «кот на балконе» (Photos.dc.html) ----------
T = [["IMG_20250612_183044.jpg", ts(2025, 6, 12, 18, 30), "#e7c9a4", "#2b2a2e", 50, 92, 1.0, "#6f5a49", 58, 7],
     ["IMG_20250612_183051.jpg", ts(2025, 6, 12, 18, 30), "#e2c09a", "#2b2a2e", 38, 92, 0.9, "#6f5a49", 58, 7],
     ["Барсик утром.jpg", ts(2026, 5, 3, 8, 10), "#c9d8e6", "#c27a3a", 56, 90, 1.1, "#e8e4dc", 60, 9],
     ["IMG_4471.HEIC.jpg", ts(2026, 4, 21, 17, 2), "#b8c6d4", "#6b6f75", 46, 94, 0.8, "#3c4148", 64, 6],
     ["Скриншот 2025-05-03.png", ts(2025, 5, 3, 22, 41), "#d6dde4", "#c27a3a", 62, 96, 0.7, "#9aa3ac", 70, 11],
     ["IMG_20240830_201502.jpg", ts(2024, 8, 30, 20, 15), "#3d4a66", "#18181c", 44, 90, 1.0, "#58617a", 56, 8],
     ["Дача, балкон.jpg", ts(2026, 7, 14, 12, 0), "#cfe0d0", "#3a3530", 30, 93, 0.75, "#efefea", 62, 6],
     ["IMG_3920.jpg", ts(2026, 3, 9, 10, 20), "#dfe3e6", "#f1efe9", 52, 95, 0.9, "#5c646c", 62, 7],
     ["кот спит.jpg", ts(2026, 2, 28, 23, 5), "#e8d8c8", "#7d7f84", 60, 92, 1.2, "#8f7a68", 66, 5],
     ["IMG_20231105_093311.jpg", ts(2023, 11, 5, 9, 33), "#c4ccd4", "#2b2a2e", 70, 94, 0.6, "#4b535c", 58, 10],
     ["Балкон после ремонта.jpg", ts(2025, 10, 17, 16, 0), "#d3dce5", "", 0, 0, 0, "#5f6770", 54, 8],
     ["IMG_2210.jpg", ts(2025, 9, 1, 19, 45), "#efd2b0", "#c27a3a", 40, 91, 0.85, "#6f5a49", 60, 7]]
for k, t in enumerate(list(T)):  # ещё 12 похожих кадров → 24 фото, как во вкладке макета
    T.append(["IMG_2026%04d_%06d.jpg" % (610 + k, 90000 + k * 731), ts(2026, 6, 10 + k, 9, k * 4)] + t[2:4]
             + [100 - t[4] if t[4] else 0, t[5], t[6], t[7], t[8], t[9]])
ARTS = {}
PHOTOS = []
for k, t in enumerate(T):
    ch = 400 + k
    ARTS[ch] = ("cat", t[2:])
    folder = H + "\\Pictures\\Телефон\\2025\\06" if k < 2 else H + "\\Pictures\\Кот"
    PHOTOS.append(item("image", t[0], folder, "Изображения › Телефон › 2025 › 06" if k < 2 else "Изображения › Кот", (3.2 if k == 0 else 1.6 + (k % 7) * 0.45) * MB,
                       t[1], [hit(ch)], "/thumb/%d" % ch, type="JPEG, 4000 × 3000", taken=t[1], dims=[4000, 3000]))
VIDS = []
for k, (name, loc, dur) in enumerate([("Дача июнь.mp4", 220, 412), ("VID_0614.mp4", 20, 95), ("Кот и голуби.mp4", 65, 140),
                                      ("VID_0702.mp4", 48, 61), ("Балкон, утро.mp4", 130, 300), ("VID_20240830.mp4", 12, 44)]):
    ch = 450 + k
    ARTS[ch] = ("cat", T[(k * 5) % 12][2:])
    VIDS.append(item("video", name, H + "\\Videos\\Кот", "Видео › Кот", (60 + k * 37) * MB, ts(2026, 6, 14 + k, 18, 0),
                     [hit(ch, "%d:%02d" % (loc // 60, loc % 60), "кадр совпал с запросом", loc)], "/thumb/%d" % ch, dur,
                     type="Видео MP4, 1920 × 1080", marks=[loc / dur], dims=[1920, 1080]))
CAT_DOCS = [item("text", "Заметки про Барсика.txt", H + "\\Documents", "Документы", 3 * KB, ts(2026, 5, 3, 9, 0),
                 [hit(480, "фрагм. 1", "…кот опять сидит на балконе и смотрит на голубей, сетку на балкон поставить до лета…")],
                 type="Текстовый файл", page_label="фрагмент 1 из 1", hit_pos=0.2),
            item("text", "Ветпаспорт Барсика.pdf", H + "\\Documents\\Кот", "Документы › Кот", 640 * KB, ts(2025, 11, 20, 12, 0),
                 [hit(481, "стр. 2", "…кот, 4 года, окрас рыжий; гуляет на застеклённом балконе…")],
                 type="PDF-документ, 4 страницы", page_label="стр. 2 из 4", hit_pos=0.25)]

# ---------- «смета» (Names.dc.html) ----------
R = [["Смета_кровля_v3.docx", "Документы › Дом › Ремонт крыши", 86 * KB, ts(2026, 3, 9, 11, 2)],
     ["Смета_кровля_v2.docx", "Документы › Дом › Ремонт крыши", 84 * KB, ts(2026, 3, 4, 15, 30)],
     ["Смета_кровля.xlsx", "Документы › Дом › Ремонт крыши", 31 * KB, ts(2026, 3, 2, 10, 0)],
     ["Смета_кровля_v3.docx", "Рабочий стол › Отправить прорабу", 86 * KB, ts(2026, 3, 9, 11, 5)],
     ["Смета на окна ПВХ.pdf", "Загрузки", 412 * KB, ts(2026, 1, 18, 13, 0)],
     ["Смета электрика.xlsx", "Документы › Дом › Кухня", 22 * KB, ts(2025, 11, 11, 9, 0)],
     ["смета-кухня-2025.pdf", "Документы › Дом › Кухня", 1.3 * MB, ts(2025, 11, 2, 20, 0)],
     ["смета_материалы.txt", "Рабочий стол", 2 * KB, ts(2025, 10, 28, 8, 0)],
     ["Смета_ванная_финал.xlsx", "Документы › Дом › Ванная", 27 * KB, ts(2025, 8, 14, 18, 0)],
     ["СМЕТА ремонт офиса (копия).docx", "OneDrive › Работа", 64 * KB, ts(2025, 5, 30, 12, 0)],
     ["Автокопия Смета_кровля_v3.asd", "AppData › Roaming › Microsoft › Word", 91 * KB, ts(2026, 3, 9, 11, 3)],
     ["Шаблон сметы.dotx", "Документы › Шаблоны Office", 40 * KB, ts(2024, 1, 12, 10, 0)]]
DIRS = {"Документы": H + "\\Documents", "Рабочий стол": H + "\\Desktop", "Загрузки": H + "\\Downloads", "Изображения": H + "\\Pictures",
        "Видео": H + "\\Videos", "OneDrive": H + "\\OneDrive", "AppData": H + "\\AppData"}
TYPES = {"DOCX": "Документ Word", "PDF": "PDF-документ", "TXT": "Текстовый файл", "XLSX": "Таблица Excel", "ASD": "Автокопия Word",
         "DOTX": "Шаблон Word", "ZIP": "Архив ZIP", "JPG": "JPEG", "PNG": "PNG", "MP4": "Видео MP4", "M4A": "Аудио M4A", "PY": "Python"}


def crumbs_dir(cr):
    parts = cr.split(" › ")
    return "\\".join([DIRS.get(parts[0], H + "\\" + parts[0])] + parts[1:])


NAMES = []
for name, cr, size, mt in R:
    folder = crumbs_dir(cr)
    path = folder + "\\" + name
    ext = os.path.splitext(name)[1][1:].upper()
    if path in INFO:
        it = dict(INFO[path]["item"], kind="name", id="n:" + path, hits=[])
    else:
        idx = ext in ("DOCX", "PDF", "TXT")
        it = item("text" if idx else "name", name, folder, cr, size, mt, type=TYPES.get(ext, "Файл " + ext), indexed=idx)
        it = dict(it, kind="name", id="n:" + path, hits=[])
    NAMES.append(it)
INFO[ROOF + "\\Смета_кровля_v3.docx"]["dupes"] = [
    {"path": H + "\\Desktop\\Отправить прорабу\\Смета_кровля_v3.docx", "crumbs": "Рабочий стол › Отправить прорабу"},
    {"path": H + "\\AppData\\Roaming\\Microsoft\\Word\\Смета_кровля_v3.docx", "crumbs": "AppData › Roaming › Microsoft › Word"}]
SMETA_ALL = [dict(INFO[NAMES[i]["path"]]["item"], hits=[hit(500 + i, "фрагм. 1", "…итого по смете: материалы, работа, доставка; смета согласована с прорабом…")])
             for i in (0, 1, 4, 6, 7, 9)]
for it in SMETA_ALL:
    it["kind"] = "text"
    it["id"] = "f%d" % it["hits"][0]["chunk"]
    INFO[it["path"]].update(kind="text", indexed=True)
    INFO[it["path"]].setdefault("page_label", "фрагмент 1 из 3")
    INFO[it["path"]].setdefault("hit_pos", 0.3)
SMETA_ALL += [item("image", "Скан сметы.jpg", ROOF, "Документы › Дом › Ремонт крыши", 900 * KB, ts(2026, 3, 9, 11, 30), [hit(205)], "/thumb/205", type="JPEG, 2480 × 3508"),
              NAMES[10], NAMES[5]]

# ---------- прочее для подсказок со стартового экрана ----------
EXTRA = [item("code", "http_client.py", H + "\\Projects\\scraper", "Projects › scraper", 7 * KB, ts(2026, 8, 21, 23, 40),
              [hit(601, "фрагм. 2", "for attempt in range(retries):\n    try:\n        return json.loads(resp.text)")],
              type="Python", code={"lines": ["def fetch_json(url, retries=5):", "    for attempt in range(retries):", "        try:", "            resp = session.get(url, timeout=10)",
                                             "            return json.loads(resp.text)", "        except (ValueError, Timeout):", "            time.sleep(2 ** attempt)", "    raise FetchError(url)"], "hl": [1, 4]}),
         item("image", "Скриншот 2026-09-30 CUDA error.png", H + "\\Pictures\\Screenshots", "Изображения › Screenshots", 240 * KB, ts(2026, 9, 30, 1, 12),
              [hit(602)], "/thumb/602", type="PNG, 1920 × 1080")]
ARTS.update({201: ("roof1",), 202: ("drone",), 203: ("drone",), 204: ("drone",), 205: ("scan",), 206: ("roof2",), 207: ("roof3",), 602: ("cuda",)})
ALL_ITEMS = MAIN + PHOTOS + VIDS + CAT_DOCS + EXTRA


# ---------- те же файлы по-английски: при языке en (/dev/lang/en) ответы API переводятся целиком ----------
# Подписи, которые собирает backend («стр. 3», типы, известные папки), — через i18n.py, как в app.py;
# остальное — имена, папки и тексты демо-файлов — по словарю. Путь из ответа превью и открытия возвращаются к русскому.
EN = {
    # папки
    "Дом": "Home", "Ремонт крыши": "Roof repair", "Крыша 2026": "Roof 2026", "Звукозаписи": "Recordings", "Архив 2024": "Archive 2024",
    "Телефон": "Phone", "Кот": "Cat", "Отправить прорабу": "Send to the foreman", "Кухня": "Kitchen", "Ванная": "Bathroom",
    "Работа": "Work", "Шаблоны Office": "Office Templates", "Архив": "Archive",
    # файлы
    "Договор подряда №14 — кровля.pdf": "Roof repair contract No. 14.pdf", "Смета_кровля_v3.docx": "Estimate_roofing_v3.docx",
    "Переписка с прорабом.txt": "Messages with the foreman.txt", "Обход крыши.mp4": "Roof walkaround.mp4",
    "Скан договора, стр. 1.jpg": "Contract scan, page 1.jpg", "Голосовая заметка 14 марта.m4a": "Voice memo, March 14.m4a",
    "Договор про крышу — сканы.zip": "Roof contract - scans.zip", "договор_про_крышу_старый.xlsx": "roof_contract_old.xlsx",
    "Барсик утром.jpg": "Barsik in the morning.jpg", "Скриншот 2025-05-03.png": "Screenshot 2025-05-03.png",
    "Дача, балкон.jpg": "Cottage, balcony.jpg", "кот спит.jpg": "cat asleep.jpg", "Балкон после ремонта.jpg": "Balcony after the repair.jpg",
    "Дача июнь.mp4": "Cottage, June.mp4", "Кот и голуби.mp4": "Cat and pigeons.mp4", "Балкон, утро.mp4": "Balcony, morning.mp4",
    "Заметки про Барсика.txt": "Notes about Barsik.txt", "Ветпаспорт Барсика.pdf": "Barsik vet passport.pdf",
    "Смета_кровля_v2.docx": "Estimate_roofing_v2.docx", "Смета_кровля.xlsx": "Estimate_roofing.xlsx",
    "Смета на окна ПВХ.pdf": "Estimate for PVC windows.pdf", "Смета электрика.xlsx": "Estimate electrics.xlsx",
    "смета-кухня-2025.pdf": "estimate-kitchen-2025.pdf", "смета_материалы.txt": "estimate_materials.txt",
    "Смета_ванная_финал.xlsx": "Estimate_bathroom_final.xlsx", "СМЕТА ремонт офиса (копия).docx": "ESTIMATE office repair (copy).docx",
    "Автокопия Смета_кровля_v3.asd": "AutoRecovery Estimate_roofing_v3.asd", "Шаблон сметы.dotx": "Estimate template.dotx",
    "Скан сметы.jpg": "Estimate scan.jpg", "Скриншот 2026-09-30 CUDA error.png": "Screenshot 2026-09-30 CUDA error.png",
    # тексты
    "…Подрядчик обязуется выполнить работы по замене кровельного покрытия жилого дома, включая демонтаж старого шифера и монтаж металлочерепицы…":
        "…The contractor shall replace the roof covering of the house, including removing the old slate and fitting metal roof tiles…",
    "…Гарантийный срок на кровельные работы — 5 (пять) лет с даты подписания акта приёмки…":
        "…The warranty on the roofing work is 5 (five) years from the date the acceptance certificate is signed…",
    "Металлочерепица 0,5 мм — 160 м² · водосточная система · монтаж кровли и снегозадержателей по договору…":
        "Metal roof tiles 0.5 mm — 160 m² · gutter system · roof and snow guard installation under the contract…",
    "…по договору гарантия на крышу пять лет, если потечёт — сразу писать Андрею, он приедет и посмотрит…":
        "…the contract gives the roof a five-year warranty; if it leaks, text Andrew right away and he will come and look…",
    'def parse_contract(path):\n    """Стороны, сумма и сроки из договора подряда."""':
        'def parse_contract(path):\n    """Parties, price and deadlines of a repair contract."""',
    '    """Стороны, сумма и сроки из договора подряда."""': '    """Parties, price and deadlines of a repair contract."""',
    '    total = find_amount(text, "Цена договора")': '    total = find_amount(text, "Contract price")',
    "…кот опять сидит на балконе и смотрит на голубей, сетку на балкон поставить до лета…":
        "…the cat is sitting on the balcony again, watching the pigeons; put a net on the balcony before summer…",
    "…кот, 4 года, окрас рыжий; гуляет на застеклённом балконе…": "…cat, 4 years old, ginger; walks on the glazed balcony…",
    "…итого по смете: материалы, работа, доставка; смета согласована с прорабом…":
        "…estimate total: materials, labour, delivery; the estimate is agreed with the foreman…",
    # /api/failed
    "PDF зашифрован паролем": "the PDF is password-protected", "файл занят другой программой": "the file is in use by another program",
    "повреждённый архив DOCX": "damaged DOCX archive", "не удалось декодировать видео": "could not decode the video",
    "слишком большой текст": "the text is too large",
}
EN_DIRS = {ru: i18n.KNOWN_DIRS["en"][k] for k, ru in i18n.KNOWN_DIRS["ru"].items()}
EN_TYPES = {ru: i18n.type_name(ext.lower(), "en") for ext, ru in TYPES.items()}
EN_QUERY = {"roof": "крыш", "contract": "договор", "cat": "кот", "balcony": "балкон", "estimate": "смет",
            "json": "json", "retr": "ретра", "cuda": "cuda", "screenshot": "скрин"}
BACK = {}  # английский путь -> русский: превью и открытие ищут файл по пути из выдачи


def lang():
    return SETTINGS["lang"] if SETTINGS["lang"] in i18n.LANGS else SETTINGS["lang_system"]


def en_str(s):
    if s in EN:
        return EN[s]
    if "\\" in s:  # путь или id вида "n:C:\…"
        out = "\\".join(en_str(p) if p else p for p in s.split("\\"))
        BACK[out] = s
        return out
    if " › " in s:
        return " › ".join(en_str(p) for p in s.split(" › "))
    if s in EN_DIRS:
        return EN_DIRS[s]
    m = re.fullmatch(r"(стр\.|фрагмент) (\d+) из (\d+)", s)
    if m:
        return (i18n.page_label if m.group(1) == "стр." else i18n.chunk_label)(m.group(2), m.group(3), "en")
    m = re.fullmatch(r"отрезок (\S+)–(\S+) из (\S+)", s)
    if m:
        return i18n.audio_hit(*m.groups(), "en")
    if s == i18n.video_hit("ru"):
        return i18n.video_hit("en")
    if s.startswith("файл_"):  # список «Не удалось прочитать»
        return "file_" + s[5:]
    head, _, tail = s.partition(", ")  # "PDF-документ, 18 страниц", "Видео MP4, 1920 × 1080"
    if head in EN_TYPES:
        m = re.fullmatch(r"(\d+) страниц[аы]?", tail)
        return EN_TYPES[head] + (", " + (i18n.pages(int(m.group(1)), "en") if m else tail) if tail else "")
    return i18n.loc(s, "en")


def en(obj):
    if isinstance(obj, str):
        return en_str(obj)
    if isinstance(obj, list):
        return [en(x) for x in obj]
    if isinstance(obj, dict):
        return {k: en(v) for k, v in obj.items()}
    return obj


def localized(obj):
    return en(obj) if lang() == "en" else obj


def ru_query(q):
    """Английский запрос -> русские ключевые слова, по которым подобраны демо-ответы ("cat on the balcony" -> "кот балкон")."""
    if lang() != "en":
        return q
    ql = q.lower()
    return " ".join(v for k, v in EN_QUERY.items() if k in ql) or q


# ---------- поиск ----------
def resp(q, tab, items, counts, ms):
    return {"q": q, "tab": tab, "ms": ms, "counts": counts, "name_scope": "C:", "items": items}


def by_kind(items, tab):
    if tab == "all":
        order = {"text": 0, "code": 0, "image": 1, "video": 1, "audio": 2, "name": 3}
        return sorted(items, key=lambda i: order[i["kind"]])
    return [i for i in items if i["kind"] == tab]


def count(items, names):
    c = {k: len([i for i in items if i["kind"] == k]) for k in ("text", "code", "image", "video", "audio")}
    c["all"] = len(items)
    c["name"] = len(names)
    return c


def words(q):
    return [w[:5] for w in re.split(r"[\s_—\-]+", q.lower()) if len(w) > 1]


def search(q, tab):
    ql = ru_query(q).lower()
    if "крыш" in ql or "договор" in ql:
        items, names, ms = MAIN, [n1, n2], 42
    elif "кот" in ql or "балкон" in ql:
        items, names, ms = CAT_DOCS + PHOTOS + VIDS, [], 35
    elif "смет" in ql:
        items, names, ms = SMETA_ALL, NAMES, 31
    else:
        ws = words(ql)
        hay = lambda i: (i["name"] + " " + " ".join(h["text"] for h in i["hits"])).lower()
        items = [i for i in ALL_ITEMS if ws and any(w in hay(i) for w in ws)]
        names = [i for i in ALL_ITEMS + NAMES if ws and all(w in i["name"].lower() for w in ws)]
        names = [dict(i, kind="name", id="n:" + i["path"], hits=[]) for i in names]
        ms = 38
    if tab == "name":
        return resp(q, tab, names, count(items, names), 4)
    return resp(q, tab, by_kind(items, tab), count(items, names), ms if tab == "all" else 35 if tab == "image" else 31)


def media_resp(tab):
    items = PHOTOS + VIDS
    c = {"all": len(items), "image": len(PHOTOS), "video": len(VIDS)}
    return resp("", tab, by_kind(items, tab) if tab != "all" else items, c, 28)


# ---------- миниатюры (плоские многоугольники как в макете) ----------
def pts(css):
    out = []
    for pair in css[css.index("(") + 1:-1].split(","):
        x, y = pair.split()
        out.append((float(x.rstrip("%")), float(y.rstrip("%"))))
    return out


MAIN_ART = {
    "roof1": ("#c7d3de", [("#56606b", "polygon(0 100%, 0 72%, 50% 32%, 100% 72%, 100% 100%)"), ("#dccdb9", "polygon(14% 100%, 14% 84%, 86% 84%, 86% 100%)")]),
    "roof2": ("#d5dbe1", [("#7b4e3f", "polygon(0 100%, 0 46%, 100% 16%, 100% 100%)"), ("#5e3a2f", "polygon(0 64%, 100% 34%, 100% 38%, 0 68%)")]),
    "roof3": ("#e6d8c4", [("#3e4650", "polygon(0 100%, 0 78%, 38% 40%, 76% 78%, 100% 60%, 100% 100%)"), ("#eea95c", "circle(9% at 80% 24%)")]),
    "drone": ("#a8bccf", [("#55604d", "polygon(0 100%, 0 64%, 30% 56%, 62% 62%, 100% 50%, 100% 100%)"), ("#5d6670", "polygon(34% 100%, 34% 78%, 52% 58%, 70% 78%, 70% 100%)")]),
    "scan": ("#d6d3cc", [("#fbfaf6", "polygon(18% 8%, 82% 8%, 82% 100%, 18% 100%)"), ("#bdb9b1", "polygon(26% 20%, 64% 20%, 64% 24%, 26% 24%)")]),
    "cuda": ("#1e1e1e", [("#2d2d30", "polygon(6% 10%, 94% 10%, 94% 90%, 6% 90%)"), ("#c42b1c", "polygon(10% 40%, 90% 40%, 90% 52%, 10% 52%)")]),
}
CAT = [(-14, 0), (-13, -22), (-10, -34), (-7, -40), (-8, -48), (-7.5, -56), (-6, -64), (-3, -58), (3, -58), (6, -64), (7.5, -56), (8, -48), (7, -40), (10, -34), (13, -22), (14, 0)]


def rail(top, n):
    p = [(0, top), (100, top), (100, top + 4)]
    step = 100 / n
    for i in range(n - 1, -1, -1):
        x1, x0 = i * step + step * 0.7, i * step + step * 0.3
        p += [(x1, top + 4), (x1, 100), (x0, 100), (x0, top + 4)]
    return p + [(0, top + 4)]


def svg_art(chunk):
    spec = ARTS.get(chunk, ("roof1",))
    if spec[0] == "cat":
        c1, c2, cx, base, s, c3, top, n = spec[1]
        layers = ([(c2, [(cx + x * s, base + y * s) for x, y in CAT])] if c2 else []) + [(c3, rail(top, n))]
    else:
        c1, raw = MAIN_ART[spec[0]]
        layers = []
        for col, css in raw:
            if css.startswith("circle"):
                r, cx, cy = map(float, re.findall(r"[\d.]+", css))
                layers.append((col, ("circle", cx * 4, cy * 3, r / 100 * math.sqrt((400 ** 2 + 300 ** 2) / 2))))
            else:
                layers.append((col, pts(css)))
    body = ['<rect width="400" height="300" fill="%s"/>' % c1]
    for col, shape in layers:
        if shape and shape[0] == "circle":
            body.append('<circle cx="%.1f" cy="%.1f" r="%.1f" fill="%s"/>' % (shape[1], shape[2], shape[3], col))
        else:
            body.append('<polygon fill="%s" points="%s"/>' % (col, " ".join("%.1f,%.1f" % (x * 4, y * 3) for x, y in shape)))
    return ('<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300" preserveAspectRatio="none">'
            + "".join(body) + "</svg>")


# ---------- статус ----------
NAMES_ST = {"count": 2104377, "drives": ["C:"], "engine": "mft", "ready": True, "updated": None, "progress": None}


def kinds(img=(18204, 18204), aud=(1377, 1377), vid=(612, 612)):
    return {"text": {"done": 3412, "total": 3412, "failed": 12}, "code": {"done": 41870, "total": 41870, "failed": 0},
            "image": {"done": img[0], "total": img[1], "failed": 0}, "audio": {"done": aud[0], "total": aud[1], "failed": 0},
            "video": {"done": vid[0], "total": vid[1], "failed": 0}}


def scenario(name):
    base = {"ready": True, "error": None, "device": "cuda", "state": "idle", "phase": None, "paused": None, "kinds": kinds(),
            "current": None, "names": dict(NAMES_ST), "eta_sec": None, "watching": True, "throttle": None, "index_bytes": int(1.3 * GB)}
    if name == "first":
        base.update(state="indexing", phase="image", kinds=kinds((11280, 18204), (0, 1377), (0, 612)),
                    current={"kind": "image", "done": 11280, "total": 18204}, eta_sec=2100)
    elif name == "video":
        base.update(state="indexing", phase="video", kinds=kinds(vid=(482, 612)), current={"kind": "video", "done": 482, "total": 612}, eta_sec=600)
    elif name in ("paused", "fullscreen", "process", "battery"):
        reason = "manual" if name == "paused" else name
        base.update(state="paused", kinds=kinds(vid=(482, 612)), paused={"reason": reason, "detail": "cs2.exe" if name == "process" else None})
    elif name == "loading":
        base.update(ready=False, device=None, state="loading", names=dict(NAMES_ST, ready=False, count=1204331, progress=0.57))
    base["boot"] = {"step": None, "first_run": False, "download": None, "gpu_note": None, "chunks": 48210, "error_kind": None}
    if name in BOOT_PLANS:
        boot_state(name, base)
    return base


# Экран запуска: шаги по времени с момента /dev/scenario/<boot|firstrun|cpu|booterror|bootnet> (или /api/boot/retry)
BOOT_PLANS = {
    "boot": [("index", 0.6), ("model", 5.0), ("vectors", 1.2), ("watch", 0.6)],
    "cpu": [("index", 0.6), ("model", 4.0), ("vectors", 1.2), ("watch", 0.6)],
    "firstrun": [("index", 0.6), ("download", 9.0), ("model", 4.0), ("vectors", 0.8), ("watch", 0.6)],
    "booterror": [("index", 0.6), ("model", 3.0), ("fail", 0)],
    "bootnet": [("index", 0.6), ("download", 3.0), ("fail", 0)],
    # остановиться на шаге — для скриншотов
    "hold-download": [("index", 0.6), ("download", 900.0)],
    "hold-model": [("index", 0.6), ("model", 900.0)],
}


def boot_state(name, base):
    t = time.time() - STATE.get("t0", time.time())
    b = base["boot"]
    b.update(first_run=name in ("firstrun", "bootnet", "hold-download"), chunks=None)
    last = "index"
    for step, dur in BOOT_PLANS[name]:
        if step == "fail":
            b["step"] = last
            b["error_kind"] = "download" if name == "bootnet" else "other"
            base.update(ready=False, device=None, state="error",
                        error="OSError: We couldn't connect to 'https://huggingface.co'" if name == "bootnet" else "RuntimeError: CUDA error: out of memory")
            return
        if t < dur:
            b["step"] = step
            if step == "download":
                total = int(1.46 * GB)
                b["download"] = {"done": int(total * min(1.0, t / dur)), "total": total}
            if step in ("vectors", "watch"):
                b["gpu_note"] = "busy" if name == "cpu" else None
            if step == "watch":
                b["chunks"] = 48210
            base.update(ready=False, device=("cpu" if name == "cpu" else "cuda") if step in ("vectors", "watch") else None,
                        state="loading", names=dict(NAMES_ST, ready=False, count=1204331, progress=0.57))
            return
        t -= dur
        last = step
    b.update(step=None, gpu_note="busy" if name == "cpu" else None, chunks=48210)
    base.update(ready=True, device="cpu" if name == "cpu" else "cuda")


STATE = {"scenario": "video", "before_pause": "video"}
SETTINGS = {
    "content_roots": [{"path": H, "files": 65475}],
    "drives": {"available": [{"drive": "C:", "label": "Windows", "fs": "NTFS"}, {"drive": "D:", "label": "Data", "fs": "NTFS"}], "selected": ["C:"]},
    "exclude_dirs": ["node_modules", "venv", "__pycache__", "AppData", "build", "target", "Temp", "Cache", "env", "site-packages", "dist", "obj",
                     "bin", "Library", "packages", "vendor", "$Recycle.Bin", "System Volume Information", "WinSxS", "Logs", "cache"],
    "hotkey": "ctrl+alt+s", "theme": "system", "accent": "#005fb8", "lang": "system", "lang_system": "ru",
    "pause_on_fullscreen": True, "pause_processes": ["cs2.exe"], "pause_on_battery": False,
    "video_frame_every_sec": 10, "video_max_frames": 40, "audio_segment_sec": 30, "audio_max_segments": 10, "max_text_mb": 2, "max_pdf_pages": 80,
    "info": {"model": "EmbeddingGemma 2", "device": "cuda", "index_bytes": int(1.3 * GB), "rescan_minutes": 360, "name_rescan_minutes": 1440, "failed": 214},
}

# ---------- приложение ----------
app = FastAPI()


@app.middleware("http")
async def guard(request: Request, call_next):
    p = request.url.path
    if p.startswith("/api/") and not request.headers.get("x-semsearch-token"):
        return JSONResponse({"detail": "нет токена"}, 403)
    if p.startswith("/thumb/") and not request.query_params.get("t"):
        return JSONResponse({"detail": "нет токена"}, 403)
    return await call_next(request)


def page(name):
    with open(os.path.join(UI, name), encoding="utf-8") as f:
        html = f.read().replace("__SEMSEARCH_TOKEN__", TOKEN)
    return HTMLResponse(html, headers={"Content-Security-Policy": CSP, "Cache-Control": "no-store"})


@app.get("/")
def p_main():
    return page("index.html")


@app.get("/settings")
def p_settings():
    return page("settings.html")


@app.get("/tray")
def p_tray():
    return page("tray.html")


@app.get("/ui/{name:path}")
def static(name: str):
    full = os.path.normpath(os.path.join(UI, name))
    if not full.startswith(UI + os.sep) or not os.path.isfile(full):
        return JSONResponse({"detail": "нет файла"}, 404)
    with open(full, "rb") as f:
        data = f.read()
    return Response(data, media_type=mimetypes.guess_type(full)[0] or "application/octet-stream", headers={"Cache-Control": "no-store"})


@app.get("/thumb/{chunk}")
def thumb(chunk: int):
    return Response(svg_art(chunk), media_type="image/svg+xml", headers={"Cache-Control": "max-age=3600"})


@app.get("/api/search")
def api_search(q: str = "", tab: str = "all"):
    if STATE.get("delay"):
        time.sleep(STATE["delay"])
    if (STATE["scenario"] == "loading" or not scenario(STATE["scenario"])["ready"]) and tab != "name":
        return JSONResponse({"detail": "модель ещё загружается"}, 503)
    return localized(search(q, tab))


@app.get("/api/preview")
def api_preview(path: str, chunks: str = "", chunk: int = None):
    path = BACK.get(path, path)
    inf = INFO.get(path)
    if not inf:
        return JSONResponse({"detail": "неизвестный путь"}, 404)
    it = inf["item"]
    kind = inf.get("kind", it["kind"])
    want = [int(c) for c in chunks.split(",") if c.strip().isdigit()] or ([chunk] if chunk is not None else [])
    hits = [h for h in it["hits"] if h["text"] and (not want or h["chunk"] in want)]
    w = [round(abs(math.sin(i * 1.7) * 0.6 + math.sin(i * 0.45) * 0.4), 3) for i in range(44)] if kind == "audio" else None
    th = it["thumb"]
    return localized({"path": path, "name": it["name"], "ext": it["ext"], "kind": kind, "crumbs": it["crumbs"], "size": it["size"],
            "mtime": it["mtime"], "ctime": inf.get("ctime"), "type": inf.get("type", ""), "taken": inf.get("taken"), "duration": it["duration"],
            "pages": inf.get("pages"), "dims": inf.get("dims"), "page_label": inf.get("page_label") if want else None,
            "hit_pos": inf.get("hit_pos") if want else None, "hits": hits, "code": inf.get("code"), "marks": inf.get("marks"),
            "wave": w, "seg": inf.get("seg"), "dupes": inf.get("dupes", []), "thumb": (th + "?size=640") if th else None,
            "can_similar": kind in ("image", "video"), "indexed": inf.get("indexed", kind != "name")})


@app.get("/api/similar")
def api_similar(chunk: int, tab: str = "image"):
    return localized(media_resp(tab))


@app.post("/api/search_image")
async def api_search_image(tab: str = "image", file: UploadFile = File(...)):
    data = await file.read()
    print("search_image:", file.filename, file.content_type, len(data), "байт")
    return localized(media_resp(tab))


@app.post("/api/open")
async def api_open(request: Request):
    body = await request.json()
    print("open:", body)
    path = BACK.get(body.get("path"), body.get("path"))
    if path not in INFO and not any(n["path"] == path for n in NAMES):
        return JSONResponse({"detail": "файл не найден"}, 404)
    return {"ok": True}


@app.get("/api/status")
def api_status():
    s = scenario(STATE["scenario"])
    s["names"]["updated"] = time.time() - 20
    return s


@app.post("/api/pause")
async def api_pause(request: Request):
    paused = bool((await request.json()).get("paused"))
    if paused and STATE["scenario"] != "paused":
        STATE["before_pause"], STATE["scenario"] = STATE["scenario"], "paused"
    elif not paused and STATE["scenario"] == "paused":
        STATE["scenario"] = STATE["before_pause"] if STATE["before_pause"] != "paused" else "video"
    return {"ok": True}


@app.post("/api/boot/retry")
async def api_boot_retry():
    STATE["t0"] = time.time()
    if STATE["scenario"] in ("booterror", "bootnet"):
        STATE["scenario"] = "boot" if STATE["scenario"] == "booterror" else "firstrun"
    return {"ok": True}


@app.post("/api/rescan")
async def api_rescan():
    print("action: rescan")
    return {"ok": True, "new_files": 12}


@app.post("/api/rebuild")
@app.post("/api/open_log")
@app.post("/api/shutdown")
async def api_ok(request: Request):
    print("action:", request.url.path)
    return {"ok": True}


@app.get("/api/settings")
def api_settings():
    return SETTINGS


@app.post("/api/settings")
async def api_settings_set(request: Request):
    body = await request.json()
    print("settings:", body)
    restart = False
    for k, v in body.items():
        if k == "content_roots":
            old = {r["path"]: r["files"] for r in SETTINGS["content_roots"]}
            SETTINGS["content_roots"] = [{"path": p, "files": old.get(p, 1234)} for p in v]
        elif k == "drives":
            SETTINGS["drives"]["selected"] = list(v)
            restart = True
        elif k in SETTINGS and k != "info":
            SETTINGS[k] = v
    return {"ok": True, "restart": restart}


@app.get("/api/failed")
def api_failed(limit: int = 200):
    errs = ["PDF зашифрован паролем", "файл занят другой программой", "повреждённый архив DOCX", "не удалось декодировать видео", "слишком большой текст"]
    items = [{"path": H + "\\Documents\\Архив\\файл_%03d.%s" % (i, ["pdf", "docx", "mp4", "txt"][i % 4]), "err": i18n.reason(errs[i % 5], lang())}
             for i in range(min(limit, 214))]
    return localized({"total": 214, "items": items})


@app.get("/api/processes")
def api_processes():
    return {"items": ["cs2.exe", "chrome.exe", "Code.exe", "Discord.exe", "explorer.exe", "obs64.exe", "steam.exe", "Telegram.exe", "vlc.exe"]}


@app.get("/dev/shell/{name}")
def dev_shell(name: str):
    """Встроенные страницы оболочки (shell/dist) — посмотреть заставку в браузере."""
    base = os.path.join(ROOT, "shell", "dist")
    full = os.path.normpath(os.path.join(base, name))
    if not full.startswith(base + os.sep) or not os.path.isfile(full):
        return JSONResponse({"detail": "нет файла"}, 404)
    with open(full, "rb") as f:
        return Response(f.read(), media_type=mimetypes.guess_type(full)[0] or "text/plain")


@app.get("/dev/scenario/{name}")
def dev_scenario(name: str):
    STATE["scenario"] = name
    STATE["t0"] = time.time()
    return {"scenario": name}


@app.get("/dev/lang/{lang}")
def dev_lang(lang: str, system: str = None):
    """Язык: /dev/lang/<system|ru|en>?system=<ru|en> — как будто сменили в настройках (окна подхватят при опросе)."""
    SETTINGS["lang"] = lang
    if system:
        SETTINGS["lang_system"] = system
    return {"lang": lang, "lang_system": SETTINGS["lang_system"]}


@app.get("/dev/delay/{ms}")
def dev_delay(ms: int):
    STATE["delay"] = ms / 1000
    return {"delay_ms": ms}


@app.get("/dev/theme/{theme}")
def dev_theme(theme: str, accent: str = None):
    SETTINGS["theme"] = theme
    if accent:
        SETTINGS["accent"] = "#" + accent.lstrip("#")
    return {"theme": theme, "accent": SETTINGS["accent"]}


if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=8799, log_level="warning")
