# Иконки типов файлов

55 иконок 16×16 для 141 расширения и 100 имён файлов (`Dockerfile`, `.gitignore`, `vite.config.ts`, `.prettierrc` и т. п.).
Те же иконки показывает дерево файлов в Claude Code Desktop.

- `light/<имя>.svg`, `dark/<имя>.svg` — один и тот же рисунок с цветами светлой и тёмной темы, цвета записаны в файле.
  Работают в `<img>`, в CSS `background` и отдельно от страницы.
- `filetypes.json` — какая иконка какому файлу:
  - `fileNames` — имя файла целиком, в нижнем регистре (`dockerfile`, `.gitignore`);
  - `fileExtensions` — расширение без точки (`py`, `md`, а также составные вроде `env.local`);
  - `defaultIcon` — когда ничего не подошло (`default`).
- `filetypes.js` и `filetypes.css` — то, что загружает сама программа; получаются из тех же данных скриптом, вручную не правятся:
  - `filetypes.js` — та же таблица, что в `filetypes.json`, как `window.SS_FILETYPES`;
  - `filetypes.css` — классы `.ft` и `.ft-<значок>`: обе темы каждого значка лежат прямо в стилях (`data:`), поэтому значки
    появляются сразу, без запросов, а тему переключает CSS (класс `dark` на `<html>` или `.win`).
- `LICENSE` — Apache-2.0 (для 49 из 55 иконок, см. «Откуда и под какой лицензией»).

## Как найти иконку

Имя приводится к нижнему регистру. Сначала ищется целое имя в `fileNames`, потом — каждый хвост после очередной точки
в `fileExtensions`: у `archive.tar.gz` сначала проверяется `tar.gz`, потом `gz` (→ `zip`); у `a.env.local` находится `env.local` (→ `text`).
Не нашлось — `defaultIcon`, серая стандартная иконка без рисунка на листе.

```js
function fileTypeIcon(map, name) {
  const n = name.toLowerCase();
  if (Object.hasOwn(map.fileNames, n)) return map.fileNames[n];
  for (let i = n.indexOf('.'); i !== -1; i = n.indexOf('.', i + 1)) {
    const tail = n.slice(i + 1);
    if (Object.hasOwn(map.fileExtensions, tail)) return map.fileExtensions[tail];
  }
  return map.defaultIcon;
}
// <img src="filetypes/dark/${fileTypeIcon(map, 'main.py')}.svg">
```

Чего в наборе нет: `c`/`cpp`/`java`/`kt`/`php`/`cs`/`lua`/`ps1`/`toml`, `epub`. Для них будет `default`.

## Как это подключено в программе

- `index.html` грузит `filetypes/filetypes.css` и `filetypes/filetypes.js`.
- `SS.ftName(имя файла)` (`common.js`) возвращает название значка по правилу выше. `R.fileIcon(имя, класс)` (`render.js`) делает
  `<span class="ft ft-pdf">`; размер задаёт место, где значок стоит: 36 px в строке результата, 24 px в списках имён, 56 px в панели просмотра.
- Значок берётся по имени файла, а не по виду («документ», «код»): `report.pdf`, `main.rs` и `Dockerfile` выглядят каждый по-своему.
- Расширения аудио и видео — те же, что индексирует программа (`AUDIO_EXT` и `VIDEO_EXT` в `core.py`).
- Новое расширение или значок: допишите его в `tools/make_filetype_icons.py` (`EXTENSIONS` или рисунок в `OWN`) и пересоберите.

## Откуда и под какой лицензией

**49 иконок** — из `@pierre/trees` 1.0.0-beta.3 (встроенный набор «complete»), Apache-2.0, Copyright 2025 Pierre Computer Company.
Claude desktop привёл их к своему формату, мы взяли оттуда. Под текст из `LICENSE` подпадают все иконки, кроме шести ниже.

Изменения относительно источника: рисунок сохранён отдельными SVG-файлами по темам, цвета из палитры набора подставлены
в файлы; там, где Claude desktop берёт цвет текста страницы (`font`, `stylelint`), записан серый цвет `text` из той же палитры.

**6 иконок — `pdf`, `slides`, `word`, `markdown`, `audio`, `video` — нарисованы для SemSearch** и к набору Pierre не относятся.
В Claude desktop на месте первых четырёх перекрашенные копии `text` и `default`, а аудио и видео там нет совсем.

- `pdf`, `word`, `markdown`, `slides`, `video` — свой знак из блоков и линий на листе с загнутым углом: у `pdf` заголовок и абзац,
  у `word` широкий блок и две колонки под ним, у `markdown` список, у `slides` колонка эскизов слева и слайд, у `video` видеокамера контуром.
  Силуэт листа повторяет остальной набор, чтобы иконки стояли в одном ряду.
- `audio` — отдельный предмет без листа, как `zip` и `image`: звуковая волна из семи столбиков.
  Цвета аудио и видео взяты из палитры набора (`javascript` — жёлтый, `css` — фиолетовый).

Рисунок — функции `_pdf`, `_slides`, `_word`, `_markdown`, `_audio`, `_video` в `tools/make_filetype_icons.py`; они же дают файлы обеих тем.

## Как пересобрать

Нужен JSON набора из Claude desktop: `…\WindowsApps\Claude_*\app\resources\ion-dist\assets\v1\cc84ca73d-*.json`
(хеш в имени меняется между версиями — ищите файл с ключами `fileExtensions` и `icons`).

```
python tools\make_filetype_icons.py <путь к JSON>
```
