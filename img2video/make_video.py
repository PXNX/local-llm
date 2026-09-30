"""Animate a still image into a short MP4 clip (image-to-video) with Wan 2.2 TI2V 5B in ComfyUI.

Pipeline per clip:
  1. Motion  - what should happen: --prompt, or Ollama's vision model looks at the image and
               writes a short motion/camera prompt (falls back to a generic one without Ollama)
  2. Animate - ComfyUI Wan 2.2 5B: the image is the first frame, the model generates the rest
  3. Save    - H.264 MP4 at 24 fps

Usage (via 10-image-to-video.bat, which uses ComfyUI's embedded Python):
  10-image-to-video.bat photo.jpg [--prompt "the dog wags its tail, camera slowly zooms in"]
                                  [--seconds 3] [--res 720p] [--steps 20] [--count 2] [--seed 42]
  --res 480p/720p is generated natively; 1080p/1440p/4k is generated at 720p (Wan 2.2 5B's maximum,
  anything bigger does not fit into VRAM) and then upscaled with ffmpeg (lanczos).
Needs models/diffusion_models/wan2.2_ti2v_5B_fp16.safetensors, models/vae/wan2.2_vae.safetensors
and models/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors (0-download-all.bat).
Result: img2video/out/<image>_<seed>.mp4
"""
import argparse
import io
import json
import random
import re
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path

from PIL import Image, ImageOps

HERE = Path(__file__).resolve().parent
COMFY_DIR = HERE.parent / "ComfyUI_windows_portable" / "ComfyUI"
COMFY_URL = "http://127.0.0.1:8188"
sys.path.insert(0, str(HERE.parent))
from common import llm  # noqa: E402

FPS = 24
FALLBACK_MOTION = "natural subtle motion, the subject moves slightly and breathes, gentle slow camera push-in"
QUALITY = "smooth natural motion, consistent lighting, high quality, detailed"


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


def slug(text):
    return re.sub(r"[^A-Za-z0-9]+", "_", text).strip("_") or "video"


RESOLUTIONS = {"480p": 480, "720p": 720, "1080p": 1080, "1440p": 1440, "4k": 2160}
NATIVE_MAX = 720


def short_side_size(img, short_side):
    """Keep the image's aspect ratio, short side ~short_side, both sides multiples of 32 (Wan 2.2 VAE)."""
    scale = short_side / min(img.width, img.height)
    return (max(32, round(img.width * scale / 32) * 32), max(32, round(img.height * scale / 32) * 32))


