"""Music for the sketch, one mood per scene: the composer and instruments of topvideos/soundtrack.py
plus brass, bowed strings, pizzicato, cymbals and toms. Every mood uses the same key (from the seed),
so the crossfades between scenes don't clash. Pure numpy, a few seconds of CPU per minute of music.

deadpan    major, 88 bpm, sparse music box, soft kick + woodblock, quiet pad
bouncy     the funny track of soundtrack.py: 124 bpm pluck lead, full drums, octave bass
tense      minor, 96 bpm, low staccato string ostinato, heartbeat kick, ticking hats, tremolo, no lead
pompous    major march, 108 bpm, oom-pah tuba, brass fanfare, snare rolls, cymbals
sad        minor, 64 bpm, slow clarinet over a pad, no drums
sneaky     minor, 112 bpm swing, walking pizzicato bass, tiptoe pizzicato, swung hats, snaps
triumphant major, 120 bpm, brass + big pad, driving octave bass, big drums, crashes, tom fills
"""
import random
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "topvideos"))
import soundtrack as st  # noqa: E402

SR = st.SR
MOODS = ["deadpan", "bouncy", "tense", "pompous", "sad", "sneaky", "triumphant"]
MAJOR = st.MAJOR
MINOR = [0, 2, 3, 5, 7, 8, 10]
RMS = 0.11  # every bed is levelled to this, so scene changes don't jump in loudness


def key(seed):
    """The root note soundtrack.music() picks for this seed (so 'bouncy' is in the same key)."""
    return 48 + random.Random(seed).choice([0, 2, 5, 7])


