'use strict';
/* SemSearch launch film, 1920x1080, 30 s at 120 BPM (a beat is 0.5 s, a bar is 2 s).
   One shape is never cut: the blue dot of the logo becomes the search field, the results panel, a tile in a wall
   of files, the fact pills, and the dot of the logo again. Every style is set from t inside seek(t): no CSS
   transitions, no timers, no state between frames. Springs are closed-form step responses, and a value that
   changes target many times is the sum of one spring per change (Track). */

const Q = new URLSearchParams(location.search);
const LANG = Q.get('lang') === 'en' ? 'en' : 'ru';
const DUR = 30;
const BLUE = '#005fb8', INK = '#1a1a1a', WHITE = '#ffffff';
const ICON = n => '../../ui/filetypes/light/' + n + '.svg';

/* ---------------------------------------------------------------- words */

const STR = {
  ru: {
    hook: 'Как назывался тот файл?',
    chips: [['pdf', 'Скан_0042.pdf'], ['image', 'IMG_20250612_183044.jpg'], ['word', 'Документ1 (3).docx'],
            ['video', 'VID_20240830_2015.mp4'], ['python', 'untitled_2.py'], ['text', 'Новый текстовый документ.txt'],
            ['image', 'IMG_4471.HEIC'], ['pdf', 'финал_v3_ФИНАЛ.pdf']],
    describe: 'Просто опишите, что в нём',
    ph: 'Что ищем? Опишите файл своими словами',
    q1: 'договор про крышу', q1mark: 'крышу',
    rows: [
      {ic: 'word', ti: 'Документ1 (3).docx', pa: 'Документы › Дом', da: '12 мар', tag: 'стр. 2',
       sn: ['…Подрядчик обязуется выполнить работы по ', 'замене кровельного покрытия', ' жилого дома, включая демонтаж старого шифера…']},
      {ic: 'table', ti: 'Смета_v3.xlsx', pa: 'Документы › Дом', da: '9 мар', tag: 'лист 1',
       sn: ['Металлочерепица 0,5 мм — 160 м² · водосточная система · монтаж под ключ', '', '']},
      {ic: 'text', ti: 'Переписка с прорабом.txt', pa: 'Рабочий стол', da: '2 апр', tag: 'фрагм. 1',
       sn: ['…по договору гарантия пять лет, если потечёт — сразу писать Андрею…', '', '']}],
    meaning: 'Нашёл по смыслу, а не по словам',
    q2: 'кот на балконе', q3: 'салют над рекой', q4: 'где парсил JSON с ретраями',
    capPhoto: 'Фото', capVideo: 'Кадры из видео', capCode: 'Код',
    photo: ['IMG_20250612_183044.jpg', 'Изображения › Телефон › 2025', 'JPEG · 4000 × 3000 · 3,2 МБ', '12 июня 2025, 18:30'],
    video: ['VID_20240830_2015.mp4', 'Видео › Телефон', 'MP4 · 1920 × 1080 · 4:12', 'Совпал кадр на 1:42'],
    code: ['untitled_2.py', 'Projects › scraper', 'Python · 7 КБ', 'Совпали строки 2–5'],
    open: 'Открыть', frame: 'кадр 1:42',
    odoSep: ' ', odoLabel: 'имён файлов на диске C:',
    speed: [['по имени', '13 мс'], ['по смыслу', '0,1 с']],
    facts: ['Работает без интернета', 'Бесплатно', 'Открытый код', 'Windows 10 · 11'],
    slogan: 'Поиск файлов своими словами',
  },
  en: {
    hook: 'What was that file called?',
    chips: [['pdf', 'Scan_0042.pdf'], ['image', 'IMG_20250612_183044.jpg'], ['word', 'Document1 (3).docx'],
            ['video', 'VID_20240830_2015.mp4'], ['python', 'untitled_2.py'], ['text', 'New Text Document.txt'],
            ['image', 'IMG_4471.HEIC'], ['pdf', 'final_v3_FINAL.pdf']],
    describe: 'Just describe what’s in it',
    ph: 'What are you looking for? Describe the file in your own words',
    q1: 'contract about the roof', q1mark: 'roof',
    rows: [
      {ic: 'word', ti: 'Document1 (3).docx', pa: 'Documents › Home', da: 'Mar 12', tag: 'p. 2',
       sn: ['…The contractor shall ', 'replace the shingles and flashing', ' on the house, including removing the old slate…']},
      {ic: 'table', ti: 'Estimate_v3.xlsx', pa: 'Documents › Home', da: 'Mar 9', tag: 'sheet 1',
       sn: ['Metal tiles 0.5 mm — 160 m² · gutter system · turnkey installation', '', '']},
      {ic: 'text', ti: 'Messages with the foreman.txt', pa: 'Desktop', da: 'Apr 2', tag: 'part 1',
       sn: ['…five-year warranty under the contract; if it leaks, write to Andrew right away…', '', '']}],
    meaning: 'Found by meaning, not by words',
    q2: 'cat on the balcony', q3: 'fireworks over the river', q4: 'where I parsed JSON with retries',
    capPhoto: 'Photos', capVideo: 'Video frames', capCode: 'Code',
    photo: ['IMG_20250612_183044.jpg', 'Pictures › Phone › 2025', 'JPEG · 4000 × 3000 · 3.2 MB', 'June 12, 2025, 6:30 PM'],
    video: ['VID_20240830_2015.mp4', 'Videos › Phone', 'MP4 · 1920 × 1080 · 4:12', 'Matching frame at 1:42'],
    code: ['untitled_2.py', 'Projects › scraper', 'Python · 7 KB', 'Matching lines 2–5'],
    open: 'Open', frame: 'frame 1:42',
    odoSep: ',', odoLabel: 'file names on drive C:',
    speed: [['by name', '13 ms'], ['by meaning', '0.1 s']],
    facts: ['Works offline', 'Free', 'Open source', 'Windows 10 · 11'],
    slogan: 'Search your files in your own words',
  },
};
const L = STR[LANG];

/* ---------------------------------------------------------------- springs and helpers */

const clamp = (x, a = 0, b = 1) => Math.min(b, Math.max(a, x));
const lerp = (a, b, k) => a + (b - a) * k;
const outQ = u => 1 - Math.pow(1 - clamp(u), 4);

