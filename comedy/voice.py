"""Voices for the dialogue, fully local.

kokoro (default): Kokoro-82M (ONNX, CPU), ~20 English voices plus a few other languages; the two
model files (~350 MB) are downloaded once into the models folder's kokoro/ (common/storage.py).
sapi: the voices built into Windows (David, Zira, Hedda ...) - used for languages Kokoro can't speak.
hf: a Hugging Face text-to-speech checkpoint run through transformers (--tts-model), TTS_MODELS below
lists the well-known ones - Audio8-TTS-Realistic-Air (0.6B, 23 languages, ~2.6 GB, CUDA) by default.
breeze: Breeze TTS 2 (~7.6 GB, English/Chinese voice design) - needs its own inference repo and a
12 GB GPU, see TTS_MODELS["breeze"]["note"].
Each character gets its own voice and pitch; lines are cached in comedy/cache/tts/. KNOWN hand-picks
voice+pitch for a handful of famous names; for everyone else assign() can ask the LLM for a pitch/speed
that fits how the character actually sounds (optional, best effort, never required).
"""
import gc
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.request
import wave
from pathlib import Path

import numpy as np
from scipy import signal

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from common import log, storage  # noqa: E402

MODELS = storage.path("kokoro")
if not (MODELS / "kokoro-v1.0.onnx").exists():
    # downloaded by an older version, until moved (GUI > Models > Move them)
    MODELS = next((d for d in (HERE / "models", HERE.parent / "kokoro") if (d / "kokoro-v1.0.onnx").exists()), MODELS)
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

# Text-to-speech models a sketch can use: --voice-engine hf --tts-model <key>, or any HF repo id.
# kind: kokoro (onnx, CPU), hf (transformers + trust_remote_code), breeze (BreezeBlue's own repo).
# langs: what the model actually speaks - a language outside the list is refused instead of guessed.
# gpu: VB of VRAM it needs, rough; the weights land in the models folder under dir.
TTS_MODELS = {
    "audio8": {
        "name": "Audio8-TTS-Realistic-Air", "kind": "hf", "repo": "Edge0/Audio8-TTS-Realistic-Air",
        "dir": "tts/Audio8-TTS-Realistic-Air", "gpu": 4, "license": "Apache-2.0",
        "langs": {"english", "chinese", "cantonese", "japanese", "korean", "german", "french", "spanish", "italian",
                  "portuguese", "dutch", "polish", "russian", "hindi", "arabic", "finnish", "indonesian", "malay",
                  "norwegian", "swedish", "thai", "turkish", "vietnamese"},
        "note": "0.6B, 44.1 kHz, zero-shot cloning; each character gets its own sampling seed as its voice",
    },
    "breeze": {
        "name": "Breeze-TTS-2", "kind": "breeze", "repo": "BreezeBlue/Breeze-TTS-2",
        "dir": "tts/Breeze-TTS-2", "gpu": 12, "license": "research / non-commercial",
        "langs": {"english", "chinese"},
        "note": "English/Chinese voice design with a natural-language instruction; needs a 12 GB GPU and the "
                "inference code from github.com/breezeblue-ai/breeze-tts (clone it to models/tts/breeze-tts or "
                "set TTS_BREEZE_DIR)",
    },
}
TTS_ALIASES = {"audio8-tts": "audio8", "audio8-tts-realistic-air": "audio8", "Edge0/Audio8-TTS-Realistic-Air": "audio8",
               "breeze-tts-2": "breeze", "BreezeBlue/Breeze-TTS-2": "breeze"}
# what a character's voice seed can be, so two characters never sound the same with a reference-less model
VOICE_SEEDS = ["v1", "v2", "v3", "v4", "v5", "v6", "v7", "v8"]

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
        listing = "\n".join(f"- {m['name']} ({m.get('who', m['name'])}, {m['gender']}"
                            f"{', ' + m['personality'] if m.get('personality') else ''})" for m in cast)
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
        log.warn("voice  ", f"LLM voice profile failed ({str(e)[:120]}), using default pitch/speed")
        return {}


