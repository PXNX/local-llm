"""Voices for the dialogue, fully local.

kokoro (default): Kokoro-82M (ONNX, CPU), ~20 English voices plus a few other languages; the two
model files (~350 MB) are downloaded once into the models folder's kokoro/ (common/storage.py).
sapi: the voices built into Windows (David, Zira, Hedda ...) - used for languages Kokoro can't speak.
Each character gets its own voice and pitch; lines are cached in comedy/cache/tts/. KNOWN hand-picks
voice+pitch for a handful of famous names; for everyone else assign() can ask the LLM for a pitch/speed
that fits how the character actually sounds (optional, best effort, never required).
"""
import hashlib
import json
import subprocess
import sys
import tempfile
import urllib.request
import wave
from pathlib import Path

import numpy as np
from scipy import signal

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from common import storage  # noqa: E402

MODELS = storage.path("kokoro")
if not (MODELS / "kokoro-v1.0.onnx").exists() and (HERE / "models" / "kokoro-v1.0.onnx").exists():
    MODELS = HERE / "models"  # downloaded by an older version, until moved (GUI > Models > Move them)
CACHE = HERE / "cache" / "tts"
SR = 44100
KOKORO_FILES = {
    "kokoro-v1.0.onnx": "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx",
    "voices-v1.0.bin": "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin",
}
# language -> kokoro lang code, male voices, female voices (first = most natural)
KOKORO_LANGS = {
    "english": ("en-us", ["am_michael", "am_onyx", "bm_george", "am_puck", "bm_lewis", "am_echo", "am_eric",
                          "bm_daniel", "am_liam", "am_adam", "bm_fable", "am_fenrir"],
                ["af_heart", "af_bella", "bf_emma", "af_nicole", "bf_isabella", "af_sarah", "af_sky"]),
    "spanish": ("es", ["em_alex", "em_santa"], ["ef_dora"]),
    "french": ("fr-fr", [], ["ff_siwis"]),
    "italian": ("it", ["im_nicola"], ["if_sara"]),
    "portuguese": ("pt-br", ["pm_alex", "pm_santa"], ["pf_dora"]),
    "hindi": ("hi", ["hm_omega", "hm_psi"], ["hf_alpha", "hf_beta"]),
    "japanese": ("ja", ["jm_kumo"], ["jf_alpha", "jf_gongitsune", "jf_nezumi", "jf_tebukuro"]),
    "chinese": ("cmn", ["zm_yunjian", "zm_yunxi", "zm_yunxia", "zm_yunyang"], ["zf_xiaobei", "zf_xiaoni", "zf_xiaoxiao"]),
}
VOICE_LANG = {"a": "en-us", "b": "en-gb", "e": "es", "f": "fr-fr", "i": "it", "p": "pt-br", "h": "hi", "j": "ja",
              "z": "cmn"}
SAPI_CULTURE = {"german": "de", "english": "en", "french": "fr", "spanish": "es", "italian": "it", "polish": "pl",
                "dutch": "nl", "portuguese": "pt", "russian": "ru", "ukrainian": "uk", "japanese": "ja", "chinese": "zh"}
# well-known characters, English: (voice, semitones) - hand-picked after listening, takes priority
KNOWN = {"trump": ("am_onyx", -1), "putin": ("am_puck", 2), "zelensky": ("am_michael", 0), "macron": ("bm_lewis", 0),
         "xi": ("am_eric", -1), "kim": ("am_echo", 3), "merz": ("bm_george", 0), "khamenei": ("bm_fable", -3),
         "orban": ("bm_daniel", 1), "biden": ("am_adam", -1), "erdogan": ("am_fenrir", -2)}
PITCHES = [0, -2, 2, -4, 3, -5, 4]
VOICE_PROMPT = """For each cartoon character below, estimate how their speaking voice should differ from a
neutral narrator, for a comedic caricature. Base it on how they actually sound publicly (real public
figures: their real pace/pitch/gravity; fictional ones: their implied persona) - exaggerate slightly
for comic effect, but keep every line understandable.

Characters:
{listing}

Reply with JSON only: {{"<name>": {{"pitch": <int -5..5, negative = deeper/older/graver, positive =
higher/younger/more nasal, 0 = unremarkable>, "speed": <float 0.85..1.15, slower for deliberate/pompous
speakers, faster for rushed/anxious ones, 1.0 = average>}}, ...}} - one entry per character, using
their exact "name" as the key.
"""