function stepR(tau, sp) {                    // closed-form damped step response, 0 -> 1
  if (tau <= 0) return 0;
  const w = sp.w, z = sp.z;
  if (z >= 1) { const x = w * tau; return 1 - Math.exp(-x) * (1 + x); }
  const wd = w * Math.sqrt(1 - z * z);
  return 1 - Math.exp(-z * w * tau) * (Math.cos(wd * tau) + (z * w / wd) * Math.sin(wd * tau));
}
class Track {                                // many target changes = sum of one spring per change
  constructor(v0) { this.v0 = v0; this.cur = v0; this.ev = []; }
  to(t, v, sp) { if (v !== this.cur) { this.ev.push([t, v - this.cur, sp]); this.cur = v; } return this; }
  at(t) { let v = this.v0; for (const [ti, d, sp] of this.ev) v += d * stepR(t - ti, sp); return v; }
}
const hex = h => [1, 3, 5].map(i => parseInt(h.slice(i, i + 2), 16));
class CTrack {
  constructor(h) { const c = hex(h); this.c = c.map(v => new Track(v)); }
  to(t, h, sp) { hex(h).forEach((v, i) => this.c[i].to(t, v, sp)); return this; }
  at(t) { const [r, g, b] = this.c.map(k => Math.round(clamp(k.at(t), 0, 255))); return `rgb(${r},${g},${b})`; }
}
const SP = {
  ui: {w: 20, z: 0.8}, open: {w: 13, z: 0.86}, shut: {w: 16, z: 0.9}, col: {w: 14, z: 1}, snap: {w: 40, z: 1},
  cam: {w: 8, z: 1}, lead: {w: 30, z: 0.8}, trail: {w: 15, z: 0.85}, pop: {w: 24, z: 0.72},
};
// enter on a spring, exit short and early: opacity, blur and a small rise
function vis(t, tin, tout, dOut = 0.12, sp = {w: 26, z: 1}) {
  const e = stepR(t - tin, sp), x = tout == null ? 0 : clamp((t - tout) / dOut), xo = x * x;
  return {o: clamp(e) * (1 - xo), b: (1 - clamp(e)) * 10 + xo * 10, dy: (1 - e) * 16 - xo * 10, e};
}
const kick = (t, t0, amp, dec) => (t < t0 ? 0 : amp * Math.exp(-(t - t0) / dec));

function el(tag, cls, parent, html) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (html != null) e.innerHTML = html;
  parent.appendChild(e);
  return e;
}
function put(e, v, x = 0, y = 0, extra = '') {   // apply a vis() result
  e.style.opacity = v.o.toFixed(4);
  e.style.filter = v.b > 0.05 && v.o > 0.001 ? `blur(${v.b.toFixed(2)}px)` : 'none';
  e.style.transform = `translate(${x}px,${(y + v.dy).toFixed(2)}px)${extra}`;
  e.style.visibility = v.o > 0.001 ? '' : 'hidden';
}
const esc = s => s.replace(/&/g, '&amp;').replace(/</g, '&lt;');

/* ---------------------------------------------------------------- stage */

const bg = document.getElementById('bg'), world = document.getElementById('world');
const hud = document.getElementById('hud'), dim = document.getElementById('dim');
const pools = [
  [el('div', 'pool', bg), 1500, 900, 'rgba(214,230,248,1)', -260, -380],
  [el('div', 'pool', bg), 1300, 820, 'rgba(223,233,246,1)', 1180, 560],
];
for (const [p, w, h, c] of pools) {
  p.style.width = w + 'px'; p.style.height = h + 'px';
  p.style.background = `radial-gradient(closest-side, ${c}, rgba(242,245,249,0))`;
}
dim.style.background = '#f2f5f9';

const M = {dur: DUR, fps: 60, cues: [], lang: LANG};
const cue = (t, k, v = 0) => M.cues.push({t: +t.toFixed(4), k, v});

/* ---------------------------------------------------------------- camera */

const CAM = {x: new Track(0), y: new Track(0), lz: new Track(Math.log(1.0))};
function camTo(t, x, y, z, sp = SP.cam) { CAM.x.to(t, x, sp); CAM.y.to(t, y, sp); CAM.lz.to(t, Math.log(z), sp); }
CAM.lz.to(0, Math.log(1.08), {w: 1.1, z: 1});

/* ---------------------------------------------------------------- the one shape */

const S = {
  L: new Track(-14), R: new Track(14), T: new Track(-14), B: new Track(14), rad: new Track(14),
  col: new CTrack(BLUE), sh: new Track(0), bd: new Track(0), punch: [],
};
function shapeTo(t, l, tp, r, b, rad, sp = SP.ui, spR = sp) {
  S.L.to(t, l, sp); S.R.to(t, r, spR); S.T.to(t, tp, sp); S.B.to(t, b, sp); S.rad.to(t, rad, sp);
}
const Sel = el('div', '', world); Sel.id = 'S';

/* ---------------------------------------------------------------- timeline */

const T = {
  chips: [1.0, 1.5, 2.0, 2.5, 2.75, 3.0, 3.125, 3.25], implode: 3.5,
  keys: [4.0, 4.25, 4.5], bar: 4.5, type1: 5.5, enter1: 8.0, push: 8.9, meaning: 9.45, shut1: 11.35,
  q2: 12.0, e2: 13.0, q3: 14.0, e3: 15.0, q4: 16.0, e4: 17.0, pull: 17.95,
  odo: 19.0, speed: [20.5, 21.0], back: 21.7, facts: [22.0, 23.0, 24.0, 25.0], dot: 25.55, logo: 26.0,
  glide: 26.3, word: 26.4, slogan: 27.25,
};

// dot birth and the absorbing punch
cue(0.02, 'blip');
const birth = t => stepR(t - 0.0, {w: 18, z: 0.7});

// dot -> search field
shapeTo(T.bar, -560, -48, 560, 48, 24, {w: 15, z: 0.82});
S.col.to(T.bar + 0.05, WHITE, SP.col);
S.sh.to(T.bar + 0.05, 1, SP.col);
S.bd.to(T.bar + 0.05, 1, SP.col);
cue(T.bar, 'morph');
// field -> result list
S.B.to(T.enter1, 592, SP.open);
S.B.to(T.shut1, 48, SP.shut);
cue(T.shut1, 'collapse');
// photo / video / code panels
for (const e of [T.e2, T.e3, T.e4]) { if (e === T.e2) S.B.to(e, 552, SP.open); S.punch.push([e, 0.012]); }
// the panel becomes one tile in a wall of files
shapeTo(T.pull, -100, -70, 100, 70, 14, {w: 12, z: 0.9});
cue(T.pull, 'pull');

// fact pills: the same shape, re-sized and re-coloured on every hit
const FACTCOL = [[BLUE, WHITE], [INK, WHITE], [WHITE, INK], [BLUE, WHITE]];

/* ---------------------------------------------------------------- scene A: names nobody remembers */