def _download():
    MODELS.mkdir(parents=True, exist_ok=True)
    for name, url in KOKORO_FILES.items():
        path = MODELS / name
        if path.exists() and path.stat().st_size > 1_000_000:
            continue
        tmp = path.with_suffix(path.suffix + ".part")
        with urllib.request.urlopen(url, timeout=60) as r, open(tmp, "wb") as f:
            total = int(r.headers.get("Content-Length") or 0)
            got, t0, last = 0, time.time(), 0.0
            while chunk := r.read(1 << 20):
                f.write(chunk)
                got += len(chunk)
                now = time.time()
                if total and now - last > 0.5:
                    rate = got / max(1e-3, now - t0) / 1e6
                    print(f"[dl   ] {name} (once): {int(got * 100 / total)}/100 ({rate:.1f} MB/s)", end="\r", flush=True)
                    last = now
        print(f"[dl   ] {name} done")
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
    def __init__(self, lang="English", engine="auto", tts_model=None):
        self.lang = lang.strip().lower()
        self.kokoro = None
        self.sapi = []
        self.model = None
        self.processor = None
        self.device = None
        if tts_model or engine in ("hf", "breeze"):
            key, spec = tts_spec(tts_model or ("audio8" if engine == "hf" else "breeze"))
            if spec.get("langs") and self.lang not in spec["langs"]:
                raise SystemExit(f"{spec['name']} can't speak {lang}; it speaks: "
                                 f"{', '.join(sorted(spec['langs']))} (or pick another --voice-engine)")
            self.engine, self.key, self.spec = spec["kind"], key, spec
        elif engine == "kokoro" and self.lang not in KOKORO_LANGS:
            raise SystemExit(f"Kokoro can't speak {lang}; supported: {', '.join(KOKORO_LANGS)} (or --voice-engine sapi)")
        elif engine in ("auto", "kokoro") and self.lang in KOKORO_LANGS:
            self.engine, self.key, self.spec = "kokoro", "kokoro", None
        else:
            self.engine, self.key, self.spec = "sapi", "sapi", None
            self.sapi = sapi_voices()
            culture = SAPI_CULTURE.get(self.lang, self.lang[:2])
            matching = [v for v in self.sapi if v[1].lower().startswith(culture)]
            if not matching:
                print(f"[voice  ] no Windows voice for {lang} installed (Settings > Time & language > Speech), "
                      f"using {self.sapi[0][0] if self.sapi else 'the default voice'}")
            self.sapi = matching or self.sapi

    def label(self):
        if self.engine == "kokoro":
            return "Kokoro-82M"
        if self.engine == "sapi":
            return "Windows SAPI"
        return f"{self.spec['name']} ({self.device or 'GPU'})"

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
                elif self.engine == "sapi":
                    pool = [v[0] for v in self.sapi if v[2].lower() == m["gender"]] or [v[0] for v in self.sapi] or [""]
                else:
                    # one sampling seed per character: a reference-less model can't be told a gender, the
                    # different timbres plus the pitch offset below keep them apart
                    pool = VOICE_SEEDS
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
        spec = {"t": text, "e": self.engine, "m": self.key, "v": member.get("voice"), "p": member.get("pitch", 0),
                "s": member.get("speed", 1.0), "l": self.lang}
        key = hashlib.sha1(json.dumps(spec, sort_keys=True).encode()).hexdigest()[:20]
        path = CACHE / f"{key}.wav"
        if path.exists():
            return read_wav(path)[0]
        ratio = 2 ** (float(spec["p"]) / 12)  # pitch up = play faster; talk slower first so the pace stays
        speed = max(0.5, min(2.0, float(spec["s"]) / ratio))
        if self.engine == "kokoro":
            audio, sr = self._kokoro(text, spec["v"], speed)
        elif self.engine == "sapi":
            audio, sr = self._sapi(text, spec["v"], speed)
        elif self.engine == "breeze":
            audio, sr = self._breeze(text, member)
        else:
            audio, sr = self._hf(text, spec["v"], member)
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

    def _load_hf(self):
        """Loads the checkpoint from the models folder (downloaded once) - CUDA in bf16, else CPU."""
        if self.model is not None:
            return
        path = storage.path(self.spec["dir"])
        if not (path / "config.json").exists():
            _download_repo(self.spec["repo"], path, self.spec["name"])
        import torch
        from transformers import AutoModel, AutoProcessor
        self.device = "cuda" if torch.cuda.is_available() else "cpu"
        dtype = torch.bfloat16 if self.device == "cuda" else torch.float32
        print(f"[voice  ] loading {self.spec['name']} on {self.device.upper()} ({path}) ...", flush=True)
        self.processor = AutoProcessor.from_pretrained(str(path), trust_remote_code=True)
        try:
            self.model = AutoModel.from_pretrained(str(path), trust_remote_code=True, dtype=dtype)
        except TypeError:  # transformers before 4.56 calls it torch_dtype
            self.model = AutoModel.from_pretrained(str(path), trust_remote_code=True, torch_dtype=dtype)
        self.model = self.model.eval().to(self.device)
        print(f"[voice  ] {self.spec['name']} ready", flush=True)

    def _hf(self, text, voice, member):
        """Text-to-speech through transformers' own TTS interface (generate -> decode_audio), so any
        checkpoint that ships AutoModel/AutoProcessor custom code works - Audio8-TTS is the reference.
        Without reference audio a model speaks one fixed timbre, so every character gets its own seed."""
        self._load_hf()
        import torch
        seed = int(hashlib.sha1(f"{self.key}:{voice}".encode()).hexdigest()[:8], 16)
        torch.manual_seed(seed)
        if self.device == "cuda":
            torch.cuda.manual_seed_all(seed)
        inputs = self.processor(text=[text.replace("\n", " ")], return_tensors="pt")
        inputs = {k: (v.to(self.device) if hasattr(v, "to") else v) for k, v in inputs.items()}
        seconds = max(2.0, len(text.split()) / 2.4)
        with torch.inference_mode():
            out = self.model.generate(**inputs, max_new_tokens=int(60 + 28 * seconds), temperature=0.85,
                                      top_p=0.95, top_k=50, do_sample=True, return_dict_in_generate=True)
            wavs, lengths = self.model.decode_audio(out.codes)
        audio = wavs[0, : int(lengths[0])].float().cpu().numpy().astype(np.float32)
        return audio, int(self.model.config.codec_sample_rate)

    def _breeze(self, text, voice, member):
        """Breeze TTS 2 shipped as its own repo (not transformers): voice design from an instruction."""
        root = Path(os.environ.get("TTS_BREEZE_DIR") or storage.path("tts/breeze-tts"))
        infer = root / "infer.py"
        if not infer.exists():
            raise SystemExit(f"Breeze TTS 2 needs its inference code: git clone https://github.com/breezeblue-ai/"
                             f"breeze-tts \"{root}\" (or set TTS_BREEZE_DIR). {self.spec['note']}")
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "out.wav"
            r = subprocess.run([sys.executable, str(infer), str(storage.path(self.spec["dir"])),
                                "--text", text.replace("\n", " "), "--instruction", breeze_instruction(member),
                                "--cfg-scale", "4", "--output", str(out)],
                               cwd=str(root), capture_output=True, text=True, timeout=1800)
            if r.returncode or not out.exists():
                raise RuntimeError(f"Breeze TTS failed: {(r.stderr or r.stdout)[-600:]}")
            return read_wav(out)

    def unload(self):
        """Frees the VRAM again, so ComfyUI can draw and the renderers get the card."""
        if self.model is None:
            return
        self.model = self.processor = None
        gc.collect()
        try:
            import torch
            if torch.cuda.is_available():
                torch.cuda.empty_cache()
        except Exception:
            pass


