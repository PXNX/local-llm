"""Sound effects and the final mix: dialogue + effects + a quiet music bed that ducks under speech.

Explosions and the rocket launch come from soundfx/generate_sfx.py (flow 5), whoosh/pop and the
backing track from topvideos/soundtrack.py (flow 9); boing, rimshot, crickets, sad trombone and
slap are synthesized here. Any WAV in soundfx/out/ can be used by name too (e.g. "slava_ukraini").
"""
import sys
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "soundfx"))
sys.path.insert(0, str(REPO / "topvideos"))
import generate_sfx as fx  # noqa: E402
import soundtrack as st  # noqa: E402
from voice import read_wav, resample, write_wav  # noqa: E402

SR = 44100
SFX_DIR = REPO / "soundfx" / "out"


def _t(dur):
    return np.arange(int(dur * SR)) / SR


def boing():
    t = _t(0.7)
    f = 180 + 260 * np.exp(-t * 5) * (1 + 0.25 * np.sin(2 * np.pi * 14 * t))
    return np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t * 4) * np.minimum(1, t * 300)


def rimshot(rng):
    """Ba-dum-tss."""
    out = np.zeros(int(1.3 * SR))
    for at, f in ((0.0, 150), (0.16, 110)):
        t = _t(0.25)
        tom = np.sin(2 * np.pi * np.cumsum(f * (1 + 0.6 * np.exp(-t * 25))) / SR) * np.exp(-t * 12)
        st._add(out, tom, at, 0.8)
    st._add(out, st.snare(rng), 0.32, 1.2)
    t = _t(0.9)
    cym = fx.bandpass_noise(t.size, rng, 5000, 15000) * np.exp(-t * 4)
    st._add(out, cym, 0.32, 0.5)
    return out


def crickets(rng):
    out = np.zeros(int(2.0 * SR))
    for k in range(6):
        t = _t(0.12)
        chirp = np.sin(2 * np.pi * 4300 * t) * (0.5 + 0.5 * np.sin(2 * np.pi * 60 * t)) * np.sin(np.pi * t / 0.12)
        st._add(out, chirp, 0.15 + k * 0.3 + rng.uniform(0, 0.04), 0.25)
    return out


def sad_trombone():
    out = np.zeros(int(2.6 * SR))
    for i, (m, d) in enumerate(((58, 0.4), (57, 0.4), (56, 0.4), (55, 1.3))):
        t = _t(d)
        vib = 1 + (0.012 * np.sin(2 * np.pi * 6 * t) if i == 3 else 0)
        f = st._hz(m) * vib
        ph = 2 * np.pi * np.cumsum(f) / SR
        tone = sum(np.sin(h * ph) / h for h in range(1, 8))  # buzzy brass-ish
        env = np.minimum(1, t / 0.04) * np.minimum(1, (d - t) / 0.08)
        st._add(out, tone * env, i * 0.42, 0.35)
    return out


def slap(rng):
    t = _t(0.12)
    return fx.bandpass_noise(t.size, rng, 800, 6000) * np.exp(-t * 60)


def ding(root=60):
    return st.chime(root)


def effect(name, rng):
    """Mono effect by name, or None."""
    makers = {
        "explosion": lambda: fx.mine_explosion(rng),
        "big_explosion": lambda: fx.shahed_impact(rng),
        "launch": lambda: fx.patriot_launch(rng, duration=1.6),
        "whoosh": lambda: st.whoosh(rng, 0.5),
        "pop": st.pop,
        "boing": boing,
        "rimshot": lambda: rimshot(rng),
        "crickets": lambda: crickets(rng),
        "sad_trombone": sad_trombone,
        "ding": ding,
        "slap": lambda: slap(rng),
    }
    if name in makers:
        return np.asarray(makers[name](), dtype=np.float64)
    for p in sorted(SFX_DIR.glob("*.wav")) if SFX_DIR.is_dir() else []:
        if name.lower() in p.stem.lower():
            a, sr = read_wav(p)
            return resample(a, sr, SR).astype(np.float64)
    return None


GAIN = {"explosion": 0.8, "big_explosion": 0.9, "launch": 0.5, "whoosh": 0.5, "pop": 0.45, "boing": 0.35,
        "rimshot": 0.5, "crickets": 0.6, "sad_trombone": 0.5, "ding": 0.5, "slap": 0.6}


def mix(path, total, lines, effects, music="calm", seed=0):
    """lines: (start, mono audio, pan -1..1); effects: (start, name). Writes a 16-bit stereo WAV."""
    n = int(round(total * SR))
    out = np.zeros((2, n + 3 * SR))
    speech = np.zeros(n + 3 * SR)
    for at, audio, pan in lines:
        pan = max(-1.0, min(1.0, pan)) * 0.35
        st._add(out, np.stack([audio * (1 - pan), audio * (1 + pan)]), at)
        st._add(speech, np.abs(audio), at)
    rng = np.random.default_rng(seed)
    for at, name in effects:
        a = effect(name, rng)
        if a is None:
            print(f"[sound  ] unknown effect {name!r}, skipped")
            continue
        a = a / max(1e-6, np.abs(a).max()) * GAIN.get(name, 0.5)
        st._add(out, np.stack([a, a]), max(0.0, at))
    out = out[:, :n]
    if music != "none":
        bed, _ = st.music(total, "funny" if music == "funny" else "cute", seed)
        # duck: fast attack, slow release envelope of the dialogue
        env = speech[:n]
        k = int(0.05 * SR)
        env = np.convolve(env, np.ones(k) / k, mode="same")
        talking = np.clip(env / 0.02, 0, 1)
        k = int(0.4 * SR)
        talking = np.convolve(talking, np.ones(k) / k, mode="same")
        gain = 0.16 - 0.10 * talking
        out += bed[:, :n] * gain
    out = np.tanh(out * 1.1)
    peak = np.abs(out).max()
    if peak > 0:
        out *= 0.89 / peak
    write_wav(path, out.astype(np.float32))
    return path