const capA = el('div', 'cap', hud);
capA.style.top = '118px'; capA.style.fontSize = '84px';
const hookWords = L.hook.split(' ').map((w, i, a) => el('span', '', capA, esc(w) + (i < a.length - 1 ? '&nbsp;' : '')));

const CHIPPOS = [[-560, -95], [450, -190], [-430, 140], [500, 85], [-110, 265], [380, 260], [-300, -215], [120, -105]];
const TILT = [-3, 2.5, 2, -2.5, -1.5, 3, 1.8, -2];
const chips = L.chips.map(([ic, name], i) => {
  const c = el('div', 'chip', world, `<img src="${ICON(ic)}"><span>${esc(name)}</span>`);
  return c;
});
T.chips.forEach((t, i) => cue(t, 'pop', i));
cue(T.implode + 0.42, 'suck');

/* ---------------------------------------------------------------- scene B: hotkey and typing */

const keys = [['Ctrl', 150], ['Alt', 124], ['S', 96]].map(([k, w]) => {
  const e = el('div', 'key', world, k); e.style.width = w + 'px'; return [e, w];
});
const KEYX = [-150, 0, 132];
T.keys.forEach(t => cue(t, 'key'));
const enterKey = el('div', 'key', world, 'Enter'); enterKey.style.width = '150px';

const capB = el('div', 'cap', hud, esc(L.describe));
capB.style.top = '118px'; capB.style.fontSize = '72px';

// field row inside the shape
const fIcon = el('div', 'a', Sel, `<svg width="44" height="44" viewBox="0 0 256 256"><g fill="${BLUE}">
  <path d="M28.04 106.54a80 80 0 1 1 160 0a80 80 0 1 1 -160 0ZM60.04 106.54a48 48 0 1 0 96 0a48 48 0 1 0 -96 0Z"/>
  <path d="M171.96 198.46a28 28 0 1 1 56 0a28 28 0 1 1 -56 0Z"/></g></svg>`);
fIcon.style.left = '30px'; fIcon.style.top = '26px';
const fPh = el('div', 'ftext ph', Sel, esc(L.ph));
const fText = el('div', 'ftext', Sel);
const uline = el('div', 'a', Sel); uline.style.cssText += 'top:93px;height:3px;background:' + BLUE;
const divider = el('div', 'a', Sel); divider.style.cssText += 'top:96px;height:1px;width:1120px;background:#e5e5e5';

// keystroke times: the first query accelerates from 16ths to 32nds over a snare build, the rest run on 32nds
function slots(n, t0, span, grid) {
  const N = Math.round(span / grid), out = []; let prev = -1;
  for (let k = 0; k < n; k++) {
    let s = Math.round(N * Math.pow(k / n, 0.78));
    if (s <= prev) s = prev + 1;
    prev = s; out.push(t0 + s * grid);
  }
  return out;
}
const even = (n, t0, span) => Array.from({length: n}, (_, k) => t0 + span * k / n);
const QUERIES = [
  {s: L.q1, mark: L.q1mark, ts: slots(L.q1.length, T.type1, 1.5, 0.0625), out: 11.55},
  {s: L.q2, ts: even(L.q2.length, T.q2, 0.875), out: T.q3 - 0.12},
  {s: L.q3, ts: even(L.q3.length, T.q3, 0.875), out: T.q4 - 0.12},
  {s: L.q4, ts: even(L.q4.length, T.q4, 0.875), out: T.pull - 0.05},
];
for (const q of QUERIES) q.ts.forEach((t, k) => cue(t, 'type', k));
[T.enter1, T.e2, T.e3, T.e4].forEach(t => cue(t, 'enter'));

/* ---------------------------------------------------------------- scene C: found by meaning */

const selBg = el('div', 'sel', Sel), selBar = el('div', 'selbar', Sel);
const rows = L.rows.map((r, i) => {
  const e = el('div', 'row', Sel);
  e.style.top = (112 + i * 166) + 'px';
  e.innerHTML = `<img class="ic" src="${ICON(r.ic)}"><div class="ti">${esc(r.ti)}</div><div class="pa">${esc(r.pa)}</div>
    <div class="da">${esc(r.da)}</div><div class="sn"><span class="tag">${esc(r.tag)}</span>${esc(r.sn[0])}` +
    (r.sn[1] ? `<span class="mk">${esc(r.sn[1])}</span>${esc(r.sn[2])}` : '') + '</div>';
  return e;
});
const snipMark = rows[0].querySelector('.mk');
selBg.style.top = '106px'; selBg.style.height = '170px'; selBar.style.top = '155px'; selBar.style.height = '72px';
[0, 1, 2].forEach(i => cue(T.enter1 + 0.1 + i * 0.08, 'row', i));
cue(T.push + 0.15, 'mark');
const capC = el('div', 'cap', hud, esc(L.meaning));
capC.style.top = '48px'; capC.style.fontSize = '66px';
cue(T.meaning, 'cap');

/* ---------------------------------------------------------------- scene D: photos, video frames, code */

const CAT = [[-14, 0], [-13, -22], [-10, -34], [-7, -40], [-8, -48], [-7.5, -56], [-6, -64], [-3, -58], [3, -58], [6, -64],
             [7.5, -56], [8, -48], [7, -40], [10, -34], [13, -22], [14, 0]];
function rail(top, n) {
  const p = [[0, top], [100, top], [100, top + 4]], step = 100 / n;
  for (let i = n - 1; i >= 0; i--) {
    const x1 = i * step + step * 0.7, x0 = i * step + step * 0.3;
    p.push([x1, top + 4], [x1, 100], [x0, 100], [x0, top + 4]);
  }
  p.push([0, top + 4]);
  return p;
}
const svgURI = s => 'data:image/svg+xml;utf8,' + encodeURIComponent(s);
function catSVG([c1, c2, cx, base, s, c3, top, n]) {           // the same cats as the README demo (mock_api.py)
  const P = pts => pts.map(([x, y]) => (x * 4).toFixed(1) + ',' + (y * 3).toFixed(1)).join(' ');
  let body = `<rect width="400" height="300" fill="${c1}"/>`;
  if (c2) body += `<polygon fill="${c2}" points="${P(CAT.map(([x, y]) => [cx + x * s, base + y * s]))}"/>`;
  body += `<polygon fill="${c3}" points="${P(rail(top, n))}"/>`;
  return svgURI(`<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300" preserveAspectRatio="none">${body}</svg>`);
}
const CATS = [['#e7c9a4', '#2b2a2e', 50, 92, 1.0, '#6f5a49', 58, 7], ['#e2c09a', '#2b2a2e', 38, 92, 0.9, '#6f5a49', 58, 7],
  ['#c9d8e6', '#c27a3a', 56, 90, 1.1, '#e8e4dc', 60, 9], ['#b8c6d4', '#6b6f75', 46, 94, 0.8, '#3c4148', 64, 6],
  ['#d6dde4', '#c27a3a', 62, 96, 0.7, '#9aa3ac', 70, 11], ['#3d4a66', '#18181c', 44, 90, 1.0, '#58617a', 56, 8],
  ['#cfe0d0', '#3a3530', 30, 93, 0.75, '#efefea', 62, 6], ['#dfe3e6', '#f1efe9', 52, 95, 0.9, '#5c646c', 62, 7],
  ['#e8d8c8', '#7d7f84', 60, 92, 1.2, '#8f7a68', 66, 5], ['#c4ccd4', '#2b2a2e', 70, 94, 0.6, '#4b535c', 58, 10],
  ['#efd2b0', '#c27a3a', 40, 91, 0.85, '#6f5a49', 60, 7]].map(catSVG);

