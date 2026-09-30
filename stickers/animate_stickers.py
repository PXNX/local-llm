"""Turn finished stickers (make_stickers.py output) into animated Telegram video stickers.

Telegram video sticker rules: WebM VP9 with transparency, 512 px on the long side, max 3 s,
max 30 fps, max 256 KB, no audio. Each input sticker gets a looping <name>.webm next to it.

Modes:
  motion (default) - the whole sticker bounces/wobbles/pulses/floats/shakes in a seamless loop.
                     Instant, CPU only, the text stays crisp.
  ai               - Wan 2.2 TI2V 5B in ComfyUI animates the sticker (the sticker is the first frame),
                     then rembg cuts every frame out again. Real motion, ~10-30 min per sticker on
                     6 GB VRAM; the baked-in text may wobble a little.

Usage (via 11-animate-stickers.bat, which uses ComfyUI's embedded Python):
  11-animate-stickers.bat stickers\\out\\<image>          (a folder: all *.webp stickers in it)
  11-animate-stickers.bat sticker_1.webp sticker_2.webp [--motion pulse] [--seconds 2]
  11-animate-stickers.bat sticker_1.webp --mode ai [--prompt "the cat waves its paw"]
"""
import argparse
import io
import json
import math
import re
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
from pathlib import Path

import imageio_ffmpeg
import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
COMFY_DIR = HERE.parent / "ComfyUI_windows_portable" / "ComfyUI"
COMFY_URL = "http://127.0.0.1:8188"
WAN_WORKFLOW = HERE.parent / "img2video" / "img2video_workflow_api.json"
SIZE = 512
MAX_BYTES = 256 * 1024
MOTIONS = ["bounce", "wobble", "pulse", "float", "shake"]
# auto: loop picked from the sticker's text/pose (stickers.json written by make_stickers.py)
MOOD_MOTIONS = [
    ("pulse", ("love", "heart", "kiss", "hug", "thank", "liebe", "herz", "danke")),
    ("float", ("tired", "sleep", "yawn", "zzz", "sleepy", "bored", "müde", "schlaf")),
    ("shake", ("oh no", "panic", "sweat", "scared", "shock", "angry", "nope", "no", "cover", "oh nein")),
    ("bounce", ("yes", "yay", "happy", "excited", "jump", "party", "celebrat", "ja")),
    ("wobble", ("hi", "hello", "hey", "wave", "waving", "bye", "hallo", "tschüss")),
]
AI_BACKGROUND = (214, 232, 247)  # plain pale blue like the FLUX sticker renders, easy for rembg
AI_MOTION = ("the character comes alive and acts out its pose with lively natural motion, static camera, "
             "plain background stays plain, the text stays still and unchanged")


# ---------------------------------------------------------------- helpers
def http_json(url, payload=None, timeout=600):
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def reachable(url):
    try:
        urllib.request.urlopen(url, timeout=3)
        return True
    except Exception:
        return False


