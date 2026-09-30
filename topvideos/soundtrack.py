"""Copyright-free audio for the top-5 video: a backing track composed and synthesized here from
scratch (numpy), plus whoosh/pop/click effects. Nothing is sampled from other recordings, so there
is nothing for YouTube Content ID or TikTok to match.

The track length is a whole number of bars (tempo is nudged to fit), so when the video loops the
music loops with it without a jump.
"""
import random
import wave

import numpy as np

SR = 44100

# chord progressions (scale degrees of a major key), picked per video
PROGRESSIONS = [[0, 4, 5, 3], [0, 5, 3, 4], [5, 3, 0, 4], [0, 3, 4, 3]]
MAJOR = [0, 2, 4, 5, 7, 9, 11]


def _t(dur):
    return np.arange(int(dur * SR)) / SR


def _hz(midi):
    return 440.0 * 2 ** ((midi - 69) / 12)


def _add(buf, sig, at, gain=1.0):
    i = int(at * SR)
    if i >= buf.shape[-1]:
        return
    n = min(sig.shape[-1], buf.shape[-1] - i)
    buf[..., i:i + n] += sig[..., :n] * gain


def _chord(root_midi, degree):
    """Triad on a scale degree, as MIDI notes."""
    notes = []
    for step in (0, 2, 4):
        d = degree + step
        notes.append(root_midi + MAJOR[d % 7] + 12 * (d // 7))
    return notes


# ---------------------------------------------------------------- instruments
def pluck(midi, dur, bright=1.0):
    """Marimba/pluck: a few harmonics with fast exponential decay."""
    t = _t(dur)
    f = _hz(midi)
    s = np.zeros_like(t)
    for h, a in ((1, 1.0), (2, 0.35 * bright), (3, 0.12 * bright), (4, 0.25 * bright)):
        s += a * np.sin(2 * np.pi * f * h * t) * np.exp(-t * (6 + 5 * h))
    return s * np.minimum(1, t * 400)


def bell(midi, dur):
    """Music box / glockenspiel: inharmonic partials, long ring."""
    t = _t(dur)
    f = _hz(midi)
    s = sum(a * np.sin(2 * np.pi * f * r * t) * np.exp(-t * d)
            for r, a, d in ((1, 1.0, 3), (2.76, 0.4, 6), (5.4, 0.2, 10), (8.93, 0.1, 14)))
    return s * np.minimum(1, t * 800)


def bass(midi, dur):
    t = _t(dur)
    f = _hz(midi)
    s = np.sin(2 * np.pi * f * t) + 0.3 * np.sin(4 * np.pi * f * t)
    return s * np.exp(-t * 3) * np.minimum(1, t * 300) * np.minimum(1, (dur - t) * 60)


def pad(notes, dur):
    t = _t(dur)
    s = np.zeros_like(t)
    for m in notes:
        f = _hz(m)
        for det in (-0.12, 0.12):  # slight detune for width
            ff = f * 2 ** (det / 12)
            s += np.sin(2 * np.pi * ff * t) + 0.25 * np.sin(4 * np.pi * ff * t) + 0.1 * np.sin(6 * np.pi * ff * t)
    env = np.minimum(1, t / 0.08) * np.minimum(1, (dur - t) / 0.12)
    return s * env / (len(notes) * 2)


def kick():
    t = _t(0.35)
    f = 45 + 110 * np.exp(-t * 30)
    return np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t * 9)


def snare(rng):
    t = _t(0.2)
    noise = rng.standard_normal(t.size)
    noise = np.diff(noise, prepend=0)  # brighter
    return (0.6 * noise * np.exp(-t * 22) + 0.5 * np.sin(2 * np.pi * 185 * t) * np.exp(-t * 30)) * 0.5


def clap(rng):
    t = _t(0.18)
    noise = np.diff(rng.standard_normal(t.size), prepend=0)
    env = sum(np.exp(-np.maximum(0, t - o) * 60) * (t >= o) for o in (0, 0.011, 0.022)) / 3
    return noise * env * 0.5


def hat(rng, open_=False):
    t = _t(0.25 if open_ else 0.05)
    noise = np.diff(np.diff(rng.standard_normal(t.size), prepend=0), prepend=0)
    return noise * np.exp(-t * (18 if open_ else 90)) * 0.05


# ---------------------------------------------------------------- effects
def whoosh(rng, dur=0.45):
    """Filtered-noise swoosh rising and falling (card transitions)."""
    n = int(dur * SR)
    noise = rng.standard_normal(n)
    t = np.arange(n) / n
    cutoff = 0.02 + 0.35 * np.sin(np.pi * t) ** 2  # one-pole low-pass coefficient over time
    out = np.empty(n)
    y = 0.0
    for i in range(n):
        y += cutoff[i] * (noise[i] - y)
        out[i] = y
    return out * np.sin(np.pi * t) ** 1.5 * 2.2


def pop():
    t = _t(0.09)
    f = 380 + 900 * t / 0.09
    return np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t * 45) * 0.8