// fireworks over a river: the frame the search lands on, drawn flat like the README pictures
function fireworksSVG(k = 1) {
  const bursts = [[118, 66, 44, '#f4b65a'], [262, 50, 56, '#ef7a5f'], [196, 98, 30, '#f7e2a8']];
  let b = '';
  for (const [x, y, R, c] of bursts) {
    const r = R * k;
    for (let i = 0; i < 18; i++) {
      const a = i / 18 * Math.PI * 2, c0 = Math.cos(a), s0 = Math.sin(a);
      b += `<line x1="${(x + c0 * r * 0.38).toFixed(1)}" y1="${(y + s0 * r * 0.38).toFixed(1)}" x2="${(x + c0 * r).toFixed(1)}" y2="${(y + s0 * r).toFixed(1)}" stroke="${c}" stroke-width="3" stroke-linecap="round"/>`;
    }
    b += `<circle cx="${x}" cy="${y}" r="3.5" fill="${c}"/>`;
    for (let j = 0; j < 4; j++)
      b += `<rect x="${x - 16 + j * 5}" y="${172 + j * 9}" width="${30 - j * 6}" height="3" rx="1.5" fill="${c}" opacity="${0.55 - j * 0.1}"/>`;
  }
  return svgURI(`<svg xmlns="http://www.w3.org/2000/svg" width="400" height="225" viewBox="0 0 400 225">
    <rect width="400" height="225" fill="#1c2443"/><rect y="164" width="400" height="61" fill="#253159"/>
    <polygon fill="#11162b" points="0,164 0,150 30,150 30,140 52,140 52,152 96,152 96,144 120,144 120,154 186,154 186,138 200,138 200,154 300,154 300,146 336,146 336,152 400,152 400,164"/>
    ${b}</svg>`);
}
const FW = fireworksSVG(1);

function metaHTML(m) {
  return `<div class="nm">${esc(m[0])}</div><div class="ln">${esc(m[1])}</div><div class="ln">${esc(m[2])}</div>` +
         `<div class="ln" style="color:${BLUE};font-weight:600">${esc(m[3])}</div>`;
}
function btn(parent) {
  const b = el('div', 'btn', parent, `<span>${esc(L.open)}</span><small>Enter</small>`);
  b.style.top = '478px'; return b;
}
// photo
const gPhoto = el('div', 'a', Sel);
const phMedia = el('div', 'media', gPhoto, `<img src="${CATS[0]}">`);
phMedia.style.width = '576px'; phMedia.style.height = '400px';
el('div', 'meta', gPhoto, metaHTML(L.photo)); btn(gPhoto);
// video
const gVideo = el('div', 'a', Sel);
const vMedia = el('div', 'media', gVideo, `<img src="${FW}">`);
vMedia.style.width = '576px'; vMedia.style.height = '324px'; vMedia.style.background = '#1c2443';
const vImg = vMedia.querySelector('img');
const vTag = el('div', 'a', vMedia, esc(L.frame));
vTag.style.cssText += 'left:16px;top:270px;padding:4px 12px;border-radius:8px;background:rgba(0,0,0,.55);color:#fff;font-size:20px;font-weight:600';
const vTrack = el('div', 'track', gVideo), vFill = el('div', 'fill', gVideo), vKnob = el('div', 'knob', gVideo);
const vMark = el('div', 'a', gVideo);
vTrack.style.left = '32px'; vTrack.style.top = '488px'; vTrack.style.width = '576px';
vFill.style.left = '32px'; vFill.style.top = '488px';
vMark.style.cssText += `top:478px;width:4px;height:28px;border-radius:2px;background:${BLUE}`;
const vTime = el('div', 'a', gVideo); vTime.style.cssText += 'left:32px;top:510px;font-size:20px;color:#5c5c5c;white-space:nowrap';
const VMATCH = 102 / 252;
el('div', 'meta', gVideo, metaHTML(L.video)); btn(gVideo);
cue(T.e3 + 0.12, 'scrub');
// code
const gCode = el('div', 'a', Sel);
const CODE = [
  ['<span class="kw">def</span> <span class="fn">fetch_json</span>(url, retries=<span class="nu">5</span>):', 0],
  ['    <span class="kw">for</span> attempt <span class="kw">in</span> range(retries):', 1],
  ['        <span class="kw">try</span>:', 1],
  ['            resp = session.get(url, timeout=<span class="nu">10</span>)', 1],
  ['            <span class="kw">return</span> json.loads(resp.text)', 1],
  ['        <span class="kw">except</span> (ValueError, Timeout):', 0],
  ['            time.sleep(<span class="nu">2</span> ** attempt)', 0],
  ['    <span class="kw">raise</span> FetchError(url)', 0],
];
const codeBox = el('div', 'code', gCode);
const codeHL = el('div', 'hl', codeBox); codeHL.style.top = (16 + 44) + 'px'; codeHL.style.height = (44 * 4) + 'px';
CODE.forEach(([h], i) => { const ln = el('div', 'ln', codeBox, h); ln.style.top = (16 + i * 44) + 'px'; });
el('div', 'meta', gCode, metaHTML(L.code)); btn(gCode);

const capD = [L.capPhoto, L.capVideo, L.capCode].map(s => {
  const c = el('div', 'cap', hud, esc(s)); c.style.top = '48px'; c.style.fontSize = '66px'; return c;
});

/* ---------------------------------------------------------------- scene E: a wall of files, real numbers */

