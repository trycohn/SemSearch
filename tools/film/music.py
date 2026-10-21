"""Original score and UI sounds for the launch film, synthesized from scratch (no samples, no licences).

    python music.py <cues.json> <out.wav>

120 BPM in A minor, chords Am F C G, one bar = 2 s:
  0-4 s    intro: filtered pad, plucked arpeggio, soft pulse
  4-8 s    build: a pitched roll 8ths -> 16ths -> 32nds under the typing, a swell, a breath before the drop
  8-22 s   drop on Enter: four-on-the-floor, a pitched backbeat, offbeat bass with sidechain, chord stabs, arpeggio,
           lead from 16 s
  22-26 s  the four facts land on soft chord hits, soft toms into the logo
  26-30 s  the logo: one big C chord, a sub and a long tail
No white noise in the mix except the reverb tail: no hats, cymbals, claps or noise risers, and no bells.
UI sounds come from the film's own cues (render.mjs cues) and every one is shifted so its measured peak lands on its cue.
"""
import json
import sys

import numpy as np
from scipy import signal

SR = 48000
BEAT = 0.5
DUR = 30.0
N = int(SR * DUR)
rng = np.random.default_rng(7)


def midi(m):
    return 440.0 * 2 ** ((m - 69) / 12)


def tt(dur):
    return np.arange(int(SR * dur)) / SR


def noise(dur):
    return rng.standard_normal(int(SR * dur))


def sos(kind, f, order=2, q=None):
    if kind == 'bp':
        return signal.butter(order, f, btype='bandpass', fs=SR, output='sos')
    return signal.butter(order, f, btype=kind, fs=SR, output='sos')


def filt(x, kind, f, order=2):
    return signal.sosfilt(sos(kind, f, order), x)


def saw(f, dur, phase=0.0):
    """Band-limited sawtooth (polyBLEP)."""
    n = int(SR * dur)
    dt = f / SR
    p = (phase + dt * np.arange(n)) % 1.0
    y = 2 * p - 1
    m = p < dt
    x = p[m] / dt
    y[m] -= x + x - x * x - 1
    m = p > 1 - dt
    x = (p[m] - 1) / dt
    y[m] -= x * x + x + x + 1
    return y


def env(n, a=0.005, d=0.1, s=0.7, r=0.2, hold=None):
    """ADSR over n samples; release starts at hold (s) or at the end minus r."""
    t = np.arange(n) / SR
    hold = (n / SR - r) if hold is None else hold
    e = np.where(t < a, t / max(a, 1e-6), s + (1 - s) * np.exp(-(t - a) / max(d, 1e-6)))
    rel = t > hold
    e[rel] *= np.exp(-(t[rel] - hold) / max(r / 4, 1e-6))
    return e


class Bus:
    def __init__(self):
        self.x = np.zeros((N, 2))

    def add(self, t0, y, gain=1.0, pan=0.0):
        i = int(round(t0 * SR))
        if y.ndim == 1:
            l, r = np.cos((pan + 1) * np.pi / 4), np.sin((pan + 1) * np.pi / 4)
            y = np.stack([y * l * 1.414, y * r * 1.414], 1)
        j0, j1 = max(0, i), min(N, i + len(y))
        if j1 > j0:
            self.x[j0:j1] += gain * y[j0 - i:j1 - i]


# ------------------------------------------------------------------ instruments
# No white noise anywhere: hats, cymbals, claps, noise risers and noise whooshes read as sand. Every sound below is
# built from band-limited oscillators and sines, and brightness changes by crossfading two fixed filters instead of
# moving a filter block by block (that zippers).

def square(f, dur, phase=0.0):
    """Band-limited square: the difference of two polyBLEP saws half a period apart."""
    return (saw(f, dur, phase) - saw(f, dur, phase + 0.5)) * 0.5


def glide(f0, f1, dur, curve=1.0):
    """Phase of a sine gliding from f0 to f1 (log scale)."""
    u = (np.arange(int(SR * dur)) / (SR * dur)) ** curve
    return 2 * np.pi * np.cumsum(f0 * (f1 / f0) ** u) / SR


