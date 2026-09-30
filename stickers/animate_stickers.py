"""Turn finished stickers (make_stickers.py output) into animated Telegram video stickers.

Telegram video sticker rules: WebM VP9 with transparency, 512 px on the long side, max 3 s,
max 30 fps, max 256 KB, no audio. Each input sticker gets a looping <name>.webm next to it.

Modes:
  motion (default) - the body sways, breathes, hops or trembles in a seamless loop: it is bent with the
                     feet planted instead of moved as a stiff picture, the text stays still on top.
                     Instant, CPU only.
  ai               - Wan 2.2 TI2V 5B in ComfyUI animates the sticker (the sticker is the first frame),
                     then rembg cuts every frame out again. Real motion, ~10-30 min per sticker on
                     6 GB VRAM; the text is added afterwards, so it stays still.
Both modes use the separate character/text layers make_stickers.py saves (layers/, stickers.json).

Usage (via 11-animate-stickers.bat, which uses ComfyUI's embedded Python):
  11-animate-stickers.bat stickers\\out\\<image>          (a folder: all *.webp stickers in it)
  11-animate-stickers.bat sticker_1.webp sticker_2.webp [--motion sway] [--seconds 2]
  11-animate-stickers.bat sticker_1.webp --mode ai [--prompt "the cat waves its paw"]
"""
import argparse
import io
import json
import math
import re
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path

import imageio_ffmpeg
import numpy as np
from PIL import Image
from scipy import ndimage

