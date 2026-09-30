"""Draw famous people (or yourself, from a photo) as simple flat 2D political-cartoon
caricatures in several facial expressions / poses - or objects and buildings (S-400,
refinery, Kremlin, oil tanker ...) in several views/states - optionally as transparent PNG and SVG.

Pipeline per person:
  1. Features - for --photo, Ollama's vision model describes the look (hair, glasses, beard ...)
  2. Draw     - ComfyUI FLUX.1 schnell (GGUF): text-to-image for known names, img2img from the photo otherwise
  3. Cut out  - rembg removes the background (--cutout)
  4. Vector   - vtracer traces an SVG, flat cartoons trace very cleanly (--vectorize)

Usage (via 7-caricatures.bat, which uses ComfyUI's embedded Python):
  7-caricatures.bat --who "Emmanuel Macron" --who "Friedrich Merz" [--vectorize]
  7-caricatures.bat --photo me.jpg --name Felix [--features "round glasses, beard"]
  8-objects.bat --thing "S-400 air defense system" --thing "oil refinery" [--vectorize]
Result: caricatures/out/<name>/<name>_<variant>_<seed>.png (+ .svg)
"""
import argparse
import base64
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
OLLAMA_URL = "http://127.0.0.1:11434"
VISION_MODEL = "qwen3-vl:4b"

STYLE = ("simple flat 2D political satire cartoon, thick black outlines, flat pastel colors, "
         "minimal shading, centered, plain white background, no text, no letters, no watermark")
PERSON = "caricature of {subject}, {variant}, oversized round head, small body, simple dot eyes, full body, " + STYLE
THING = "{subject}, {variant}, cute simplified cartoon drawing, chunky rounded shapes, " + STYLE

EXPRESSIONS = [
    "neutral expression, standing",
    "laughing with open mouth, arms raised",
    "angry and shouting, pointing finger",
    "shocked with wide eyes, hands on cheeks",
    "smug grin, arms crossed",
    "crying with big tears",
]
VIEWS = [
    "side view",
    "three-quarter view",
    "on fire with black smoke clouds",
    "broken and damaged, cracks and debris",
]


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
    return re.sub(r"[^A-Za-z0-9]+", "_", text).strip("_") or "person"


# ---------------------------------------------------------------- 1. features
def describe(img):
    buf = io.BytesIO()
    ImageOps.contain(img.convert("RGB"), (768, 768)).save(buf, "JPEG", quality=90)
    prompt = ("Describe this person's most recognizable visual features for a caricature artist, "
              "in max 20 English words: gender, age, hair (color, style), facial hair, glasses, "
              "face shape, typical clothing. Reply with JSON only: {\"features\": \"...\"}")
    payload = {
        "model": VISION_MODEL,
        "messages": [{"role": "user", "content": prompt, "images": [base64.b64encode(buf.getvalue()).decode()]}],
        "format": "json",
        "stream": False,
        "think": False,
        "keep_alive": 0,  # free the VRAM right away for ComfyUI
        "options": {"temperature": 0.2},
    }
    try:
        res = http_json(f"{OLLAMA_URL}/api/chat", payload)
    except urllib.error.HTTPError:
        payload.pop("think")  # model without thinking support
        res = http_json(f"{OLLAMA_URL}/api/chat", payload)
    return str(json.loads(res["message"]["content"]).get("features", "")).strip()


# ---------------------------------------------------------------- 2. draw
def draw(prompt, seed, photo=None, strength=0.75):
    wf = json.loads((HERE / "caricature_workflow_api.json").read_text())
    wf["6"]["inputs"]["text"] = prompt
    wf["7"]["inputs"]["seed"] = seed
    if photo is not None:
        # img2img: start from the photo (pre-scaled to ~1 MP, multiples of 64) instead of noise
        scale = (1024 * 1024 / (photo.width * photo.height)) ** 0.5
        w, h = max(64, round(photo.width * scale / 64) * 64), max(64, round(photo.height * scale / 64) * 64)
        photo.convert("RGB").resize((w, h), Image.LANCZOS).save(COMFY_DIR / "input" / "caricature_input.png")
        wf["10"] = {"class_type": "LoadImage", "inputs": {"image": "caricature_input.png"}}
        wf["11"] = {"class_type": "VAEEncode", "inputs": {"pixels": ["10", 0], "vae": ["3", 0]}}
        wf["7"]["inputs"]["latent_image"] = ["11", 0]
        wf["7"]["inputs"]["denoise"] = strength

    pid = http_json(f"{COMFY_URL}/prompt", {"prompt": wf})["prompt_id"]
    while True:
        hist = http_json(f"{COMFY_URL}/history/{pid}")
        if pid in hist:
            entry = hist[pid]
            if entry.get("status", {}).get("status_str") == "error":
                raise RuntimeError(f"ComfyUI error: {entry['status']}")
            out = entry["outputs"]["8"]["images"][0]
            break
        time.sleep(1)
    q = urllib.parse.urlencode({"filename": out["filename"], "subfolder": out["subfolder"], "type": out["type"]})
    with urllib.request.urlopen(f"{COMFY_URL}/view?{q}") as r:
        return Image.open(io.BytesIO(r.read())).convert("RGB")


# ---------------------------------------------------------------- 3. cut out
_session = None