def kick(level=1.0):
    t = tt(0.55)
    f = 46 + 120 * np.exp(-t / 0.032)
    y = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t / 0.32) * np.minimum(1, t / 0.002)
    y += np.sin(2 * np.pi * 1100 * t) * np.exp(-t / 0.0015) * 0.12          # a clean tick for definition
    return np.tanh(1.4 * y) * level


def snap(level=1.0):
    """Backbeat on 2 and 4: a short pitched body, like a tight tom, no noise."""
    t = tt(0.25)
    y = np.sin(2 * np.pi * np.cumsum(200 + 140 * np.exp(-t / 0.02)) / SR) * np.exp(-t / 0.07)
    y += np.sin(2 * np.pi * np.cumsum(330 + 200 * np.exp(-t / 0.015)) / SR) * np.exp(-t / 0.045) * 0.5
    return y * np.minimum(1, t / 0.0015) * level


def tom(f, lv=1.0):
    t = tt(0.3)
    fr = f + f * 0.6 * np.exp(-t / 0.03)
    return np.sin(2 * np.pi * np.cumsum(fr) / SR) * np.exp(-t / 0.11) * np.minimum(1, t / 0.003) * lv


def boom(dur=2.0):
    t = tt(dur)
    f = 32 + 40 * np.exp(-t / 0.18)
    y = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t / 0.9) * np.minimum(1, t / 0.004)
    return np.tanh(1.3 * y)


def swell(dur, f0=220, f1=660, curve=1.4):
    """A rising swell from two detuned sines and their fifth, for builds and into the logo."""
    u = tt(dur) / dur
    y = np.sin(glide(f0, f1, dur, curve)) + np.sin(glide(f0 * 1.006, f1 * 1.006, dur, curve))
    y += 0.4 * np.sin(glide(f0 * 1.5, f1 * 1.5, dur, curve))
    return y * 0.4 * u ** 2.2


def supersaw(m_list, dur, cutoff=2400, a=0.01, d=0.25, s=0.6, r=0.25, voices=5, spread=0.16):
    """Detuned saw chord, stereo, lowpassed."""
    out = np.zeros((int(SR * dur), 2))
    for m in m_list:
        for v in range(voices):
            det = (v - (voices - 1) / 2) / ((voices - 1) / 2) * spread
            y = saw(midi(m + det), dur, rng.random())
            pan = (v - (voices - 1) / 2) / ((voices - 1) / 2) * 0.7
            out[:, 0] += y * np.cos((pan + 1) * np.pi / 4)
            out[:, 1] += y * np.sin((pan + 1) * np.pi / 4)
    out /= voices * len(m_list) ** 0.5
    e = env(len(out), a, d, s, r)
    for c in range(2):
        out[:, c] = filt(filt(out[:, c] * e, 'lowpass', cutoff, 2), 'lowpass', cutoff * 1.6, 2)
    return out


def pad(m_list, dur, f0, f1):
    """Soft saw pad that opens from cutoff f0 to f1 (two fixed filters, crossfaded)."""
    raw = np.zeros((int(SR * dur), 2))
    for m in m_list:
        for v in range(4):
            det = (v - 1.5) * 0.09
            y = saw(midi(m + det), dur, rng.random())
            pan = (v - 1.5) / 1.5 * 0.8
            raw[:, 0] += y * np.cos((pan + 1) * np.pi / 4)
            raw[:, 1] += y * np.sin((pan + 1) * np.pi / 4)
    raw /= 4 * len(m_list) ** 0.5
    raw *= env(len(raw), 0.35, 1.0, 1.0, 0.5)[:, None]
    u = (tt(dur) / dur)[:, None]
    lo = np.stack([filt(raw[:, c], 'lowpass', f0, 2) for c in range(2)], 1)
    hi = np.stack([filt(raw[:, c], 'lowpass', f1, 2) for c in range(2)], 1)
    return lo * (1 - u) + hi * u


