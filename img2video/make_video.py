"""Animate a still image into a short MP4 clip (image-to-video), or make a clip from a description alone
(text-to-video), with Wan 2.2 TI2V 5B in ComfyUI - the same model does both.

Pipeline per clip:
  1. Motion  - what should happen: --prompt, or Ollama's vision model looks at the image and
               writes a short motion/camera prompt (falls back to a generic one without Ollama)
  2. Animate - ComfyUI Wan 2.2 5B: the image is the first frame, the model generates the rest
  3. Save    - H.264 MP4 at 24 fps
Text-to-video (--text, no image): the LLM turns the idea into detailed video prompts (--exact: use the text
as it is), the clip gets the --aspect ratio.

Usage (via 10-image-to-video.bat, which uses ComfyUI's embedded Python):
  10-image-to-video.bat photo.jpg [--prompt "the dog wags its tail, camera slowly zooms in"]
                                  [--seconds 3] [--res 720p] [--steps 20] [--count 2] [--seed 42]
  10-image-to-video.bat --text "a red fox runs through deep snow at sunrise" [--aspect 9:16] [--count 2]
  --res 480p/720p is generated natively; 1080p/1440p/4k is generated at 720p (Wan 2.2 5B's maximum,
  anything bigger does not fit into VRAM) and then upscaled with ffmpeg (lanczos).
Needs models/diffusion_models/wan2.2_ti2v_5B_fp16.safetensors, models/vae/wan2.2_vae.safetensors
and models/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors (0-download-all.bat).
Result: img2video/out/<image or text>_<seed>.mp4
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


ASPECTS = {"16:9": (16, 9), "9:16": (9, 16), "1:1": (1, 1), "4:3": (4, 3), "3:4": (3, 4)}


def describe_scene(text, count):
    """Text-to-video: the idea as detailed prompts, the way Wan was trained (subject, action, place, light, camera)."""
    prompt = (f"Turn this idea into {count} different English prompts for a text-to-video model that makes one "
              f"continuous shot of a few seconds: {text!r}. Each prompt 40-70 words in one paragraph: the subject and "
              "what it looks like, what it does (one clear motion), the place, the light and mood, the visual "
              "style, and one simple camera move (static, slow push-in, pan, tracking shot ...). No scene cuts, "
              "no on-screen text. Reply with JSON only: {\"prompts\": [\"...\"]}")
    prompts = llm.text_json(prompt, temperature=0.8).get("prompts", [])
    return [str(p).strip() for p in prompts if str(p).strip()]


# ---------------------------------------------------------------- 2. animate
def animate(img, prompt, seed, size, frames, steps, cfg):
    """img: the first frame, or None for text-to-video."""
    wf = json.loads((HERE / "img2video_workflow_api.json").read_text())
    w, h = size
    if img is None:  # the latent node starts from noise without a start image
        del wf["5"], wf["8"]["inputs"]["start_image"]
    else:
        img.convert("RGB").resize((w, h), Image.LANCZOS).save(COMFY_DIR / "input" / "img2video_input.png")
    wf["6"]["inputs"]["text"] = f"{prompt}, {QUALITY}"
    wf["8"]["inputs"].update(width=w, height=h, length=frames)
    wf["9"]["inputs"].update(seed=seed, steps=steps, cfg=cfg)
    wf["11"]["inputs"]["fps"] = float(FPS)

    pid = http_json(f"{COMFY_URL}/prompt", {"prompt": wf})["prompt_id"]
    start, down = time.time(), 0
    while True:
        try:
            hist = http_json(f"{COMFY_URL}/history/{pid}", timeout=30)
            down = 0
        except OSError as e:  # busy loading a model is fine, gone for a minute is not
            down += 1
            if down >= 12:
                raise RuntimeError(f"ComfyUI stopped answering ({e}) - it may have run out of memory, see its window")
            time.sleep(5)
            continue
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
    ap.add_argument("image", type=Path, nargs="?", help="start image (jpg/png/webp); leave out with --text")
    ap.add_argument("--text", help="text-to-video: what the clip shows, no start image")
    ap.add_argument("--exact", action="store_true", help="with --text: use the text as the prompt as it is "
                    "(default: the LLM writes detailed prompts from it, which Wan follows much better)")
    ap.add_argument("--aspect", choices=list(ASPECTS), default="16:9", help="with --text: shape of the clip (default 16:9)")
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

    if bool(args.image) == bool(args.text):
        ap.error("give a start image or --text \"...\" (one of them)")
    if args.image and not args.image.is_file():
        ap.error(f"image not found: {args.image}")
    if not reachable(COMFY_URL):
        sys.exit("ComfyUI is not running (start-comfyui.bat)")

    count = max(args.count, len(args.prompt))
    prompts = list(args.prompt)
    if args.text:
        aw, ah = ASPECTS[args.aspect]
        img, shape, name = None, Image.new("RGB", (aw * 64, ah * 64)), slug(args.text)[:50]
        if len(prompts) < count and not args.exact and llm.available():
            print(f"[prompt ] {llm.text_label()} writes the video prompt ...")
            try:
                prompts += describe_scene(args.text, count - len(prompts))
                llm.unload()
            except Exception as e:
                print(f"[prompt ] LLM failed ({str(e)[:150]}), using the text as it is")
        prompts = (prompts or [args.text])
    else:
        img = shape = ImageOps.exif_transpose(Image.open(args.image)).convert("RGB")
        name = slug(args.image.stem)
    if img is not None and len(prompts) < count and llm.available():
        print(f"[motion ] asking {llm.label()} ...")
        try:
            prompts += describe_motion(img, count - len(prompts))
        except Exception as e:
            print(f"[motion ] vision model failed ({e}), using a generic motion prompt")
    prompts = (prompts or [FALLBACK_MOTION])
    prompts = (prompts * count)[:count]

    target = RESOLUTIONS.get(args.res)
    size = short_side_size(shape, min(target, NATIVE_MAX)) if target else video_size(shape, args.size)
    factor = target / NATIVE_MAX if target and target > NATIVE_MAX else None
    frames = max(1, round(args.seconds * FPS / 4)) * 4 + 1  # Wan needs 4n+1 frames
    seed = args.seed if args.seed is not None else random.randint(0, 2**31)
    args.out.mkdir(parents=True, exist_ok=True)
    for i, prompt in enumerate(prompts):
        s = seed + i
        print(f"[animate] clip {i + 1}/{count}: {size[0]}x{size[1]}, {frames} frames, seed {s}")
        print(f"[prompt ] {prompt}")
        mp4 = args.out / f"{name}_{s}.mp4"
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
