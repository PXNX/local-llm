"""Flow 5: procedural game sound effects (explosions, rocket motor) + Slava Ukraini voice line.

No AI model involved - explosions and the rocket motor are synthesized from noise/tone
layers with envelopes and filters (numpy/scipy, already bundled with ComfyUI's embedded
python). The voice line is not synthesized: it is trimmed/normalized from a real recorded
clip you supply.
"""
import argparse
import wave
from pathlib import Path

import numpy as np
from scipy import signal

SR = 44100


# ---------- low-level helpers ----------

def write_wav(path, audio, sr=SR):
    audio = np.clip(audio, -1.0, 1.0)
    pcm = (audio * 32767).astype(np.int16)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(pcm.tobytes())


def normalize(audio, peak=0.95):
    m = np.max(np.abs(audio))
    return audio * (peak / m) if m > 0 else audio


def bandpass_noise(n, rng, low, high, sr=SR):
    noise = rng.standard_normal(n)
    sos = signal.butter(4, [low, high], btype="band", fs=sr, output="sos")
    return signal.sosfilt(sos, noise)


def lowpass_noise(n, rng, cutoff, sr=SR):
    noise = rng.standard_normal(n)
    sos = signal.butter(4, cutoff, btype="low", fs=sr, output="sos")
    return signal.sosfilt(sos, noise)


def exp_env(n, sr, attack_ms, tau):
    t = np.arange(n) / sr
    env = np.exp(-t / tau)
    a = int(sr * attack_ms / 1000)
    if a > 1:
        env[:a] *= np.linspace(0, 1, a)
    return env


def pitch_thump(n, sr, f_start, f_end, tau, attack_ms=2):
    freq = np.linspace(f_start, f_end, n)
    phase = 2 * np.pi * np.cumsum(freq) / sr
    return np.sin(phase) * exp_env(n, sr, attack_ms, tau)


# ---------- sound designs ----------

def make_explosion(rng, duration, crack_hp, sub_f0, sub_f1, sub_tau, rumble_tau, rumble_mix, sr=SR):
    n = int(duration * sr)
    crack = bandpass_noise(n, rng, *crack_hp, sr) * exp_env(n, sr, 1, 0.03)
    sub = pitch_thump(n, sr, sub_f0, sub_f1, sub_tau)
    early_rumble = lowpass_noise(n, rng, 700, sr) * exp_env(n, sr, 3, rumble_tau * 0.4)
    late_rumble = lowpass_noise(n, rng, 180, sr) * exp_env(n, sr, 20, rumble_tau)
    mix = crack * 0.9 + sub * 1.0 + early_rumble * 0.5 + late_rumble * rumble_mix
    return normalize(mix)


def mine_explosion(rng):
    # small charge: sharp crack-dominant, short low-end
    return make_explosion(rng, duration=1.6, crack_hp=(900, 3500),
                           sub_f0=110, sub_f1=45, sub_tau=0.18,
                           rumble_tau=0.5, rumble_mix=0.6)


def shahed_impact(rng):
    # warhead + fuel hitting a target: bigger fireball, longer rumble tail
    return make_explosion(rng, duration=2.6, crack_hp=(1500, 5000),
                           sub_f0=85, sub_f1=30, sub_tau=0.35,
                           rumble_tau=1.1, rumble_mix=1.0)


def patriot_launch(rng, duration=3.0, sr=SR):
    n = int(duration * sr)
    ign_n = int(0.25 * sr)
    ignition = bandpass_noise(ign_n, rng, 400, 6000, sr) * exp_env(ign_n, sr, 1, 0.05)
    thump = pitch_thump(int(0.3 * sr), sr, 70, 35, 0.12)

    burn = bandpass_noise(n, rng, 150, 2200, sr)
    flutter_phase = rng.uniform(0, 2 * np.pi)
    lfo = 1 + 0.15 * np.sin(2 * np.pi * np.linspace(0, duration * 11, n) + flutter_phase)
    rise = np.linspace(0, 1, int(0.08 * sr))
    burn_env = np.concatenate([rise, np.ones(n - len(rise))])
    fade = np.linspace(1, 0, int(0.5 * sr))
    burn_env[-len(fade):] *= fade
    burn = burn * lfo * burn_env * 0.8

    out = np.zeros(n)
    out[:ign_n] += ignition
    out[:len(thump)] += thump * 1.2
    out += burn
    return normalize(out)


def process_voice(src_path, out_path, silence_thresh=0.02, pad_s=0.05):
    with wave.open(str(src_path), "rb") as w:
        sr = w.getframerate()
        n_frames = w.getnframes()
        sampwidth = w.getsampwidth()
        n_channels = w.getnchannels()
        raw = w.readframes(n_frames)
    if sampwidth != 2:
        raise ValueError(f"expected 16-bit PCM wav, got {sampwidth * 8}-bit: {src_path}")

    audio = np.frombuffer(raw, dtype=np.int16).astype(np.float32) / 32768.0
    if n_channels == 2:
        audio = audio.reshape(-1, 2).mean(axis=1)

    mask = np.abs(audio) > silence_thresh
    idx = np.where(mask)[0]
    if len(idx):
        pad = int(pad_s * sr)
        audio = audio[max(0, idx[0] - pad): idx[-1] + pad]

    audio = normalize(audio, peak=0.9)
    write_wav(out_path, audio, sr)


# ---------- CLI ----------

def main():
    ap = argparse.ArgumentParser(
        description="Generate procedural mine/Shahed explosions and a Patriot rocket-motor "
                    "sound, and process the Slava Ukraini voice line, into an output folder."
    )
    ap.add_argument("--count", type=int, default=3, help="variations per sound (default 3)")
    ap.add_argument("--seed", type=int, default=0, help="base random seed")
    ap.add_argument("--out", default=str(Path(__file__).parent / "out"), help="output folder")
    ap.add_argument("--voice", default=r"C:\Users\meli\Downloads\wav.wav",
                     help="source clip for the Slava Ukraini voice line (16-bit PCM wav)")
    args = ap.parse_args()

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    generators = {"mine_explosion": mine_explosion, "shahed_impact": shahed_impact}
    for name, fn in generators.items():
        for i in range(args.count):
            rng = np.random.default_rng(args.seed + i)
            fname = f"{name}_{i + 1:02d}.wav"
            write_wav(out_dir / fname, fn(rng))
            print(f"wrote {fname}")

    for i in range(args.count):
        rng = np.random.default_rng(args.seed + 100 + i)
        fname = f"patriot_launch_{i + 1:02d}.wav"
        write_wav(out_dir / fname, patriot_launch(rng))
        print(f"wrote {fname}")

    voice_src = Path(args.voice)
    if voice_src.exists():
        process_voice(voice_src, out_dir / "slava_ukraini.wav")
        print("wrote slava_ukraini.wav")
    else:
        print(f"Voice source not found: {voice_src} - skipping slava_ukraini.wav (pass --voice PATH)")


if __name__ == "__main__":
    main()
