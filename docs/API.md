# SemSearch — контракты между частями

Источник правды для всех частей. Меняет только ведущий (lead); остальные — по согласованию.
Макет: `design/*.dc.html` (исходники холста «SemSearch Desktop», 1280 px шириной, тёмная/светлая тема, акцент #005fb8).

## 1. Процессы

| Часть | Где | Что делает |
|---|---|---|
| **Backend** (Python, FastAPI) | `server.py` → `app.py`, порт `SEMSEARCH_PORT` (выбирает shell) или `config.json:port` (8765), только 127.0.0.1 | ML-индексация, поиск по смыслу, JSON API, отдаёт страницы UI. Запускает движок. |
| **Engine** (Rust) | `bin/semsearch-engine.exe`, исходники `engine/` | Индекс имён всех файлов: MFT + USN-журнал (NTFS, с правами администратора) или обход + ReadDirectoryChangesW (без них и на других ФС). Named pipe `\\.\pipe\semsearch-engine-<SessionId>`. С ним говорит только backend. |
| **Shell** (Rust, Tauri v2) | `SemSearch.exe` в корне, исходники `shell/` | Окна без рамки (main / settings / tray-popup) с страницами backend, значок в трее, глобальная клавиша, автозапуск, запуск/остановка backend. |
| **UI** | `ui/*.html, *.js, *.css` | Страницы 1:1 по макету. Работают и в Tauri, и в обычном браузере. |

### Папка данных `D`

`D` = `SEMSEARCH_DATA_DIR` (только абсолютный путь), иначе `%LOCALAPPDATA%\SemSearch` (Known Folder LocalAppData).
Все части находят её одинаково; shell передаёт свою `D` backend'у, backend — движку (через окружение). Движок из задачи
Планировщика видит `SEMSEARCH_DATA_DIR`, только если это постоянная переменная пользователя.

- В `D`: `config.json`, `index.db` и `names.db` (+ `-wal`, `-shm`), `thumbs\`, `models\` (кэш Hugging Face с моделью),
  `token`, `semsearch.log`, `shell.log`, `engine.log` / `engine-selftest.*` (движок без прав; с правами —
  `<ProgramFiles>\SemSearch\logs`). Размер индекса в `/api/status` (`index_bytes`) — без `models\`.
- Модель: backend (`core.py` при импорте) ставит `HF_HUB_CACHE=D\models` и `HF_XET_CACHE=D\models\xet`, если
  пользователь не задал `HF_HUB_CACHE` / `HUGGINGFACE_HUB_CACHE`. Модель из общего кэша (`HF_HOME\hub`, по умолчанию
  `%USERPROFILE%\.cache\huggingface\hub`) переносится в `D\models` один раз, до загрузки; на другой том — копией через
  `<имя>.part`. Не удалось (файлы заняты) — в этот запуск модель берётся со старого места, перенос повторится.
- Папка программы `<root>` (где `server.py`) во время работы только читается.
- Старая раскладка (`<root>\config.json`, `<root>\data\`): backend при старте один раз переносит в `D` config.json,
  базы (вместе с `-wal`/`-shm` или никак) и `thumbs\`. Пока `D\config.json` нет, shell и движок читают `<root>\config.json`.
- `config.json`: `name_drives` нет или `null` — все несъёмные диски с файловой системой; `content_roots` по умолчанию —
  папка пользователя и известные папки (Документы, Рабочий стол, Изображения, Видео, Музыка, Загрузки) вне её.

### Переменные окружения

| Переменная | Кто читает | Что делает |
|---|---|---|
| `SEMSEARCH_DATA_DIR` | все | папка данных `D` |
| `SEMSEARCH_PORT` | backend | порт (shell передаёт выбранный, §3) |
| `SEMSEARCH_TOKEN` | backend | токен сессии (shell создаёт его сам) |
| `SEMSEARCH_SHELL=1` | backend | запущен из shell: окно, трей и горячую клавишу ведёт shell |
| `SEMSEARCH_ENGINE_PIPE` | backend, `engine_client.py` | канал отладочного движка (`--pipe <имя>`) вместо `semsearch-engine-<SessionId>`, §4 |
| `SEMSEARCH_ROOT` | shell | папка с `server.py` вместо поиска вверх от exe |
| `SEMSEARCH_DEV_URL` | shell | брать страницы с этого адреса (вёрстка с `mock_api.py`) |
| `SEMSEARCH_NO_BACKEND=1` | shell | не запускать и не останавливать backend |
| `HF_HUB_CACHE` | backend | свой кэш моделей вместо `D\models` (тогда модель не переносится) |

## 2. Backend HTTP

### Страницы и статика
- `GET /` → `ui/index.html`, `GET /settings` → `ui/settings.html`, `GET /tray` → `ui/tray.html`.
  Backend заменяет в HTML литерал `__SEMSEARCH_TOKEN__` на токен сессии (UI кладёт его в `<meta name="semsearch-token" content="__SEMSEARCH_TOKEN__">`).
- `GET /ui/<file>` — статика без токена (js, css, svg, шрифты).
- Заголовок для страниц: `Content-Security-Policy: default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'`.
  ⇒ **никаких inline `<script>` и `onclick=`**, только внешние .js + addEventListener.

### Авторизация
- Все `/api/*`, кроме `GET /api/ping`, требуют заголовок `X-SemSearch-Token: <token>`. Без него — 403.
- `GET /thumb/*` принимает токен в query: `?t=<token>` (для `<img src>`).
- Токен — `SEMSEARCH_TOKEN` (его создаёт shell) или случайный; backend пишет его в `D\token`.
- Ошибки: HTTP 4xx/5xx с `{"detail": "..."}`. Пока модель грузится, поисковые запросы → 503.

### Типы
```
Hit  = { "chunk": int, "loc": "стр. 3" | "фрагм. 2" | "1:20" | "", "t": float|null /*сек, для видео/аудио*/,
         "t_end": float|null /*конец отрезка, аудио*/, "text": str }
Item = {
  "id": str,                 // "f123" для файлов из индекса содержимого, "n:<path>" для найденных только по имени
  "kind": "text"|"code"|"image"|"video"|"audio"|"name",
  "name": str, "ext": "PDF"   /*верхний регистр, без точки, "" если нет*/,
  "path": str, "crumbs": "Документы › Дом › Ремонт крыши"   /*папка файла, по-русски для известных папок*/,
  "size": int|null, "mtime": float|null /*epoch, сек*/,
  "score": float,
  "hits": [Hit],             // ≤3, лучший первым; у видео — совпавшие моменты, у аудио — отрезки; у "name" — []
  "thumb": "/thumb/55" | null,   // для image/video; UI сам дописывает ?t=<token>
  "duration": float|null,
  "copies": int              // точные копии этого файла (по содержимому) — в выдаче «ещё N копий»
}
SearchResponse = {
  "q": str, "tab": str, "ms": int,
  "counts": { "all": int, "text": int, "code": int, "image": int, "video": int, "audio": int, "name": int },
  "name_scope": "C:",        // по каким дискам искались имена
  "items": [Item]
}
```

### Эндпоинты
- `GET /api/ping` (без токена) → `{"app": "semsearch", "pid": int}` — по нему shell и второй запуск отличают SemSearch от чужой программы на порту.
- `GET /api/search?q=<text>&tab=all|text|code|image|video|audio|name`
  - Если поиск по смыслу упал (модель, видеокарта), ответ всё равно приходит — с совпадениями по имени.
  - `tab=all`: items идут секциями в порядке: текст+код (≤30) → фото+видео (≤24) → аудио (≤5) → «только по имени» (≤5, файлы, чьё содержимое НЕ индексируется). `counts.all` = длина items.
  - `tab=name`: совпадения по имени по всему диску (≤200). `counts.name` = полное число совпадений (может быть > 200).
  - Остальные `counts.*` = сколько карточек покажет соответствующая вкладка.
- `GET /api/preview?path=<path>&chunks=<id,id,id>` (или `chunk=<id>`) — передавайте id из `item.hits[].chunk`, тогда `hits`/`marks`/`page_label`/`seg` описывают именно найденные места →
  ```
  { "path","name","ext","kind","crumbs","size","mtime","ctime",
    "type": "PDF-документ, 18 страниц" | "JPEG, 4032 × 3024" | "Видео MP4, 1920 × 1080" | "Аудио M4A" | "Python" | "Архив ZIP" | …,
    "taken": float|null, "duration": float|null, "pages": int|null, "dims": [w,h]|null,
    "page_label": "стр. 3 из 18" | "фрагмент 2 из 4" | null,
    "hit_pos": float|null,         // 0..1 — где в документе лучшее совпадение (для полос-«строк» на миниатюре страницы)
    "hits": [Hit],
    "code": { "lines": [str], "hl": [int] } | null,   // ~8 строк вокруг совпадения, hl — индексы подсвеченных строк
    "marks": [float]|null,         // видео: позиции моментов 0..1
    "wave": [float]|null,          // аудио: 44 столбика 0..1
    "seg": [start, end]|null,      // аудио: совпавший отрезок, сек
    "dupes": [{ "path", "crumbs" }],  // другие файлы с таким же именем (для вкладки «По имени»)
    "thumb": "/thumb/55?size=640" | null,
    "can_similar": bool, "indexed": bool,
    "state": "indexed"|"pending"|"failed"|"skipped"|"outside"|"unsupported",   // почему найден только по имени
    "reason": str|null,     // причина для "failed" и "skipped" (чужой код: подмодуль, архив, .gitignore, сгенерированный)
    "copy_of": str|null }   // путь оригинала, если файл — его точная копия
  // dupes[]: { "path", "crumbs", "exact": bool } — exact: та же начинка (копия), иначе только то же имя
  ```
  `kind` здесь — настоящий вид файла (если файл из вкладки «По имени» проиндексирован как документ — будет "text").
- `GET /api/similar?chunk=<int>&tab=image|video|all` → SearchResponse (`q` = "", заполнены только `counts.image/video/all`).
- `POST /api/search_image?tab=image|video|all` (multipart, поле `file`) → то же. Переключение вкладки в режиме «похожие» = повторный запрос с другим `tab`.
- `POST /api/open {"path": str, "reveal": bool}` → `{"ok": true}`; 404, если путь неизвестен индексам или файла нет.
- `GET /api/status` →
  ```
  { "ready": bool, "error": str|null, "device": "cuda"|"cpu"|null,   // cpu — нет подходящей видеокарты, причина в semsearch.log
    "state": "loading"|"indexing"|"idle"|"paused"|"error",
    "phase": "scan"|"text"|"code"|"image"|"audio"|"video"|null,
    "paused": null | { "reason": "manual"|"fullscreen"|"process"|"battery", "detail": str|null },
    "kinds": { "text": {"done","total","failed","skipped","copies"}, "code": {…}, "image": {…}, "audio": {…}, "video": {…} },
             // done включает пропущенные правилом «чужого кода» (skipped) и точные копии (copies)
    "current": { "kind": "image", "done": 11280, "total": 18204 } | null,
    "names": { "count": int, "drives": ["C:"], "engine": "mft"|"walk"|"python", "ready": bool, "updated": float|null,
               "progress": float|null /*0..1 во время пересборки, если известно*/ },
    "eta_sec": int|null, "watching": bool, "throttle": str|null, "index_bytes": int,
    "boot": { "step": "index"|"download"|"model"|"vectors"|"watch"|null,   // шаг запуска; null — запуск закончен,
                                                                       // при ошибке — шаг, на котором упали
               "first_run": bool,                                       // модели не было в кэше: будет шаг download
               "download": { "done": int, "total": int|null } | null,   // байт скачано / всего (null — сеть не сказала)
               "gpu_note": null|"unsupported"|"busy"|"failed",         // почему модель на процессоре
               "chunks": int|null,                                      // фрагментов загружено в память
               "error_kind": null|"download"|"other" } }              // модель не скачалась / другой сбой
  ```
  Экран запуска (`ui/index.html #boot`) показывает эти шаги, пока `ready` = false.
- `POST /api/boot/retry {}` — запуск упал (`state` = "error"): повторить без перезапуска программы; 409, если не падал.
- `POST /api/pause {"paused": bool}`, `POST /api/rescan {}`, `POST /api/rebuild {}` (стереть индекс содержимого и собрать заново).
- `GET /api/settings` →
  ```
  { "content_roots": [{ "path": str, "files": int }],
    "drives": { "available": [{ "drive": "C:", "label": str, "fs": "NTFS" }], "selected": ["C:"] },
    "exclude_dirs": [str], "hotkey": "ctrl+alt+s", "theme": "system"|"light"|"dark", "accent": "#005fb8",
    "lang": "system"|"ru"|"en",   // язык интерфейса; system — как в Windows (русский → ru, иначе en)
    "lang_system": "ru"|"en",      // только чтение: во что превращается system на этой машине
    "pause_on_fullscreen": bool, "pause_processes": [str], "pause_on_battery": bool,
    "video_frame_every_sec": int, "video_max_frames": int, "audio_segment_sec": int, "audio_max_segments": int,
    "max_text_mb": int, "max_pdf_pages": int,
    "foreign_code": "skip"|"last"|"normal",   // чужой код: не индексировать / в последнюю очередь / как обычный
    "info": { "model": "EmbeddingGemma 2", "device": "cuda", "index_bytes": int, "rescan_minutes": int,
              "name_rescan_minutes": int, "failed": int, "foreign_skipped": int, "copies": int } }
  ```
- `POST /api/settings {…любые редактируемые ключи выше; content_roots — список строк, drives — список выбранных или null (все несъёмные)…}` → `{"ok": true, "restart": bool}`.
  Подписи, которые собирает backend (`crumbs`, `type`, `page_label`, `loc` и `text` у hits, `reason`, `err`, тексты ошибок `detail`), — на языке `lang`.
- `GET /api/failed?limit=200` → `{"total": int, "items": [{ "path", "err" }]}`.
- `GET /api/processes` → `{"items": ["cs2.exe", …]}` — имена запущенных процессов (для «Добавить процесс»).
- `POST /api/open_log {}` — открыть `D\semsearch.log`.
- `POST /api/shutdown {}` — завершить backend.
- `GET /thumb/<chunk>?t=<token>&size=360|640` → image/jpeg.

## 3. UI ⇄ Shell (Tauri v2, `withGlobalTauri: true`)

UI проверяет `window.__TAURI__`; без него (обычный браузер) кнопки окна скрыты, «Настройки» открывают `/settings` в новой вкладке.

Окна (label → URL): `main` → `/`, `settings` → `/settings`, `tray` → `/tray`. Все — без рамки.
Перетаскивание окна: атрибут `data-tauri-drag-region` на пустых областях шапки.

Команды `__TAURI__.core.invoke(name, args)` (реализует shell):
| Команда | Аргументы | Результат |
|---|---|---|
| `win_minimize`, `win_toggle_maximize`, `win_hide` | — | действуют на вызвавшее окно |
| `show_main` | — | показать и сфокусировать главное окно (+ событие `focus-search`) |
| `show_settings` | — | открыть/сфокусировать окно настроек |
| `tray_hide` | — | спрятать попап трея |
| `quit_app` | — | `POST /api/shutdown` backend и выход |
| `pick_folder` | — | `string \| null` — системный выбор папки |
| `get_autostart` / `set_autostart` | `{enabled: bool}` | `bool` / — |
| `get_hotkey` / `set_hotkey` | `{accel: "ctrl+alt+s"}` | `string` / `null` или текст ошибки |
| `set_lang` | `{lang: "ru"\|"en"}` | — язык, которым страницы сейчас показывают интерфейс: на нём shell пишет свои тексты (заголовки окон, ошибки, строку на заставке) |

До загрузки любой страницы shell выполняет в окне `window.__SEMSEARCH_BOOT = {lang, sys, theme, accent}` (из `config.json` и языка Windows): по нему встроенная заставка (`shell/dist`) сразу рисуется на нужном языке и в нужной теме, а `ui/i18n.js` берёт начальный язык. С заставки главное окно переходит на `/#boot` — экран запуска продолжает её кадр без повторного появления.

События shell → UI (`__TAURI__.event.listen`): `focus-search` (главное окно: фокус в поле, выделить текст), `tray-shown` (попап: обновить статус).

Shell запускает backend: `<root>\.venv\Scripts\pythonw.exe <root>\server.py --no-browser`, cwd = `<root>`, env `SEMSEARCH_SHELL=1` (backend тогда не регистрирует свою горячую клавишу), `SEMSEARCH_PORT`, `SEMSEARCH_TOKEN`, `SEMSEARCH_DATA_DIR`. `<root>` — папка с `server.py` (поиск вверх от exe или `SEMSEARCH_ROOT`).

Порт (выбирается заново при каждом запуске backend). Предпочтительный `P` = `config.json:port` или 8765.
- На `P` отвечает `/api/ping` с `"app": "semsearch"`, и `GET /api/status` с токеном из `D\token` даёт 200 → shell подключается к этому backend.
- Иначе `P` свободен → backend запускается на нём; занят (чужая программа или SemSearch другого пользователя) → на первом свободном из `P+1…P+20`, иначе на порту от системы. Свободен = привязка к 127.0.0.1 удалась и никто не отвечает (соединение с самим собой не считается).
- Готовность — `/api/ping` отвечает `"app": "semsearch"` и `GET /api/status` с токеном даёт 200.
- Backend без shell (`start.bat`): если на его порту SemSearch — открывает браузер там и выходит; если чужая программа — берёт свободный порт так же. Под shell порт не меняет: занят — выходит с кодом 1, shell выберет другой.

Автозапуск: shell пишет `"<exe>" --hidden` в `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`; если exe переехал, при старте путь в записи исправляется.

## 4. Backend ⇄ Engine (named pipe `\\.\pipe\semsearch-engine-<SessionId>`)

`<SessionId>` — номер сеанса Windows (`ProcessIdToSessionId` текущего процесса, десятичный): у каждого вошедшего пользователя свой движок.
Построчный JSON (UTF-8, `\n`). Запрос `{"id": 1, "op": "...", ...}` → ответ `{"id": 1, "ok": true, ...}` или `{"id": 1, "ok": false, "error": "..."}`.

| op | Запрос | Ответ |
|---|---|---|
| `ping` | — | `{"version", "mode": "mft"\|"walk", "elevated": bool}` |
| `status` | — | `{"mode", "ready", "files", "dirs", "volumes": [{"drive","fs","mode","files","ready"}], "updated": epoch_ms, "build_ms"}` |
| `search` | `{"q", "mode": "auto"\|"substr"\|"glob"\|"regex", "limit": 200, "offset": 0, "files_only": true, "roots": [..]?, "exts": [".pdf"]?}` | `{"total", "ms", "items": [{"path","name","dir","size","mtime","ctime"}]}` |
| `same_name` | `{"name", "limit": 10}` | `{"items": [...]}` |
| `list` | `{"roots", "exts", "exclude_dirs", "exclude_paths", "skip_dot_dirs": true, "skip_cloud": true, "classify": bool, "smart_excludes": bool}` | `{"items": [{"path","size","mtime","attrs","tag","repo"}]}` |
| `exists` | `{"path"}` | `{"exists": bool, "indexed": bool}` |
| `subscribe` | `{"roots": [..]}` | `{"ok": true}`, затем поток событий `{"ev": "created"\|"modified"\|"deleted"\|"renamed"\|"rescan", "path", "old": str\|null, "dir": bool}` |
| `reload_config` | — | перечитать `D\config.json` (диски) |
| `shutdown` | — | завершиться |

`auto`: есть `*`/`?` → glob; префикс `re:` → regex; иначе слова через пробел (AND, без учёта регистра, кириллица тоже); слово с `\` или `/` сравнивается с полным путём, остальные — с именем.
Сортировка: точное совпадение имени → имя начинается с первого слова → короче имя → короче путь.
«modified» — только по закрытию файла (USN_REASON_CLOSE), чтобы недописанный файл не индексировался.

`list` с `smart_excludes` не отдаёт папки сборки и зависимостей, узнавая их по соседним файлам проекта (`bin`/`obj` рядом с `*.csproj`, `target` рядом с `Cargo.toml`, `Library`/`Temp` в проекте Unity, `Intermediate`/`Saved` рядом с `*.uproject`, `build`/`dist` рядом с `package.json`, папка с `pyvenv.cfg`, `node_modules` всегда…) — те же правила, что `core.is_build_dir`.
С `classify` у каждого файла `tag`: `submodule` (путь из `.gitmodules` или `.git`-файл, ведущий в `.git/modules`; worktree — не подмодуль), `gitignored` (по `.gitignore` проекта и `info/exclude`), `archive` (вне git, папка-предок `*-main`, `*-master`, `*-1.2.3`), `foreign` (клон без коммитов локального пользователя git), иначе `null`. Что с ними делать, решает backend (настройка `foreign_code`).
Для разработки: `semsearch-engine.exe --pipe <имя>` и переменная `SEMSEARCH_ENGINE_PIPE` для backend/клиента — свой канал, установленный движок не трогается.
Канал доступен только текущему пользователю (DACL: SYSTEM, Администраторы, SID пользователя; метка целостности Medium). Движок ничего не меняет на диске.