const wall = document.createElement('div'); wall.className = 'a'; world.insertBefore(wall, world.firstChild);   // under the shape
const TILES = [];
const rnd = (i, j, s) => { const x = Math.sin(i * 127.1 + j * 311.7 + s * 74.7) * 43758.5453; return x - Math.floor(x); };
const DOCIC = ['word', 'pdf', 'text', 'table', 'slides', 'word', 'pdf'];
for (let j = -10; j <= 10; j++) for (let i = -13; i <= 13; i++) {
  if (i === 0 && j === 0) continue;
  const x = i * 224, y = j * 164, r = rnd(i, j, 1);
  const tl = el('div', 'tile', wall);
  tl.style.left = (x - 100) + 'px'; tl.style.top = (y - 70) + 'px';
  if (r < 0.3) tl.innerHTML = `<img class="th" src="${CATS[Math.floor(rnd(i, j, 2) * CATS.length)]}">`;
  else if (r < 0.4) tl.innerHTML = `<img class="th" src="${FW}">`;
  else if (r < 0.55) {
    tl.style.background = '#fafafa';
    let h = '';
    for (let k = 0; k < 5; k++)
      h += `<div class="bar" style="left:${18 + (k % 3) * 14}px;top:${22 + k * 21}px;width:${50 + rnd(i, j, k + 3) * 100}px;background:${k % 2 ? '#e3e6ea' : '#d5e3f3'}"></div>`;
    tl.innerHTML = h;
  } else {
    const ic = DOCIC[Math.floor(rnd(i, j, 4) * DOCIC.length)];
    tl.innerHTML = `<img class="ic" src="${ICON(ic)}"><div class="bar" style="left:18px;top:78px;width:${110 + rnd(i, j, 5) * 60}px"></div>` +
      `<div class="bar" style="left:18px;top:100px;width:${70 + rnd(i, j, 6) * 70}px"></div>`;
  }
  TILES.push({e: tl, d: Math.hypot(x, y), x, y});
}
const tileMini = el('div', 'a', Sel);
for (let k = 0; k < 5; k++) {
  const b = el('div', 'bar', tileMini);
  b.style.cssText += `left:${18 + (k % 3) * 14}px;top:${22 + k * 21}px;width:${70 + (k * 37) % 90}px;background:${k === 1 || k === 4 ? '#d5e3f3' : '#e3e6ea'}`;
}

// odometer
const odo = el('div', 'odo', hud);
const NFILES = 3498852, DIG = String(NFILES).length;
const strips = [];
for (let p = DIG - 1; p >= 0; p--) {
  const slot = el('div', 'slot', odo), strip = el('div', 'strip', slot);
  for (let d = 0; d <= 10; d++) el('span', '', strip, String(d % 10));
  strips.push({p, slot, strip});
  if (p % 3 === 0 && p > 0) el('div', 'sep', odo, L.odoSep === ',' ? ',' : '&#8239;&#8239;');
}
const odoLabel = el('div', 'cap', hud, esc(L.odoLabel));
odoLabel.style.top = '560px'; odoLabel.style.fontSize = '42px'; odoLabel.style.fontWeight = '400'; odoLabel.style.color = '#3a3a3a';
const speedPills = L.speed.map(([a, b]) => el('div', 'pill', hud,
  `<span style="color:#5c5c5c;font-weight:400">${esc(a)}</span><span style="color:#c4c4c4;margin:0 14px">·</span><span style="color:${BLUE}">${esc(b)}</span>`));
for (const p of speedPills) { p.style.background = WHITE; p.style.boxShadow = '0 0 0 1px rgba(0,0,0,.06),0 10px 30px rgba(16,42,80,.12)'; }
T.speed.forEach(t => cue(t, 'slam'));
cue(T.back, 'zoomin');

/* ---------------------------------------------------------------- scene F: facts */

const facts = L.facts.map(s => el('div', 'fact', Sel, esc(s)));
T.facts.forEach((t, i) => cue(t, 'fact', i));

/* ---------------------------------------------------------------- scene G: the logo */

const LS = 0.9;                                   // lockup scale: horizontal-blue.svg drawn at 0.9
const DOT = 24.675 * LS;                          // dot radius
const X0 = -151.505 * LS, Y0 = -151.51 * LS;      // lockup origin, so that the dot sits at world (0, 0)
const ring = el('div', 'a', world, `<svg width="${141 * LS}" height="${141 * LS}" viewBox="0 0 141 141"><path fill="${BLUE}" d="M0 70.5a70.5 70.5 0 1 1 141 0a70.5 70.5 0 1 1 -141 0ZM28.2 70.5a42.3 42.3 0 1 0 84.6 0a42.3 42.3 0 1 0 -84.6 0Z"/></svg>`);
ring.style.left = X0 + 'px'; ring.style.top = Y0 + 'px';
ring.style.transformOrigin = `${70.5 * LS}px ${70.5 * LS}px`;
// wordmark paths of brand/logo/svg/horizontal-blue.svg, one group per letter
const WM = [
  ['M295.03 21.9A39 39 0 1 0 262.02 77.95L260.97 57.97A19 19 0 1 1 277.06 30.67Z', 'M257.81 58.06A41.5 41.5 0 1 1 222.68 117.69L240.66 108.92A21.5 21.5 0 1 0 258.85 78.03Z'],
  ['M420.48 88.5A52.5 52.5 0 1 0 407 123.63L392.88 110.92A33.5 33.5 0 1 1 401.48 88.5Z', 'M333.98 80.33H420.48V96.67H333.98Z'],
  ['M437.98 38.5H456.98V138.5H437.98Z', 'M437.98 77.5A40.5 40.5 0 0 1 518.98 77.5L499.98 77.5A21.5 21.5 0 0 0 456.98 77.5Z', 'M499.98 75.5H518.98V138.5H499.98Z',
   'M499.98 77.5A40.5 40.5 0 0 1 580.98 77.5L561.98 77.5A21.5 21.5 0 0 0 518.98 77.5Z', 'M561.98 75.5H580.98V138.5H561.98Z'],
  ['M675.03 21.9A39 39 0 1 0 642.02 77.95L640.97 57.97A19 19 0 1 1 657.06 30.67Z', 'M637.81 58.06A41.5 41.5 0 1 1 602.68 117.69L620.66 108.92A21.5 21.5 0 1 0 638.85 78.03Z'],
  ['M800.48 88.5A52.5 52.5 0 1 0 787 123.63L772.88 110.92A33.5 33.5 0 1 1 781.48 88.5Z', 'M713.98 80.33H800.48V96.67H713.98Z'],
  ['M814.48 88.5a52.5 52.5 0 1 1 105 0a52.5 52.5 0 1 1 -105 0ZM833.48 88.5a33.5 33.5 0 1 0 67 0a33.5 33.5 0 1 0 -67 0Z', 'M901.24 38.5H919.48V138.5H901.24Z'],
  ['M940.48 38.5H959.48V138.5H940.48Z', 'M940.48 79.5A42.5 42.5 0 0 1 1002.93 41.97L994.01 58.75A23.5 23.5 0 0 0 959.48 79.5Z'],
  ['M1102.95 53.37A52.5 52.5 0 1 0 1102.95 123.63L1088.83 110.92A33.5 33.5 0 1 1 1088.83 66.08Z'],
  ['M1130.43 2.5H1149.43V138.5H1130.43Z', 'M1130.43 79.5A42.5 42.5 0 0 1 1215.43 79.5L1196.43 79.5A23.5 23.5 0 0 0 1149.43 79.5Z', 'M1196.43 77.5H1215.43V138.5H1196.43Z'],
];
const VBW = 1215.44, VBH = 176.18;
const letters = WM.map(ps => {
  const e = el('div', 'letter', world, `<svg width="${VBW * LS}" height="${VBH * LS}" viewBox="0 0 ${VBW} ${VBH}"><g fill="${BLUE}">${ps.map(d => `<path d="${d}"/>`).join('')}</g></svg>`);
  e.style.left = X0 + 'px'; e.style.top = Y0 + 'px';
  return e;
});
const slogan = el('div', 'a nw', world, esc(L.slogan));
slogan.style.fontSize = '50px'; slogan.style.color = '#3a3a3a';
const LOCKX = X0 + VBW * LS / 2, LOCKB = Y0 + VBH * LS;
cue(T.dot, 'suck2');
cue(T.logo, 'logo');
letters.forEach((_, i) => cue(T.word + i * 0.06, 'reveal', i));
cue(T.slogan, 'slogan');