HERE = Path(__file__).resolve().parent
COMFY_DIR = HERE.parent / "ComfyUI_windows_portable" / "ComfyUI"
COMFY_URL = "http://127.0.0.1:8188"
WAN_WORKFLOW = HERE.parent / "img2video" / "img2video_workflow_api.json"
SIZE = 512
MAX_BYTES = 256 * 1024
MOTIONS = ["sway", "breathe", "hop", "tremble"]
# auto: loop picked from the sticker's text/pose (stickers.json written by make_stickers.py)
MOOD_MOTIONS = [
    ("sway", ("love", "heart", "kiss", "hug", "thank", "liebe", "herz", "danke")),
    ("breathe", ("tired", "sleep", "yawn", "zzz", "sleepy", "bored", "müde", "schlaf")),
    ("tremble", ("oh no", "panic", "sweat", "scared", "shock", "angry", "nope", "no", "cover", "oh nein")),
    ("hop", ("yes", "yay", "happy", "excited", "jump", "party", "celebrat", "ja")),
    ("sway", ("hi", "hello", "hey", "wave", "waving", "bye", "hallo", "tschüss")),
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
        if any(re.search(rf"{re.escape(k)}" + (r"" if len(k) <= 3 else ""), words) for k in keys):
            return motion
    return MOTIONS[index % len(MOTIONS)]


def bump(t, center, width):
    return math.exp(-((t - center) / width) ** 2)


def pose(motion, t):
    """(lean px at the head, breath/squash factor, hop px) at loop time t in [0, 1).
    Integer frequencies only, so the loop is seamless."""
    lean = breath = hop = 0.0
    if motion == "sway":  # leans left/right with the feet planted, breathing twice per sway
        lean = 9 * math.sin(2 * math.pi * t)
        breath = 0.012 * (0.5 - 0.5 * math.cos(4 * math.pi * t))
    elif motion == "breathe":  # one slow deep breath with a small drowsy nod
        breath = 0.03 * (0.5 - 0.5 * math.cos(2 * math.pi * t))
        lean = 2.5 * math.sin(2 * math.pi * t + 1)
    elif motion == "hop":  # crouch, small jump with a little stretch, squash on landing, settle
        p = (t - 0.2) / 0.4
        hop = 26 * math.sin(math.pi * p) if 0 < p < 1 else 0.0
        breath = -0.035 * bump(t, 0.1, 0.07) + 0.02 * bump(t, 0.3, 0.06) - 0.03 * bump(t, 0.66, 0.06)
    elif motion == "tremble":  # nervous shiver in one burst, strongest at the head
        env = (0.5 - 0.5 * math.cos(2 * math.pi * t)) ** 1.5
        lean = 3 * env * math.sin(2 * math.pi * 9 * t)
        breath = 0.006 * env * math.sin(2 * math.pi * 9 * t + 1)
    return lean, breath, hop


def warp(arr, feet, head, cx, lean, breath, hop):
    """Bend the body instead of moving the whole picture rigidly: the feet stay planted, the lean grows
    towards the head, breathing stretches the body upwards and widens the chest a little."""
    h, w = arr.shape[:2]
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    height = max(feet - head, 1)
    u = np.clip((feet - y) / height, 0, 1)  # 0 at the feet .. 1 at the top of the head
    ys = feet - (feet - (y + hop)) / (1 + breath)
    xs = cx + (x - cx) / (1 + 0.5 * breath * np.sin(np.pi * u)) - lean * u ** 1.6
    premul = arr.copy()
    premul[..., :3] *= premul[..., 3:] / 255
    out = np.stack([ndimage.map_coordinates(premul[..., c], [ys, xs], order=1, mode="constant")
                    for c in range(4)], axis=-1)
    out[..., :3] /= np.maximum(out[..., 3:], 1e-3) / 255
    return Image.fromarray(np.clip(out, 0, 255).astype(np.uint8), "RGBA")


def motion_frames(char, text_layer, motion, seconds, fps):
    """Only the character moves; the text layer (if the sticker has one) stays still on top."""
    char = fit_canvas(char, scale=0.92)  # headroom for hops and leans, same transform for the text
    text_layer = fit_canvas(text_layer, scale=0.92) if text_layer is not None else None
    arr = np.asarray(char).astype(np.float32)
    rows = np.flatnonzero(arr[..., 3].max(axis=1) > 0)
    cols = np.flatnonzero(arr[..., 3].max(axis=0) > 0)
    head, feet, cx = rows[0], rows[-1], (cols[0] + cols[-1]) / 2
    n = max(2, round(seconds * fps))
    frames = []
    for i in range(n):
        frame = warp(arr, feet, head, cx, *pose(motion, i / n))
        if text_layer is not None:
            frame.alpha_composite(text_layer)
        frames.append(frame)
    return frames


# ---------------------------------------------------------------- ai mode
def ai_frames(char, text_layer, prompt, seconds, seed, steps):
    wf = json.loads(WAN_WORKFLOW.read_text())
    start = Image.new("RGB", (SIZE, SIZE), AI_BACKGROUND)
    fitted = fit_canvas(char, scale=0.9)
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
    frames = [Image.fromarray(np.dstack([np.asarray(f)[..., :3], a.astype(np.uint8)]), "RGBA")
              for f, a in zip(frames, smooth)]
    if text_layer is not None:  # the text was not part of the video, so it stays crisp and still
        text_layer = fit_canvas(text_layer, scale=0.9)
        for f in frames:
            f.alpha_composite(text_layer)
    return frames


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
                    help="motion mode: loop type; auto (default) matches the sticker's text/pose, e.g. sway for love/hi, breathe for tired, tremble for oh no, hop for yes")
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
        meta = {}
        if (f.parent / "stickers.json").is_file():
            meta = next((m for m in json.loads((f.parent / "stickers.json").read_text(encoding="utf-8"))
                         if m.get("file") == f.name), {})
        # separate layers from make_stickers.py: move the body, keep the text still
        char_file, text_file = (f.parent / meta.get("character", "-"), f.parent / meta.get("text_layer", "-"))
        if char_file.is_file():
            char = Image.open(char_file).convert("RGBA")
            text_layer = Image.open(text_file).convert("RGBA") if text_file.is_file() else None
        else:  # an older/foreign sticker: the whole picture bends, text included
            char, text_layer = Image.open(f).convert("RGBA"), None
        if args.mode == "motion":
            motion = pick_motion(meta, i) if args.motion == "auto" else args.motion
            print(f"[motion ] {f.name}: {motion}")
            fps, frames = 30, motion_frames(char, text_layer, motion, seconds, 30)
        else:
            print(f"[animate] {f.name}: Wan 2.2, {seconds:g} s")
            prompt = args.prompt or (f"{meta['expression']}, {AI_MOTION}" if meta.get("expression") else AI_MOTION)
            fps, frames = 24, ai_frames(char, text_layer, prompt, seconds, args.seed + i, args.steps)
        out = f.with_suffix(".webm")
        size, crf = encode_webm(frames, fps, out)
        warn = "" if size <= MAX_BYTES else "  - still over 256 KB, try --seconds 1.5"
        print(f"[done   ] {out}  ({size // 1024} KB, crf {crf}){warn}")


if __name__ == "__main__":
    sys.exit(main())