def pluck(m, dur=0.32, bright=3200, level=1.0):
    """Saw + square, a bright layer that dies fast over a darker body: the pluck without a moving filter."""
    t = tt(dur)
    y = saw(midi(m), dur) * 0.6 + square(midi(m), dur) * 0.3
    body = filt(y, 'lowpass', 700, 2)
    top = filt(y, 'lowpass', bright, 2) - body
    out = body * np.exp(-t / 0.16) + top * np.exp(-t / 0.045)
    return out * np.minimum(1, t / 0.002) * level


def bass(m, dur):
    t = tt(dur)
    y = saw(midi(m), dur) * 0.55 + np.sin(2 * np.pi * midi(m - 12) * t) * 0.8
    y = filt(y, 'lowpass', 520, 2)
    return y * env(len(t), 0.004, 0.12, 0.75, 0.05)


def lead(m, dur):
    t = tt(dur)
    vib = 1 + 0.004 * np.sin(2 * np.pi * 5.5 * t) * np.minimum(1, t / 0.2)
    ph = np.cumsum(midi(m) * vib) / SR
    y = filt(2 * (ph % 1) - 1, 'lowpass', 2200, 2) * 0.6 + np.sin(2 * np.pi * ph) * 0.4
    return y * env(len(t), 0.012, 0.15, 0.65, 0.08)


def reverb_ir(rt=1.9, dur=2.6):
    t = tt(dur)
    ir = np.stack([noise(dur), noise(dur)], 1) * np.exp(-6.9 * t / rt)[:, None]
    for c in range(2):
        ir[:, c] = filt(ir[:, c], 'lowpass', 3800, 2)
    ir[: int(0.012 * SR)] = 0
    return ir / np.sqrt((ir ** 2).sum(0))


# ------------------------------------------------------------------ the score

CHORDS = {'Am': ([57, 60, 64, 69], 45), 'F': ([53, 57, 60, 65], 41), 'C': ([55, 60, 64, 67], 48), 'G': ([55, 59, 62, 67], 43)}
PROG = ['Am', 'F', 'C', 'G']
PENTA = [57, 60, 62, 64, 67, 69, 72, 74, 76, 79, 81]   # A minor pentatonic


def chord_at(bar):
    return PROG[bar % 4]