def breeze_instruction(member):
    """The natural-language voice design Breeze TTS 2 wants, from the character's gender/personality."""
    woman = member.get("gender") == "female"
    personality = (member.get("personality") or "").strip().rstrip(".")
    if personality:
        return (f"A {'woman' if woman else 'man'} whose voice fits this personality: {personality}. "
                f"Natural and believable on stage, clearly articulated, comedic timing.")
    return f"A {'warm' if woman else 'deep'} {'female' if woman else 'male'} voice, natural and clearly articulated."


def tts_spec(name):
    """(key, spec) for a TTS_MODELS entry, an alias, or any Hugging Face repo id."""
    key = TTS_ALIASES.get(name, name or "audio8")
    if key in TTS_MODELS:
        return key, TTS_MODELS[key]
    if "/" in key:  # any transformers TTS checkpoint: same loader, same folder
        return key, {"name": key, "kind": "hf", "repo": key, "dir": f"tts/{key.replace('/', '--')}", "gpu": 0,
                     "langs": None, "note": "custom transformers code, loaded with trust_remote_code"}
    raise SystemExit(f"unknown TTS model {name!r}; known: {', '.join(TTS_MODELS)} - or pass a Hugging Face repo id")


def _download_repo(repo, target, label=None):
    """Snapshot of a Hugging Face model into the models folder, with progress (once)."""
    print(f"[dl   ] {label or repo} (once) -> {target}", flush=True)
    target.mkdir(parents=True, exist_ok=True)
    try:
        from huggingface_hub import snapshot_download
    except ImportError:
        raise SystemExit("huggingface_hub is missing - setup.bat installs it with ComfyUI's requirements")
    snapshot_download(repo_id=repo, local_dir=str(target),
                      ignore_patterns=["*.md", "*.pdf", "assets/*", "*.jpeg", "*.jpg", "*.png", "*.svg",
                                       ".gitattributes"])
    print(f"[dl   ] {label or repo} done", flush=True)
