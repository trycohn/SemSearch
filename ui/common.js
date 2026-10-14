/* SemSearch UI — общие помощники (токен, API, Tauri, форматирование, тема, DOM).
   Строки из API (имена, пути, фрагменты) НИКОГДА не вставляются как HTML — только textContent. */
(function () {
  'use strict';
  const SS = (window.SS = {});

  /* ---------- токен, Tauri ---------- */
  const meta = document.querySelector('meta[name="semsearch-token"]');
  SS.token = meta ? meta.getAttribute('content') || '' : '';
  SS.tauri = window.__TAURI__ && window.__TAURI__.core ? window.__TAURI__ : null;
  SS.invoke = (cmd, args) =>
    SS.tauri ? SS.tauri.core.invoke(cmd, args || {}) : Promise.reject(new Error('Tauri недоступен'));
  SS.listen = (ev, cb) => {
    if (SS.tauri && SS.tauri.event) SS.tauri.event.listen(ev, cb).catch(() => {});
  };

  /* ---------- HTTP ---------- */
  class ApiError extends Error {
    constructor(status, detail) {
      super(detail || 'HTTP ' + status);
      this.status = status;
      this.detail = detail || '';
    }
  }
  SS.ApiError = ApiError;
  SS.api = async (path, opts) => {
    opts = opts || {};
    const headers = { 'X-SemSearch-Token': SS.token };
    let body;
    if (opts.form) body = opts.form;
    else if (opts.body !== undefined) {
      headers['Content-Type'] = 'application/json';
      body = JSON.stringify(opts.body);
    }
    const res = await fetch(path, {
      method: opts.method || (body !== undefined ? 'POST' : 'GET'),
      headers, body, signal: opts.signal, cache: 'no-store'
    });
    let data = null;
    try { data = await res.json(); } catch (e) { data = null; }
    if (!res.ok) throw new ApiError(res.status, data && typeof data.detail === 'string' ? data.detail : '');
    return data;
  };
  SS.q = (params) => {
    const u = new URLSearchParams();
    Object.keys(params).forEach((k) => {
      const v = params[k];
      if (v !== undefined && v !== null && v !== '') u.set(k, String(v));
    });
    return u.toString();
  };
  SS.thumbUrl = (u, size) => {
    if (typeof u !== 'string' || !/^\/thumb\/[^/?#]/.test(u)) return '';
    let url = u;
    if (size && !/[?&]size=/.test(url)) url += (url.indexOf('?') < 0 ? '?' : '&') + 'size=' + size;
    return url + (url.indexOf('?') < 0 ? '?' : '&') + 't=' + encodeURIComponent(SS.token);
  };

  /* ---------- форматирование (по-русски) ---------- */
  const NB = ' ';
  SS.pl = (n, f) => {
    n = Math.abs(Math.floor(n));
    const a = n % 10, b = n % 100;
    return f[a === 1 && b !== 11 ? 0 : a >= 2 && a <= 4 && (b < 10 || b >= 20) ? 1 : 2];
  };
  SS.num = (n) => {
    if (n === null || n === undefined || isNaN(n)) return '0';
    const s = String(Math.round(Math.abs(n)));
    return (n < 0 ? '−' : '') + s.replace(/\B(?=(\d{3})+(?!\d))/g, NB);
  };
  const dec1 = (v) => v.toFixed(1).replace('.', ',');
  SS.size = (b) => {
    if (b === null || b === undefined || isNaN(b)) return '';
    if (b < 1024) return Math.round(b) + NB + 'Б';
    const kb = b / 1024;
    if (kb < 999.5) return Math.round(kb) + NB + 'КБ';
    const units = ['МБ', 'ГБ', 'ТБ'];
    let v = kb / 1024, i = 0;
    while (v >= 999.5 && i < units.length - 1) { v /= 1024; i++; }
    return (v < 99.95 ? dec1(v) : String(Math.round(v))) + NB + units[i];
  };
  SS.bigNum = (n) => {
    if (n >= 1e6) return dec1(n / 1e6).replace(/,0$/, '') + NB + 'млн';
    return SS.num(n);
  };
  const MON = ['янв', 'фев', 'мар', 'апр', 'май', 'июн', 'июл', 'авг', 'сен', 'окт', 'ноя', 'дек'];
  const MONG = ['января', 'февраля', 'марта', 'апреля', 'мая', 'июня', 'июля', 'августа', 'сентября', 'октября', 'ноября', 'декабря'];
  SS.dateShort = (ts, withYear) => {
    if (!ts) return '';
    const d = new Date(ts * 1000);
    let s = d.getDate() + NB + MON[d.getMonth()];
    if (withYear || d.getFullYear() !== new Date().getFullYear()) s += NB + d.getFullYear();
    return s;
  };
  SS.dateLong = (ts) => {
    if (!ts) return '';
    const d = new Date(ts * 1000);
    return d.getDate() + ' ' + MONG[d.getMonth()] + ' ' + d.getFullYear() + ', ' +
      String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0');
  };
  SS.dur = (sec) => {
    if (sec === null || sec === undefined || isNaN(sec)) return '';
    sec = Math.max(0, Math.round(sec));
    const h = Math.floor(sec / 3600), m = Math.floor((sec % 3600) / 60), s = sec % 60;
    return (h ? h + ':' + String(m).padStart(2, '0') : String(m)) + ':' + String(s).padStart(2, '0');
  };
  SS.ago = (ts) => {
    if (!ts) return '';
    const d = Date.now() / 1000 - ts;
    if (d < 60) return 'только что';
    if (d < 3600) return Math.floor(d / 60) + ' мин назад';
    if (d < 86400) return Math.floor(d / 3600) + ' ч назад';
    return SS.dateShort(ts);
  };
  SS.drives = (list) => {
    list = (list || []).filter(Boolean);
    if (list.length <= 1) return list[0] || '';
    return list.slice(0, -1).join(', ') + ' и ' + list[list.length - 1];
  };
  SS.light = (hex, t) => {
    if (!/^#[0-9a-f]{6}$/i.test(hex)) return hex;
    const n = parseInt(hex.slice(1), 16);
    const f = (c) => Math.round(c + (255 - c) * t).toString(16).padStart(2, '0');
    return '#' + f((n >> 16) & 255) + f((n >> 8) & 255) + f(n & 255);
  };
  /* "ctrl+alt+s" → ["Ctrl","Alt","S"] */
  SS.caps = (accel) => String(accel || '').split('+').filter(Boolean).map((p) => {
    const k = p.trim().toLowerCase();
    const map = { ctrl: 'Ctrl', control: 'Ctrl', alt: 'Alt', shift: 'Shift', win: 'Win', super: 'Win', meta: 'Win', cmd: 'Win', space: 'Space', enter: 'Enter', esc: 'Esc', escape: 'Esc', tab: 'Tab' };
    return map[k] || (k.length === 1 ? k.toUpperCase() : k.charAt(0).toUpperCase() + k.slice(1));
  });

  /* ---------- тема ---------- */
  const mq = window.matchMedia ? window.matchMedia('(prefers-color-scheme: dark)') : null;
  let themeCur = { theme: 'system', accent: '#005fb8' };
  let themeSig = '', themeT = 0;
  SS.isDark = () => themeCur.theme === 'dark' || (themeCur.theme !== 'light' && !!(mq && mq.matches));
  SS.applyTheme = (theme, accent) => {
    themeCur = { theme: theme || 'system', accent: /^#[0-9a-f]{6}$/i.test(accent || '') ? accent : '#005fb8' };
    const dark = SS.isDark();
    const root = document.documentElement;
    const sig = (dark ? 'd' : 'l') + themeCur.accent;
    if (themeSig && themeSig !== sig && document.readyState !== 'loading' && !SS.reduced()) {
      root.classList.add('theming');
      clearTimeout(themeT);
      themeT = setTimeout(() => root.classList.remove('theming'), 340);
    }
    themeSig = sig;
    root.style.setProperty('--acc', SS.light(themeCur.accent, 0.45));
    if (root.hasAttribute('data-fixed-theme')) return; /* попап трея: всегда тёмный и прозрачный */
    root.classList.toggle('dark', dark);
    root.style.colorScheme = dark ? 'dark' : 'light';
    const acc = dark ? SS.light(themeCur.accent, 0.45) : themeCur.accent;
    root.style.setProperty('--accent', acc);
    root.style.setProperty('--onAccent', dark ? '#111111' : '#ffffff');
    document.querySelectorAll('.win').forEach((w) => w.classList.toggle('dark', dark));
    try { localStorage.setItem('semsearch.theme', JSON.stringify(themeCur)); } catch (e) { /* нет хранилища */ }
  };
  try {
    const c = JSON.parse(localStorage.getItem('semsearch.theme') || 'null');
    if (c) themeCur = c;
  } catch (e) { /* нет хранилища */ }
  SS.applyTheme(themeCur.theme, themeCur.accent);
  if (mq && mq.addEventListener) mq.addEventListener('change', () => SS.applyTheme(themeCur.theme, themeCur.accent));
  window.addEventListener('storage', (e) => {
    if (e.key !== 'semsearch.theme' || !e.newValue) return;
    try { const c = JSON.parse(e.newValue); SS.applyTheme(c.theme, c.accent); } catch (x) { /* мусор */ }
  });
  document.addEventListener('DOMContentLoaded', () => SS.applyTheme(themeCur.theme, themeCur.accent));

  /* ---------- DOM ---------- */
  /* h('div.cls1.cls2', {attr}, child | 'text' | [children]) */
  SS.h = function (sel, attrs) {
    const m = String(sel).split('.');
    const el = document.createElement(m[0] || 'div');
    if (m.length > 1) el.className = m.slice(1).join(' ');
    let i = 1;
    if (attrs && typeof attrs === 'object' && !(attrs instanceof Node) && !Array.isArray(attrs)) {
      Object.keys(attrs).forEach((k) => {
        const v = attrs[k];
        if (v === null || v === undefined || v === false) return;
        if (k === 'text') el.textContent = String(v);
        else if (k === 'style') el.style.cssText = v;
        else if (k.slice(0, 2) === 'on' && typeof v === 'function') el.addEventListener(k.slice(2), v);
        else el.setAttribute(k, v === true ? '' : String(v));
      });
      i = 2;
    }
    for (; i < arguments.length; i++) SS.append(el, arguments[i]);
    return el;
  };
  SS.append = (el, c) => {
    if (c === null || c === undefined || c === false) return;
    if (Array.isArray(c)) c.forEach((x) => SS.append(el, x));
    else if (c instanceof Node) el.appendChild(c);
    else el.appendChild(document.createTextNode(String(c)));
  };
  const NS = 'http://www.w3.org/2000/svg';
  /* svg(path, {s: size, sw: stroke-width, cap, join, cls, style, vb}) */
  SS.svg = (d, o) => {
    o = o || {};
    const s = document.createElementNS(NS, 'svg');
    const size = o.s || 16;
    s.setAttribute('width', size);
    s.setAttribute('height', size);
    s.setAttribute('viewBox', o.vb || '0 0 24 24');
    s.setAttribute('aria-hidden', 'true');
    if (!o.fill) {
      s.setAttribute('fill', 'none');
      s.setAttribute('stroke', 'currentColor');
      s.setAttribute('stroke-width', o.sw || 1.8);
      if (o.cap !== false) s.setAttribute('stroke-linecap', 'round');
      if (o.join) s.setAttribute('stroke-linejoin', 'round');
    }
    if (o.cls) s.setAttribute('class', o.cls);
    if (o.style) s.setAttribute('style', o.style);
    const p = document.createElementNS(NS, 'path');
    p.setAttribute('d', d);
    if (o.fill) p.setAttribute('fill', 'currentColor');
    s.appendChild(p);
    return s;
  };
  SS.ic = {
    search: 'M10.5 4a6.5 6.5 0 1 0 0 13a6.5 6.5 0 0 0 0-13z M15.5 15.5L20 20',
    x: 'M6 6l12 12M18 6L6 18',
    image: 'M4 5h16v14H4z M4 15l4.5-4.5 4 4 2.5-2.5L20 17 M15.5 8.5h.01',
    text: 'M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z M14 3v5h5 M9 13h6 M9 17h4',
    code: 'M8 8l-4 4 4 4 M16 8l4 4-4 4 M13.5 5l-3 14',
    audio: 'M4 10v4 M8 7v10 M12 4v16 M16 8v8 M20 11v2',
    video: 'M3 6h12v12H3z M15 10l6-3v10l-6-3',
    file: 'M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z M14 3v5h5',
    sheet: 'M5 3h14v18H5z M5 9h14 M5 15h14 M11 9v12',
    copy: 'M9 9h11v11H9z M5 15H4V4h11v1',
    pause: 'M9 5v14 M15 5v14',
    play: 'M8 5l11 7-11 7z',
    sliders: 'M4 7h10 M18 7h2 M16 5v4 M4 17h4 M12 17h8 M10 15v4',
    check: 'M5 12l5 5L20 7',
    info: 'M12 8v5 M12 16.5h.01 M12 3a9 9 0 1 0 0 18a9 9 0 0 0 0-18z',
    folder: 'M3 6h6l2 2h10v11H3z',
    chevron: 'M6 9l6 6 6-6',
    rescan: 'M20 11a8 8 0 1 0-2.3 5.7 M20 4v7h-7',
    power: 'M12 3v9 M6.5 6.5a8 8 0 1 0 11 0'
  };

  /* ---------- движение ---------- */
  const rm = window.matchMedia ? window.matchMedia('(prefers-reduced-motion: reduce)') : null;
  const rmDev = /(^|[#&])rm($|&)/.test(location.hash);
  if (rmDev) document.documentElement.classList.add('rm');
  SS.reduced = () => rmDev || !!(rm && rm.matches);
  /* ждать конца анимации (или запасного таймера) — без циклов на JS */
  SS.after = (el, ms, fn) => {
    let done = false;
    const fin = () => { if (done) return; done = true; el.removeEventListener('animationend', onEnd); fn(); };
    const onEnd = (e) => { if (e.target === el) fin(); };
    el.addEventListener('animationend', onEnd);
    setTimeout(fin, SS.reduced() ? 0 : ms + 60);
  };
  document.documentElement.classList.toggle('doc-hidden', document.hidden);
  document.addEventListener('visibilitychange', () => document.documentElement.classList.toggle('doc-hidden', document.hidden));
  /* одноразовые входы снимаются после конца, чтобы не мешать нажатию (scale .97/.98) */
  const ONCE = { rs: 'rs', pop: 'pop', fadein: 'fadein', stin: 'enter', rowin: 'rowin' };
  document.addEventListener('animationend', (e) => {
    const c = ONCE[e.animationName];
    if (!c || !e.target.classList || !e.target.classList.contains(c)) return;
    e.target.classList.remove(c);
    e.target.style.animationDelay = '';
  });

  /* ---------- подсказка: 200 мс вход, 1,6 с показ, 150 мс выход; новая заменяет старую ---------- */
  let toastEl = null, toastT = 0;
  SS.toast = (text) => {
    clearTimeout(toastT);
    if (toastEl) toastEl.remove();
    const ico = SS.svg(SS.ic.check, { s: 13, sw: 2.6, join: true });
    ico.firstChild.setAttribute('class', 'chk');
    toastEl = SS.h('div.toast', { role: 'status', 'aria-live': 'polite' }, ico, SS.h('span', { text: text }));
    (document.querySelector('.win') || document.body).appendChild(toastEl);
    const el = toastEl;
    toastT = setTimeout(() => {
      el.classList.add('out');
      SS.after(el, 150, () => { el.remove(); if (toastEl === el) toastEl = null; });
    }, 1800);
  };
  SS.copy = async (text) => {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch (e) {
      const ta = SS.h('textarea', { style: 'position:fixed;left:-9999px;top:0' });
      ta.value = text;
      document.body.appendChild(ta);
      ta.select();
      let ok = false;
      try { ok = document.execCommand('copy'); } catch (x) { ok = false; }
      ta.remove();
      return ok;
    }
  };
  SS.openSettings = () => {
    if (SS.tauri) SS.invoke('show_settings').catch(() => {});
    else window.open('/settings', '_blank', 'noopener');
  };
})();
