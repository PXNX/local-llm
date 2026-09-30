"""Turn finished stickers (make_stickers.py output) into animated Telegram video stickers.

Telegram video sticker rules: WebM VP9 with transparency, 512 px on the long side, max 3 s,
max 30 fps, max 256 KB, no audio. Each input sticker gets a looping <name>.webm next to it.

Modes:
  ai (default) - the character on the sticker really acts out its pose: Wan 2.2 TI2V 5B in ComfyUI
                 starts from the sticker and animates the action the LLM derives from the pose in
                 stickers.json (the raised paw waves, the hug squeezes tighter, the dancer dances ...),
                 then rembg cuts every frame out again and the text is laid back on top, still.
                 Played forth and back (ping-pong) so it loops smoothly. ~5-15 min per sticker on 6 GB VRAM.
  loop         - quick fallback without GPU: the body is bent in a simple loop (sway/breathe/hop/...).
Both modes use the separate character/text layers make_stickers.py saves (layers/, stickers.json).

Usage (via 11-animate-stickers.bat, which uses ComfyUI's embedded Python):
  11-animate-stickers.bat stickers\\out\\<image>          (a folder: all *.webp stickers in it)
  11-animate-stickers.bat sticker_1.webp [--prompt "the cat waves its raised paw side to side"] [--seconds 3]
  11-animate-stickers.bat sticker_1.webp --mode loop [--motion sway]
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
sys.path.insert(0, str(HERE.parent))
from common import llm  # noqa: E402

COMFY_DIR = HERE.parent / "ComfyUI_windows_portable" / "ComfyUI"
COMFY_URL = "http://127.0.0.1:8188"
WAN_WORKFLOW = HERE.parent / "img2video" / "img2video_workflow_api.json"
SIZE = 512
MAX_BYTES = 256 * 1024
MOTIONS = ["sway", "breathe", "hop", "tremble", "dance", "squeeze"]
# auto: loop picked from the sticker's text/pose (stickers.json written by make_stickers.py)
MOOD_MOTIONS = [
    ("dance", ("danc", "party", "groove", "boogie", "celebrat", "tanz", "feier")),
    ("squeeze", ("hug", "cuddl", "squeez", "embrac", "miss you", "kuschel", "umarm")),
    ("sway", ("love", "heart", "kiss", "thank", "liebe", "herz", "danke")),
    ("breathe", ("tired", "sleep", "yawn", "zzz", "sleepy", "bored", "müde", "schlaf")),
    ("tremble", ("oh no", "panic", "sweat", "scared", "shock", "angry", "nope", "no", "cover", "oh nein")),
    ("hop", ("yes", "yay", "happy", "excited", "jump", "joy", "ja")),
    ("sway", ("hi", "hello", "hey", "wave", "waving", "bye", "hallo", "tschüss")),
]
AI_BACKGROUND = (214, 232, 247)  # plain pale blue like the FLUX sticker renders, easy for rembg
AI_STYLE = ("static camera, the whole character stays in frame, same look and colors, plain pale blue "
            "background stays plain, smooth clear natural motion, high quality")


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
    elif motion == "dance":  # grooves to a beat: sways left/right while bobbing down on every beat
        lean = 11 * math.sin(2 * math.pi * 2 * t)
        bob = 0.5 - 0.5 * math.cos(2 * math.pi * 4 * t)  # 4 beats per loop
        hop = 10 * (1 - bob)
        breath = -0.025 * bob
    elif motion == "squeeze":  # a tight hug: the body squeezes in twice with a happy little lean
        hug = bump(t, 0.25, 0.1) + bump(t, 0.7, 0.1)
        breath = -0.02 * hug
        lean = 3 * math.sin(2 * math.pi * t)
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
def action_prompt(meta):
    """Turn the sticker's still pose into the movement that pose implies (LLM, text only)."""
    subject, expression = meta.get("subject") or "the character", meta.get("expression", "")
    fallback = f"{subject}, {expression}, clearly acting it out with lively natural movement"
    if not expression or not llm.available():
        return fallback
    ask = ("A chat sticker shows this still image: " + subject + " - " + expression + ". "
           "Write one English prompt for an image-to-video model that brings exactly this pose to life as a short, "
           "clearly visible action of about 1.5 seconds, e.g. 'the cat waves its raised paw side to side twice', "
           "'the cat squeezes the kitten tighter and rocks it gently, eyes stay closed', 'the cat dances, "
           "stepping side to side with both paws up', 'the cat yawns wide and blinks sleepily'. Describe the "
           "moving body parts; the character's look stays the same. Max 40 words. "
           'Reply with JSON only: {"prompt": "..."}')
    try:
        reply = llm.text_json(ask, temperature=0.4, ollama_options={"num_ctx": 4096, "num_predict": 2000})
        return str(reply.get("prompt", "")).strip() or fallback
    except Exception as e:
        print(f"[animate] LLM failed ({str(e)[:100]}), using the pose as prompt")
        return fallback


def ai_frames(char, text_layer, prompt, seconds, seed, steps):
    wf = json.loads(WAN_WORKFLOW.read_text())
    start = Image.new("RGB", (SIZE, SIZE), AI_BACKGROUND)
    fitted = fit_canvas(char, scale=0.9)
    start.paste(fitted, (0, 0), fitted)
    start.save(COMFY_DIR / "input" / "animate_sticker_input.png")
    frames_n = max(1, round(seconds * 24 / 4)) * 4 + 1  # Wan needs 4n+1 frames at 24 fps
    wf["5"]["inputs"]["image"] = "animate_sticker_input.png"
    wf["6"]["inputs"]["text"] = f"{prompt}, {AI_STYLE}"
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
    ap.add_argument("--mode", choices=["ai", "loop"], default="ai",
                    help="ai (default): Wan 2.2 animates the character's pose; loop: quick CPU-only body loop")
    ap.add_argument("--motion", choices=["auto"] + MOTIONS, default="auto",
                    help="loop mode: loop type; auto (default) matches the sticker's text/pose")
    ap.add_argument("--seconds", type=float, default=3.0, help="clip length, max 3 (default 3)")
    ap.add_argument("--no-pingpong", action="store_true",
                    help="ai mode: play the generated clip only forwards (default: forth and back, loops smoothly)")
    ap.add_argument("--prompt", default=None,
                    help="ai mode: the action, e.g. 'the cat waves its raised paw' (default: derived from the pose)")
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
        if args.mode == "loop":
            motion = pick_motion(meta, i) if args.motion == "auto" else args.motion
            print(f"[motion ] {f.name}: {motion}")
            fps, frames = 30, motion_frames(char, text_layer, motion, seconds, 30)
        else:
            prompt = args.prompt or action_prompt(meta)
            pingpong = not args.no_pingpong
            print(f"[animate] {f.name}: {prompt}")
            # ping-pong: render half the length, then play it back in reverse - half the GPU time, seamless loop
            fps, frames = 24, ai_frames(char, text_layer, prompt, seconds / 2 if pingpong else seconds,
                                        args.seed + i, args.steps)
            if pingpong:
                frames = frames + frames[-2:0:-1]
        out = f.with_suffix(".webm")
        size, crf = encode_webm(frames, fps, out)
        warn = "" if size <= MAX_BYTES else "  - still over 256 KB, try --seconds 1.5"
        print(f"[done   ] {out}  ({size // 1024} KB, crf {crf}){warn}")


if __name__ == "__main__":
    sys.exit(main())
