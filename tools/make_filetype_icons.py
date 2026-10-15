"""Builds ui/filetypes/* (file-type icons): 49 from the icon-set JSON that ships inside Claude desktop, 6 drawn here.

One-off conversion: the result is committed, so nobody needs the source to run SemSearch.
The JSON is the web bundle of Claude desktop, file assets/v1/cc84ca73d-*.json under
<install dir>\\resources\\ion-dist (WindowsApps\\Claude_*\\app\\resources\\ion-dist\\assets\\v1).
Its "source" field names the origin: @pierre/trees 1.0.0-beta.3 (Apache-2.0, Pierre Computer Company).
pdf, slides, word and markdown are not taken from it (there they are recoloured or redrawn copies of the plain
text/default icons), audio and video it does not have at all: OWN below draws them, in the same duotone style, on the
same 16x16 grid (audio as an object like the set's zip and image, the rest as signs on a sheet).
audio and video borrow the colours of the set's "javascript" (yellow) and "css" (violet).

Run: .venv\\Scripts\\python.exe tools\\make_filetype_icons.py <path to cc84ca73d-*.json>
Writes per theme ui/filetypes/<light|dark>/<icon>.svg (16x16, colours of that theme baked in; no currentColor),
ui/filetypes/filetypes.json (extension / file name -> icon), ui/filetypes/LICENSE and what the app itself loads:
ui/filetypes/filetypes.js (the same table for the page) and ui/filetypes/filetypes.css (classes .ft-<icon>: both
themes of every icon inside the stylesheet as data: URIs, so rows show them at once and the theme switches in CSS).
"""
import json
import sys
from html import escape
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "ui" / "filetypes"
THEMES = ("light", "dark")  # palette values are [light, dark]


def nodes_to_svg(nodes, palette, theme: int) -> str:
    out = []
    for tag, attrs, *rest in nodes:
        parts = []
        for name, value in attrs.items():
            if value.startswith("$"):
                value = palette[value[1:]][theme]
            elif value == "currentColor":  # font, stylelint: Claude desktop paints them in the page text colour
                value = palette["text"][theme]  # a file in <img> has no page, so currentColor would be black
            parts.append(f' {name}="{escape(value)}"')
        inner = nodes_to_svg(rest[0], palette, theme) if rest else ""
        out.append(f"<{tag}{''.join(parts)}>{inner}</{tag}>")
    return "".join(out)


# ---- Own icons -------------------------------------------------------------------------------
# Style of the set: a sheet with a folded corner in 40% colour, the corner and the sign on it in solid colour.
# The sheet is 12x14 at x 2..14, y 1..15 so that our icons stand in one row with the others. The corner is a
# separate flap (x 9..14, y 1..6); the body is cut round it by a concentric arc one unit wider.
# The signs are layouts of the page: rounded blocks and 1px lines with round ends, as in the set's own "text".

def _page(c):
    return (f'<path fill="{c}" opacity=".4" d="M4.5 1H8v3A3 3 0 0 0 11 7H14v5.5A2.5 2.5 0 0 1 11.5 15h-7A2.5 2.5 0 0 1 2 12.5v-9A2.5 2.5 0 0 1 4.5 1z"/>'
            f'<path fill="{c}" d="M9.5 1 13.85 5.15Q14 5.3 14 5.9V6H11A2 2 0 0 1 9 4V1.5Q9 1 9.5 1z"/>')


def _rrect(x, y, w, h, r):
    n = lambda v: f"{v:g}"  # no float noise like 6.199999999999999 in the files
    return (f"M{n(x + r)} {n(y)}H{n(x + w - r)}A{n(r)} {n(r)} 0 0 1 {n(x + w)} {n(y + r)}V{n(y + h - r)}A{n(r)} {n(r)} 0 0 1 {n(x + w - r)} {n(y + h)}"
            f"H{n(x + r)}A{n(r)} {n(r)} 0 0 1 {n(x)} {n(y + h - r)}V{n(y + r)}A{n(r)} {n(r)} 0 0 1 {n(x + r)} {n(y)}z")


def _block(c, x, y, w, h, r):
    return f'<path fill="{c}" d="{_rrect(x, y, w, h, r)}"/>'


def _bar(c, x, y, w):  # a line of text, 1px
    return f'<rect fill="{c}" x="{x}" y="{y}" width="{w}" height="1" rx=".5"/>'


def _dot(c, cx, cy):
    return f'<circle fill="{c}" cx="{cx}" cy="{cy}" r=".65"/>'


def _pdf(c):  # a heading and a paragraph
    return _page(c) + _block(c, 4, 8.25, 6, 1.5, .75) + _bar(c, 4, 10.75, 8) + _bar(c, 4, 12.5, 5.5)


def _word(c):  # a printed page: a wide block (picture) and two columns under it
    return _page(c) + _block(c, 4, 8.5, 8, 2.5, .7) + _bar(c, 4, 12.25, 3.5) + _bar(c, 8.5, 12.25, 3.5)


def _markdown(c):  # a bullet list
    return (_page(c) + _dot(c, 4.6, 9) + _bar(c, 6.2, 8.5, 5.8) + _dot(c, 4.6, 10.9) + _bar(c, 6.2, 10.4, 4.5)
            + _dot(c, 4.6, 12.75) + _bar(c, 6.2, 12.25, 5.3))


def _slides(c):  # the slide panel of a presentation program: three thumbnails and the slide
    return (_page(c) + _block(c, 4, 8.3, 2, 1.3, .5) + _block(c, 4, 10.2, 2, 1.3, .5) + _block(c, 4, 12.1, 2, 1.3, .5)
            + _block(c, 7, 8.3, 5, 5.1, .9))