def llm_profiles(cast, llm):
    """Asks the LLM for a pitch/speed per character that matches how they actually sound (real public
    figures) or their persona (fictional) - fills in the gap KNOWN only covers for a handful of names.
    Best effort: returns {} on any failure so a sketch never breaks over a missing/flaky LLM."""
    if llm is None or not cast:
        return {}
    try:
        if not llm.available():
            return {}
        listing = "\n".join(f"- {m['name']} ({m.get('who', m['name'])}, {m['gender']})" for m in cast)
        reply = llm.text_json(VOICE_PROMPT.format(listing=listing), temperature=0.4)
        llm.unload()
        out = {}
        if isinstance(reply, dict):
            for m in cast:
                p = reply.get(m["name"])
                if isinstance(p, dict):
                    try:
                        out[m["name"]] = {"pitch": max(-5, min(5, int(round(float(p.get("pitch", 0)))))),
                                          "speed": max(0.85, min(1.15, float(p.get("speed", 1.0))))}
                    except (TypeError, ValueError):
                        pass
        return out
    except Exception as e:
        print(f"[voice  ] LLM voice profile failed ({str(e)[:120]}), using default pitch/speed")
        return {}


def _download():
    MODELS.mkdir(parents=True, exist_ok=True)
    for name, url in KOKORO_FILES.items():
        path = MODELS / name
        if path.exists() and path.stat().st_size > 1_000_000:
            continue
        print(f"[voice  ] downloading {name} (once) ...")
        tmp = path.with_suffix(path.suffix + ".part")
        with urllib.request.urlopen(url, timeout=60) as r, open(tmp, "wb") as f:
            while chunk := r.read(1 << 20):
                f.write(chunk)
        tmp.replace(path)


def sapi_voices():
    ps = ("Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; "
          "$s.GetInstalledVoices() | % { $_.VoiceInfo.Name + '|' + $_.VoiceInfo.Culture + '|' + $_.VoiceInfo.Gender }")
    try:
        out = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True, timeout=60).stdout
    except (OSError, subprocess.TimeoutExpired):
        return []
    return [tuple(line.strip().split("|")) for line in out.splitlines() if line.count("|") == 2]


SAPI_SCRIPT = r"""param([string]$voice, [string]$textFile, [string]$out, [int]$rate)
Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
if ($voice) { $s.SelectVoice($voice) }
$s.Rate = $rate
$s.SetOutputToWaveFile($out)
$s.Speak([IO.File]::ReadAllText($textFile, [Text.Encoding]::UTF8))
$s.Dispose()
"""


def read_wav(path):
    with wave.open(str(path)) as w:
        sr, n, ch, width = w.getframerate(), w.getnframes(), w.getnchannels(), w.getsampwidth()
        raw = w.readframes(n)
    a = np.frombuffer(raw, dtype={1: np.uint8, 2: np.int16, 4: np.int32}[width]).astype(np.float32)
    a = (a - 128) / 128 if width == 1 else a / float(2 ** (8 * width - 1))
    if ch > 1:
        a = a.reshape(-1, ch).mean(axis=1)
    return a, sr


def write_wav(path, audio, sr=SR):
    pcm = (np.clip(audio, -1, 1) * 32767).astype(np.int16)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1 if pcm.ndim == 1 else pcm.shape[0])
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes((pcm if pcm.ndim == 1 else pcm.T).tobytes())