/* ---------------------------------------------------------------- layout that needs measuring */

function layout() {
  // chips
  chips.forEach(c => { c._w = c.offsetWidth; c._h = c.offsetHeight; });
  // keys
  // fact pills: one width per fact, measured
  const fw = facts.map(f => f.offsetWidth);
  let prevW = 200;
  T.facts.forEach((t, i) => {
    const w = fw[i] + 190, issue = i === 0 ? T.back + 0.08 : t - 0.12;
    const spL = i === 0 ? SP.ui : (w > prevW ? SP.trail : SP.lead), spR = i === 0 ? SP.ui : (w > prevW ? SP.lead : SP.trail);
    shapeTo(issue, -w / 2, -78, w / 2, 78, 78, spL, spR);
    S.col.to(t - 0.04, FACTCOL[i][0], SP.snap);
    S.bd.to(t - 0.04, FACTCOL[i][0] === WHITE ? 1 : 0, SP.snap);
    if (i === 0) S.sh.to(issue, 1, SP.col);
    S.punch.push([t, 0.05]);
    facts[i]._w = fw[i];
    prevW = w;
  });
  // back to the dot
  shapeTo(T.dot, -DOT, -DOT, DOT, DOT, DOT, {w: 16, z: 0.86});
  S.col.to(T.dot, BLUE, SP.col);
  S.bd.to(T.dot, 0, SP.col);
  S.sh.to(T.dot, 0, SP.col);
  S.punch.push([T.logo, 0.14]);
  // speed pills side by side
  const pw = speedPills.map(p => p.offsetWidth), gap = 28, tot = pw[0] + pw[1] + gap;
  speedPills[0]._x = 960 - tot / 2; speedPills[1]._x = 960 - tot / 2 + pw[0] + gap;
  // slogan centred under the wordmark
  slogan._w = slogan.offsetWidth;
  // the first query's mark, measured in the field
  // cameras
  camTo(T.bar - 0.05, 0, 0, 1.12, {w: 6, z: 1});
  CAM.lz.to(T.type1 - 0.1, Math.log(1.42), {w: 2.6, z: 1});
  camTo(T.enter1 - 0.05, 0, 262, 1.3, {w: 9, z: 1});
  camTo(T.push, 0, 150, 1.58, {w: 6.5, z: 1});
  camTo(T.shut1, 0, 0, 1.42, {w: 8, z: 1});
  camTo(T.e2 - 0.05, 0, 246, 1.3, {w: 9, z: 1});
  camTo(T.pull + 0.03, 0, 0, 0.34, {w: 3.4, z: 1});
  camTo(T.back - 0.02, 0, 0, 1.15, {w: 6.5, z: 1});
  camTo(T.dot, 0, 0, 1.7, {w: 8, z: 1});
  camTo(T.glide, LOCKX, (Y0 + LOCKB + 150) / 2 - 10, 1.12, {w: 4.6, z: 1});
  CAM.lz.to(T.slogan + 0.3, Math.log(1.17), {w: 0.9, z: 1});     // the end card keeps breathing
  // odometer ticks follow the ten-thousands digit
  let last = -1, lt = -1;
  for (let t = T.odo; t <= T.odo + 1.4; t += 1 / 240) {
    const d = Math.floor(odoValue(t) / 1e4);
    if (d !== last && t - lt > 0.045) { cue(t, 'tick'); lt = t; }
    last = d;
  }
  M.cues.sort((a, b) => a.t - b.t);
}
const odoValue = t => NFILES * outQ((t - T.odo - 0.05) / 1.3);

/* ---------------------------------------------------------------- seek */

let typedKey = '';
function typed(t) {
  for (let qi = QUERIES.length - 1; qi >= 0; qi--) {
    const q = QUERIES[qi];
    if (t < q.ts[0] - 0.02 || t >= q.out + 0.12) continue;
    let n = 0; while (n < q.ts.length && q.ts[n] <= t) n++;
    return {qi, q, n, v: vis(t, q.ts[0] - 0.02, q.out, 0.1, {w: 60, z: 1}), last: q.ts[Math.max(0, n - 1)]};
  }
  return null;
}

