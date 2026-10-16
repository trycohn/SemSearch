<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="brand/logo/svg/horizontal-white.svg">
    <img src="brand/logo/svg/horizontal-blue.svg" alt="SemSearch" width="340">
  </picture>
</p>

<h3 align="center">Search your computer by meaning and by name</h3>

<p align="center">
  Describe a file in your own words and SemSearch finds the document, photo, video frame or piece of code,
  even when those words are not in its name. Everything runs on your computer.
</p>

<p align="center">
  <img alt="Windows 10 · 11" src="https://img.shields.io/badge/Windows-10%20%C2%B7%2011-005FB8?style=flat-square">
  <img alt="Python 3.11+" src="https://img.shields.io/badge/Python-3.11%2B-005FB8?style=flat-square&logo=python&logoColor=white">
  <img alt="Rust · Tauri 2" src="https://img.shields.io/badge/Rust-Tauri%202-005FB8?style=flat-square&logo=rust&logoColor=white">
  <img alt="Model: EmbeddingGemma 2" src="https://img.shields.io/badge/model-EmbeddingGemma%202-005FB8?style=flat-square">
  <a href="LICENSE"><img alt="License: Apache-2.0" src="https://img.shields.io/badge/license-Apache--2.0-005FB8?style=flat-square"></a>
  <img alt="Works offline" src="https://img.shields.io/badge/works-offline-1A1A1A?style=flat-square">
</p>

<p align="center">
  <a href="#features">Features</a> ·
  <a href="#installation">Installation</a> ·
  <a href="#usage">Usage</a> ·
  <a href="#how-it-works">How it works</a> ·
  <a href="#troubleshooting">Troubleshooting</a> ·
  <a href="#development">Development</a>
</p>

<p align="center"><a href="README.md">Русский</a> · <b>English</b></p>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/clip-en-dark.webp">
  <img src="docs/images/clip-en-light.webp" alt="Typing roof repair contract, cat on the balcony, where I parsed JSON with retries and estimate into the SemSearch window finds documents, photos, code and files by name">
</picture>

## Features

- **By meaning, not by words** — "roof repair contract" finds "Roof repair contract No. 14.pdf" and shows the page that matched.
- **Photos and video** — by a description or by an example picture; in a video, down to the second.
- **Code** — finds the function you mean; third-party libraries and build output stay out of the way.
- **Names** — every file on every drive in milliseconds, like Everything.
- **Stays out of your games** — gives the GPU back and pauses while a game runs full screen.
- **Offline** — files and queries never leave your computer.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/speed-en-dark.png">
  <img src="docs/images/speed-en-light.png" alt="Search by name over the whole C: drive: SemSearch 13 ms, Windows Search (Explorer) 3 min 50 s. Search by meaning: SemSearch 102 ms, Windows Search cannot do it">
</picture>

<details>
<summary><b>How it was measured</b></summary>
<br>

The script [`tools/bench_search.py`](tools/bench_search.py): 5 queries by a part of the name and 5 phrases, each run
10 times after a warm-up; the table shows medians. Timed is the wait from sending the query to having every result.
Windows Search was asked through the same index Explorer uses. The chart uses a log scale.

| | SemSearch | Windows Search |
|---|---|---|
| By name over the whole C: drive, first 200 files | 13 ms — its own name index of the whole drive (3.5 million files). Together with the search by meaning that the window runs for every query, 148 ms | 3 min 50 s — a walk of the drive (`dir /s`), as Explorer does outside its index. Windows has its own index only for `C:\Users` and the Start menu by default: 14 ms there |
| By meaning, a phrase | 102 ms; 21–54 files found per phrase | Cannot. Its search by words (FREETEXT) answers in 4.8 ms but found no file at all for 2 of 5 phrases |

