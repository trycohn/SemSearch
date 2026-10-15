/* SemSearch — построение DOM главного окна. Всё из API кладётся только через textContent. */
(function () {
  'use strict';
  const { h, svg, ic } = SS;
  const R = (window.R = {});

  /* [ключ вкладки, подпись]; подпись — на языке интерфейса в момент обращения */
  const TAB_KEYS = ['all', 'text', 'code', 'image', 'video', 'audio', 'name'];
  Object.defineProperty(R, 'TABS', { get: () => TAB_KEYS.map((k) => [k, SS.t('tab.' + k)]) });
  const KINDW = (k) => SS.t('kindw.' + (['text', 'code', 'image', 'audio', 'video'].indexOf(k) >= 0 ? k : 'any'));

  /* ---------- подсветка совпадений в имени ---------- */
  const esc = (w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  R.hlRe = (q) => {
    q = String(q || '').trim();
    if (!q) return null;
    if (/^re:/i.test(q)) { try { return new RegExp(q.slice(3), 'gi'); } catch (e) { return null; } }
    const ws = q.toLowerCase().split(/[\s_—*?-]+/).filter((w) => w.length > 1 && !/[\\/]/.test(w));
    if (!ws.length) return null;
    ws.sort((a, b) => b.length - a.length);
    return new RegExp('(' + ws.map(esc).join('|') + ')', 'gi');
  };
  R.parts = (s, re) => {
    s = String(s || '');
    if (!re) return [s];
    const out = [];
    let last = 0, m, guard = 0;
    re.lastIndex = 0;
    while ((m = re.exec(s)) !== null && guard++ < 200) {
      if (!m[0]) { re.lastIndex++; continue; }
      if (m.index > last) out.push(s.slice(last, m.index));
      out.push(h('span.hl', { text: m[0] }));
      last = m.index + m[0].length;
    }
    if (last < s.length) out.push(s.slice(last));
    return out;
  };

  /* ---------- значок типа файла: по имени (ui/filetypes), нет такого — серый стандартный ---------- */
  R.fileIcon = (name, cls) => h('span.ft.ft-' + SS.ftName(name) + (cls ? '.' + cls : ''), { 'aria-hidden': 'true' });

  /* ---------- вкладки ---------- */
  R.tabs = (box, counts, tab, meta) => {
    let tl = box.querySelector('.tl');
    if (!tl) {
      tl = h('div.tl', { role: 'tablist', 'aria-label': SS.t('tabs.label') });
      R.TABS.forEach(([k, label]) => tl.appendChild(h('button.tab', { type: 'button', role: 'tab', 'aria-selected': 'false', 'data-tab': k },
        h('span.tlab', { text: label }), h('span.cnt'))));
      tl.appendChild(h('span.pill.now', { 'aria-hidden': 'true' }));
      box.appendChild(tl);
      box.appendChild(h('span.metaline', { 'data-tauri-drag-region': '' }));
    }
    tl.setAttribute('aria-label', SS.t('tabs.label'));
    tl.querySelectorAll('[data-tab]').forEach((b) => {
      const k = b.getAttribute('data-tab');
      b.querySelector('.tlab').textContent = SS.t('tab.' + k);
      b.setAttribute('aria-selected', k === tab ? 'true' : 'false');
      b.querySelector('.cnt').textContent = counts ? SS.num(counts[k] || 0) : '';
    });
    R.pill(box, tab);
    const ml = box.querySelector('.metaline');
    if (meta) ml.textContent = meta;
    ml.style.opacity = meta ? '1' : '0';
  };
  /* полоска под вкладкой: transform, 250 мс; при первом показе — без езды */
  R.pill = (box, tab, hide) => {
    const pill = box.querySelector('.pill');
    const b = box.querySelector('[data-tab="' + tab + '"]');
    if (!pill || !b || !b.offsetWidth) return;
    pill.style.transform = 'translateX(' + (b.offsetLeft + b.offsetWidth / 2 - 9) + 'px)';
    pill.style.opacity = hide ? '0' : '1';
    if (pill.classList.contains('now')) requestAnimationFrame(() => requestAnimationFrame(() => pill.classList.remove('now')));
  };
  R.metaLine = (tab, resp) => {
    if (!resp) return '';
    const c = resp.counts || {};
    const ms = ' · ' + SS.t('meta.ms', { n: SS.num(resp.ms || 0) });
    if (tab === 'name') {
      const n = c.name || 0, sc = String(resp.name_scope || 'C:');
      const many = /[,\s]/.test(sc.trim());
      return SS.tn('meta.files', n) + ' ' + SS.t(many ? 'meta.onDrives' : 'meta.onDrive', { d: sc }) + ms;
    }
    const n = tab === 'all' ? c.all || 0 : c[tab] || 0;
    if (tab === 'image') return SS.tn('meta.photos', n) + ms;
    if (tab === 'video') return SS.tn('meta.videos', n) + ms;
    return SS.tn('meta.files', n) + ms;
  };

  /* ---------- элементы списка ---------- */
  const audioSnip = (hit, it) => {
    if (hit && typeof hit.t === 'number' && typeof hit.t_end === 'number') {
      return SS.t('snip.audio', { a: SS.dur(hit.t), b: SS.dur(hit.t_end) }) + (it && it.duration ? SS.t('snip.audioLen', { d: SS.dur(it.duration) }) : '');
    }
    const t = hit ? String(hit.text || '') : '';
    const m = /^(?:отрезок|segment)\s+(.+?)[–-](.+?)\s+(?:из|of)\s+(.+)$/i.exec(t);
    if (m) return SS.t('snip.audio', { a: m[1], b: m[2] }) + SS.t('snip.audioLen', { d: m[3] });
    return t ? SS.t('snip.matched', { t: t }) : '';
  };
  const selbar = () => h('span.selbar', { 'aria-hidden': 'true' });

  R.row = (it) => {
    const k = it.kind;
    const hit = (it.hits || [])[0] || null;
    const loc = hit ? String(hit.loc || '') : '';
    const snip = k === 'audio' ? audioSnip(hit, it) : hit ? String(hit.text || '').replace(/\n\s*/g, ' ') : '';
    return h('button.row', { type: 'button', 'aria-pressed': 'false' },
      selbar(),
      R.fileIcon(it.name, 'ico'),
      h('span.bd',
        h('span.l1', h('span.nm', { text: it.name }), it.ext ? h('span.ext', { text: it.ext }) : null),
        /* точные копии считаются один раз — в выдаче одна строка и «ещё N копий» */
        h('span.cr', { text: (it.crumbs || '') + (it.copies ? ' · ' + SS.tn('row.copies', it.copies) : '') }),
        loc || snip ? h('span.l3', loc ? h('span.loc', { text: loc }) : null, h('span.snip' + (k === 'code' ? '.mono' : ''), { text: snip })) : null),
      h('span.sd', h('span', { text: SS.dateShort(it.mtime) }), h('span', { text: SS.size(it.size) })));
  };

  /* миниатюра проявляется из размытия (240 мс) по очереди: шаг 150 мс от момента показа выдачи */
  R.reveal = (img, due) => {
    const go = () => {
      img.style.animationDelay = Math.max(0, due - performance.now()) + 'ms';
      img.classList.add('th-in');
    };
    if (img.complete && img.naturalWidth) go();
    else img.addEventListener('load', go, { once: true });
  };
  R.tile = (it, big, due) => {
    const th = h('span.th');
    const url = SS.thumbUrl(it.thumb, 360);
    if (url) {
      const img = h('img', { src: url, alt: '', loading: 'lazy', decoding: 'async', draggable: 'false' });
      th.appendChild(img);
      R.reveal(img, due || 0);
    }
    if (it.kind === 'video') {
      const hit = (it.hits || [])[0];
      const b = h('span.badge.pop', svg('M2 1l7 4-7 4z', { s: 8, vb: '0 0 10 10', fill: true }), (hit && hit.loc) || SS.dur(it.duration));
      b.style.animationDelay = Math.max(0, (due || 0) - performance.now()) + 'ms';
      th.appendChild(b);
    }
    const label = big
      ? h('span.tl', h('span.tn', { text: it.name }), h('span.td', { text: SS.dateShort(it.mtime) }))
      : h('span.tn', { text: it.name });
    return h('button.tile', { type: 'button', 'aria-pressed': 'false', title: it.name }, th, label);
  };

  R.nmRow = (it, re) => h('button.row.nm-row', { type: 'button', 'aria-pressed': 'false' },
    selbar(),
    R.fileIcon(it.name, 'nico'),
    h('span.nb', h('span.nn', R.parts(it.name, re)), h('span.ncr', { text: it.crumbs || '' })),
    h('span.nsd', { text: [SS.size(it.size), SS.dateShort(it.mtime)].filter(Boolean).join(' · ') }));

  R.nRow = (it, re) => h('button.nrow', { type: 'button', 'aria-pressed': 'false', title: it.path || '' },
    selbar(),
    R.fileIcon(it.name),
    h('span.c1', R.parts(it.name, re)),
    h('span.c2', { text: it.crumbs || '' }),
    h('span.c3', { text: SS.size(it.size) }),
    h('span.c4', { text: SS.dateShort(it.mtime, true) }));

  /* ---------- колонка результатов; возвращает плоский список [item] в порядке показа ---------- */
  R.results = (box, ctx) => {
    const { tab, resp, re } = ctx;
    const items = (resp && resp.items) || [];
    const c = (resp && resp.counts) || {};
    const grid = tab === 'image' || tab === 'video';
    box.className = 'results' + (grid ? ' t-grid' : tab === 'name' ? ' t-name' : '');
    box.textContent = '';
    const flat = [];
    let step = 0, tiles = 0;
    const t0 = performance.now();
    const stag = (el) => {
      if (!ctx.anim) return el;
      el.classList.add('rs');
      el.style.animationDelay = Math.min(step++, 8) * 25 + 'ms';
      return el;
    };
    const due = () => t0 + (ctx.anim ? Math.min(tiles++, 8) * 150 : 0);
    const put = (parent, el, it) => {
      el.setAttribute('data-i', String(flat.length));
      flat.push(it);
      parent.appendChild(stag(el));
    };
    if (!items.length) {
      box.appendChild(R.empty(ctx));
      return flat;
    }
    if (tab === 'all') {
      const docs = items.filter((i) => i.kind === 'text' || i.kind === 'code');
      const media = items.filter((i) => i.kind === 'image' || i.kind === 'video');
      const audio = items.filter((i) => i.kind === 'audio');
      const names = items.filter((i) => i.kind === 'name');
      const sh = (title, note, more) => stag(h('div.sh', h('h2', { text: title }), h('span.note', { text: note }),
        more ? h('button.more', { type: 'button', 'data-go': 'image' }, SS.t('sec.allMedia')) : null));
      if (docs.length) {
        const l = h('div.list');
        docs.forEach((it) => put(l, R.row(it), it));
        box.appendChild(h('div', sh(SS.t('sec.docs'), SS.num(Math.max(docs.length, (c.text || 0) + (c.code || 0)))), l));
      }
      if (media.length) {
        const g = h('div.grid');
        media.slice(0, 5).forEach((it) => put(g, R.tile(it, false, due()), it));
        const total = Math.max(media.length, (c.image || 0) + (c.video || 0));
        box.appendChild(h('div', sh(SS.t('sec.media'), SS.num(total), !ctx.mediaOnly || total > 5), g));
      }
      if (audio.length) {
        const l = h('div.list');
        audio.forEach((it) => put(l, R.row(it), it));
        box.appendChild(h('div', sh(SS.t('sec.audio'), SS.num(Math.max(audio.length, c.audio || 0))), l));
      }
      if (names.length) {
        const l = h('div.list');
        names.forEach((it) => put(l, R.nmRow(it, re), it));
        box.appendChild(h('div', sh(SS.t('sec.names'), SS.t('sec.namesNote')), l));
      }
    } else if (grid) {
      const g = h('div.grid.big');
      items.forEach((it) => put(g, R.tile(it, true, due()), it));
      box.appendChild(g);
      if (ctx.videoNote) box.appendChild(ctx.videoNote);
    } else if (tab === 'name') {
      const l = h('div.nlist');
      items.forEach((it) => put(l, R.nRow(it, re), it));
      box.appendChild(h('div.ntable',
        h('div.nhead', h('span', { text: SS.t('nh.name') }), h('span', { text: SS.t('nh.folder') }), h('span.r', { text: SS.t('nh.size') }), h('span.rr', { text: SS.t('nh.mtime') })),
        l));
    } else {
      const l = h('div.list');
      items.forEach((it) => put(l, R.row(it), it));
      box.appendChild(l);
    }
    return flat;
  };

  R.videoNote = (q, videos, total) => {
    const named = videos.slice(0, 2).map((v) => {
      const hit = (v.hits || [])[0];
      return hit && hit.loc ? SS.t('vnote.at', { name: v.name, loc: hit.loc }) : SS.t('vnote.name', { name: v.name });
    });
    let t = q ? SS.tn('vnote.q', total, { q: q }) : SS.tn('vnote.sim', total);
    t += named.length ? ': ' + named.join(', ') + (total > named.length ? '…' : '') : '';
    return h('div.vnote', svg(ic.video, { s: 18, sw: 1.6, join: true, cap: false }), h('span.tx', { text: t }),
      h('button.btn2', { type: 'button', 'data-go': 'video' }, SS.t('vnote.open')));
  };

  const NS = 'http://www.w3.org/2000/svg';
  /* кольцо загрузки: серый круг + дуга акцента */
  R.ring = (size, sw) => {
    const s2 = document.createElementNS(NS, 'svg');
    [['width', size], ['height', size], ['viewBox', '0 0 24 24'], ['fill', 'none'], ['stroke-width', sw], ['stroke-linecap', 'round'], ['aria-hidden', 'true'], ['class', 'spin']]
      .forEach(([k, v]) => s2.setAttribute(k, v));
    const c = document.createElementNS(NS, 'circle');
    [['cx', 12], ['cy', 12], ['r', 9], ['stroke', 'var(--line2)']].forEach(([k, v]) => c.setAttribute(k, v));
    const p = document.createElementNS(NS, 'path');
    p.setAttribute('d', 'M12 3a9 9 0 0 1 9 9');
    p.setAttribute('stroke', 'var(--accent)');
    s2.append(c, p);
    return s2;
  };
  R.check = (size, sw) => {
    const c = svg(ic.check, { s: size, sw: sw, join: true });
    c.firstChild.setAttribute('class', 'chk');
    return c;
  };
  R.pendingText = (st) => {
    if (!st || !st.kinds) return '';
    const out = [];
    ['video', 'image', 'audio', 'text', 'code'].forEach((k) => {
      const v = st.kinds[k];
      const left = v ? (v.total || 0) - (v.done || 0) - (v.failed || 0) : 0;
      if (left > 0 && out.length < 2) out.push(SS.tn('kw.' + k, left));
    });
    return out.join(SS.t('none.and'));
  };
  R.modelLoading = (st, settings, done) => {
    const dev = (st && st.device) || (settings && settings.info && settings.info.device);
    if (done) return h('div.mload', R.check(22, 2.4), h('span', { text: SS.t('load.done') }));
    return h('div.mload.fadein', R.ring(22, 2.4), h('span', { text: SS.t(dev === 'cuda' ? 'load.gpu' : 'load.any') }),
      h('span.sub', { text: SS.t('load.sub') }),
      h('button.btn2', { type: 'button', 'data-go': 'name', style: 'margin-top:4px;height:32px;padding:0 12px;border:1px solid var(--line2);border-radius:4px;background:var(--surface)' }, SS.t('load.byName')));
  };
  R.empty = (ctx) => {
    if (ctx.error === 503) return R.modelLoading(ctx.status, ctx.settings, false);
    const icon = svg(ic.search + ' M8 10.5h5', { s: 26, sw: 1.7 });
    if (ctx.error) {
      return h('div.none.fadein', h('span.ic.pop', icon), h('h2', { text: SS.t('err.h') }),
        h('p', { text: ctx.errorText || SS.t('err.p') }));
    }
    const nameTab = ctx.tab === 'name';
    const pend = nameTab ? '' : R.pendingText(ctx.status);
    const pt = pend ? SS.t('none.pend', { what: pend }) : '';
    const p = nameTab ? SS.t('none.name') : (pt ? pt.charAt(0).toUpperCase() + pt.slice(1) : '') + SS.t('none.try');
    return h('div.none.fadein', h('span.ic.pop', icon), h('h2', { text: SS.t('none.h') }), h('p', { text: p }),
      h('button.btn2', { type: 'button', 'data-go': nameTab ? 'all' : 'name' }, SS.t(nameTab ? 'load.byMeaning' : 'load.byName')));
  };
  const shim = (css) => h('div.shim', { style: css });
  R.skel = (tab) => (tab === 'image' || tab === 'video'
    ? h('div.skel.fadein', h('div.skgrid', Array.from({ length: 8 }, () => shim('aspect-ratio:4/3;border-radius:6px'))))
    : h('div.skel.fadein', Array.from({ length: 5 }, () => h('div.skrow', shim('flex:none;width:36px;height:36px;border-radius:6px'),
      h('div.c', shim('height:10px;width:46%;border-radius:5px'), shim('height:8px;width:30%;border-radius:4px'), shim('height:8px;width:88%;border-radius:4px'))))));
  R.paneSkel = () => h('div.pvskel.fadein', shim('height:190px;border-radius:8px'), shim('height:14px;width:70%;border-radius:7px'),
    shim('height:10px;width:90%;border-radius:5px'), shim('height:10px;width:55%;border-radius:5px'));
  R.paneEmpty = () => h('div.pvempty.fadein', { text: SS.t('pane.empty') });

  /* ---------- панель просмотра ---------- */
  const W_HIT = [56, 100, 94, 98, 62, 100, 96, 100, 70, 100, 92, 97, 88, 40];
  const W_FIRST = [48, 100, 96, 100, 64, 100, 92, 100, 72, 100, 94, 98, 86, 44];
  const btn = (cls, act, label, key) => h('button.' + cls, { type: 'button', 'data-act': act }, h('span', { text: label }), h('span.k', { text: key }));

  R.pane = (pane, ctx) => {
    const { it, tab } = ctx;
    const pv = ctx.pv || null;
    pane.textContent = '';
    pane.hidden = false;
    /* прокручиваемое содержимое; кнопки — отдельным закреплённым блоком под ним (MainLive.dc.html) */
    const body = h('div.pvbody');
    pane.appendChild(body);
    if (!it) { body.appendChild(ctx.skel ? R.paneSkel() : R.paneEmpty()); return; }
    const root = pane;
    pane = h('div.pvin' + (ctx.anim === false ? '' : '.fadein'));
    body.appendChild(pane);
    const kind = pv && pv.kind ? pv.kind : it.kind;
    const nameTab = tab === 'name';
    const thumb = (pv && pv.thumb) || it.thumb;
    const indexed = pv ? pv.indexed !== false : it.kind !== 'name';
    const dur = (pv && pv.duration) || it.duration;
    const hits = ((pv && pv.hits && pv.hits.length ? pv.hits : it.hits) || []).filter((x) => x && (x.text || x.loc));

    /* картинка */
    const box = h('div.pvbox' + ((tab === 'image' || tab === 'video') && (kind === 'image' || kind === 'video') && thumb ? '.big' : ''));
    if ((kind === 'image' || kind === 'video') && thumb) {
      const img = h('img', { src: SS.thumbUrl(thumb, 640), alt: '', decoding: 'async', draggable: 'false' });
      img.addEventListener('load', () => {
        const r = img.naturalWidth / Math.max(1, img.naturalHeight), b = box.clientWidth / Math.max(1, box.clientHeight);
        img.classList.toggle('fit', r / b > 1.3 || b / r > 1.3);
      });
      box.appendChild(img);
      R.reveal(img, 0);
    } else if (kind === 'text' && indexed) {
      const pg = h('div.page');
      const hp = !nameTab && pv && typeof pv.hit_pos === 'number' ? 1 + Math.round(Math.max(0, Math.min(1, pv.hit_pos)) * 10) : -99;
      (nameTab ? W_FIRST : W_HIT).forEach((w, i) => {
        const on = i >= hp && i < hp + 3;
        const s = h('span' + (i === 0 ? '.t' : on ? '.h.mark' : ''));
        s.style.width = w + '%';
        if (on) s.style.animationDelay = 200 + (i - hp) * 60 + 'ms';
        pg.appendChild(s);
      });
      box.appendChild(h('div.pgw', pg, h('span.pgl', { text: nameTab ? SS.t('pv.firstPage') : (pv && pv.page_label) || '' })));
    } else if (kind === 'code' && indexed) {
      const cb = h('div.codebox');
      const lines = (pv && pv.code && pv.code.lines) || [];
      const hl = new Set((pv && pv.code && pv.code.hl) || []);
      lines.forEach((t, i) => cb.appendChild(h('span' + (hl.has(i) && !nameTab ? '.h' : ''), { text: t || ' ' })));
      box.appendChild(cb);
    } else if (kind === 'audio' && indexed) {
      const wave = h('div.wave', { 'aria-hidden': 'true' });
      const w = (pv && pv.wave) || [];
      const th0 = hits.find((x) => typeof x.t === 'number' && typeof x.t_end === 'number');
      const seg = (pv && pv.seg) || (th0 ? [th0.t, th0.t_end] : null);
      for (let i = 0; i < 44; i++) {
        const v = typeof w[i] === 'number' ? Math.max(0, Math.min(1, w[i])) : 0;
        const mid = dur ? ((i + 0.5) / 44) * dur : -1;
        const s = h('span.mark' + (seg && mid >= seg[0] && mid <= seg[1] ? '.on' : ''));
        s.style.height = 14 + Math.round(v * 82) + 'px';
        s.style.animationDelay = i * 12 + 'ms';
        wave.appendChild(s);
      }
      const lab = seg ? SS.t('pv.seg', { a: SS.dur(seg[0]), b: SS.dur(seg[1]) }) + (dur ? SS.t('pv.segOf', { d: SS.dur(dur) }) : '') : dur ? SS.t('pv.len', { d: SS.dur(dur) }) : '';
      box.appendChild(h('div.wvw', wave, h('span.wvl', { text: lab })));
    } else {
      const ext = (pv && pv.ext) || it.ext;
      const st = pv && pv.state;
      const why = st === 'pending' ? SS.t('pv.pending')
        : st === 'failed' ? SS.t('pv.failed', { why: pv.reason ? ': ' + pv.reason : '' })
        : st === 'skipped' ? SS.t('pv.skipped', { why: pv.reason ? ' — ' + pv.reason : '' })
        : st === 'outside' ? SS.t('pv.outside') : '';
      box.appendChild(h('div.nmbox',
        R.fileIcon((pv && pv.name) || it.name, 'ib.pop'),
        h('span.tx', { text: why || (ext ? SS.t('pv.noIndexExt', { ext: String(ext).toLowerCase() }) : SS.t('pv.noIndex')) })));
    }
    pane.appendChild(box);

    /* шкала видео */
    if (kind === 'video' && dur) {
      const bar = h('div.bar');
      const marks = pv && pv.marks ? pv.marks : hits.map((x) => (typeof x.t === 'number' ? x.t / dur : null));
      marks.forEach((m, i) => {
        if (typeof m !== 'number') return;
        const mk = h('span.mk.mark', { title: (hits[i] && hits[i].loc) || SS.dur(m * dur) });
        mk.style.left = Math.max(0, Math.min(1, m)) * 100 + '%';
        mk.style.animationDelay = 250 + i * 90 + 'ms';
        bar.appendChild(mk);
      });
      pane.appendChild(h('div.tline', bar, h('div.lab', h('span', { text: '0:00' }), h('span', { text: SS.dur(dur) }))));
    }

    /* имя и путь */
    const path = (pv && pv.path) || it.path || '';
    pane.appendChild(h('div',
      h('h2.pvn', { text: (pv && pv.name) || it.name }),
      h('div.pvp', h('span.p', { text: path }),
        h('button.copyb', { type: 'button', 'data-act': 'copy', 'aria-label': SS.t('act.copy'), title: SS.t('act.copyTip') },
          svg(ic.copy, { s: 15, sw: 1.7, cap: false, join: true })))));

    /* свойства */
    const meta = [];
    if (pv && pv.type) meta.push([SS.t('pv.type'), pv.type]);
    const size = pv && pv.size != null ? pv.size : it.size;
    if (size != null) meta.push([SS.t('pv.size'), SS.size(size)]);
    if (dur && (kind === 'video' || kind === 'audio')) meta.push([SS.t('pv.len2'), SS.dur(dur)]);
    if (pv && pv.taken) meta.push([SS.t('pv.taken'), SS.dateLong(pv.taken)]);
    const mt = (pv && pv.mtime) || it.mtime;
    if (mt) meta.push([SS.t('pv.mtime'), SS.dateLong(mt)]);
    if (nameTab && pv && pv.ctime) meta.push([SS.t('pv.ctime'), SS.dateLong(pv.ctime)]);
    pane.appendChild(h('div.metas', meta.map(([k, v]) => h('div', h('span.k', { text: k }), h('span.v', { text: v })))));

    /* где нашлось */
    if (!nameTab && kind !== 'image' && hits.length) {
      const title = SS.t(kind === 'video' ? 'pv.hitsVideo' : kind === 'audio' ? 'pv.hitsAudio' : 'pv.hits');
      pane.appendChild(h('div.hits', h('h3', { text: title }),
        h('div.hitl', hits.slice(0, 3).map((x, i) => {
          const d = h('div.hit.rs', h('span.hlc', { text: x.loc || '' }), h('span.hlt' + (kind === 'code' ? '.mono' : ''), { text: x.text || '' }));
          d.style.animationDelay = 120 + i * 50 + 'ms';
          return d;
        }))));
    }

    /* копии: точные (по содержимому) — на любой вкладке; просто с тем же именем — на «По имени» */
    const exact = pv && pv.dupes ? pv.dupes.filter((d) => d.exact) : [];
    const sameName = pv && pv.dupes ? pv.dupes.filter((d) => !d.exact) : [];
    const dupeNote = (list, tail) => {
      const n = list.length;
      const where = R.places(list.slice(0, 3)) + (n > 3 ? SS.t('pv.andMore', { n: n - 3 }) : '');
      pane.appendChild(h('div.dupes', { title: list.map((d) => d.path).join('\n') },
        svg(ic.info, { s: 16, sw: 1.7 }),
        h('span', { text: SS.tn('pv.copies', n) + tail + ': ' + where + '.' })));
    };
    if (pv && pv.copy_of) {
      pane.appendChild(h('div.dupes', { title: pv.copy_of }, svg(ic.info, { s: 16, sw: 1.7 }),
        h('span', { text: SS.t('pv.copyOf', { name: String(pv.copy_of).split(/[\\/]/).pop() }) })));
    }
    if (exact.length) dupeNote(exact, SS.t('pv.copiesExact'));
    else if (nameTab && sameName.length) dupeNote(sameName, SS.t('pv.copiesName'));

    /* действия */
    const canSim = pv ? !!pv.can_similar : kind === 'image' || kind === 'video';
    root.appendChild(h('div.acts' + (ctx.anim === false ? '' : '.fadein'),
      btn('prim', 'open', SS.t('act.open'), 'Enter'),
      btn('act.btn2', 'reveal', SS.t('act.reveal'), 'Ctrl+Enter'),
      nameTab ? btn('act.btn2', 'copy', SS.t('act.copy'), 'Ctrl+Shift+C') : canSim ? btn('act.btn2.pop', 'similar', SS.t('act.similar'), 'Ctrl+F') : null));
  };

  /* «на рабочем столе», «в папке «Word»» — где лежат копии */
  const PLACE = { 'Рабочий стол': 'desktop', 'Загрузки': 'downloads', 'Документы': 'documents', 'Изображения': 'pictures',
    'Видео': 'videos', 'Музыка': 'music', 'OneDrive': 'onedrive', 'Desktop': 'desktop', 'Downloads': 'downloads',
    'Documents': 'documents', 'Pictures': 'pictures', 'Videos': 'videos', 'Music': 'music' };
  R.places = (list) => {
    const p = list.map((d) => {
      const cr = String(d.crumbs || '').split(' › ').filter(Boolean);
      if (cr.length === 1 && PLACE[cr[0]]) return SS.t('place.' + PLACE[cr[0]]);
      const last = cr.length ? cr[cr.length - 1] : String(d.path || '').split(/[\\/]/).slice(-2, -1)[0] || '';
      return SS.t('place.folder', { name: last });
    });
    return p.length > 1 ? p.slice(0, -1).join(', ') + SS.t('none.and') + p[p.length - 1] : p[0] || '';
  };

  /* ---------- статус: kind = run | pause | done | load | plain ---------- */
  R.status = (s) => {
    if (!s) return { kind: 'plain', text: SS.t('st.noLink') };
    if (s.state === 'error' || (s.error && !s.ready)) return { kind: 'plain', text: SS.t('st.error', { e: s.error || SS.t('st.unknown') }) };
    if (s.state === 'loading' || !s.ready) return { kind: 'load', text: SS.t('st.loading') };
    if (s.paused || s.state === 'paused') {
      const p = s.paused || {};
      const t = p.reason === 'fullscreen' ? SS.t('st.pFull')
        : p.reason === 'process' ? SS.t('st.pProc', { p: p.detail || SS.t('st.pProcAny') })
        : p.reason === 'battery' ? SS.t('st.pBatt')
        : SS.t('st.paused');
      return { kind: 'pause', text: t };
    }
    if (s.state === 'indexing') {
      const c = s.current;
      if (c && c.total) {
        return { kind: 'run', text: SS.t('st.indexing', { what: KINDW(c.kind), a: SS.num(c.done), b: SS.num(c.total) }),
          pct: Math.max(0, Math.min(100, (c.done / c.total) * 100)), label: SS.t('st.ixLabel', { what: KINDW(c.kind) }), now: c.done, max: c.total };
      }
      return { kind: 'run', text: SS.t(s.phase === 'scan' ? 'st.scan' : 'st.ixAny') };
    }
    return { kind: 'done', text: SS.t(s.watching === false ? 'st.done' : 'st.doneWatch') };
  };
  R.namesStatus = (s) => {
    const n = (s && s.names) || null;
    if (!n) return { kind: 'plain', text: SS.t('nst.none') };
    const where = SS.drives(n.drives) || 'C:';
    if (!n.ready) {
      const r = { kind: 'run', text: SS.t('nst.collect', { where: where, n: SS.num(n.count || 0) }) };
      if (typeof n.progress === 'number') Object.assign(r, { pct: Math.max(0, Math.min(100, n.progress * 100)), label: SS.t('nst.label'), now: Math.round(n.progress * 100), max: 100 });
      return r;
    }
    return { kind: 'done', text: SS.t('nst.done', { files: SS.tn('meta.files', n.count || 0), where: where, when: n.updated ? SS.ago(n.updated) : SS.ago(Date.now() / 1000) }) };
  };
  /* слой статуса в подвале; смена вида — наплыв 200 мс (старый уезжает вверх на 6 px), цифры обновляются на месте */
  R.paintStatus = (box, info, opts) => {
    const key = info.kind + (info.pct !== undefined ? '+' : '');
    const cur = box.querySelector('.stl:not(.leave)');
    if (cur && cur.getAttribute('data-k') === key) {
      cur.querySelector('.stt').textContent = info.text;
      const pb = cur.querySelector('.pbar');
      if (pb && info.pct !== undefined) {
        pb.firstChild.style.transform = 'scaleX(' + (info.pct / 100).toFixed(4) + ')';
        pb.setAttribute('aria-valuenow', String(info.now));
        pb.setAttribute('aria-valuemax', String(info.max));
      }
      return;
    }
    const l = h('span.stl', { 'data-k': key });
    if (info.kind === 'run') l.appendChild(h('span.dot.pulse', { 'aria-hidden': 'true' }));
    if (info.kind === 'pause') l.appendChild(h('span.dot.idle', { 'aria-hidden': 'true' }));
    if (info.kind === 'load') l.appendChild(R.ring(14, 2.6));
    if (info.kind === 'done') l.appendChild(opts && opts.quiet ? svg(ic.check, { s: 14, sw: 2.2, join: true }) : R.check(14, 2.2));
    if (info.pct !== undefined) {
      const fill = h('span');
      fill.style.transform = 'scaleX(' + (info.pct / 100).toFixed(4) + ')';
      l.appendChild(h('span.pbar', { role: 'progressbar', 'aria-label': info.label, 'aria-valuemin': '0', 'aria-valuemax': String(info.max), 'aria-valuenow': String(info.now) }, fill));
    }
    l.appendChild(h('span.stt', { text: info.text }));
    if (cur && !(opts && opts.quiet)) {
      cur.classList.add('leave');
      setTimeout(() => cur.remove(), SS.reduced() ? 0 : 220);
      l.classList.add('enter');
    } else if (cur) cur.remove();
    box.appendChild(l);
  };
  const namesCount = (n) => (n >= 1e6 ? SS.t('sum.namesBig', { n: SS.bigNum(n) }) : SS.tn('sum.names', n));
  R.summary = (s) => {
    if (!s || !s.kinds) return '';
    const k = s.kinds;
    const d = (x) => (k[x] && k[x].done) || 0;
    const p = ['text', 'code', 'image', 'audio', 'video'].map((x) => SS.tn('sum.' + x, d(x)));
    if (s.names && s.names.count) p.push(SS.t('sum.on', { a: namesCount(s.names.count), b: SS.drives(s.names.drives) || 'C:' }));
    return SS.t('sum.prefix', { list: p.join(' · ') });
  };
  R.device = (s) => SS.t(!s || !s.ready ? 'dev.loading' : s.device === 'cuda' ? 'dev.gpu' : 'dev.cpu');

  /* ---------- первая индексация ---------- */
  R.eta = (sec) => {
    if (!sec || sec <= 0) return '';
    if (sec < 60) return SS.t('eta.lt1');
    const m = Math.round(sec / 60);
    if (m < 60) return SS.tn('eta.min', m);
    return SS.tn('eta.hour', Math.round(sec / 3600));
  };
  R.pending = (s) => {
    if (!s || !s.kinds) return false;
    const k = s.kinds;
    const any = Object.keys(k).some((x) => k[x] && (k[x].done || 0) + (k[x].failed || 0) < (k[x].total || 0));
    return any || (s.names && s.names.ready === false);
  };
  R.fixRows = (box, s) => {
    box.textContent = '';
    const K = ['text', 'code', 'image', 'audio', 'video'].map((k) => [k, SS.t('fix.' + k)]);
    const row = (icon, label, pct, note, on, cls) => {
      const fill = h('span' + (cls ? '.' + cls : ''));
      if (cls !== 'indet') fill.style.transform = 'scaleX(' + (pct / 100).toFixed(4) + ')';
      return h('div.fix-row', svg(icon, { s: 18, sw: 1.6, join: true }), h('span', { text: label }),
        h('span.kbar', { role: 'progressbar', 'aria-label': label, 'aria-valuemin': '0', 'aria-valuemax': '100', 'aria-valuenow': String(Math.round(pct)) }, fill),
        h('span.fix-n' + (on ? '.on' : ''), { text: note }));
    };
    K.forEach(([k, label]) => {
      const v = (s.kinds && s.kinds[k]) || { done: 0, total: 0, failed: 0 };
      const done = v.done || 0, tot = v.total || 0, proc = done + (v.failed || 0);
      const cur = s.current && s.current.kind === k;
      if (tot === 0 || proc >= tot) box.appendChild(row(ic[k], label, tot ? 100 : 0, tot ? SS.t('fix.done', { n: SS.num(done) }) : SS.t('fix.none'), false, 'done'));
      else if (proc > 0 || cur) {
        const pct = Math.round((proc / tot) * 100);
        box.appendChild(row(ic[k], label, pct, SS.t('fix.of', { a: SS.num(done), b: SS.num(tot) }), true, ''));
      } else box.appendChild(row(ic[k], label, 0, SS.t('fix.queued', { n: SS.num(tot) }), false, ''));
    });
    const n = s.names || { count: 0, drives: ['C:'], ready: false };
    const nl = SS.t('fix.names', { where: SS.drives(n.drives) || 'C:' });
    box.appendChild(n.ready ? row(ic.file, nl, 100, SS.t('fix.done', { n: SS.bigNum(n.count || 0) }), false, 'done')
      : typeof n.progress === 'number' ? row(ic.file, nl, Math.round(n.progress * 100), SS.t('fix.collect', { n: SS.bigNum(n.count || 0) }), true, '')
      : row(ic.file, nl, 0, SS.t('fix.collect', { n: SS.bigNum(n.count || 0) }), true, 'indet'));
  };
  R.keys = (box, hotkey, tauri) => {
    box.textContent = '';
    [[SS.caps(hotkey || 'ctrl+alt+s'), SS.t('key.hotkey')],
      [['Enter'], SS.t('key.enter')], [['Ctrl', 'Enter'], SS.t('key.reveal')], [['↑', '↓'], SS.t('key.arrows')],
      [['Ctrl', '1…7'], SS.t('key.tabs')], [['Esc'], SS.t(tauri ? 'key.escTray' : 'key.escClear')]
    ].forEach(([caps, what]) => box.appendChild(h('div.krow',
      h('span.kcaps', caps.map((c) => h('kbd.kc', { text: c }))), h('span.w', { text: what }))));
  };
})();