def upscale(src, dst, factor):
    """Lanczos upscale of the finished clip on the CPU (the GPU is idle again by then)."""
    import subprocess

    import imageio_ffmpeg

    cmd = [imageio_ffmpeg.get_ffmpeg_exe(), "-y", "-loglevel", "error", "-i", str(src),
           "-vf", f"scale=trunc(iw*{factor}/2)*2:trunc(ih*{factor}/2)*2:flags=lanczos",
           "-c:v", "libx264", "-preset", "medium", "-crf", "16", "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(dst)]
    subprocess.run(cmd, check=True)


def video_size(img, long_side):
    """Keep the image's aspect ratio, long side ~long_side, both sides multiples of 32 (Wan 2.2 VAE)."""
    scale = long_side / max(img.width, img.height)
    return (max(32, round(img.width * scale / 32) * 32), max(32, round(img.height * scale / 32) * 32))


# ---------------------------------------------------------------- 1. motion
def describe_motion(img, count):
    buf = io.BytesIO()
    ImageOps.contain(img.convert("RGB"), (768, 768)).save(buf, "JPEG", quality=90)
    prompt = (f"You direct a short few-second video clip that starts exactly with this image. "
              f"Write {count} different English prompts for an image-to-video model, each max 40 words: "
              "first what the scene shows, then what moves (people, animals, objects, water, hair, clouds ...) "
              "and one simple camera move (slow zoom in, pan left, static, orbit ...). Keep it plausible, "
              "no scene cuts, no new characters. Reply with JSON only: {\"prompts\": [\"...\"]}")
    prompts = llm.vision_json(prompt, buf.getvalue(), temperature=0.7).get("prompts", [])
    return [str(p).strip() for p in prompts if str(p).strip()]


# ---------------------------------------------------------------- 2. animate
def animate(img, prompt, seed, size, frames, steps, cfg):
    wf = json.loads((HERE / "img2video_workflow_api.json").read_text())
    w, h = size
    img.convert("RGB").resize((w, h), Image.LANCZOS).save(COMFY_DIR / "input" / "img2video_input.png")
    wf["6"]["inputs"]["text"] = f"{prompt}, {QUALITY}"
    wf["8"]["inputs"].update(width=w, height=h, length=frames)
    wf["9"]["inputs"].update(seed=seed, steps=steps, cfg=cfg)
    wf["11"]["inputs"]["fps"] = float(FPS)

    pid = http_json(f"{COMFY_URL}/prompt", {"prompt": wf})["prompt_id"]
    start = time.time()
    while True:
        hist = http_json(f"{COMFY_URL}/history/{pid}")
        if pid in hist:
            entry = hist[pid]
            if entry.get("status", {}).get("status_str") == "error":
                raise RuntimeError(f"ComfyUI error: {entry['status']}")
            out = entry["outputs"]["12"]["images"][0]
            break
        print(f"\r[animate] {int(time.time() - start) // 60} min {int(time.time() - start) % 60:02d} s ...", end="", flush=True)
        time.sleep(5)
    print()
    q = urllib.parse.urlencode({"filename": out["filename"], "subfolder": out["subfolder"], "type": out["type"]})
    with urllib.request.urlopen(f"{COMFY_URL}/view?{q}") as r:
        return r.read()


# ---------------------------------------------------------------- main
def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("image", type=Path, help="start image (jpg/png/webp)")
    ap.add_argument("--prompt", action="append", default=[],
                    help="what should move/happen, English works best (repeatable, one per clip); default: ask the vision model")
    ap.add_argument("--count", type=int, default=1, help="number of clips, each with its own prompt and seed (default 1)")
    ap.add_argument("--seconds", type=float, default=3.0, help=f"clip length in seconds at {FPS} fps (default 3, max ~5 on 6 GB VRAM)")
    ap.add_argument("--size", type=int, default=832, help="long side in pixels, aspect ratio follows the image (default 832; 640 is faster)")
    ap.add_argument("--res", choices=list(RESOLUTIONS),
                    help="output resolution (short side), overrides --size; above 720p the clip is upscaled from 720p")
    ap.add_argument("--steps", type=int, default=20, help="sampling steps (default 20; 12-15 is faster but blurrier)")
    ap.add_argument("--cfg", type=float, default=5.0, help="prompt strength (default 5)")
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--out", type=Path, default=HERE / "out")
    args = ap.parse_args()

    if not args.image.is_file():
        ap.error(f"image not found: {args.image}")
    if not reachable(COMFY_URL):
        sys.exit("ComfyUI is not running (start-comfyui.bat)")

    img = ImageOps.exif_transpose(Image.open(args.image)).convert("RGB")
    count = max(args.count, len(args.prompt))
    prompts = list(args.prompt)
    if len(prompts) < count and llm.available():
        print(f"[motion ] asking {llm.label()} ...")
        try:
            prompts += describe_motion(img, count - len(prompts))
        except Exception as e:
            print(f"[motion ] vision model failed ({e}), using a generic motion prompt")
    prompts = (prompts or [FALLBACK_MOTION])
    prompts = (prompts * count)[:count]

    target = RESOLUTIONS.get(args.res)
    size = short_side_size(img, min(target, NATIVE_MAX)) if target else video_size(img, args.size)
    factor = target / NATIVE_MAX if target and target > NATIVE_MAX else None
    frames = max(1, round(args.seconds * FPS / 4)) * 4 + 1  # Wan needs 4n+1 frames
    seed = args.seed if args.seed is not None else random.randint(0, 2**31)
    args.out.mkdir(parents=True, exist_ok=True)
    for i, prompt in enumerate(prompts):
        s = seed + i
        print(f"[animate] clip {i + 1}/{count}: {size[0]}x{size[1]}, {frames} frames, seed {s}")
        print(f"[prompt ] {prompt}")
        mp4 = args.out / f"{slug(args.image.stem)}_{s}.mp4"
        clip = animate(img, prompt, s, size, frames, args.steps, args.cfg)
        if factor:
            raw = mp4.with_name(mp4.stem + "_720p.mp4")
            raw.write_bytes(clip)
            print(f"[upscale] {args.res}: x{factor:g} (lanczos)")
            upscale(raw, mp4, factor)
            raw.unlink()
        else:
            mp4.write_bytes(clip)
        mp4.with_suffix(".txt").write_text(f"{prompt}\n", encoding="utf-8")
        print(f"[done   ] {mp4}")


if __name__ == "__main__":
    sys.exit(main())