function seek(t) {
  /* camera, with a punch on the fact hits */
  let z = Math.exp(CAM.lz.at(t));
  for (const h of T.facts) z *= 1 + kick(t, h, 0.018, 0.12);
  z *= 1 + kick(t, T.logo, 0.03, 0.18);
  const cx = CAM.x.at(t), cy = CAM.y.at(t);
  world.style.transform = `translate(960px,540px) scale(${z.toFixed(5)}) translate(${(-cx).toFixed(2)}px,${(-cy).toFixed(2)}px)`;

  /* background pools drift */
  pools.forEach(([p, w, h, c, x, y], i) => {
    p.style.transform = `translate(${(x + Math.sin(t * 0.21 + i * 2) * 60).toFixed(1)}px,${(y + Math.cos(t * 0.17 + i) * 40).toFixed(1)}px)`;
  });

  /* the shape */
  const l = S.L.at(t), r = S.R.at(t), tp = S.T.at(t), b = S.B.at(t);
  const w = Math.max(0, r - l), h = Math.max(0, b - tp);
  let sc = birth(t);
  for (const [tp0, a] of S.punch) sc *= 1 + kick(t, tp0, a, 0.09) * Math.cos(Math.min(t - tp0, 0.6) * 9);
  sc *= 1 + kick(t, T.implode + 0.42, 0.45, 0.08);
  T.chips.forEach(ct => { sc *= 1 + kick(t, ct, 0.18, 0.07); });
  Sel.style.left = l.toFixed(2) + 'px'; Sel.style.top = tp.toFixed(2) + 'px';
  Sel.style.width = w.toFixed(2) + 'px'; Sel.style.height = h.toFixed(2) + 'px';
  Sel.style.borderRadius = Math.min(S.rad.at(t), w / 2, h / 2).toFixed(2) + 'px';
  Sel.style.transform = `scale(${sc.toFixed(5)})`;
  Sel.style.background = S.col.at(t);
  const sh = clamp(S.sh.at(t), 0, 1.2), bd = clamp(S.bd.at(t), 0, 1);
  Sel.style.boxShadow = `inset 0 0 0 1px rgba(0,0,0,${(0.07 * bd).toFixed(3)}),0 ${(2 * sh).toFixed(1)}px ${(6 * sh).toFixed(1)}px rgba(16,42,80,${(0.07 * sh).toFixed(3)}),0 ${(22 * sh).toFixed(1)}px ${(60 * sh).toFixed(1)}px rgba(16,42,80,${(0.16 * sh).toFixed(3)})`;

  /* scene A */
  hookWords.forEach((s, i) => put(s, vis(t, 0.5 + i * 0.07, T.implode - 0.05 + i * 0.02, 0.14)));
  chips.forEach((c, i) => {
    const [x0, y0] = CHIPPOS[i], t0 = T.chips[i];
    if (t < t0 - 0.01 || t > T.implode + 0.7) { c.style.visibility = 'hidden'; return; }
    const e = stepR(t - t0, SP.pop), ti = T.implode + i * 0.015, p = stepR(t - ti, {w: 13, z: 1});
    const dx = Math.sin(t * 0.9 + i * 1.7) * 7, dy = Math.cos(t * 0.75 + i * 2.3) * 6;
    const x = lerp(x0 + dx, 0, p), y = lerp(y0 + dy, 0, p);
    const s = lerp(0.62 + 0.38 * e, 0.06, p), o = clamp(e * 3) * (1 - p * p);
    c.style.visibility = o > 0.002 ? '' : 'hidden';
    c.style.opacity = o.toFixed(4);
    const bl = (1 - clamp(e)) * 9 + p * 8;
    c.style.filter = bl > 0.05 ? `blur(${bl.toFixed(2)}px)` : 'none';
    c.style.transform = `translate(${(x - c._w / 2).toFixed(2)}px,${(y - c._h / 2).toFixed(2)}px) rotate(${(TILT[i] * (1 - p) + (1 - e) * 6).toFixed(2)}deg) scale(${s.toFixed(4)})`;
  });

  /* scene B: keys */
  keys.forEach(([k, kw], i) => {
    const tp0 = T.keys[i], v = vis(t, tp0 - 0.16, 5.25 + i * 0.04, 0.14, SP.pop);
    const press = t >= tp0 ? Math.max(0, 1 - stepR(t - tp0 - 0.07, {w: 30, z: 0.9})) : 0;
    k.style.borderBottomWidth = (7 - 5 * press).toFixed(2) + 'px';
    k.style.background = press > 0.5 ? '#f3f3f3' : WHITE;
    put(k, v, KEYX[i] - kw / 2, 196 - 43 + 5 * press);
  });
  {
    const v = vis(t, T.enter1 - 0.5, T.enter1 + 0.18, 0.12, SP.pop);
    const press = t >= T.enter1 ? Math.max(0, 1 - stepR(t - T.enter1 - 0.07, {w: 30, z: 0.9})) : 0;
    enterKey.style.borderBottomWidth = (7 - 5 * press).toFixed(2) + 'px';
    enterKey.style.background = press > 0.5 ? '#f3f3f3' : WHITE;
    put(enterKey, v, 410, 84 + 5 * press);
  }
  put(capB, vis(t, T.bar + 0.15, T.enter1 - 0.12, 0.14));

  /* the field */
  const fieldOut = T.pull - 0.05;
  put(fIcon, vis(t, T.bar + 0.25, fieldOut, 0.1), 0, 0);
  put(fPh, vis(t, T.bar + 0.35, T.type1 - 0.06, 0.08), 0, 0);
  const vu = vis(t, T.bar + 0.3, fieldOut, 0.1);
  uline.style.opacity = vu.o.toFixed(3);
  uline.style.left = (w / 2 * (1 - vu.e)).toFixed(1) + 'px';
  uline.style.width = (w * clamp(vu.e)).toFixed(1) + 'px';
  divider.style.opacity = (clamp((h - 110) / 60) * (t < fieldOut ? 1 : 0)).toFixed(3);
  divider.style.width = w.toFixed(1) + 'px';
  const ty = typed(t);
  if (ty) {
    const {q, n} = ty;
    const s = q.s.slice(0, n);
    let mk = 0;
    if (q.mark) mk = clamp((t - T.push) / 0.3);
    const blink = t - ty.last < 0.45 || (t * 1.9) % 1 < 0.55;
    const key = ty.qi + '|' + n + '|' + mk.toFixed(3) + '|' + blink;
    if (key !== typedKey) {
      typedKey = key;
      let html;
      const at = q.mark ? q.s.indexOf(q.mark) : -1;
      if (at >= 0 && n >= at + q.mark.length && mk > 0)
        html = esc(s.slice(0, at)) + `<span class="mk" style="background-size:${(mk * 100).toFixed(1)}% 100%">${esc(q.mark)}</span>` + esc(s.slice(at + q.mark.length));
      else html = esc(s);
      fText.innerHTML = html + `<span class="caret" style="opacity:${blink ? 1 : 0}"></span>`;
    }
    put(fText, ty.v);
  } else { fText.style.visibility = 'hidden'; }

  /* scene C */
  rows.forEach((rw, i) => put(rw, vis(t, T.enter1 + 0.1 + i * 0.08, T.shut1 - 0.05, 0.1)));
  const vs = vis(t, T.enter1 + 0.62, T.shut1 - 0.05, 0.1);
  put(selBg, vs); put(selBar, vs);
  const mk2 = clamp((t - T.push - 0.15) / 0.35);
  snipMark.style.backgroundSize = (outQ(mk2) * 100).toFixed(1) + '% 100%';
  put(capC, vis(t, T.meaning, T.shut1 + 0.2, 0.14));

  /* scene D */
  const stale = (t0, t1) => { const k = clamp((t - t0) / 0.15) * (t < t1 ? 1 : 0); return k; };
  {
    const v = vis(t, T.e2 + 0.06, T.e3 - 0.04, 0.1), k = stale(T.q3, T.e3);
    v.o *= 1 - 0.6 * k; v.b += 3 * k;
    put(gPhoto, v);
    const pz = 1.06 - 0.06 * outQ((t - T.e2) / 0.9);
    phMedia.querySelector('img').style.transform = `scale(${pz.toFixed(4)})`;
  }
  {
    const v = vis(t, T.e3 + 0.06, T.e4 - 0.04, 0.1), k = stale(T.q4, T.e4);
    v.o *= 1 - 0.6 * k; v.b += 3 * k;
    put(gVideo, v);
    // playhead: the two ends of the fill ride different springs, so the knob runs ahead and the fill catches up
    const lead = stepR(t - T.e3 - 0.12, {w: 20, z: 0.8}), trail = stepR(t - T.e3 - 0.12, {w: 12, z: 0.9});
    const kx = 576 * VMATCH * lead, fx = 576 * VMATCH * trail;
    vKnob.style.transform = `translate(${(32 + kx - 13).toFixed(2)}px,${(488 + 4 - 13).toFixed(2)}px)`;
    vFill.style.width = Math.max(0, fx).toFixed(2) + 'px';
    vMark.style.left = (32 + 576 * VMATCH - 2).toFixed(1) + 'px';
    vMark.style.opacity = clamp((t - T.e3 - 0.05) / 0.12).toFixed(3);
    const secs = Math.round(252 * VMATCH * lead);
    vTime.textContent = `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, '0')} / 4:12`;
    const fv = vis(t, T.e3 + 0.32, null, 0, {w: 16, z: 1});
    vImg.style.opacity = fv.o.toFixed(3);
    vImg.style.filter = fv.b > 0.05 ? `blur(${fv.b.toFixed(2)}px)` : 'none';
    vImg.style.transform = `scale(${(1.08 - 0.08 * fv.e).toFixed(4)})`;
    put(vTag, vis(t, T.e3 + 0.45, null));
  }
  {
    const v = vis(t, T.e4 + 0.06, T.pull - 0.08, 0.1);
    put(gCode, v);
    codeHL.style.opacity = clamp((t - T.e4 - 0.35) / 0.2).toFixed(3);
  }
  put(capD[0], vis(t, T.q2, T.q3 - 0.1, 0.1));
  put(capD[1], vis(t, T.q3, T.q4 - 0.1, 0.1));
  put(capD[2], vis(t, T.q4, T.pull - 0.05, 0.1));

  /* scene E */
  if (t > T.pull - 0.1 && t < T.back + 0.6) {
    wall.style.display = '';
    const fade = 1 - clamp((t - T.back - 0.1) / 0.3);
    for (const tl of TILES) {
      const t0 = T.pull + 0.08 + tl.d / 2900, e = stepR(t - t0, {w: 16, z: 0.85});
      const o = clamp(e * 2) * fade;
      tl.e.style.opacity = o.toFixed(3);
      tl.e.style.visibility = o > 0.002 ? '' : 'hidden';
      tl.e.style.transform = `scale(${(0.7 + 0.3 * e).toFixed(4)})`;
    }
  } else wall.style.display = 'none';
  put(tileMini, vis(t, T.pull + 0.25, T.back + 0.05, 0.1));
  const dv = vis(t, T.odo - 0.1, T.back, 0.25, {w: 9, z: 1});
  dim.style.opacity = (0.86 * dv.o).toFixed(3);
  dim.style.visibility = dv.o > 0.001 ? '' : 'hidden';
  const ov = vis(t, T.odo, T.back - 0.05, 0.12);
  put(odo, ov);
  const val = odoValue(t), lg = val > 0 ? Math.log10(val) : 0;
  for (const s of strips) {
    const P10 = Math.pow(10, s.p);
    const d = Math.floor(val / P10) % 10 + Math.max(0, (val % P10) - (P10 - 1));
    s.strip.style.transform = `translateY(${(-d * 210).toFixed(2)}px)`;
    s.slot.style.opacity = s.p === 0 ? 1 : clamp((lg - s.p) * 6).toFixed(3);
  }
  put(odoLabel, vis(t, T.odo + 0.12, T.back - 0.05, 0.12));
  speedPills.forEach((p, i) => {
    const t0 = T.speed[i], v = vis(t, t0 - 0.04, T.back - 0.05, 0.12, SP.pop);
    const s = 1.3 - 0.3 * stepR(t - t0 + 0.04, SP.pop);
    put(p, v, p._x, 650, ` scale(${s.toFixed(4)})`);
  });

  /* scene F */
  facts.forEach((f, i) => {
    const t0 = T.facts[i], t1 = i < 3 ? T.facts[i + 1] - 0.1 : T.dot - 0.05;
    const v = vis(t, t0 - 0.03, t1, 0.09, {w: 30, z: 1});
    f.style.color = FACTCOL[i][1];
    put(f, v, (w - f._w) / 2, 0);
  });

  /* scene G */
  const rv = vis(t, T.logo - 0.01, null, 0, {w: 26, z: 1});
  const rs = 0.45 + 0.55 * stepR(t - T.logo, {w: 17, z: 0.74});
  ring.style.opacity = rv.o.toFixed(3);
  ring.style.visibility = rv.o > 0.001 ? '' : 'hidden';
  ring.style.filter = rv.b > 0.05 ? `blur(${rv.b.toFixed(2)}px)` : 'none';
  ring.style.transform = `scale(${rs.toFixed(4)})`;
  letters.forEach((e, i) => put(e, vis(t, T.word + i * 0.06, null, 0, {w: 20, z: 1}), 0, 0, ` translateX(${(-(1 - stepR(t - T.word - i * 0.06, {w: 16, z: 1})) * 24).toFixed(2)}px)`));
  put(slogan, vis(t, T.slogan, null), LOCKX - slogan._w / 2, LOCKB + 46);
}

/* ---------------------------------------------------------------- boot */

Promise.all([document.fonts.ready, ...[...document.images].map(i => i.decode().catch(() => 0))]).then(() => {
  layout();
  window.M = M;
  window.seek = seek;
  const t0 = Q.get('t');
  seek(t0 != null ? +t0 : 0);
  window.READY = true;
});