def _chord(root, degree, scale):
    return [root + scale[(degree + s) % 7] + 12 * ((degree + s) // 7) for s in (0, 2, 4)]


def _put(buf, sig, at, gain=1.0, pan=0.0):
    st._add(buf[0], sig, at, gain * (1 - pan * 0.6))
    st._add(buf[1], sig, at, gain * (1 + pan * 0.6))


# ---------------------------------------------------------------- extra instruments
def brass(midi, dur, bright=1.0):
    """Saw-like stack whose upper harmonics swell in (a brassy 'blat'), slight vibrato."""
    t = st._t(dur)
    f = st._hz(midi) * (1 + 0.004 * np.sin(2 * np.pi * 5.5 * t) * np.minimum(1, t / 0.4))
    ph = 2 * np.pi * np.cumsum(f) / SR
    swell = np.minimum(1, t / 0.07)
    s = sum(np.sin(h * ph) / h * swell ** (1 + 0.5 * h / bright) for h in range(1, 9))
    return s * np.minimum(1, t / 0.02) * np.minimum(1, (dur - t) / 0.06) * 0.6


def bowed(midi, dur):
    """Short bowed low string (staccato cello)."""
    t = st._t(dur)
    f = st._hz(midi)
    s = sum(np.sin(2 * np.pi * f * h * t) / h for h in range(1, 7))
    return s * np.minimum(1, t / 0.012) * np.exp(-t * 5) * np.minimum(1, (dur - t) / 0.03) * 0.7


def pizz(midi, dur):
    t = st._t(dur)
    f = st._hz(midi)
    s = np.sin(2 * np.pi * f * t) + 0.5 * np.sin(4 * np.pi * f * t) * np.exp(-t * 20) + 0.2 * np.sin(6 * np.pi * f * t)
    return s * np.exp(-t * 9) * np.minimum(1, t * 600)


def woodblock():
    t = st._t(0.08)
    return np.sin(2 * np.pi * 1150 * t) * np.exp(-t * 70)


def tom(midi):
    t = st._t(0.45)
    f = st._hz(midi) * (1 + 0.5 * np.exp(-t * 20))
    return np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t * 7)


def crash(rng, dur=1.6):
    t = st._t(dur)
    noise = np.diff(rng.standard_normal(t.size), prepend=0)
    return noise * np.exp(-t * 2.5) * np.minimum(1, t * 2000) * 0.25


def roll(rng, dur):
    """Snare roll (32nds) with a crescendo."""
    out = np.zeros(int((dur + 0.2) * SR))
    n = max(1, int(dur / 0.06))
    for i in range(n):
        st._add(out, st.snare(rng), i * dur / n, 0.25 + 0.75 * i / n)
    return out


# ---------------------------------------------------------------- moods
def _deadpan(out, b, t0, beat, root, rng, nrng, d):
    ch = _chord(root, [0, 5, 3, 4][b % 4], MAJOR)
    _put(out, d["k"], t0, 0.3)
    _put(out, woodblock(), t0 + 2 * beat, 0.12, 0.4)
    _put(out, st.bass(ch[0] - 12, beat * 1.5), t0, 0.45)
    _put(out, st.pad([m + 12 for m in ch], beat * 4), t0, 0.16)
    if b % 2 == 0:
        for pos in d["motif"][:2]:
            _put(out, st.bell(ch[rng.randrange(3)] + 24, beat * 2), t0 + pos * beat, 0.22, -0.3)


def _tense(out, b, t0, beat, root, rng, nrng, d):
    ch = _chord(root, [0, 0, 5, 6][b % 4], MINOR)
    lo = ch[0] - 12
    for e, step in enumerate([0, 0, 12, 0, 0, 1, 0, 3]):
        _put(out, bowed(lo + step, beat * 0.45), t0 + e * beat / 2, 0.5 if e % 2 == 0 else 0.35)
    for q in range(4):  # heartbeat
        _put(out, d["k"], t0 + q * beat, 0.45)
        if q % 2 == 0:
            _put(out, d["k"], t0 + (q + 0.3) * beat, 0.25)
    for s in range(16):
        _put(out, st.hat(nrng), t0 + s * beat / 4, 0.25 if s % 4 == 0 else 0.12, 0.5)
    p = st.pad([m + 24 for m in ch] + [ch[0] + 25], beat * 4)  # + a minor second on top: uneasy
    p *= 0.6 + 0.4 * np.sin(2 * np.pi * 11 * np.arange(p.size) / SR)  # tremolo
    _put(out, p, t0, 0.12)
    if b % 4 == 3:
        for i in range(4):
            _put(out, tom(lo + 12 - i * 2), t0 + (3 + i / 4) * beat, 0.3 + 0.1 * i)


def _pompous(out, b, t0, beat, root, rng, nrng, d):
    ch = _chord(root, [0, 3, 4, 0][b % 4], MAJOR)
    for q in range(4):
        if q in (0, 2):  # oom ...
            _put(out, d["k"], t0 + q * beat, 0.55)
            _put(out, brass(ch[0] - 24 + (7 if q == 2 else 0), beat * 0.8, 0.5), t0 + q * beat, 0.5)
        else:  # ... pah
            _put(out, d["sn"], t0 + q * beat, 0.3)
            for m in ch:
                _put(out, brass(m, beat * 0.35), t0 + q * beat, 0.12, 0.3)
    if b % 4 == 0:
        _put(out, crash(nrng), t0, 0.5, -0.3)
    if b % 4 == 3:
        _put(out, roll(nrng, beat * 0.9), t0 + 3 * beat, 0.35)
    if b % 4 < 3:  # fanfare: dotted rhythm on chord tones
        tones = ch + [ch[0] + 12]
        for pos, length, idx in d["fanfare"][b % 2]:
            _put(out, brass(tones[idx] + 12, beat * length * 0.9), t0 + pos * beat, 0.32, -0.2)


def _sad(out, b, t0, beat, root, rng, nrng, d):
    ch = _chord(root, [0, 5, 3, 4][b % 4], MINOR)
    _put(out, st.pad([m + 12 for m in ch], beat * 4.2), t0, 0.35)
    _put(out, st.pad([ch[0] - 12], beat * 4.2), t0, 0.35)
    tones = sorted(ch + [ch[0] + 12], reverse=True)
    for i, pos in enumerate((0, 2) if b % 4 != 3 else (0,)):
        _put(out, st.clarinet(tones[(b + i) % 4] + 12, beat * (1.9 if b % 4 != 3 else 3.8)), t0 + pos * beat, 0.4, -0.2)


def _sneaky(out, b, t0, beat, root, rng, nrng, d):
    deg = [0, 3, 0, 4][b % 4]
    ch = _chord(root, deg, MINOR)
    nxt = _chord(root, [0, 3, 0, 4][(b + 1) % 4], MINOR)[0]
    walk = [ch[0], ch[1], ch[2], nxt + (1 if nxt <= ch[2] else -1)]  # chromatic approach to the next root
    swing = beat * 2 / 3
    for q, m in enumerate(walk):
        _put(out, pizz(m - 12, beat * 0.9), t0 + q * beat, 0.6)
        _put(out, st.hat(nrng), t0 + q * beat, 0.35, 0.4)
        _put(out, st.hat(nrng), t0 + q * beat + swing, 0.6, 0.4)
        if q in (1, 3):
            _put(out, d["cl"], t0 + q * beat, 0.12, -0.4)  # finger snap
    if b % 2 == 0:  # tiptoe line
        for pos, step in d["tiptoe"]:
            m = root + 24 + MINOR[step % 7]
            _put(out, pizz(m, beat * 0.4), t0 + int(pos) * beat + (swing if pos % 1 else 0), 0.3, 0.3)


def _triumphant(out, b, t0, beat, root, rng, nrng, d):
    ch = _chord(root, [0, 4, 5, 3][b % 4], MAJOR)
    for q in range(4):
        _put(out, d["k"], t0 + q * beat, 0.7 if q in (0, 2) else 0.4)
        if q in (1, 3):
            _put(out, d["sn"], t0 + q * beat, 0.35)
            _put(out, d["cl"], t0 + q * beat, 0.25)
    for e in range(8):
        _put(out, st.bass(ch[0] - 12 + (12 if e % 2 else 0), beat * 0.45), t0 + e * beat / 2, 0.45)
    _put(out, st.pad([m + 12 for m in ch] + [ch[0] + 24], beat * 4), t0, 0.35)
    if b % 4 == 0:
        _put(out, crash(nrng), t0, 0.6, 0.3)
    if b % 4 == 3:
        for i in range(4):
            _put(out, tom(55 - i * 4), t0 + (3 + i / 4) * beat, 0.5)
    tones = ch + [ch[0] + 12]
    for pos, idx in ((0, 3), (2, d["lead"][b % 4])):
        _put(out, brass(tones[idx] + 12, beat * 1.9), t0 + pos * beat, 0.3, -0.2)


SPEC = {"deadpan": (88, _deadpan), "tense": (96, _tense), "pompous": (108, _pompous), "sad": (64, _sad),
        "sneaky": (112, _sneaky), "triumphant": (120, _triumphant)}


def bed(mood, total, seed=0):
    """Stereo bed of `total` seconds in the given mood (starts on a downbeat), levelled to RMS."""
    n = int(round(total * SR))
    if mood == "bouncy" or mood not in SPEC:
        out = st.music(total, "funny", seed)[0]
    else:
        bpm, part = SPEC[mood]
        rng, nrng = random.Random(seed), np.random.default_rng(seed)
        root = key(seed)
        beat = 60 / bpm
        d = {"k": st.kick(), "sn": st.snare(nrng), "cl": st.clap(nrng),
             "motif": sorted(rng.sample([0, 0.5, 1, 1.5, 2, 2.5, 3], 3)),
             "fanfare": [[(0, 0.75, 0), (0.75, 0.25, 1), (1, 1, 2), (2, 2, rng.choice([2, 3]))],
                         [(0, 0.75, 2), (0.75, 0.25, 1), (1, 1, 0), (2, 2, rng.choice([1, 3]))]],
             "tiptoe": [(p, rng.randrange(7)) for p in sorted(rng.sample([0, 0.5, 1, 1.5, 2, 2.5, 3, 3.5], 4))],
             "lead": [rng.choice([1, 2]) for _ in range(4)]}
        out = np.zeros((2, n + 3 * SR))
        for b in range(int(np.ceil(total / (beat * 4)))):
            part(out, b, b * beat * 4, beat, root, rng, nrng, d)
        out = out[:, :n]
    rms = np.sqrt(np.mean(out ** 2)) if out.size else 0
    return out * (RMS / rms) if rms > 0 else out


def sting(mood, seed=0):
    """Short stereo punchline sting: 'ta-daa' (major moods) or 'dun-dun-duuun' (minor moods)."""
    root = key(seed)
    nrng = np.random.default_rng(seed)
    out = np.zeros((2, int(2.6 * SR)))
    if mood in ("tense", "sad", "sneaky"):
        for i, (m, at, d) in enumerate(((root, 0, 0.25), (root - 1, 0.32, 0.25), (root - 5, 0.64, 1.6))):
            for o in (0, -12):
                _put(out, brass(m + o, d, 0.7), at, 0.35)
            _put(out, tom(m - 12), at, 0.5)
    else:
        ch = _chord(root, 0, MAJOR)
        for at, d in ((0, 0.14), (0.2, 1.4)):
            for m in ch + [ch[0] + 12]:
                _put(out, brass(m + 12, d), at, 0.2)
        _put(out, crash(nrng), 0.2, 0.7)
        _put(out, st.kick(), 0.2, 0.6)
    return out