Computer: Intel Core i5-13600KF, NVIDIA GeForce RTX 4070 SUPER, SSD; October 16, 2026. To repeat it (SemSearch running
and not indexing): `.venv\Scripts\python.exe tools\bench_search.py --walk`, then
`.venv\Scripts\python.exe tools\make_readme_images.py speed` redraws the chart.

</details>

## Installation

1. Download `SemSearch-Setup-<version>.exe` from [Releases](../../releases) and run it. SemSearch installs for the
   current user (into `%LOCALAPPDATA%\Programs\SemSearch` by default); no administrator rights are needed.
2. Windows asks for permission only for Visual C++ (when it is missing) and for fast search by name, a Task Scheduler
   task that reads the MFT. You can say no to it: names are then found by an ordinary walk, slower.
3. The model (~1.5 GB) and PyTorch (with CUDA when there is an NVIDIA GPU, the CPU build otherwise) are downloaded
   during setup, so it needs the internet; if the connection drops, press Retry.

The installer is not signed yet, so SmartScreen may warn about it: More info → Run anyway. A new version installs over
the old one with the same installer, keeping settings, index and model. SemSearch is removed in Apps or Add or remove
programs; removal can also wipe the settings, the index and the model.

| Requirements | |
|---|---|
| OS | Windows 11 or Windows 10 (version 1709 and later), 64-bit. macOS and Linux are not supported |
| GPU | NVIDIA RTX 20xx or newer, optional. Without one (or with an older card) the model runs on the CPU, slower |
| WebView2 Runtime | Built into Windows 11; on Windows 10 it usually comes with Edge; if it is missing, the installer offers to add it |
| Disk space | ~7 GB with an NVIDIA GPU, ~3.5 GB without: the Python environment, the model ~1.5 GB and the index |

<details>
<summary><b>Building from source</b></summary>
<br>