def fit_canvas(img, size=SIZE, scale=1.0):
    """RGBA sticker centered on a transparent size x size canvas, long side = size * scale."""
    img = img.convert("RGBA")
    f = size * scale / max(img.size)
    img = img.resize((max(1, round(img.width * f)), max(1, round(img.height * f))), Image.LANCZOS)
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    canvas.paste(img, ((size - img.width) // 2, (size - img.height) // 2), img)
    return canvas


# ---------------------------------------------------------------- motion mode
def pick_motion(meta, index):
    words = f"{meta.get('text', '')} {meta.get('expression', '')}".lower()
    for motion, keys in MOOD_MOTIONS:
        # short keys as whole words ("hi" must not hit "while"), longer ones also as word stems
        if any(re.search(rf"\b{re.escape(k)}" + (r"\b" if len(k) <= 3 else ""), words) for k in keys):
            return motion
    return MOTIONS[index % len(MOTIONS)]


def motion_frames(sticker, motion, seconds, fps):
    """Seamless loop: every curve is periodic over the clip, so the last frame flows into the first."""
    base = fit_canvas(sticker, scale=0.86)  # headroom so hops/rotations stay inside 512x512
    n = max(2, round(seconds * fps))
    frames = []
    for i in range(n):
        t = i / n  # 0..1, one loop
        s = math.sin(2 * math.pi * t)
        angle, dx, dy, sx, sy = 0.0, 0.0, 0.0, 1.0, 1.0
        if motion == "bounce":  # two hops with squash on landing
            h = abs(math.sin(2 * math.pi * t))
            dy = -38 * h
            sx, sy = 1 + 0.06 * (1 - h) ** 4, 1 - 0.06 * (1 - h) ** 4
        elif motion == "wobble":  # rock left/right around the bottom
            angle = 7 * s
        elif motion == "pulse":  # heartbeat: two quick beats per loop
            beat = max(0.0, math.sin(4 * math.pi * t)) ** 3
            sx = sy = 1 + 0.08 * beat
        elif motion == "float":  # sleepy drift up and down with a slight tilt
            dy, angle = 14 * s, 3 * math.sin(2 * math.pi * t + 0.8)
        elif motion == "shake":  # nervous jitter in bursts
            burst = max(0.0, math.sin(2 * math.pi * t)) ** 2
            dx, angle = 9 * burst * math.sin(16 * math.pi * t), 2.5 * burst * math.sin(16 * math.pi * t + 1)
        w, h_px = round(SIZE * sx), round(SIZE * sy)
        img = base.resize((w, h_px), Image.BICUBIC) if (w, h_px) != (SIZE, SIZE) else base
        # scale/rotate around the bottom center so bounces land and wobbles rock on the "feet"
        pivot = (w / 2, h_px * 0.93)
        img = img.rotate(angle, resample=Image.BICUBIC, center=pivot)
        frame = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
        frame.paste(img, (round((SIZE - w) / 2 + dx), round(SIZE - h_px + dy)), img)
        frames.append(frame)
    return frames


# ---------------------------------------------------------------- ai mode
def ai_frames(sticker, prompt, seconds, seed, steps):
    wf = json.loads(WAN_WORKFLOW.read_text())
    start = Image.new("RGB", (SIZE, SIZE), AI_BACKGROUND)
    fitted = fit_canvas(sticker, scale=0.9)
    start.paste(fitted, (0, 0), fitted)
    start.save(COMFY_DIR / "input" / "animate_sticker_input.png")
    frames_n = max(1, round(seconds * 24 / 4)) * 4 + 1  # Wan needs 4n+1 frames at 24 fps
    wf["5"]["inputs"]["image"] = "animate_sticker_input.png"
    wf["6"]["inputs"]["text"] = f"{prompt}, smooth natural motion, consistent colors, high quality"
    wf["8"]["inputs"].update(width=SIZE, height=SIZE, length=frames_n)
    wf["9"]["inputs"].update(seed=seed, steps=steps)
    # save the raw frames as PNGs instead of an MP4, they are cut out one by one afterwards
    wf["12"] = {"class_type": "SaveImage", "inputs": {"images": ["10", 0], "filename_prefix": "sticker_anim/frame"}}
    wf.pop("11")

    pid = http_json(f"{COMFY_URL}/prompt", {"prompt": wf})["prompt_id"]
    t0 = time.time()
    while True:
        hist = http_json(f"{COMFY_URL}/history/{pid}")
        if pid in hist:
            entry = hist[pid]
            if entry.get("status", {}).get("status_str") == "error":
                raise RuntimeError(f"ComfyUI error: {entry['status']}")
            images = entry["outputs"]["12"]["images"]
            break
        print(f"\r[animate] {int(time.time() - t0) // 60} min {int(time.time() - t0) % 60:02d} s ...", end="", flush=True)
        time.sleep(5)
    print()

    from rembg import new_session, remove
    session = new_session("isnet-general-use")
    frames = []
    for k, im in enumerate(images):
        q = urllib.parse.urlencode({"filename": im["filename"], "subfolder": im["subfolder"], "type": im["type"]})
        with urllib.request.urlopen(f"{COMFY_URL}/view?{q}") as r:
            frame = Image.open(io.BytesIO(r.read())).convert("RGB")
        print(f"\r[cutout ] frame {k + 1}/{len(images)}", end="", flush=True)
        frames.append(remove(frame, session=session))
    print()
    # steady the cutout: a pixel only flickers in/out if it does so in neighbouring frames too
    alphas = np.stack([np.asarray(f.getchannel("A")) for f in frames]).astype(np.float32)
    smooth = np.median(np.stack([np.roll(alphas, 1, 0), alphas, np.roll(alphas, -1, 0)]), axis=0)
    smooth[0], smooth[-1] = alphas[0], alphas[-1]
    return [Image.fromarray(np.dstack([np.asarray(f)[..., :3], a.astype(np.uint8)]), "RGBA")
            for f, a in zip(frames, smooth)]


# ---------------------------------------------------------------- encode
def encode_webm(frames, fps, path):
    """VP9 + alpha; raise the CRF until the file fits Telegram's 256 KB."""
    raw = b"".join(np.asarray(f.convert("RGBA")).tobytes() for f in frames)
    for crf in (30, 36, 42, 48, 54, 60):
        cmd = [imageio_ffmpeg.get_ffmpeg_exe(), "-y", "-loglevel", "error",
               "-f", "rawvideo", "-pix_fmt", "rgba", "-s", f"{SIZE}x{SIZE}", "-r", str(fps), "-i", "-",
               "-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-b:v", "0", "-crf", str(crf),
               "-auto-alt-ref", "0", "-deadline", "good", "-an", str(path)]
        subprocess.run(cmd, input=raw, check=True)
        if path.stat().st_size <= MAX_BYTES:
            return path.stat().st_size, crf
    return path.stat().st_size, crf


# ---------------------------------------------------------------- main
def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("inputs", type=Path, nargs="+", help="sticker .webp/.png files or folders of them")
    ap.add_argument("--mode", choices=["motion", "ai"], default="motion", help="motion (instant loop, default) or ai (Wan 2.2)")
    ap.add_argument("--motion", choices=["auto"] + MOTIONS, default="auto",
                    help="motion mode: loop type; auto (default) matches the sticker's text/pose, e.g. pulse for love, float for tired")
    ap.add_argument("--seconds", type=float, default=2.0, help="clip length, max 3 (default 2)")
    ap.add_argument("--prompt", default=None, help="ai mode: what should move (default: act out the sticker's pose)")
    ap.add_argument("--steps", type=int, default=20, help="ai mode: sampling steps (default 20)")
    ap.add_argument("--seed", type=int, default=0, help="ai mode: seed (default 0)")
    args = ap.parse_args()

    files = []
    for p in args.inputs:
        if p.is_dir():
            files += sorted(f for f in p.iterdir() if f.suffix.lower() in (".webp", ".png"))
        elif p.is_file():
            files.append(p)
        else:
            ap.error(f"not found: {p}")
    if not files:
        ap.error("no .webp/.png stickers found")
    seconds = min(max(args.seconds, 0.5), 3.0)
    if args.mode == "ai" and not reachable(COMFY_URL):
        sys.exit("ComfyUI is not running (start-comfyui.bat)")

    for i, f in enumerate(files):
        sticker = Image.open(f).convert("RGBA")
        meta = {}
        if (f.parent / "stickers.json").is_file():
            meta = next((m for m in json.loads((f.parent / "stickers.json").read_text(encoding="utf-8"))
                         if m.get("file") == f.name), {})
        if args.mode == "motion":
            motion = pick_motion(meta, i) if args.motion == "auto" else args.motion
            print(f"[motion ] {f.name}: {motion}")
            fps, frames = 30, motion_frames(sticker, motion, seconds, 30)
        else:
            print(f"[animate] {f.name}: Wan 2.2, {seconds:g} s")
            prompt = args.prompt or (f"{meta['expression']}, {AI_MOTION}" if meta.get("expression") else AI_MOTION)
            fps, frames = 24, ai_frames(sticker, prompt, seconds, args.seed + i, args.steps)
        out = f.with_suffix(".webm")
        size, crf = encode_webm(frames, fps, out)
        warn = "" if size <= MAX_BYTES else "  - still over 256 KB, try --seconds 1.5"
        print(f"[done   ] {out}  ({size // 1024} KB, crf {crf}){warn}")


if __name__ == "__main__":
    sys.exit(main())
