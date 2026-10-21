"""Builds the SemSearch launch film: render -> motion blur -> H.264 60 fps -> soundtrack -> checks.

    python tools/film/make_film.py [--lang ru,en] [--sub 4] [--workers 3] [--crf 16] [--audio-only]

--audio-only keeps the picture of the existing out/semsearch-<lang>.mp4 and only rebuilds and remuxes the sound.

Needs Node 20+, Edge or Chrome, and the project's .venv (numpy, scipy, Pillow, PyAV with libx264).
For each language it writes to tools/film/out/:
  semsearch-<lang>.mp4        1920x1080, 60 fps, H.264 + AAC
  storyboard-<lang>.png       one frame per second, taken from the MP4
  poster-<lang>.png           the end card
and prints the loudness and the biggest frame-to-frame jumps (cuts there should only be intended hits).
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor

import av
import numpy as np
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, 'out')
FPS = 60


def node(*args, **kw):
    return subprocess.Popen(['node', os.path.join(HERE, 'render.mjs'), *args], **kw)


def load(path):
    for _ in range(200):
        try:
            with Image.open(path) as im:
                return np.asarray(im.convert('RGB'), dtype=np.uint16)
        except (FileNotFoundError, PermissionError, OSError):
            time.sleep(0.05)
    raise RuntimeError('cannot read ' + path)


def wait_for(path, proc):
    while not os.path.exists(path):
        if proc.poll() is not None and not os.path.exists(path):
            raise RuntimeError('renderer exited (%s) before %s' % (proc.returncode, path))
        time.sleep(0.02)


def encode_video(lang, nframes, sub, workers, crf, dst):
    tmp = os.path.join(OUT, 'tmp-' + lang)
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp)
    proc = node('frames', lang, tmp, '0', str(nframes * sub), str(workers), str(sub))
    out = av.open(dst, 'w')
    st = out.add_stream('libx264', rate=FPS)
    st.width, st.height, st.pix_fmt = 1920, 1080, 'yuv420p'
    st.options = {'crf': str(crf), 'preset': 'medium', 'x264-params': 'colorprim=bt709:transfer=bt709:colormatrix=bt709:range=tv'}
    st.codec_context.color_range = 1          # MPEG (limited)
    st.codec_context.colorspace = 1           # BT.709
    st.codec_context.color_primaries = 1
    st.codec_context.color_trc = 1
    fmt = av.video.reformatter.VideoReformatter()
    pool = ThreadPoolExecutor(4)
    names = lambda i: [os.path.join(tmp, 's%06d.png' % (i * sub + k)) for k in range(sub)]

    def frame(i):
        ps = names(i)
        for p in ps:
            wait_for(p, proc)
        acc = sum(load(p) for p in ps)
        for p in ps:
            os.remove(p)
        return ((acc + sub // 2) // sub).astype(np.uint8)   # tmix: the mean of the subframes is the motion blur

    t0 = time.time()
    futs = {i: pool.submit(frame, i) for i in range(min(nframes, 8))}
    for i in range(nframes):
        rgb = futs.pop(i).result()
        if i + 8 < nframes:
            futs[i + 8] = pool.submit(frame, i + 8)
        vf = av.VideoFrame.from_ndarray(rgb, format='rgb24')
        vf = fmt.reformat(vf, format='yuv420p', src_colorspace='itu709', dst_colorspace='itu709',
                          src_color_range='JPEG', dst_color_range='MPEG')
        vf.pts = i
        for pkt in st.encode(vf):
            out.mux(pkt)
        if i % 120 == 0:
            print('  %s frame %d/%d  %.0fs' % (lang, i, nframes, time.time() - t0), flush=True)
    for pkt in st.encode():
        out.mux(pkt)
    out.close()
    proc.wait()
    shutil.rmtree(tmp, ignore_errors=True)


def mux(video, wav, dst):
    vin, ain = av.open(video), av.open(wav)
    out = av.open(dst, 'w', options={'movflags': '+faststart'})
    vs = out.add_stream_from_template(vin.streams.video[0])
    a = out.add_stream('aac', rate=48000)
    a.layout = 'stereo'
    a.bit_rate = 192000
    for pkt in vin.demux(vin.streams.video[0]):
        if pkt.dts is None:
            continue
        pkt.stream = vs
        out.mux(pkt)
    for fr in ain.decode(audio=0):
        fr.pts = None
        for pkt in a.encode(fr):
            out.mux(pkt)
    for pkt in a.encode():
        out.mux(pkt)
    out.close()
    vin.close()
    ain.close()


def check(mp4, lang):
    """Storyboard (1 per second), the poster, and a frame-difference scan of the finished MP4."""
    c = av.open(mp4)
    prev, diffs, shots = None, [], []
    for i, fr in enumerate(c.decode(video=0)):
        small = np.asarray(fr.to_image().resize((320, 180)), dtype=np.int16)
        if prev is not None:
            diffs.append((float(np.abs(small - prev).mean()), i))
        prev = small
        if i % FPS == 30:
            shots.append(fr.to_image())
        if i == int(28.5 * FPS):
            fr.to_image().save(os.path.join(OUT, 'poster-%s.png' % lang))
    c.close()
    cols, tw, th = 6, 480, 270
    rows = (len(shots) + cols - 1) // cols
    sheet = Image.new('RGB', (cols * tw, rows * th), 'white')
    for k, im in enumerate(shots):
        sheet.paste(im.resize((tw, th), Image.LANCZOS), ((k % cols) * tw, (k // cols) * th))
    sheet.save(os.path.join(OUT, 'storyboard-%s.png' % lang))
    top = sorted(diffs, reverse=True)[:8]
    print('  biggest frame jumps: ' + ', '.join('%.2fs %.1f' % (i / FPS, d) for d, i in top))
    return len(diffs) + 1


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--lang', default='ru,en')
    ap.add_argument('--sub', type=int, default=4)
    ap.add_argument('--workers', type=int, default=3)
    ap.add_argument('--crf', type=int, default=16)
    ap.add_argument('--audio-only', action='store_true')
    a = ap.parse_args()
    os.makedirs(OUT, exist_ok=True)
    sys.path.insert(0, HERE)
    for lang in a.lang.split(','):
        cues = os.path.join(OUT, 'cues-%s.json' % lang)
        node('cues', lang, cues).wait()
        meta = json.load(open(cues, encoding='utf-8'))
        wav = os.path.join(OUT, 'mix-%s.wav' % lang)
        subprocess.check_call([sys.executable, os.path.join(HERE, 'music.py'), cues, wav])
        silent = os.path.join(OUT, 'video-%s.mp4' % lang)
        final = os.path.join(OUT, 'semsearch-%s.mp4' % lang)
        t0 = time.time()
        if a.audio_only:
            os.replace(final, silent)               # mux() takes only the video stream from it
        else:
            encode_video(lang, int(round(meta['dur'] * FPS)), a.sub, a.workers, a.crf, silent)
        mux(silent, wav, final)
        os.remove(silent)
        n = check(final, lang)
        print('%s: %s  %d frames  %.1f MB  %.0fs' % (lang, final, n, os.path.getsize(final) / 1e6, time.time() - t0))


if __name__ == '__main__':
    main()