You need Python 3.11 or later, [Rust](https://rustup.rs) and Visual Studio Build Tools with the "Desktop development
with C++" workload.

1. The Python environment — PyTorch with CUDA when an NVIDIA driver is present, the CPU build otherwise:
   ```bat
   setup.bat
   ```
2. `SemSearch.exe` and the name engine `bin\semsearch-engine.exe`:
   ```bat
   build.cmd
   ```
3. Run `SemSearch.exe`. On the first start the model (~1.5 GB) is downloaded and the launch screen shows how much is
   left. Search by name works right away.

Without `build.cmd`, after `setup.bat` you can run `start.bat`: the search opens in the browser, the hotkey opens it
there too, there is no tray icon, and names are found by a built-in walk, slower. `build-installer.cmd` builds your own
installer (see Development).

</details>

<details>
<summary><b>Fast name search through the MFT (administrator rights)</b></summary>
<br>

Reading the MFT and the USN journal needs administrator rights. The "SemSearch Engine" Task Scheduler task gives the
engine those at sign-in without a UAC prompt. The installer sets it up when you agree; when building from source:

```bat
engine\install-task.cmd
```

The script asks for UAC (an administrator account is needed) and copies the engine to `%ProgramFiles%\SemSearch\`,
where it cannot be replaced without administrator rights. The engine's log is `%ProgramFiles%\SemSearch\logs\engine.log`;
from the data folder it reads only the list of drives (`config.json`). Run the script again after a rebuild to update
the copy. To remove the task and the copy: `engine\uninstall-task.cmd`.

Without the task the backend starts `bin\semsearch-engine.exe` itself, without rights: a drive walk plus
`ReadDirectoryChangesW`. The name index takes longer to build; everything else is the same. Non-NTFS drives
(exFAT, FAT32, ReFS) are always indexed this way.

</details>

## Usage

Press <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>S</kbd> and describe what you are looking for: "roof repair contract",
"screenshot with a CUDA error", "where I parsed JSON with retries".

| Keys | What they do |
|---|---|
| <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>S</kbd> | Show or hide the window from any app. Where AltGr+S types a letter in your layouts (Polish, Czech, Hungarian…), the first start picks <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Space</kbd> instead. Can be changed in Settings |
| <kbd>Esc</kbd> | Hide the window |
| <kbd>↑</kbd> <kbd>↓</kbd> | Pick a result |
| <kbd>Enter</kbd> | Open the file |
| <kbd>Ctrl</kbd>+<kbd>Enter</kbd> | Show it in its folder |
| <kbd>Ctrl</kbd>+<kbd>1</kbd> … <kbd>7</kbd> | Tabs: all, documents, code, photos, video, audio, by name |
| <kbd>Ctrl</kbd>+<kbd>F</kbd> | Find similar photos and frames |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> | Copy the path |
| <kbd>Ctrl</kbd>+<kbd>V</kbd> | Search by the picture on the clipboard (or drop one on the window) |

Language (English, Russian or "Same as Windows"), theme, autostart, folders, drives, exclusions and pauses are in Settings:
the button at the bottom of the window or the tray menu.

## How it works

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/architecture-en-dark.png">
  <img src="docs/images/architecture-en-light.png" alt="Diagram: SemSearch.exe, the Python backend and the Rust name engine run on your computer; only the model is downloaded once">
</picture>

| Part | What it does |
|---|---|
| `SemSearch.exe` (Rust, Tauri 2; source in `shell/`) | Frameless windows, tray, hotkey, autostart. Starts the backend and restarts it if it crashes |
| `server.py` → `app.py` (Python) | The EmbeddingGemma 2 model (on the GPU when it is supported), the content index, the API for the interface |
| `ui/` | The interface, 1:1 with the "SemSearch Desktop" mockup (its source is in `design/`). File-type icons are in `ui/filetypes/` |
| Name engine (Rust; source in `engine/`) | Names of every file on the drives: MFT + USN journal or a folder walk. Search by name in milliseconds and a stream of file changes |
| Installer (Rust, Tauri 2; source in `installer/`) | Installs Python, PyTorch, the model, the engine task and shortcuts for the current user; updates and removes the program. Details in [`docs/INSTALLER.md`](docs/INSTALLER.md) (in Russian) |

The contracts between the parts are in [`docs/API.md`](docs/API.md) (in Russian).

<details>
<summary><b>What gets indexed, and in what order</b></summary>
<br>

- **By meaning**: the user folder and the known folders outside it (for example Documents moved to another drive),
  except AppData, dot folders and `node_modules`. OneDrive files that are on the disk are indexed; cloud-only ones are
  not downloaded, but are found by name. Order: documents → code → photos → audio → video.
- **By name**: all fixed drives; the list can be changed in Settings.
- **Builds and dependencies** are recognised by the project files next to them, not by a folder name: `bin`/`obj` next to a
  `.csproj`, `Library` in a Unity project, `Saved`/`Intermediate` next to a `.uproject`, a folder with `pyvenv.cfg` and so on.
  A plain "Library" folder full of books is still searched.
- **Third-party code** (the "Third-party code and libraries" setting): git submodules, downloaded source archives (`*-main`,
  `*-master`, `*-1.2.3`), whatever the project's `.gitignore` says is not source, and generated files are skipped by default.
  Clones of other people's repositories without your commits and files with a library header go last. This applies only to
  code and `.md`/`.rst`.
- **Scanned PDFs**: a page with almost no text goes to the model as a picture, so a scanned contract is found too.
- **Changes** arrive through the change journal within seconds: a file is picked up once nothing has written to it for
  3 seconds. Every 6 hours a full safety pass runs.
- **External and slow drives**: HDDs, USB sticks and network drives are read with fewer threads. An external drive can
  be safely removed — SemSearch lets go of it by itself and picks it up again once it is back; the index of its contents
  is kept meanwhile.

| Type | Extensions |
|---|---|
| Documents | PDF, DOCX, TXT, MD, RST, ORG, TEX |
| Code | 45 extensions: Python, JS/TS, C#, C/C++, Java, Kotlin, Go, Rust, PHP, Lua, SQL, shaders, YAML, TOML… |
| Photos | JPG, PNG, WEBP, BMP, GIF, TIFF |
| Audio | MP3, WAV, FLAC, OGG, M4A, OPUS, AAC, WMA |
| Video | MP4, MKV, AVI, MOV, WEBM, WMV, M4V |

</details>

<details>
<summary><b>How SemSearch gives the GPU back</b></summary>
<br>

The load adjusts itself (`throttle.py`): work is cut into 1–5 ms slices so the frames of games and of the desktop get
through between them. While you are at the computer it runs gently, with a heavy game in a window it is strongly limited,
and while you are away it runs at full speed (but not on a laptop on battery or in battery saver). Fullscreen games and
movies, and the processes listed in Settings, pause indexing.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/settings-en-dark.png">
  <img src="docs/images/settings-en-light.png" alt="Settings: pausing for full-screen apps and chosen programs, video and audio segments, index state">
</picture>

The text part of the model runs as CUDA graphs, one per layer (`cudagraphs.py`): a slice reaches the GPU as one call
instead of ~50 small ones and gets 1.5–2.5 times more work done in the same time.

The slices adapt to the GPU: smaller on a slow one, sized by free video memory on a card with 2–4 GB. If torch cannot
use your GPU (the CUDA 12.8 build supports RTX 20xx and newer, not GTX 10xx), the driver is too old, or even the model
does not fit in video memory, it runs on the CPU; the reason is written to `semsearch.log`.

</details>

### Where the data lives

`%LOCALAPPDATA%\SemSearch`: settings (`config.json`), indexes, previews, logs and the model (`models\`, ~1.5 GB). The
program folder is only read while running, so it can be put anywhere, `Program Files` included.

- To use another data folder, set the `SEMSEARCH_DATA_DIR` environment variable (an absolute path). Make it a permanent
  user variable so the engine started by the Task Scheduler task sees it too.
- `data\` and `config.json` left next to the program by an old version are moved there on the first start.
- A model that older versions downloaded into the shared Hugging Face cache (`%USERPROFILE%\.cache\huggingface\hub`)
  is moved there on the first start too, so it is not downloaded again. A cache of your own (`HF_HUB_CACHE`) is left alone.

## Troubleshooting

The logs are in `%LOCALAPPDATA%\SemSearch`: `semsearch.log` (backend), `shell.log` (window and tray), `engine.log`
(the engine without the task). The log of the engine run by the task is `%ProgramFiles%\SemSearch\logs\engine.log`.

<details>
<summary><b>A file is not found</b></summary>
<br>

Check that its folder is under "Search by meaning" and is not excluded (Settings → Where to search). The Index section
there lists the files that could not be read and has a "Rebuild index…" button.

</details>

<details>
<summary><b>The hotkey types a letter or does not open the window</b></summary>
<br>

<kbd>Ctrl</kbd>+<kbd>Alt</kbd>+letter is <kbd>AltGr</kbd>+letter, which types letters in Polish, Czech, Hungarian and other
layouts. SemSearch takes the layouts installed at its first start into account; if a layout was added later and the hotkey
gets in the way, change it in Settings. If another program has taken the shortcut, Settings say so.

</details>

<details>
<summary><b>SemSearch does not start</b></summary>
<br>

- No WebView2 Runtime: SemSearch says so at start and opens the download page.
- The search server fails with code 0xC0000135: the Microsoft Visual C++ Redistributable (x64) that PyTorch needs is
  missing. The installer adds it by itself.

</details>

<details>
<summary><b>Port 8765 is taken by another program</b></summary>
<br>

AnkiConnect, for example. SemSearch picks a free port nearby by itself; you can set your own in `config.json` (`"port"`).

</details>

<details>
<summary><b>Indexing runs on the CPU although there is a GPU</b></summary>
<br>

The reason is in `semsearch.log`. Usually the card is older than RTX 20xx (the CUDA 12.8 build of torch does not support
it), the NVIDIA driver is too old (the log then says it should be updated), or there is not enough video memory.

</details>

## Development

- `setup.bat` sets up the Python environment; versions are pinned in `requirements-torch.txt` (torch, torchvision) and
  `requirements.txt`.
- `build.cmd` makes release builds of the engine (`engine\`) and the shell (`shell\`) and puts `bin\semsearch-engine.exe`
  and `SemSearch.exe` in the root. It needs Rust (the minimum version is `rust-version` in `Cargo.toml`) and Visual Studio
  Build Tools (MSVC and the Windows SDK). The engine and the shell are linked with the static CRT, so neither
  `SemSearch.exe` nor the engine needs the VC++ Redistributable.
- `installer\` and `build-installer.cmd` build the installer `dist\SemSearch-Setup-<version>.exe`; what it does and which
  files it ships (`installer\payload.txt`) is in [`docs/INSTALLER.md`](docs/INSTALLER.md). `tools\fetch_model.py` is the
  installer step that downloads the model; by hand: `.venv\Scripts\python.exe tools\fetch_model.py`.
- Signing the installer, `SemSearch.exe` and the engine is optional; without it the build is unsigned. The certificate
  is chosen by `SEMSEARCH_SIGN` with signtool arguments: a file (`/f C:\keys\semsearch.pfx /p <password>`), the Windows
  store or a token (`/sha1 <thumbprint>`), or Azure Trusted Signing (`/dlib <…\Azure.CodeSigning.Dlib.dll> /dmdf <…\metadata.json>`).
  Also: `SEMSEARCH_SIGNTOOL` (path to signtool.exe, from Windows Kits 10 by default), `SEMSEARCH_SIGN_TS` (timestamp
  server, DigiCert by default), `SEMSEARCH_SIGN_TEST=1` (a self-signed certificate to test the build).
- `mock_api.py` serves the mockup's data for interface work (port 8799). Launch screen: `/dev/scenario/boot`,
  `firstrun`, `cpu`, `booterror`, `bootnet`, `hold-download`, `hold-model`; language: `/dev/lang/<system|ru|en>` (in English the demo files are English too).
- `tools\make_readme_images.py` makes the pictures of both READMEs (`docs\images\`): interface screenshots and the clip
  on the `mock_api.py` data and the diagram, in English and Russian. It needs Edge or Chrome; rerun it when the interface changes.
- `tools\bench_search.py` measures search speed against Windows Search for the chart in this README.
- `tools\make_filetype_icons.py` makes the file-type icons (`ui\filetypes\`); run it only when the icons or the list of
  extensions change.
- `tools\frameprobe` (Rust) and `tools\gpu_bench.py` measure frame pacing under indexing load.
- Environment variables (`SEMSEARCH_DATA_DIR`, `SEMSEARCH_PORT`, `SEMSEARCH_ROOT`, `SEMSEARCH_DEV_URL`,
  `SEMSEARCH_NO_BACKEND`) and the contracts between the parts are in [`docs/API.md`](docs/API.md).
- The logo, icons and colours are in [`brand/`](brand/README.md).

## License

SemSearch is licensed under [Apache-2.0](LICENSE): you may use, change and redistribute the code, in commercial products
too, as long as you include the license text, keep the attribution notices and mark your changes. The license grants no
rights to the SemSearch name or logo.

- The file-type icons in `ui/filetypes/` are based on the `@pierre/trees` set (also Apache-2.0, see
  [`ui/filetypes/LICENSE`](ui/filetypes/LICENSE)).
- The EmbeddingGemma 2 model is not part of the repository: it is downloaded from Hugging Face during setup (on the
  first start when built from source) and comes under Google's terms; see the model page.