def _audio(c):  # a waveform, an object like the set's zip and image: seven 1px bars with round ends
    # x and the heights are whole numbers, so every bar is exactly one pixel column at 16px (at 1.2px it smeared into a blob)
    return ''.join(f'<rect fill="{c}" x="{1 + 2 * i}" y="{8 - h // 2}" width="1" height="{h}" rx=".5"/>'
                   for i, h in enumerate((4, 8, 12, 6, 10, 6, 4)))


def _video(c):  # a video camera on a sheet: an outline body with a soft screen, a solid lens
    screen = _rrect(5.1, 9.3, 3.8, 2.6, .5)
    lens = "M10.5 10.35Q10.5 9.9 10.874 9.65L11.926 8.95Q12.3 8.7 12.3 9.15L12.3 12.45Q12.3 12.9 11.926 12.65L10.874 11.95Q10.5 11.7 10.5 11.25z"
    return (_page(c) + f'<path fill="{c}" fill-rule="evenodd" d="{_rrect(4, 8.2, 6, 4.8, 1.2)}{screen}"/>'
            f'<path fill="{c}" opacity=".3" d="{screen}"/><path fill="{c}" d="{lens}"/>')


def data_uri(svg):
    """An SVG as a CSS url("data:..."): quotes inside become single, what CSS and URLs dislike is escaped."""
    s = svg.strip().replace("\n", "").replace('"', "'")
    for char, code in (("%", "%25"), ("#", "%23"), ("<", "%3C"), (">", "%3E")):
        s = s.replace(char, code)
    return 'url("data:image/svg+xml,' + s + '")'


def build_css(svgs):
    """svgs: {theme: {icon: svg}}. One class per icon; the dark variant wins under .dark (on <html> or on .win)."""
    lines = ["/* Создано tools/make_filetype_icons.py — вручную не править.",
             '   Значок типа файла: <span class="ft ft-pdf" aria-hidden="true"></span>; размер задаёт место, где он стоит.',
             "   Тёмная тема — класс dark на <html> или на .win (его ставит SS.applyTheme). */",
             ".ft{display:block;flex:none;background-position:50% 50%;background-repeat:no-repeat;background-size:contain}"]
    for name in sorted(svgs["light"]):
        lines.append(f".ft-{name}{{background-image:{data_uri(svgs['light'][name])}}}")
        lines.append(f".dark .ft-{name}{{background-image:{data_uri(svgs['dark'][name])}}}")
    return "\n".join(lines) + "\n"


def build_js(table):
    return ("/* Создано tools/make_filetype_icons.py — вручную не править. Таблица «имя / расширение → значок» для SS.ftName (common.js). */\n"
            "window.SS_FILETYPES = " + json.dumps(table, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + ";\n")


COLOUR_OF = {"audio": "javascript", "video": "css"}  # palette entries of the source set, for icons it does not have
EXTENSIONS = {  # the same lists as the app indexes (AUDIO_EXT and VIDEO_EXT in core.py)
    "audio": "mp3 m4a wav flac ogg opus aac wma",
    "video": "mp4 mkv avi mov webm wmv m4v",
}
OWN = {"markdown": _markdown, "word": _word, "slides": _slides, "pdf": _pdf, "audio": _audio, "video": _video}


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    src = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    for key in ("defaultIcon", "palette", "fileNames", "fileExtensions", "icons", "license"):
        if key not in src:
            sys.exit(f"Not an icon-set file: no '{key}'")

    svgs = {theme: {} for theme in THEMES}
    for t, theme in enumerate(THEMES):
        folder = OUT / theme
        folder.mkdir(parents=True, exist_ok=True)
        for name in sorted(set(src["icons"]) | set(OWN)):
            if name in OWN:
                body = OWN[name](src["palette"][COLOUR_OF.get(name, name)][t])
            else:
                body = nodes_to_svg(json.loads(src["icons"][name]), src["palette"], t)
            svg = f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="16" height="16" fill="none">{body}</svg>\n'
            (folder / f"{name}.svg").write_text(svg, encoding="utf-8", newline="\n")
            svgs[theme][name] = svg

    # Keys are lower-case; a file name is matched whole first, then each dotted tail
    # of it ("a.b.env" -> "b.env", "env"), as the Claude desktop does.
    table = {k: src[k] for k in ("defaultIcon", "fileNames", "fileExtensions")}
    for icon, exts in EXTENSIONS.items():
        for ext in exts.split():
            assert ext not in table["fileExtensions"], ext
            table["fileExtensions"][ext] = icon
    (OUT / "filetypes.json").write_text(
        json.dumps(table, ensure_ascii=False, indent=1, sort_keys=True) + "\n", encoding="utf-8", newline="\n")
    used = set(table["fileNames"].values()) | set(table["fileExtensions"].values()) | {table["defaultIcon"]}
    assert used <= set(svgs["light"]), used - set(svgs["light"])
    (OUT / "filetypes.js").write_text(build_js(table), encoding="utf-8", newline="\n")
    (OUT / "filetypes.css").write_text(build_css(svgs), encoding="utf-8", newline="\n")
    (OUT / "LICENSE").write_text(src["license"].rstrip() + "\n", encoding="utf-8", newline="\n")
    print(f"{len(set(src['icons']) | set(OWN))} icons x {len(THEMES)} themes, "
          f"{len(table['fileExtensions'])} extensions, {len(table['fileNames'])} file names -> {OUT}")


if __name__ == "__main__":
    main()
