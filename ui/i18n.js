/* SemSearch UI — язык интерфейса: словари ru/en, выбор языка, перевод разметки.
   Подключается ПЕРВЫМ (до common.js). Настройка — config.json "lang": system | ru | en; "system" — язык Windows
   (его сообщают backend в /api/settings → lang_system и shell в window.__SEMSEARCH_BOOT.sys).
   Окна синхронизируются через localStorage "semsearch.lang" (как тема).
   Разметка: data-i18n="ключ" — текст; data-i18n-attr="placeholder:ключ;aria-label:ключ" — атрибуты.
   Значение-массив — формы числа: ru [1, 2–4, 5+], en [1, другое]; SS.tn(ключ, n) подставляет {n}. */
(function () {
  'use strict';
  const SS = (window.SS = window.SS || {});

  const D = {
    ru: {
      /* окно, общее */
      'win.min': 'Свернуть', 'win.max': 'Развернуть', 'win.restore': 'Восстановить', 'win.hide': 'Скрыть в трей',
      'win.hideTip': 'Скрыть в трей (Esc)', 'win.close': 'Закрыть',
      'title.main': 'SemSearch — поиск', 'title.settings': 'SemSearch — настройки', 'title.tray': 'SemSearch — меню в трее',
      'q.label': 'Запрос', 'q.ph': 'Что ищем? Опишите файл своими словами', 'q.clear': 'Очистить запрос',
      'img.btn': 'Искать по картинке', 'img.tip': 'Искать по картинке — или перетащите её в окно, или Ctrl+V',
      'img.similar': 'похоже на {name}', 'img.anImage': 'картинку', 'img.remove': 'Убрать картинку',
      'img.need': 'Нужна картинка: JPG, PNG, WebP…', 'img.drop': 'Отпустите — найду похожие фото и кадры',
      'pane.label': 'Просмотр файла', 'pane.empty': 'Здесь будет просмотр выбранного файла',
      'foot.pause': 'Пауза', 'foot.resume': 'Продолжить', 'foot.settings': 'Настройки',
      'foot.pauseFail': 'Не удалось переключить паузу',
      'act.open': 'Открыть', 'act.reveal': 'Показать в папке', 'act.copy': 'Копировать путь',
      'act.copyTip': 'Копировать путь (Ctrl+Shift+C)', 'act.similar': 'Найти похожие',
      'toast.notFound': 'Файл не найден — возможно, его переместили', 'toast.openFail': 'Не удалось открыть файл',
      'toast.copied': 'Путь скопирован', 'toast.copyFail': 'Не удалось скопировать путь',
      'toast.saved': 'Сохранено', 'toast.restart': 'Применится после перезапуска',

      /* пустое окно */
      'start.h1': 'Ищите так, как описали бы файл человеку',
      'start.ex1': 'договор про крышу', 'start.ex2': 'кот на балконе', 'start.ex3': 'где парсил JSON с ретраями',
      'start.ex4': 'скриншот с ошибкой CUDA',
      'start.hint': 'Картинку можно перетащить в окно или вставить через Ctrl+V — найдутся похожие фото и кадры из видео.',
      'start.first': 'Первая индексация',
      'start.firstNote': 'Искать уже можно — найдётся всё, что успело проиндексироваться. Пока запущена игра или фильм на весь экран, индексация сама встаёт на паузу.',
      'start.keys': 'Клавиши', 'start.keysNote': 'меняются в настройках',
      'key.hotkey': 'открыть окно из любой программы', 'key.enter': 'открыть файл', 'key.reveal': 'показать в папке',
      'key.arrows': 'выбрать результат', 'key.tabs': 'вкладки: всё, документы, код…', 'key.escTray': 'спрятать окно в трей',
      'key.escClear': 'очистить запрос',
      'fix.text': 'Документы', 'fix.code': 'Код', 'fix.image': 'Фото', 'fix.audio': 'Аудио', 'fix.video': 'Видео',
      'fix.names': 'Имена файлов на {where}', 'fix.done': 'готово · {n}', 'fix.none': 'нет файлов',
      'fix.of': '{a} из {b}', 'fix.queued': 'в очереди · {n}', 'fix.collect': 'собираю · {n}',
      'eta.lt1': 'осталось меньше минуты', 'eta.min': ['осталось около {n} минуты', 'осталось около {n} минут', 'осталось около {n} минут'],
      'eta.hour': ['осталось около {n} часа', 'осталось около {n} часов', 'осталось около {n} часов'],

      /* вкладки и выдача */
      'tabs.label': 'Что искать',
      'tab.all': 'Всё', 'tab.text': 'Документы', 'tab.code': 'Код', 'tab.image': 'Фото', 'tab.video': 'Видео',
      'tab.audio': 'Аудио', 'tab.name': 'По имени',
      'meta.ms': '{n} мс', 'meta.files': ['{n} файл', '{n} файла', '{n} файлов'], 'meta.photos': '{n} фото', 'meta.videos': '{n} видео',
      'meta.onDrive': 'на диске {d}', 'meta.onDrives': 'на дисках {d}',
      'sec.docs': 'Документы и код', 'sec.media': 'Фото и видео', 'sec.audio': 'Аудио', 'sec.names': 'Только по имени',
      'sec.namesNote': 'содержимое этих файлов не индексируется', 'sec.allMedia': 'Все фото и видео',
      'row.copies': ['ещё {n} копия', 'ещё {n} копии', 'ещё {n} копий'],
      'snip.audio': 'Совпал отрезок {a}–{b}', 'snip.audioLen': ' · длительность {d}', 'snip.matched': 'Совпал {t}',
      'nh.name': 'Имя', 'nh.folder': 'Папка', 'nh.size': 'Размер', 'nh.mtime': 'Изменён',
      'vnote.q': 'Ещё {n} видео, где {q} попадает в кадр', 'vnote.sim': 'Ещё {n} видео с похожими кадрами',
      'vnote.at': '«{name}» на {loc}', 'vnote.name': '«{name}»', 'vnote.open': 'Открыть вкладку «Видео»',
      'load.done': 'Готово — можно искать', 'load.gpu': 'Загружаю модель на видеокарту', 'load.any': 'Загружаю модель поиска',
      'load.sub': 'Поиск по смыслу начнётся сам, как только модель будет готова. Поиск по имени работает уже сейчас.',
      'load.byName': 'Искать по имени', 'load.byMeaning': 'Искать по смыслу',
      'err.h': 'Не получилось выполнить поиск', 'err.p': 'SemSearch не отвечает. Попробуйте ещё раз через минуту.',
      'none.h': 'Ничего не нашлось',
      'none.name': 'Ни одно имя файла не подходит. Проверьте написание или поищите по смыслу.',
      'none.pend': 'Ещё индексируются {what} — подходящее может появиться позже. ',
      'none.try': 'Попробуйте описать иначе или поискать по имени.', 'none.and': ' и ',
      'kw.video': ['{n} видео', '{n} видео', '{n} видео'], 'kw.image': ['{n} фото', '{n} фото', '{n} фото'],
      'kw.audio': ['{n} аудио', '{n} аудио', '{n} аудио'], 'kw.text': ['{n} документ', '{n} документа', '{n} документов'],
      'kw.code': ['{n} файл кода', '{n} файла кода', '{n} файлов кода'],

      /* просмотр */
      'pv.firstPage': 'первая страница', 'pv.seg': 'совпал отрезок {a}–{b}', 'pv.segOf': ' из {d}', 'pv.len': 'длительность {d}',
      'pv.pending': 'Найдено по имени. Файл ещё ждёт индексации — по содержимому он найдётся, когда до него дойдёт очередь.',
      'pv.failed': 'Найдено по имени. Содержимое не удалось прочитать{why}.',
      'pv.skipped': 'Найдено по имени. По содержимому не индексируется{why}. Это меняется в настройках: «Чужой код и библиотеки».',
      'pv.outside': 'Найдено по имени. Эта папка не входит в поиск по смыслу — её можно добавить в настройках.',
      'pv.noIndexExt': 'Найдено по имени. Содержимое файлов .{ext} не индексируется.',
      'pv.noIndex': 'Найдено по имени. Содержимое этого файла не индексируется.',
      'pv.type': 'Тип', 'pv.size': 'Размер', 'pv.len2': 'Длительность', 'pv.taken': 'Снято', 'pv.mtime': 'Изменён',
      'pv.ctime': 'Создан',
      'pv.hitsVideo': 'Совпавшие моменты', 'pv.hitsAudio': 'Совпавший отрезок', 'pv.hits': 'Где нашлось',
      'pv.copies': ['Ещё {n} копия', 'Ещё {n} копии', 'Ещё {n} копий'], 'pv.copiesExact': ' с точно таким же содержимым',
      'pv.copiesName': ' с таким же именем', 'pv.andMore': ' и ещё в {n}',
      'pv.copyOf': 'Точная копия файла «{name}» — содержимое ищется по нему.',
      'place.desktop': 'на рабочем столе', 'place.downloads': 'в загрузках', 'place.documents': 'в документах',
      'place.pictures': 'в изображениях', 'place.videos': 'в видео', 'place.music': 'в музыке', 'place.onedrive': 'в OneDrive',
      'place.folder': 'в папке «{name}»',

      /* статус */
      'st.noLink': 'Нет связи с SemSearch', 'st.error': 'Ошибка: {e}', 'st.unknown': 'неизвестная',
      'st.errDownload': 'Не удалось скачать модель', 'st.errModel': 'Не удалось загрузить модель',
      'st.loading': 'Загружаю модель поиска…', 'st.pFull': 'Пауза: полноэкранное приложение',
      'st.pProc': 'Пауза: запущен {p}', 'st.pProcAny': 'процесс из списка', 'st.pBatt': 'Пауза: работа от батареи',
      'st.paused': 'Индексация на паузе', 'st.indexing': 'Индексирую {what}: {a} из {b}', 'st.ixLabel': 'Индексация {what}',
      'st.scan': 'Ищу новые и изменённые файлы…', 'st.ixAny': 'Индексирую…', 'st.done': 'Индекс актуален',
      'st.doneWatch': 'Индекс актуален, новые файлы подхватываются сразу',
      'kindw.text': 'документы', 'kindw.code': 'код', 'kindw.image': 'фото', 'kindw.audio': 'аудио', 'kindw.video': 'видео',
      'kindw.any': 'файлы',
      'nst.none': 'Имена: нет данных', 'nst.collect': 'Имена: собираю список файлов на {where} — {n}',
      'nst.label': 'Сбор имён файлов', 'nst.done': 'Имена: {files} на {where}, обновлено {when}',
      'mid.allDrives': 'По имени ищутся все выбранные диски, включая AppData и Program Files',
      'mid.oneDrive': 'По имени ищется весь диск, включая AppData и Program Files',
      'sum.prefix': 'В индексе: {list}', 'sum.text': ['{n} документ', '{n} документа', '{n} документов'],
      'sum.code': ['{n} файл кода', '{n} файла кода', '{n} файлов кода'], 'sum.image': '{n} фото', 'sum.audio': '{n} аудио',
      'sum.video': '{n} видео', 'sum.names': ['{n} имя', '{n} имени', '{n} имён'], 'sum.namesBig': '{n} имён', 'sum.on': '{a} на {b}',
      'dev.none': 'Поиск по смыслу недоступен', 'dev.loading': 'Модель загружается…', 'dev.gpu': 'Модель работает на видеокарте', 'dev.cpu': 'Модель работает на процессоре',

      /* экран запуска */
      'boot.title': 'SemSearch', 'boot.starting': 'Запускаю SemSearch…', 'boot.firstRun': 'Первый запуск SemSearch', 'boot.ready': 'Готово',
      'boot.opening': 'Открываю окно поиска…', 'boot.openingFirst': 'Открываю окно поиска — индексация пойдёт в нём',
      'boot.progress': 'Запуск',
      'boot.s.server': 'Запускаю сервер поиска', 'boot.s.index': 'Открываю индекс',
      'boot.s.download': 'Скачиваю модель EmbeddingGemma 2', 'boot.s.model': 'Загружаю модель',
      'boot.s.vectors': 'Загружаю векторы в память',
      'boot.s.watch': 'Включаю слежение за файлами',
      'boot.n.once': 'один раз', 'boot.n.of': '{a} из {b}', 'boot.n.got': 'скачано {a}',
      'boot.n.files': ['{n} файл', '{n} файла', '{n} файлов'], 'boot.n.chunks': ['{n} фрагмент', '{n} фрагмента', '{n} фрагментов'],
      'boot.n.gpu': 'на видеокарте', 'boot.n.cpu': 'на процессоре', 'boot.n.busy': 'на процессоре — видеокарта занята',
      'boot.n.unsupported': 'на процессоре — видеокарта не поддерживается', 'boot.n.failed': 'на процессоре — видеокарта не запустилась',
      'boot.names': 'Пока ждём — поиск по имени уже работает', 'boot.byName': 'Искать по имени',
      'boot.errDownload': 'Не удалось скачать модель', 'boot.errDownloadNote': 'Проверьте подключение к интернету — модель скачивается один раз.',
      'boot.errOther': 'Не удалось загрузить модель', 'boot.errOtherNote': 'Подробности — в журнале.',
      'boot.retry': 'Повторить', 'boot.log': 'Открыть журнал', 'boot.logLink': 'открыть журнал', 'boot.noModel': 'Поиск по имени работает и без модели',
      'boot.foot': 'открыть из любой программы — {hk}', 'boot.retryFail': 'Не удалось перезапустить',

      /* трей */
      'tray.connecting': 'Подключаюсь…', 'tray.open': 'Открыть поиск', 'tray.pause': 'Пауза индексации',
      'tray.resume': 'Продолжить индексацию', 'tray.rescan': 'Пересканировать', 'tray.settings': 'Настройки',
      'tray.quit': 'Выйти из SemSearch', 'tray.done': 'Индекс актуален',

      /* настройки */
      'set.win': 'Настройки SemSearch', 'set.h1': 'Настройки',
      'set.where': 'Где искать', 'set.sem': 'Поиск по смыслу', 'set.semNote': 'Документы, код, фото, аудио и видео в этих папках',
      'set.addFolder': 'Добавить папку', 'set.name': 'Поиск по имени',
      'set.nameNote': 'Имена всех файлов на выбранных дисках, включая системные папки',
      'set.excl': 'Не индексировать', 'set.exclNote': 'Папки с такими именами пропускаются везде. Скрытые папки с точкой (.ssh, .aws) — всегда.',
      'set.edit': 'Изменить', 'set.foreign': 'Чужой код и библиотеки',
      'set.foreignNote': 'Git-подмодули, скачанные архивы исходников, сборки и зависимости, сгенерированные файлы. Ваш код и документы не трогаются.',
      'set.f.skip': 'Не индексировать', 'set.f.last': 'В последнюю очередь', 'set.f.normal': 'Как обычный код',
      'set.winSec': 'Окно и запуск', 'set.lang': 'Язык', 'set.langNote': 'Язык окна, меню в трее и подсказок',
      'set.l.system': 'Как в Windows', 'set.hotkey': 'Горячая клавиша', 'set.hotkeyNote': 'Открывает окно поиска из любой программы',
      'set.auto': 'Запускать вместе с Windows', 'set.autoNote': 'Работает в трее, окно открывается по горячей клавише',
      'set.theme': 'Тема', 'set.themeNote': 'Светлая, тёмная или как в Windows',
      'set.t.light': 'Светлая', 'set.t.dark': 'Тёмная', 'set.t.system': 'Как в Windows',
      'set.games': 'Игры и нагрузка', 'set.full': 'Пауза при полноэкранных приложениях',
      'set.fullNote': 'Игры, фильмы, презентации — и окна без рамки на весь экран. Видеопамять отдаётся игре.',
      'set.procs': 'Пауза, пока запущены', 'set.procsNote': 'Например, игры в окне', 'set.addProc': 'Добавить процесс',
      'set.batt': 'Пауза от батареи', 'set.battNote': 'На ноутбуке без зарядки индексация ждёт',
      'set.media': 'Видео, аудио и большие файлы',
      'set.m.frame': 'Кадр из видео каждые, с', 'set.m.frames': 'Кадров на видео, не больше', 'set.m.seg': 'Отрезок аудио, с',
      'set.m.segs': 'Отрезков, не больше', 'set.m.text': 'Текст больше, МБ — пропуск', 'set.m.pdf': 'Страниц PDF, не больше',
      'set.index': 'Индекс', 'set.i.model': 'Модель', 'set.i.size': 'Размер на диске', 'set.i.rescan': 'Проверка всего диска',
      'set.i.failed': 'Не удалось прочитать', 'set.i.foreign': 'Чужой код и копии',
      'set.rescan': 'Пересканировать сейчас', 'set.log': 'Открыть журнал', 'set.rebuild': 'Собрать индекс заново…',
      'set.model': 'модель', 'set.onGpu': 'на видеокарте', 'set.onCpu': 'на процессоре', 'set.loading': 'загружается',
      'set.countingFiles': 'считаю файлы…', 'set.files': ['{n} файл', '{n} файла', '{n} файлов'],
      'set.removeFolder': 'Убрать папку {p}', 'set.removeProc': 'Убрать {p}', 'set.moreN': 'и ещё {n}',
      'set.rescanEvery': '{a}, имена — {b}', 'set.manual': 'вручную', 'set.daily': 'раз в сутки', 'set.everyDays': 'каждые {n} дн.',
      'set.hourly': 'каждый час', 'set.everyHours': 'каждые {n} ч', 'set.everyMin': 'каждые {n} мин',
      'set.foreignInfo': 'пропущено {a} · копий {b}', 'set.failedN': ['{n} файл', '{n} файла', '{n} файлов'],
      'set.why': 'почему', 'set.no': 'нет',
      'set.saveFail': 'Не удалось сохранить: {e}', 'set.noAnswer': 'SemSearch не отвечает', 'set.loadFail': 'Не удалось загрузить настройки',
      'set.folderPrompt': 'Папка для поиска по смыслу:', 'set.folderDup': 'Эта папка уже в списке',
      'set.needDrive': 'Нужен хотя бы один диск',
      'set.exclLabel': 'Папки, по одной в строке', 'set.exclText': 'По одному имени папки в строке. Такие папки пропускаются на любой глубине.',
      'set.save': 'Сохранить', 'set.cancel': 'Отмена', 'set.close': 'Закрыть',
      'set.hkPress': 'Нажмите сочетание…', 'set.hkNeedMod': 'Нужно сочетание с Ctrl, Alt или Win — например, Ctrl+Alt+S',
      'set.hkFail': 'Не получилось назначить {k}: {e}', 'set.hkFailAny': 'не удалось назначить',
      'set.hkBusy': 'Сочетание {k} занято другой программой — окно по нему не откроется. Выберите другое.',
      'set.autoFail': 'Не удалось изменить автозапуск',
      'set.procPh': 'Имя процесса, например game.exe', 'set.procLabel': 'Имя процесса',
      'set.procLoading': 'Загружаю список запущенных программ…', 'set.procAddHint': 'Нажмите «Добавить», чтобы добавить «{p}».',
      'set.procNone': 'Запущенных программ не нашлось.', 'set.procTitle': 'Пауза, пока запущен процесс',
      'set.procText': 'Выберите запущенную программу или впишите имя .exe.', 'set.add': 'Добавить',
      'set.scanning': 'Сканирую…', 'set.scanDone': 'Готово', 'set.scanNew': 'Готово, новых файлов: {n}', 'set.scanFail': 'Не удалось запустить',
      'set.logFail': 'Не удалось открыть журнал',
      'set.rebuildQ': 'Собрать индекс заново?', 'set.rebuildText': 'Текущий индекс удалится и соберётся с нуля.',
      'set.rebuildGo': 'Собрать заново', 'set.rebuildOk': 'Индекс собирается заново', 'set.rebuildFail': 'Не удалось начать сборку',
      'set.failedTitle': 'Не удалось прочитать', 'set.failedLog': 'Подробности — в журнале.', 'set.loadingDots': 'Загружаю…',
      'set.failedCount': '{files}{shown}. Подробности — в журнале.', 'set.failedShown': ', показаны первые {n}',
      'set.failedListFail': 'Не удалось получить список.'
    },

    en: {
      'win.min': 'Minimize', 'win.max': 'Maximize', 'win.restore': 'Restore', 'win.hide': 'Hide to tray',
      'win.hideTip': 'Hide to tray (Esc)', 'win.close': 'Close',
      'title.main': 'SemSearch — search', 'title.settings': 'SemSearch — settings', 'title.tray': 'SemSearch — tray menu',
      'q.label': 'Query', 'q.ph': 'What are you looking for? Describe the file in your own words', 'q.clear': 'Clear query',
      'img.btn': 'Search by image', 'img.tip': 'Search by image — or drag one into the window, or press Ctrl+V',
      'img.similar': 'similar to {name}', 'img.anImage': 'an image', 'img.remove': 'Remove the image',
      'img.need': 'Needs an image: JPG, PNG, WebP…', 'img.drop': 'Drop it — I’ll find similar photos and frames',
      'pane.label': 'File preview', 'pane.empty': 'The selected file will be previewed here',
      'foot.pause': 'Pause', 'foot.resume': 'Resume', 'foot.settings': 'Settings',
      'foot.pauseFail': 'Couldn’t toggle the pause',
      'act.open': 'Open', 'act.reveal': 'Show in folder', 'act.copy': 'Copy path',
      'act.copyTip': 'Copy path (Ctrl+Shift+C)', 'act.similar': 'Find similar',
      'toast.notFound': 'File not found — it may have been moved', 'toast.openFail': 'Couldn’t open the file',
      'toast.copied': 'Path copied', 'toast.copyFail': 'Couldn’t copy the path',
      'toast.saved': 'Saved', 'toast.restart': 'Takes effect after a restart',

      'start.h1': 'Search the way you’d describe a file to a person',
      'start.ex1': 'roof repair contract', 'start.ex2': 'cat on the balcony', 'start.ex3': 'where I parsed JSON with retries',
      'start.ex4': 'screenshot with a CUDA error',
      'start.hint': 'Drag an image into the window or paste it with Ctrl+V to find similar photos and video frames.',
      'start.first': 'First indexing',
      'start.firstNote': 'You can already search — everything indexed so far will be found. While a game or film runs full screen, indexing pauses on its own.',
      'start.keys': 'Keys', 'start.keysNote': 'change them in Settings',
      'key.hotkey': 'open the window from any app', 'key.enter': 'open the file', 'key.reveal': 'show in folder',
      'key.arrows': 'pick a result', 'key.tabs': 'tabs: all, documents, code…', 'key.escTray': 'hide the window to the tray',
      'key.escClear': 'clear the query',
      'fix.text': 'Documents', 'fix.code': 'Code', 'fix.image': 'Photos', 'fix.audio': 'Audio', 'fix.video': 'Video',
      'fix.names': 'File names on {where}', 'fix.done': 'done · {n}', 'fix.none': 'no files',
      'fix.of': '{a} of {b}', 'fix.queued': 'queued · {n}', 'fix.collect': 'collecting · {n}',
      'eta.lt1': 'less than a minute left', 'eta.min': ['about {n} minute left', 'about {n} minutes left'],
      'eta.hour': ['about {n} hour left', 'about {n} hours left'],

      'tabs.label': 'What to search',
      'tab.all': 'All', 'tab.text': 'Documents', 'tab.code': 'Code', 'tab.image': 'Photos', 'tab.video': 'Video',
      'tab.audio': 'Audio', 'tab.name': 'By name',
      'meta.ms': '{n} ms', 'meta.files': ['{n} file', '{n} files'], 'meta.photos': ['{n} photo', '{n} photos'],
      'meta.videos': ['{n} video', '{n} videos'], 'meta.onDrive': 'on drive {d}', 'meta.onDrives': 'on drives {d}',
      'sec.docs': 'Documents and code', 'sec.media': 'Photos and video', 'sec.audio': 'Audio', 'sec.names': 'Name only',
      'sec.namesNote': 'the contents of these files aren’t indexed', 'sec.allMedia': 'All photos and video',
      'row.copies': ['{n} more copy', '{n} more copies'],
      'snip.audio': 'Matched segment {a}–{b}', 'snip.audioLen': ' · length {d}', 'snip.matched': 'Matched {t}',
      'nh.name': 'Name', 'nh.folder': 'Folder', 'nh.size': 'Size', 'nh.mtime': 'Modified',
      'vnote.q': ['{n} more video with “{q}” in the frame', '{n} more videos with “{q}” in the frame'],
      'vnote.sim': ['{n} more video with similar frames', '{n} more videos with similar frames'],
      'vnote.at': '“{name}” at {loc}', 'vnote.name': '“{name}”', 'vnote.open': 'Open the Video tab',
      'load.done': 'Ready — you can search', 'load.gpu': 'Loading the model onto the GPU', 'load.any': 'Loading the search model',
      'load.sub': 'Search by meaning starts on its own once the model is ready. Search by name already works.',
      'load.byName': 'Search by name', 'load.byMeaning': 'Search by meaning',
      'err.h': 'The search didn’t work', 'err.p': 'SemSearch isn’t answering. Try again in a minute.',
      'none.h': 'Nothing found',
      'none.name': 'No file name matches. Check the spelling or search by meaning.',
      'none.pend': '{what} are still being indexed — a match may show up later. ',
      'none.try': 'Try describing it differently or search by name.', 'none.and': ' and ',
      'kw.video': ['{n} video', '{n} videos'], 'kw.image': ['{n} photo', '{n} photos'],
      'kw.audio': ['{n} audio file', '{n} audio files'], 'kw.text': ['{n} document', '{n} documents'],
      'kw.code': ['{n} code file', '{n} code files'],

      'pv.firstPage': 'first page', 'pv.seg': 'matched segment {a}–{b}', 'pv.segOf': ' of {d}', 'pv.len': 'length {d}',
      'pv.pending': 'Found by name. The file is still waiting to be indexed — it will be found by its contents once its turn comes.',
      'pv.failed': 'Found by name. Its contents couldn’t be read{why}.',
      'pv.skipped': 'Found by name. Not indexed by contents{why}. You can change this in Settings: “Third-party code and libraries”.',
      'pv.outside': 'Found by name. This folder isn’t part of search by meaning — you can add it in Settings.',
      'pv.noIndexExt': 'Found by name. The contents of .{ext} files aren’t indexed.',
      'pv.noIndex': 'Found by name. The contents of this file aren’t indexed.',
      'pv.type': 'Type', 'pv.size': 'Size', 'pv.len2': 'Length', 'pv.taken': 'Taken', 'pv.mtime': 'Modified',
      'pv.ctime': 'Created',
      'pv.hitsVideo': 'Matching moments', 'pv.hitsAudio': 'Matching segment', 'pv.hits': 'Where it matched',
      'pv.copies': ['{n} more copy', '{n} more copies'], 'pv.copiesExact': ' with exactly the same contents',
      'pv.copiesName': ' with the same name', 'pv.andMore': ' and in {n} more',
      'pv.copyOf': 'An exact copy of “{name}” — its contents are searched through that file.',
      'place.desktop': 'on the desktop', 'place.downloads': 'in Downloads', 'place.documents': 'in Documents',
      'place.pictures': 'in Pictures', 'place.videos': 'in Videos', 'place.music': 'in Music', 'place.onedrive': 'in OneDrive',
      'place.folder': 'in the “{name}” folder',

      'st.noLink': 'No connection to SemSearch', 'st.error': 'Error: {e}', 'st.unknown': 'unknown',
      'st.errDownload': 'Couldn’t download the model', 'st.errModel': 'Couldn’t load the model',
      'st.loading': 'Loading the search model…', 'st.pFull': 'Paused: full-screen app',
      'st.pProc': 'Paused: {p} is running', 'st.pProcAny': 'a listed process', 'st.pBatt': 'Paused: on battery',
      'st.paused': 'Indexing paused', 'st.indexing': 'Indexing {what}: {a} of {b}', 'st.ixLabel': 'Indexing {what}',
      'st.scan': 'Looking for new and changed files…', 'st.ixAny': 'Indexing…', 'st.done': 'Index is up to date',
      'st.doneWatch': 'Index is up to date, new files are picked up right away',
      'kindw.text': 'documents', 'kindw.code': 'code', 'kindw.image': 'photos', 'kindw.audio': 'audio', 'kindw.video': 'videos',
      'kindw.any': 'files',
      'nst.none': 'Names: no data', 'nst.collect': 'Names: collecting the file list on {where} — {n}',
      'nst.label': 'Collecting file names', 'nst.done': 'Names: {files} on {where}, updated {when}',
      'mid.allDrives': 'Search by name covers all selected drives, AppData and Program Files included',
      'mid.oneDrive': 'Search by name covers the whole drive, AppData and Program Files included',
      'sum.prefix': 'Indexed: {list}', 'sum.text': ['{n} document', '{n} documents'],
      'sum.code': ['{n} code file', '{n} code files'], 'sum.image': ['{n} photo', '{n} photos'], 'sum.audio': ['{n} audio file', '{n} audio files'],
      'sum.video': ['{n} video', '{n} videos'], 'sum.names': ['{n} name', '{n} names'], 'sum.namesBig': '{n} names', 'sum.on': '{a} on {b}',
      'dev.none': 'Search by meaning is unavailable', 'dev.loading': 'The model is loading…', 'dev.gpu': 'The model runs on the GPU', 'dev.cpu': 'The model runs on the CPU',

      'boot.title': 'SemSearch', 'boot.starting': 'Starting SemSearch…', 'boot.firstRun': 'First start of SemSearch', 'boot.ready': 'Ready',
      'boot.opening': 'Opening the search window…', 'boot.openingFirst': 'Opening the search window — indexing continues there',
      'boot.progress': 'Startup',
      'boot.s.server': 'Starting the search server', 'boot.s.index': 'Opening the index',
      'boot.s.download': 'Downloading the EmbeddingGemma 2 model', 'boot.s.model': 'Loading the model',
      'boot.s.vectors': 'Loading vectors into memory',
      'boot.s.watch': 'Watching files for changes',
      'boot.n.once': 'one time only', 'boot.n.of': '{a} of {b}', 'boot.n.got': '{a} downloaded',
      'boot.n.files': ['{n} file', '{n} files'], 'boot.n.chunks': ['{n} chunk', '{n} chunks'],
      'boot.n.gpu': 'on the GPU', 'boot.n.cpu': 'on the CPU', 'boot.n.busy': 'on the CPU — the GPU is busy',
      'boot.n.unsupported': 'on the CPU — the GPU isn’t supported', 'boot.n.failed': 'on the CPU — the GPU didn’t start',
      'boot.names': 'While you wait, search by name already works', 'boot.byName': 'Search by name',
      'boot.errDownload': 'Couldn’t download the model', 'boot.errDownloadNote': 'Check your internet connection — the model is downloaded only once.',
      'boot.errOther': 'Couldn’t load the model', 'boot.errOtherNote': 'The details are in the log.',
      'boot.retry': 'Try again', 'boot.log': 'Open the log', 'boot.logLink': 'open the log', 'boot.noModel': 'Search by name works without the model',
      'boot.foot': 'open from any app — {hk}', 'boot.retryFail': 'Couldn’t restart',

      'tray.connecting': 'Connecting…', 'tray.open': 'Open search', 'tray.pause': 'Pause indexing',
      'tray.resume': 'Resume indexing', 'tray.rescan': 'Rescan', 'tray.settings': 'Settings',
      'tray.quit': 'Quit SemSearch', 'tray.done': 'Index is up to date',

      'set.win': 'SemSearch Settings', 'set.h1': 'Settings',
      'set.where': 'Where to search', 'set.sem': 'Search by meaning', 'set.semNote': 'Documents, code, photos, audio and video in these folders',
      'set.addFolder': 'Add folder', 'set.name': 'Search by name',
      'set.nameNote': 'Names of every file on the selected drives, system folders included',
      'set.excl': 'Don’t index', 'set.exclNote': 'Folders with these names are skipped everywhere. Hidden dot folders (.ssh, .aws) always are.',
      'set.edit': 'Change', 'set.foreign': 'Third-party code and libraries',
      'set.foreignNote': 'Git submodules, downloaded source archives, builds and dependencies, generated files. Your own code and documents aren’t affected.',
      'set.f.skip': 'Don’t index', 'set.f.last': 'Index last', 'set.f.normal': 'Like regular code',
      'set.winSec': 'Window and startup', 'set.lang': 'Language', 'set.langNote': 'Language of the window, tray menu and tips',
      'set.l.system': 'Same as Windows', 'set.hotkey': 'Shortcut', 'set.hotkeyNote': 'Opens the search window from any app',
      'set.auto': 'Start with Windows', 'set.autoNote': 'Runs in the tray; the shortcut opens the window',
      'set.theme': 'Theme', 'set.themeNote': 'Light, dark or same as Windows',
      'set.t.light': 'Light', 'set.t.dark': 'Dark', 'set.t.system': 'Same as Windows',
      'set.games': 'Games and load', 'set.full': 'Pause for full-screen apps',
      'set.fullNote': 'Games, films, presentations — and borderless full-screen windows. Video memory goes to the game.',
      'set.procs': 'Pause while running', 'set.procsNote': 'For example, windowed games', 'set.addProc': 'Add process',
      'set.batt': 'Pause on battery', 'set.battNote': 'On a laptop without a charger, indexing waits',
      'set.media': 'Video, audio and large files',
      'set.m.frame': 'Video frame every, s', 'set.m.frames': 'Frames per video, max', 'set.m.seg': 'Audio segment, s',
      'set.m.segs': 'Segments, max', 'set.m.text': 'Skip text larger than, MB', 'set.m.pdf': 'PDF pages, max',
      'set.index': 'Index', 'set.i.model': 'Model', 'set.i.size': 'Size on disk', 'set.i.rescan': 'Full disk check',
      'set.i.failed': 'Couldn’t read', 'set.i.foreign': 'Third-party code and copies',
      'set.rescan': 'Rescan now', 'set.log': 'Open log', 'set.rebuild': 'Rebuild index…',
      'set.model': 'model', 'set.onGpu': 'on the GPU', 'set.onCpu': 'on the CPU', 'set.loading': 'loading',
      'set.countingFiles': 'counting files…', 'set.files': ['{n} file', '{n} files'],
      'set.removeFolder': 'Remove folder {p}', 'set.removeProc': 'Remove {p}', 'set.moreN': 'and {n} more',
      'set.rescanEvery': '{a}, names {b}', 'set.manual': 'manually', 'set.daily': 'once a day', 'set.everyDays': 'every {n} days',
      'set.hourly': 'every hour', 'set.everyHours': 'every {n} h', 'set.everyMin': 'every {n} min',
      'set.foreignInfo': 'skipped {a} · copies {b}', 'set.failedN': ['{n} file', '{n} files'],
      'set.why': 'why', 'set.no': 'none',
      'set.saveFail': 'Couldn’t save: {e}', 'set.noAnswer': 'SemSearch isn’t answering', 'set.loadFail': 'Couldn’t load the settings',
      'set.folderPrompt': 'Folder for search by meaning:', 'set.folderDup': 'This folder is already in the list',
      'set.needDrive': 'At least one drive is needed',
      'set.exclLabel': 'Folders, one per line', 'set.exclText': 'One folder name per line. Such folders are skipped at any depth.',
      'set.save': 'Save', 'set.cancel': 'Cancel', 'set.close': 'Close',
      'set.hkPress': 'Press a shortcut…', 'set.hkNeedMod': 'Use Ctrl, Alt or Win in the shortcut — for example, Ctrl+Alt+S',
      'set.hkFail': 'Couldn’t set {k}: {e}', 'set.hkFailAny': 'couldn’t set it',
      'set.hkBusy': '{k} is used by another program, so it can’t open the window. Pick another shortcut.',
      'set.autoFail': 'Couldn’t change startup with Windows',
      'set.procPh': 'Process name, for example game.exe', 'set.procLabel': 'Process name',
      'set.procLoading': 'Loading the list of running programs…', 'set.procAddHint': 'Press “Add” to add “{p}”.',
      'set.procNone': 'No running programs found.', 'set.procTitle': 'Pause while a process runs',
      'set.procText': 'Pick a running program or type an .exe name.', 'set.add': 'Add',
      'set.scanning': 'Scanning…', 'set.scanDone': 'Done', 'set.scanNew': 'Done, new files: {n}', 'set.scanFail': 'Couldn’t start',
      'set.logFail': 'Couldn’t open the log',
      'set.rebuildQ': 'Rebuild the index?', 'set.rebuildText': 'The current index will be deleted and built from scratch.',
      'set.rebuildGo': 'Rebuild', 'set.rebuildOk': 'The index is being rebuilt', 'set.rebuildFail': 'Couldn’t start the rebuild',
      'set.failedTitle': 'Couldn’t read', 'set.failedLog': 'The details are in the log.', 'set.loadingDots': 'Loading…',
      'set.failedCount': '{files}{shown}. The details are in the log.', 'set.failedShown': ', showing the first {n}',
      'set.failedListFail': 'Couldn’t get the list.'
    }
  };

  /* ---------- выбор языка ---------- */
  const KEY = 'semsearch.lang';
  const boot = window.__SEMSEARCH_BOOT || {};
  let pref = 'system', sys = '';
  const navSys = () => (/^ru\b/i.test(navigator.language || '') ? 'ru' : 'en');
  const resolve = () => (pref === 'ru' || pref === 'en' ? pref : sys === 'ru' || sys === 'en' ? sys : navSys());
  try {
    const c = JSON.parse(localStorage.getItem(KEY) || 'null');
    if (c && typeof c === 'object') { pref = c.pref || pref; sys = c.sys || sys; }
  } catch (e) { /* нет хранилища */ }
  if (boot.lang && !(function () { try { return localStorage.getItem(KEY); } catch (e) { return null; } })()) pref = boot.lang;
  if (boot.sys) sys = boot.sys;
  let cur = resolve();
  const listeners = [];

  SS.lang = () => cur;
  SS.langPref = () => pref;
  SS.langSys = () => sys || navSys();
  SS.onLang = (cb) => listeners.push(cb);

  const fill = (s, vars) => (vars ? String(s).replace(/\{(\w+)\}/g, (m, k) => (vars[k] !== undefined && vars[k] !== null ? vars[k] : m)) : s);
  const raw = (key) => {
    const d = D[cur] || D.ru;
    return d[key] !== undefined ? d[key] : D.ru[key] !== undefined ? D.ru[key] : key;
  };
  /* форма числа: ru — [1, 2–4, 5+], en — [1, другое] */
  const form = (forms, n) => {
    if (!Array.isArray(forms)) return forms;
    n = Math.abs(Math.floor(n || 0));
    if (cur === 'en' || forms.length === 2) return forms[n === 1 ? 0 : 1];
    const a = n % 10, b = n % 100;
    return forms[a === 1 && b !== 11 ? 0 : a >= 2 && a <= 4 && (b < 10 || b >= 20) ? 1 : 2];
  };
  SS.t = (key, vars) => fill(form(raw(key), 1), vars);
  /* SS.tn('meta.files', 5) -> «5 файлов»; {n} — число в формате языка, остальные — из vars */
  SS.tn = (key, n, vars) => fill(form(raw(key), n), Object.assign({ n: SS.num ? SS.num(n) : String(n) }, vars || {}));

  /* разметка: data-i18n, data-i18n-attr="attr:key;attr2:key2", <title data-i18n> */
  SS.i18n = (root) => {
    root = root || document;
    document.documentElement.setAttribute('lang', cur);
    root.querySelectorAll('[data-i18n]').forEach((el) => { el.textContent = SS.t(el.getAttribute('data-i18n')); });
    root.querySelectorAll('[data-i18n-attr]').forEach((el) => {
      el.getAttribute('data-i18n-attr').split(';').forEach((p) => {
        const i = p.indexOf(':');
        if (i > 0) el.setAttribute(p.slice(0, i).trim(), SS.t(p.slice(i + 1).trim()));
      });
    });
  };

  /* setLang(pref, sys): pref — настройка (system | ru | en), sys — язык Windows по данным backend.
     Сохраняет выбор для других окон; перерисовывает разметку и зовёт подписчиков, если язык сменился. */
  SS.setLang = (p, s) => {
    if (p) pref = p;
    if (s === 'ru' || s === 'en') sys = s;
    try { localStorage.setItem(KEY, JSON.stringify({ pref, sys })); } catch (e) { /* нет хранилища */ }
    apply();
  };
  function apply() {
    const next = resolve();
    if (next === cur && document.documentElement.getAttribute('lang') === cur) return;
    cur = next;
    SS.i18n();
    listeners.forEach((cb) => { try { cb(cur); } catch (e) { /* окно перерисует при следующем опросе */ } });
  }
  window.addEventListener('storage', (e) => {
    if (e.key !== KEY || !e.newValue) return;
    try {
      const c = JSON.parse(e.newValue);
      pref = c.pref || pref;
      sys = c.sys || sys;
      apply();
    } catch (x) { /* мусор */ }
  });
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', () => SS.i18n());
  else SS.i18n();
})();