def resample(audio, sr_from, sr_to):
    sr_from = int(round(sr_from / 100) * 100)  # coarse rates keep the polyphase factors small
    if sr_from == sr_to:
        return audio
    g = np.gcd(sr_from, int(sr_to))
    return signal.resample_poly(audio, sr_to // g, sr_from // g).astype(np.float32)


def tidy(audio, sr=SR):
    """Trim silence, even out the loudness, soft limit."""
    idx = np.where(np.abs(audio) > 0.02 * max(1e-6, np.abs(audio).max()))[0]
    if len(idx):
        pad = int(0.04 * sr)
        audio = audio[max(0, idx[0] - pad): idx[-1] + pad]
    rms = np.sqrt(np.mean(audio ** 2)) if audio.size else 0
    if rms > 1e-5:
        audio = audio * (0.12 / rms)
    return np.tanh(audio * 1.1).astype(np.float32)


class Voices:
    def __init__(self, lang="English", engine="auto"):
        self.lang = lang.strip().lower()
        self.kokoro = None
        self.sapi = []
        if engine in ("auto", "kokoro") and self.lang in KOKORO_LANGS:
            self.engine = "kokoro"
        elif engine == "kokoro":
            raise SystemExit(f"Kokoro can't speak {lang}; supported: {', '.join(KOKORO_LANGS)} (or --voice-engine sapi)")
        else:
            self.engine = "sapi"
            self.sapi = sapi_voices()
            culture = SAPI_CULTURE.get(self.lang, self.lang[:2])
            matching = [v for v in self.sapi if v[1].lower().startswith(culture)]
            if not matching:
                print(f"[voice  ] no Windows voice for {lang} installed (Settings > Time & language > Speech), "
                      f"using {self.sapi[0][0] if self.sapi else 'the default voice'}")
            self.sapi = matching or self.sapi

    def label(self):
        return "Kokoro-82M" if self.engine == "kokoro" else "Windows SAPI"

    def assign(self, cast, llm=None):
        """Fills voice/pitch into each cast member that has none (a script.json may set them).
        Characters that have to share a voice get different pitches. If an llm module is given, it
        suggests a pitch/speed fitting how the character actually sounds, for names KNOWN doesn't
        hardcode - same priority as KNOWN: set before the voice-collision fallback."""
        used = []
        profiles = llm_profiles(cast, llm)
        for i, m in enumerate(cast):
            if "voice" not in m:
                if self.engine == "kokoro":
                    _, males, females = KOKORO_LANGS[self.lang]
                    pool = (females if m["gender"] == "female" else males) or males or females
                    known = KNOWN.get(m["name"].lower().split()[-1]) or KNOWN.get(m["name"].lower().split()[0])
                    if known and self.lang == "english" and known[0] not in used:
                        m["voice"] = known[0]
                        m.setdefault("pitch", known[1])
                else:
                    pool = [v[0] for v in self.sapi if v[2].lower() == m["gender"]] or [v[0] for v in self.sapi] or [""]
                if "voice" not in m:
                    free = [v for v in pool if v not in used]
                    m["voice"] = free[0] if free else pool[i % len(pool)]
            p = profiles.get(m["name"])
            if p:
                m.setdefault("pitch", p["pitch"])
                m.setdefault("speed", p["speed"])
            if m["voice"] in used:
                m.setdefault("pitch", PITCHES[1 + used.count(m["voice"]) % (len(PITCHES) - 1)])
            m.setdefault("pitch", 0)
            m.setdefault("speed", 1.0)
            used.append(m["voice"])
        return cast

    def say(self, text, member):
        """Mono float32 audio at 44.1 kHz, cached."""
        spec = {"t": text, "e": self.engine, "v": member.get("voice"), "p": member.get("pitch", 0),
                "s": member.get("speed", 1.0), "l": self.lang}
        key = hashlib.sha1(json.dumps(spec, sort_keys=True).encode()).hexdigest()[:20]
        path = CACHE / f"{key}.wav"
        if path.exists():
            return read_wav(path)[0]
        ratio = 2 ** (float(spec["p"]) / 12)  # pitch up = play faster; talk slower first so the pace stays
        speed = max(0.5, min(2.0, float(spec["s"]) / ratio))
        if self.engine == "kokoro":
            audio, sr = self._kokoro(text, spec["v"], speed)
        else:
            audio, sr = self._sapi(text, spec["v"], speed)
        audio = resample(audio, sr * ratio, SR)
        audio = tidy(audio)
        CACHE.mkdir(parents=True, exist_ok=True)
        write_wav(path, audio)
        return audio

    def _kokoro(self, text, voice, speed):
        if self.kokoro is None:
            _download()
            from kokoro_onnx import Kokoro
            self.kokoro = Kokoro(str(MODELS / "kokoro-v1.0.onnx"), str(MODELS / "voices-v1.0.bin"))
        lang = VOICE_LANG.get(voice[0], "en-us")
        audio, sr = self.kokoro.create(text, voice=voice, speed=speed, lang=lang)
        return np.asarray(audio, dtype=np.float32), sr

    def _sapi(self, text, voice, speed):
        rate = int(round(max(-10, min(10, (speed - 1) * 10))))
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            (tmp / "say.ps1").write_text(SAPI_SCRIPT, encoding="utf-8")
            (tmp / "text.txt").write_text(text, encoding="utf-8")
            out = tmp / "out.wav"
            r = subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(tmp / "say.ps1"),
                                "-voice", voice or "", "-textFile", str(tmp / "text.txt"), "-out", str(out),
                                "-rate", str(rate)], capture_output=True, text=True, timeout=120)
            if r.returncode or not out.exists():
                raise RuntimeError(f"Windows speech failed: {r.stderr[-500:]}")
            return read_wav(out)