def score():
    drums, music, send = Bus(), Bus(), Bus()
    kicks = []

    def K(t, lv=1.0):
        drums.add(t, kick(lv), 0.95)
        kicks.append((t, lv))

    # intro: pad opening up, pluck arpeggio, soft pulse
    for bar in range(2):
        ch, root = CHORDS[chord_at(bar)]
        p = pad(ch, 2.3, 500 + bar * 500, 900 + bar * 700)
        music.add(bar * 2, p, 0.30)
        send.add(bar * 2, p, 0.25)
        for k in range(8):
            m = ch[[0, 1, 2, 3, 2, 1, 3, 2][k]] + 12
            y = pluck(m, 0.3, 3400 + k * 150, 0.5)
            music.add(bar * 2 + k * 0.25, y, 0.26, pan=-0.4 if k % 2 else 0.4)
            send.add(bar * 2 + k * 0.25, y, 0.22)
        K(bar * 2, 0.45)
        K(bar * 2 + 1.0, 0.3)

    # build: 4-8 s, a pitched roll that climbs under the typing, and a swell
    for bar in (2, 3):
        ch, root = CHORDS[chord_at(bar)]
        p = pad(ch, 2.1 if bar == 3 else 2.3, 1500, 3200 if bar == 3 else 2400)
        music.add(bar * 2, p, 0.30)
        send.add(bar * 2, p, 0.25)
        for k in range(4):
            K(bar * 2 + k * 0.5, 0.7 if bar == 2 else 0.85)
        for k in range(8):
            m = ch[[0, 1, 2, 3, 2, 1, 3, 2][k]] + 12
            music.add(bar * 2 + k * 0.25, pluck(m, 0.28, 4200, 0.5), 0.24, pan=-0.4 if k % 2 else 0.4)
        if bar == 3:                                    # offbeat bass creeps in
            for k in range(4):
                music.add(bar * 2 + k * 0.5 + 0.25, bass(root, 0.2), 0.32)
    roll = [4.0 + k * 0.25 for k in range(8)] + [6.0 + k * 0.125 for k in range(8)] + [7.0 + k * 0.0625 for k in range(12)]
    for i, t in enumerate(roll):
        u = i / len(roll)
        drums.add(t, tom(150 * 2 ** (u * 1.2), 0.3 + 0.7 * u ** 1.5), 0.42, pan=(-0.2, 0.2)[i % 2])
    music.add(5.0, swell(2.75, 196, 784), 0.22)
    send.add(5.0, swell(2.75, 196, 784), 0.15)
    # (7.75 - 8.0: a breath, nothing new starts)

    # drop: 8-22 s, and the facts 22-26 s
    stab_pat = [0, 0.75, 1.5, 2.25, 3.0, 3.5]          # in beats inside a bar (4 beats)
    for bar in range(4, 13):
        t0 = bar * 2.0
        ch, root = CHORDS[chord_at(bar)]
        for k in range(4):
            K(t0 + k * 0.5, 1.0)
            if k in (1, 3):
                drums.add(t0 + k * 0.5, snap(), 0.30)
                send.add(t0 + k * 0.5, snap(), 0.08)
        for k in range(4):                              # offbeat bass
            music.add(t0 + k * 0.5 + 0.25, bass(root, 0.22), 0.55)
            music.add(t0 + k * 0.5 + 0.375, bass(root + 12, 0.1), 0.22)
        for b in stab_pat:
            st = supersaw([m + 12 for m in ch[:3]], 0.3, cutoff=3000, d=0.1, s=0.35, r=0.12)
            music.add(t0 + b * 0.5, st, 0.36)
            send.add(t0 + b * 0.5, st, 0.18)
        p = pad(ch, 2.05, 2000, 2000)
        music.add(t0, p, 0.16)
        for k in range(16):                             # arpeggio 16ths over the chord
            m = ch[k % 4] + 12 + (12 if k % 8 >= 4 else 0)
            music.add(t0 + k * 0.125, pluck(m, 0.2, 4800, 0.4), 0.16, pan=-0.5 if k % 2 else 0.5)
    # the drop lands on a long chord and a sub instead of a cymbal
    hit = supersaw([m + 12 for m in CHORDS['Am'][0][:3]], 1.2, cutoff=2600, a=0.004, d=0.5, s=0.2, r=0.4)
    music.add(8.0, hit, 0.30)
    send.add(8.0, hit, 0.30)
    drums.add(8.0, boom(1.4), 0.40)
    # lead from 16 s (bars 8-10): a short motif in A minor pentatonic
    motif = [(0, 76, 0.75), (0.75, 74, 0.25), (1.0, 72, 0.5), (1.5, 69, 0.5), (2.0, 72, 1.0), (3.0, 74, 0.5), (3.5, 76, 0.5),
             (4.0, 79, 1.0), (5.0, 76, 0.5), (5.5, 74, 0.5), (6.0, 76, 1.5), (7.5, 72, 0.5)]
    for b, m, l in motif:
        y = lead(m, l * 0.5 + 0.05)
        music.add(16.0 + b * 0.5, y, 0.17, pan=0.1)
        send.add(16.0 + b * 0.5, y, 0.22)
    music.add(17.25, swell(0.7, 330, 990), 0.12)
    # facts: a soft chord hit on every fact
    for t in (22.0, 23.0, 24.0, 25.0):
        ch, root = CHORDS[chord_at(int(t // 2))]
        hit = supersaw([m + 12 for m in ch[:3]], 0.95, cutoff=1400, a=0.006, d=0.3, s=0.1, r=0.3)
        music.add(t, hit, 0.20)
        send.add(t, hit, 0.14)
        drums.add(t, boom(1.0), 0.24)
    # soft toms and a warm swell into the logo, then a breath
    for i, t in enumerate([25.0 + k * 0.125 for k in range(4)] + [25.5 + k * 0.0625 for k in range(4)]):
        drums.add(t, tom(110 * 2 ** (i / 8), 0.5 + 0.5 * i / 8), 0.32, pan=(-0.3, 0.3)[i % 2])
    music.add(24.6, swell(1.15, 196, 523), 0.16)

    # the logo: 26 s, one big chord and a sub, no cymbal
    big = supersaw([48, 55, 60, 64, 67, 71, 74], 4.0, cutoff=3400, a=0.005, d=1.2, s=0.45, r=1.6, voices=7, spread=0.2)
    music.add(26.0, big, 0.55)
    send.add(26.0, big, 0.45)
    music.add(26.0, bass(36, 3.6) * np.exp(-tt(3.6) / 1.6), 0.7)
    K(26.0, 1.0)
    drums.add(26.0, boom(2.4), 0.55)
    for i, (dt, m) in enumerate([(0.5, 72), (1.0, 76), (1.25, 79), (2.0, 84)]):
        y = pluck(m, 0.6, 4200, 0.6)
        music.add(26.0 + dt, y, 0.13, pan=(-0.4, 0.4, -0.2, 0.2)[i])
        send.add(26.0 + dt, y, 0.30)

    # sidechain the music bus under the kicks
    t = np.arange(N) / SR
    duck = np.ones(N)
    for tk, lv in kicks:
        i = int(tk * SR)
        seg = np.arange(N - i) / SR
        g = 1 - 0.55 * lv * lv * np.exp(-seg / 0.11)
        duck[i:] = np.minimum(duck[i:], g)
    music.x *= duck[:, None]

    ir = reverb_ir()
    wet = np.stack([signal.fftconvolve(send.x[:, c], ir[:, c])[:N] for c in range(2)], 1)
    mix = drums.x + music.x + wet * 0.55
    fade = np.clip((DUR - 0.15 - t) / 2.4, 0, 1) ** 1.5
    fade[t < 26] = 1
    return mix * fade[:, None]


# ------------------------------------------------------------------ UI sounds

def sfx_lib():
    def tick(f=1500, d=0.004, lv=1.0):              # a clean "tk": a sine burst with a tiny pitch drop
        t = tt(max(0.03, d * 8))
        return np.sin(2 * np.pi * np.cumsum(f * (1 + 0.3 * np.exp(-t / 0.002))) / SR) * np.exp(-t / d) * lv

    def swoosh(d, f0, f1, lv=1.0, rev=False):        # a soft tonal glide instead of a noise whoosh
        t = tt(d)
        e = np.sin(np.pi * t / d) ** 2
        if rev:                                     # swells into its end, then stops
            e = (t / d) ** 3 * np.minimum(1, (d - t) / 0.01)
        y = np.sin(glide(f0, f1, d)) + 0.5 * np.sin(glide(f0 * 1.5, f1 * 1.5, d))
        return y * e * 0.5 * lv

    def thump(f0=110, f1=50, d=0.25):
        t = tt(d)
        f = f1 + (f0 - f1) * np.exp(-t / 0.04)
        return np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t / 0.08) * np.minimum(1, t / 0.002)

    def blip(m, d=0.16, lv=1.0):
        t = tt(d)
        f = midi(m) * (1 + 0.25 * np.exp(-t / 0.01))
        return np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t / 0.05) * np.minimum(1, t / 0.0015) * lv

    def mx(*ys):                                        # sum sounds of different lengths
        out = np.zeros(max(len(y) for y in ys))
        for y in ys:
            out[:len(y)] += y
        return out

    lib = {
        'blip': lambda v: blip(76, 0.3, 0.6),
        'pop': lambda v: blip(PENTA[v % len(PENTA)] + 12, 0.14, 0.8),
        'suck': lambda v: swoosh(0.42, 900, 220, 0.6, rev=True),
        'key': lambda v: mx(thump(160, 90, 0.08) * 0.8, tick(1700, 0.004, 0.35)),
        'morph': lambda v: swoosh(0.4, 260, 700, 0.35),
        'type': lambda v: mx(tick(1300 + 160 * ((v * 7) % 5), 0.003, 0.28), thump(240 + 30 * (v % 3), 160, 0.03) * 0.3),
        'enter': lambda v: mx(thump(140, 70, 0.12) * 0.5, tick(1400, 0.005, 0.3)),
        'row': lambda v: blip(PENTA[4 + v] + 12, 0.08, 0.3),
        'mark': lambda v: swoosh(0.3, 500, 1100, 0.18),
        'cap': lambda v: blip(72, 0.2, 0.3),
        'collapse': lambda v: swoosh(0.3, 600, 240, 0.3),
        'scrub': lambda v: swoosh(0.3, 300, 800, 0.25),
        'pull': lambda v: mx(swoosh(0.9, 700, 110, 0.5), thump(80, 35, 0.6) * 0.6),
        'tick': lambda v: tick(1900, 0.002, 0.2),
        'slam': lambda v: mx(thump(120, 55, 0.25) * 0.8, tick(1100, 0.005, 0.25)),
        'zoomin': lambda v: swoosh(0.45, 200, 700, 0.4),
        'fact': lambda v: thump(110, 55, 0.3) * 0.7,
        'suck2': lambda v: swoosh(0.45, 700, 200, 0.4, rev=True),
        'logo': lambda v: thump(90, 40, 0.8) * 0.8,
        'reveal': lambda v: tick(1500 + 90 * v, 0.003, 0.15),
        'slogan': lambda v: blip(72, 0.25, 0.25),
    }
    return lib


def place_sfx(cues):
    lib = sfx_lib()
    bus = Bus()
    peaks = {}
    for c in cues:
        y = lib[c['k']](c.get('v', 0))
        y = np.asarray(y, dtype=float)
        pk = int(np.argmax(np.abs(y)))
        pan = 0.0
        if c['k'] == 'pop':
            pan = [-0.6, 0.5, -0.45, 0.6, -0.1, 0.4, -0.3, 0.15][c['v'] % 8]
        elif c['k'] == 'type':
            pan = ((c['v'] * 37) % 7 - 3) / 12
        bus.add(c['t'] - pk / SR, y, 1.0, pan)
        peaks.setdefault(c['k'], pk / SR)
    return bus.x, peaks


# ------------------------------------------------------------------ loudness (ITU-R BS.1770, gated)

def lufs(x):
    k = signal.lfilter([1.53512485958697, -2.69169618940638, 1.19839281085285], [1, -1.69065929318241, 0.73248077421585], x, axis=0)
    k = signal.lfilter([1.0, -2.0, 1.0], [1, -1.99004745483398, 0.99007225036621], k, axis=0)
    blk, hop = int(0.4 * SR), int(0.1 * SR)
    ms = np.array([np.mean(k[i:i + blk] ** 2, axis=0).sum() for i in range(0, len(x) - blk, hop)])
    ld = -0.691 + 10 * np.log10(ms + 1e-12)
    g = ms[ld > -70]
    rel = -0.691 + 10 * np.log10(g.mean()) - 10
    g = ms[(ld > -70) & (ld > rel)]
    return -0.691 + 10 * np.log10(g.mean())


def main():
    cues = json.load(open(sys.argv[1], encoding='utf-8'))['cues']
    out = sys.argv[2]
    song = score()
    sfx, peaks = place_sfx(cues)
    mix = song * 0.5 + sfx * 0.62
    mix *= 0.93 / np.abs(mix).max()
    import wave
    pcm = (np.clip(mix, -1, 1) * 32767).astype('<i2')
    with wave.open(out, 'wb') as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())
    print('wrote %s  peak %.2f dBFS  ~%.1f LUFS  cues %d' % (out, 20 * np.log10(np.abs(mix).max()), lufs(mix), len(cues)))


if __name__ == '__main__':
    main()
