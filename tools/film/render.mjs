// Renders film.html in headless Edge (or Chrome) over the DevTools protocol. Node 20+ (built-in WebSocket).
//   node render.mjs snap   <lang> <outdir> <t1> [t2 ...]      one PNG per time, named t<seconds>.png
//   node render.mjs frames <lang> <outdir> <from> <to> [workers] [sub]
//        subframes from..to-1 (sub per frame, centred on the frame time) as s<index>.png; stops writing ahead
//        while more than 1200 PNGs wait in outdir, so make_film.py can blend and delete them as they come.
//   node render.mjs cues   <lang> <out.json>                    the film's sound cues and duration
// <lang> is ru or en; ru:v / en:v render the upright 1080x1920 cut for stories and reels.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const FPS = 60;

function findBrowser() {
  const env = process.env;
  const c = [env.SEMSEARCH_FILM_BROWSER,
    path.join(env['ProgramFiles(x86)'] || '', 'Microsoft/Edge/Application/msedge.exe'),
    path.join(env.ProgramFiles || '', 'Microsoft/Edge/Application/msedge.exe'),
    path.join(env.ProgramFiles || '', 'Google/Chrome/Application/chrome.exe')];
  for (const p of c) if (p && fs.existsSync(p)) return p;
  throw new Error('no Edge or Chrome found; set SEMSEARCH_FILM_BROWSER');
}

const sleep = ms => new Promise(r => setTimeout(r, ms));

class Page {
  static async open(token) {
    const [lang, fmt = 'h'] = token.split(':'), W = fmt === 'v' ? 1080 : 1920, H = fmt === 'v' ? 1920 : 1080;
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ssfilm-'));
    const proc = spawn(findBrowser(), ['--headless=new', '--remote-debugging-port=0', `--user-data-dir=${dir}`,
      '--no-first-run', '--no-default-browser-check', '--hide-scrollbars', '--mute-audio', '--allow-file-access-from-files',
      '--force-device-scale-factor=1', `--window-size=${W},${H}`, '--disable-background-timer-throttling',
      '--disable-renderer-backgrounding', 'about:blank'], {stdio: 'ignore'});
    let port = null;
    for (let i = 0; i < 200 && !port; i++) {
      await sleep(50);
      try { port = fs.readFileSync(path.join(dir, 'DevToolsActivePort'), 'utf8').split('\n')[0].trim(); } catch {}
    }
    if (!port) throw new Error('browser did not start');
    let list = [];
    for (let i = 0; i < 100 && !list.some(t => t.type === 'page'); i++) {
      try { list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json(); } catch {}
      if (!list.some(t => t.type === 'page')) await sleep(50);
    }
    const p = new Page(proc, dir, list.find(t => t.type === 'page').webSocketDebuggerUrl);
    await p.connect();
    await p.send('Page.enable');
    await p.send('Emulation.setDeviceMetricsOverride', {width: W, height: H, deviceScaleFactor: 1, mobile: false});
    const url = pathToFileURL(path.join(HERE, 'film.html')).href + `?lang=${lang}&fmt=${fmt}`;
    await p.send('Page.navigate', {url});
    for (let i = 0; i < 400; i++) {
      const r = await p.eval('window.READY === true');
      if (r === true) return p;
      await sleep(50);
    }
    throw new Error('film did not become READY');
  }
  constructor(proc, dir, ws) { this.proc = proc; this.dir = dir; this.wsUrl = ws; this.id = 0; this.wait = new Map(); }
  connect() {
    return new Promise((res, rej) => {
      this.ws = new WebSocket(this.wsUrl);
      this.ws.onopen = res; this.ws.onerror = rej;
      this.ws.onmessage = ev => {
        const m = JSON.parse(ev.data);
        if (m.id && this.wait.has(m.id)) {
          const [ok, bad] = this.wait.get(m.id); this.wait.delete(m.id);
          m.error ? bad(new Error(m.error.message)) : ok(m.result);
        }
      };
    });
  }
  send(method, params = {}) {
    const id = ++this.id;
    return new Promise((ok, bad) => { this.wait.set(id, [ok, bad]); this.ws.send(JSON.stringify({id, method, params})); });
  }
  async eval(expr) {
    const r = await this.send('Runtime.evaluate', {expression: expr, returnByValue: true, awaitPromise: true});
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || 'eval failed');
    return r.result.value;
  }
  async shot(t, file) {
    await this.eval(`seek(${t}); 0`);
    const r = await this.send('Page.captureScreenshot', {format: 'png', fromSurface: true});
    fs.writeFileSync(file + '.part', Buffer.from(r.data, 'base64'));
    fs.renameSync(file + '.part', file);
  }
  close() {
    try { this.ws.close(); } catch {}
    try { this.proc.kill(); } catch {}
    setTimeout(() => { try { fs.rmSync(this.dir, {recursive: true, force: true}); } catch {} }, 1500);
  }
}

const [mode, lang, out, ...rest] = process.argv.slice(2);
if (mode === 'snap') {
  fs.mkdirSync(out, {recursive: true});
  const p = await Page.open(lang);
  for (const t of rest) await p.shot(+t, path.join(out, `t${(+t).toFixed(2).padStart(5, '0')}.png`));
  p.close();
} else if (mode === 'cues') {
  const p = await Page.open(lang);
  fs.writeFileSync(out, JSON.stringify(await p.eval('window.M'), null, 1));
  p.close();
} else if (mode === 'frames') {
  fs.mkdirSync(out, {recursive: true});
  const from = +rest[0], to = +rest[1], workers = +(rest[2] || 3), sub = +(rest[3] || 4);
  const subT = i => Math.floor(i / sub) / FPS + ((i % sub) - (sub - 1) / 2) / (FPS * sub);
  let next = from;
  const pending = () => fs.readdirSync(out).filter(f => f.endsWith('.png')).length;
  async function worker() {
    let p = await Page.open(lang);
    for (;;) {
      while (pending() > 1200) await sleep(200);
      const i = next++;
      if (i >= to) break;
      const file = path.join(out, `s${String(i).padStart(6, '0')}.png`);
      for (let a = 0; ; a++) {
        try { await p.shot(subT(i), file); break; } catch (e) {
          if (a >= 3) throw e;
          try { p.close(); } catch {}
          p = await Page.open(lang);
        }
      }
    }
    p.close();
  }
  await Promise.all(Array.from({length: workers}, worker));
  fs.writeFileSync(path.join(out, 'DONE'), '');
} else {
  console.error('usage: node render.mjs snap|frames|cues ...');
  process.exit(2);
}
process.exit(0);