def flood_cut_out(img, tolerance=40):
    """Flat cartoons on a plain background: flood-fill the background from the border. Unlike
    rembg it keeps white parts inside the drawing (cabins, teeth ...) and gives a hard 0/255
    alpha, which vtracer needs to trace the cutout cleanly. None if the background is not plain."""
    import numpy as np
    from scipy import ndimage
    px = np.asarray(img.convert("RGB")).astype(int)
    border = np.concatenate([px[0], px[-1], px[:, 0], px[:, -1]])
    bg_color = np.median(border, axis=0)
    labels, _ = ndimage.label(np.abs(px - bg_color).max(axis=2) <= tolerance)
    touching = set(labels[0]) | set(labels[-1]) | set(labels[:, 0]) | set(labels[:, -1])
    touching.discard(0)
    fg = ~np.isin(labels, list(touching))
    fg = ndimage.binary_opening(fg)  # drop specks
    labels, count = ndimage.label(fg)
    if count == 0 or not 0.03 < fg.mean() < 0.9:
        return None
    biggest = 1 + int(np.argmax(ndimage.sum(fg, labels, range(1, count + 1))))
    mask = ndimage.binary_fill_holes(labels == biggest)
    rgba = Image.fromarray(np.dstack([np.asarray(img.convert("RGB")), mask.astype(np.uint8) * 255]), "RGBA")
    return rgba.crop(rgba.getchannel("A").getbbox())


def cut_out(img, plain_background=True):
    rgba = flood_cut_out(img) if plain_background else None
    if rgba is not None:
        return rgba
    global _session
    from rembg import new_session, remove
    if _session is None:
        _session = new_session("isnet-general-use")
    rgba = remove(img.convert("RGB"), session=_session)
    # hard alpha: vtracer mis-traces semi-transparent pixels
    rgba.putalpha(rgba.getchannel("A").point(lambda v: 255 if v > 128 else 0))
    bbox = rgba.getchannel("A").getbbox()
    return rgba.crop(bbox) if bbox else rgba


# ---------------------------------------------------------------- 4. vectorize
def vectorize(png, svg):
    import vtracer
    # "logo" preset of vectorize/trace_svg.py: crisp flat-color shapes
    vtracer.convert_image_to_svg_py(str(png), str(svg), colormode="color", hierarchical="stacked",
                                    path_precision=8, mode="polygon", filter_speckle=8,
                                    color_precision=8, corner_threshold=30, layer_difference=8)


# ---------------------------------------------------------------- main
def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--who", action="append", default=[], help="famous person to draw (repeatable)")
    ap.add_argument("--thing", action="append", default=[], help="object/building to draw, e.g. \"oil tanker\" (repeatable)")
    ap.add_argument("--photo", type=Path, help="photo of a person (e.g. yourself) to caricature")
    ap.add_argument("--name", help="folder/file name for --photo (default: photo file name)")
    ap.add_argument("--features", help="extra look hints, e.g. \"grey hair, rimless glasses\" (added to the prompt)")
    ap.add_argument("--strength", type=float, default=0.75,
                    help="--photo img2img denoise: 0.5 close to the photo .. 0.9 free cartoon (default 0.75)")
    ap.add_argument("--count", type=int, default=None,
                    help=f"images per person/thing, one per expression/view (default {len(EXPRESSIONS)} / {len(VIEWS)})")
    ap.add_argument("--variant", "--expression", action="append",
                    help="own expression/pose (people) or view/state (things) instead of the built-in list (repeatable)")
    ap.add_argument("--cutout", action="store_true", help="also save a transparent PNG (rembg)")
    ap.add_argument("--vectorize", action="store_true", help="also trace an SVG (of the cutout if --cutout)")
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--out", type=Path, default=HERE / "out")
    args = ap.parse_args()

    if not args.who and not args.photo and not args.thing:
        ap.error("give --who \"Name\", --thing \"object\" and/or --photo image.jpg")
    if args.vectorize:
        args.cutout = True  # tracing the uncut image would also trace the white background
    if not reachable(COMFY_URL):
        sys.exit("ComfyUI is not running (start-comfyui.bat)")

    jobs = [(slug(w), PERSON, w, None) for w in args.who] + [(slug(t), THING, t, None) for t in args.thing]
    if args.photo:
        photo = ImageOps.exif_transpose(Image.open(args.photo)).convert("RGB")
        feats = ""
        if reachable(OLLAMA_URL):
            print(f"[looks  ] asking {VISION_MODEL} ...")
            feats = describe(photo)
            print(f"[looks  ] {feats}")
        jobs.append((slug(args.name or args.photo.stem), PERSON, feats or "a person", photo))

    seed = args.seed if args.seed is not None else random.randint(0, 2**31)
    for name, template, subject, photo in jobs:
        variants = args.variant or (VIEWS if template is THING else EXPRESSIONS)
        count = args.count or len(variants)
        variants = (variants * (count // len(variants) + 1))[:count]
        if args.features:
            subject = f"{subject}, {args.features}"
        folder = args.out / name
        folder.mkdir(parents=True, exist_ok=True)
        for i, variant in enumerate(variants):
            s = seed + i
            print(f"[draw   ] {name} {i + 1}/{len(variants)}: {variant} (seed {s}) ...")
            img = draw(template.format(subject=subject, variant=variant), s, photo, args.strength)
            png = folder / f"{name}_{slug(variant.split(',')[0])}_{s}.png"
            img.save(png)
            if args.cutout:
                png = png.with_name(png.stem + "_cutout.png")
                cut_out(img).save(png)
            if args.vectorize:
                vectorize(png, png.with_suffix(".svg"))
            print(f"[done   ] {png}" + ("  + .svg" if args.vectorize else ""))


if __name__ == "__main__":
    sys.exit(main())