def click(rng):
    t = _t(0.03)
    return (np.sin(2 * np.pi * 2300 * t) * np.exp(-t * 300) + 0.4 * rng.standard_normal(t.size) * np.exp(-t * 500)) * 0.7


def chime(root_midi):
    """Little 'ding-ding' after the subscribe click."""
    s = np.zeros(int(0.9 * SR))
    for k, m in enumerate((root_midi + 24 + 7, root_midi + 24 + 12)):
        _add(s, bell(m, 0.8), k * 0.11)
    return s * 0.5


# ---------------------------------------------------------------- composer
def music(total, topic="funny", seed=0):
    """Stereo backing track of exactly `total` seconds, a whole number of bars long."""
    rng = random.Random(seed)
    nrng = np.random.default_rng(seed)
    base_bpm = 124 if topic == "funny" else 100
    bars = max(4, round(total / (240 / base_bpm)))
    bar = total / bars
    beat = bar / 4
    root = 48 + rng.choice([0, 2, 5, 7])  # C, D, F or G
    prog = rng.choice(PROGRESSIONS)
    # a 2-bar lead motif (8ths/16ths over chord tones), repeated with variations every 8 bars
    motif = [(rng.choice([0, 0.5, 1, 1.5, 2, 2.5, 3, 3.5]), rng.choice([0, 1, 2, 3])) for _ in range(6)]
    motif = sorted(set(motif))
    n = int(round(total * SR))
    tail = 2 * SR  # room for notes ringing past the end
    L, R = np.zeros(n + tail), np.zeros(n + tail)
    k, sn, cl = kick(), snare(nrng), clap(nrng)
    lead = pluck if topic == "funny" else bell
    for b in range(bars):
        t0 = b * bar
        deg = prog[b % len(prog)]
        ch = _chord(root, deg)
        section = (b // 8) % 2  # alternate a lighter and a fuller 8-bar section
        # drums
        for q in range(4):
            if topic == "funny" or q in (0, 2):
                _add(L, k, t0 + q * beat, 0.6)
                _add(R, k, t0 + q * beat, 0.6)
            if q in (1, 3):
                s = sn if topic == "funny" else cl
                _add(L, s, t0 + q * beat, 0.22)
                _add(R, s, t0 + q * beat, 0.22)
        for e in range(8):
            h = hat(nrng, open_=(e % 4 == 3 and section == 1))
            _add(L, h, t0 + e * beat / 2, 0.9 if e % 2 else 0.5)
            _add(R, h, t0 + e * beat / 2, 0.5 if e % 2 else 0.9)
        # bass: root on 1 and 3, bouncy octave on the offbeats in the funny style
        for q, off, o in ((0, 0, 0), (2, 0, 0), (1, 0.5, 12), (3, 0.5, 12)):
            if o and topic != "funny":
                continue
            bs = bass(ch[0] - 12, beat * (0.9 if not o else 0.4))
            _add(L, bs, t0 + (q + off) * beat, 0.55)
            _add(R, bs, t0 + (q + off) * beat, 0.55)
        # pad, ducked by the kick
        p = pad([m + 12 for m in ch], bar)
        duck = 1 - 0.55 * np.exp(-((np.arange(p.size) / SR) % beat) * 12)
        _add(L, p * duck, t0, 0.32)
        _add(R, p * duck, t0, 0.32)
        # lead
        tones = ch + [ch[0] + 12]
        for pos, idx in motif:
            if (b % 2 == 1) and pos < 2:
                continue  # second bar of the motif: answer in the back half
            if section == 0 and (b % 4 == 3):
                continue  # breathe
            m = tones[idx] + 24 + (12 if section and idx == 3 else 0)
            _add(L, lead(m, beat * 1.5), t0 + pos * beat, 0.42)
            _add(R, lead(m, beat * 1.5), t0 + pos * beat, 0.32)
    # the tail of the last notes wraps around to the start, so the loop point is seamless
    L[:tail] += L[n:]
    R[:tail] += R[n:]
    return np.stack([L[:n], R[:n]]), root


def soundtrack(path, total, events, topic="funny", seed=0):
    """events: list of (seconds, kind) with kind in whoosh/pop/click. Writes a 16-bit stereo WAV."""
    mix, root = music(total, topic, seed)
    rng = np.random.default_rng(seed + 1)
    for at, kind in events:
        if kind == "whoosh":
            w = whoosh(rng)
            _add(mix, np.stack([w * 0.9, w * 0.7]), max(0.0, at - 0.2), 0.35)
        elif kind == "pop":
            _add(mix, np.stack([pop(), pop()]), at, 0.5)
        elif kind == "click":
            c = click(rng)
            _add(mix, np.stack([c, c]), at, 0.8)
            ch = chime(root)
            _add(mix, np.stack([ch, ch]), at + 0.08, 0.6)
    mix = np.tanh(mix * 1.2)  # gentle saturation / limiting
    mix *= 0.89 / max(1e-6, np.abs(mix).max())
    pcm = (mix.T * 32767).astype(np.int16)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())
    return path
