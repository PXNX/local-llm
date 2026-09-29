"""Turn an input image into transparent WebP stickers with a funny caption.

Pipeline per sticker:
  1. Caption  - Ollama vision model describes the image and writes funny texts
  2. Stylize  - ComfyUI SDXL img2img turns it into a cartoon sticker (optional)
  3. Cut out  - rembg removes the background
  4. Compose  - white die-cut outline + meme text, 512x512 transparent WebP

Usage (via 2-stickers.bat, which uses ComfyUI's embedded Python):
  2-stickers.bat photo.jpg [--count 3] [--lang German] [--text "Custom text"]
                           [--engine sdxl] [--strength 0.55] [--no-stylize] [--seed 42]

Stylize engines (--engine):
  sdxl          DreamShaperXL Turbo img2img (default). Fast, fits 6 GB VRAM, but only loosely
                based on the photo since it starts from noise seeded with it.
  photomaker    Same SDXL checkpoint plus PhotoMaker face-identity conditioning. Needs
                models/photomaker/photomaker-v2.bin. Best resemblance-for-VRAM tradeoff.
  qwen-image21  Qwen-Image 2.1 edit model, native reference-image conditioning. Needs
                models/diffusion_models/qwen-image-2.1-UC-fp8.safetensors (or -NVFP4),
                models/text_encoders/qwen3vl_8b_text_encoder.safetensors and
                models/vae/qwen_image_2.1_vae.safetensors. Large model, slow without a lot of VRAM.
  flux2-klein   FLUX.2 [klein] edit model. Needs models/diffusion_models/flux2-klein-base-9b-fp8.safetensors,
                models/text_encoders/flux2_klein_qwen3_text_encoder.safetensors and
                models/vae/flux2_vae.safetensors. Large model, slow without a lot of VRAM.
"""
import argparse
import base64
import io
import json
import random
import shutil
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont, ImageOps

HERE = Path(__file__).resolve().parent
COMFY_DIR = HERE.parent / "ComfyUI_windows_portable" / "ComfyUI"
COMFY_URL = "http://127.0.0.1:8188"
OLLAMA_URL = "http://127.0.0.1:11434"
VISION_MODEL = "qwen3-vl:4b"
FONT = Path("C:/Windows/Fonts/impact.ttf")

SIZE = 512          # sticker canvas (WhatsApp/Telegram spec)
BORDER = 10         # white die-cut outline in px
MAX_BYTES = 100_000  # WhatsApp limit for static stickers

ENGINES = {
    "sdxl": "sticker_workflow_api.json",
    "photomaker": "sticker_workflow_photomaker_api.json",
    "qwen-image21": "sticker_workflow_qwen_image21_api.json",
    "flux2-klein": "sticker_workflow_flux2_klein_api.json",
}


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


# ---------------------------------------------------------------- 1. caption
def caption(img, count, lang):
    buf = io.BytesIO()
    img.convert("RGB").resize((min(img.width, 768), int(img.height * min(img.width, 768) / img.width))).save(buf, "JPEG", quality=90)
    prompt = (
        "Look at this image. Reply with JSON only, in this exact shape: "
        '{"subject": "<short English description of the main subject for an image generator, max 15 words>", '
        f'"captions": [<{count} different short reaction-sticker texts in {lang}, 1-3 words each, '
        "like chat-sticker classics: \"Hi there!\", \"Yes!\", \"Nope\", \"Thank youuuu\", \"Wait, what?\", "
        "\"Please?\", \"Oh no!\", \"Kisses!\" - punchy everyday reactions/exclamations that fit this image; "
        "no hashtags, no emojis>]}"
    )
    payload = {
        "model": VISION_MODEL,
        "messages": [{"role": "user", "content": prompt, "images": [base64.b64encode(buf.getvalue()).decode()]}],
        "format": "json",
        "stream": False,
        "think": False,
        "keep_alive": 0,  # free the VRAM right away for ComfyUI
        "options": {"temperature": 0.9},
    }
    try:
        res = http_json(f"{OLLAMA_URL}/api/chat", payload)
    except urllib.error.HTTPError:
        payload.pop("think")  # model without thinking support
        res = http_json(f"{OLLAMA_URL}/api/chat", payload)
    data = json.loads(res["message"]["content"])
    caps = [str(c).strip() for c in data.get("captions", []) if str(c).strip()]
    return str(data.get("subject", "the subject")).strip(), caps


# ---------------------------------------------------------------- 2. stylize
def stylize(img, subject, seed, strength, idx, engine):
    # Pre-scale to ~1 megapixel, multiples of 64 (fits SDXL and the DiT edit models alike)
    scale = (1024 * 1024 / (img.width * img.height)) ** 0.5
    w, h = max(64, round(img.width * scale / 64) * 64), max(64, round(img.height * scale / 64) * 64)
    name = f"sticker_input_{idx}.png"
    img.convert("RGB").resize((w, h), Image.LANCZOS).save(COMFY_DIR / "input" / name)

    wf = json.loads((HERE / ENGINES[engine]).read_text())
    if engine == "sdxl":
        wf["2"]["inputs"]["image"] = name
        wf["4"]["inputs"]["text"] = wf["4"]["inputs"]["text"].replace("SUBJECT", subject)
        wf["6"]["inputs"]["seed"] = seed
        wf["6"]["inputs"]["denoise"] = strength
    elif engine == "photomaker":
        wf["2"]["inputs"]["image"] = name
        wf["10"]["inputs"]["text"] = wf["10"]["inputs"]["text"].replace("SUBJECT", subject)
        wf["6"]["inputs"]["seed"] = seed
        wf["6"]["inputs"]["denoise"] = strength
    elif engine == "qwen-image21":
        wf["4"]["inputs"]["image"] = name
        wf["5"]["inputs"]["prompt"] = wf["5"]["inputs"]["prompt"].replace("SUBJECT", subject)
        wf["6"]["inputs"]["seed"] = seed
    elif engine == "flux2-klein":
        wf["4"]["inputs"]["image"] = name
        wf["6"]["inputs"]["text"] = wf["6"]["inputs"]["text"].replace("SUBJECT", subject)
        wf["11"]["inputs"]["width"], wf["11"]["inputs"]["height"] = w, h
        wf["12"]["inputs"]["seed"] = seed

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


def cut_out(img):
    global _session
    from rembg import new_session, remove
    if _session is None:
        _session = new_session("isnet-general-use")
    rgba = remove(img.convert("RGB"), session=_session)
    bbox = rgba.getchannel("A").point(lambda v: 255 if v > 16 else 0).getbbox()
    return rgba.crop(bbox) if bbox else rgba


# ---------------------------------------------------------------- 4. compose
def fit_text(draw, text, max_w, max_h):
    """Largest Impact size where the text (wrapped to <= 2 lines) fits max_w x max_h."""
    words = text.upper().split()
    candidates = [[" ".join(words)]]
    for i in range(1, len(words)):
        candidates.append([" ".join(words[:i]), " ".join(words[i:])])
    for size in range(96, 17, -2):
        font = ImageFont.truetype(str(FONT), size)
        stroke = max(3, size // 12)
        best = None
        for lines in candidates:
            widths = [draw.textbbox((0, 0), l, font=font, stroke_width=stroke)[2] for l in lines]
            height = len(lines) * (size * 1.08 + stroke)
            if max(widths) <= max_w and height <= max_h:
                # prefer the most balanced wrap
                score = max(widths) - min(widths)
                if best is None or score < best[0]:
                    best = (score, lines)
        if best:
            return font, stroke, best[1]
    return ImageFont.truetype(str(FONT), 18), 3, [" ".join(words)]


def compose(cutout, text):
    canvas = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    pad = BORDER + 6
    text_h = int(SIZE * 0.26) if text else 0

    # Subject: fit into the area above the text (it may overlap the text a bit)
    area_w, area_h = SIZE - 2 * pad, SIZE - 2 * pad - int(text_h * 0.6)
    subj = ImageOps.contain(cutout, (area_w, area_h), Image.LANCZOS)
    x, y = (SIZE - subj.width) // 2, pad + (area_h - subj.height) // 2

    layer = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    layer.paste(subj, (x, y), subj)
    alpha = layer.getchannel("A").point(lambda v: 255 if v > 40 else 0)

    # White die-cut outline + soft shadow
    outline = alpha.filter(ImageFilter.MaxFilter(2 * BORDER + 1)).filter(ImageFilter.GaussianBlur(1.2))
    shadow = outline.filter(ImageFilter.GaussianBlur(4)).point(lambda v: v * 0.35)
    canvas.paste((0, 0, 0, 255), (0, 3), shadow)
    canvas.paste((255, 255, 255, 255), (0, 0), outline)
    canvas.alpha_composite(layer)

    if text:
        draw = ImageDraw.Draw(canvas)
        font, stroke, lines = fit_text(draw, text, SIZE - 2 * pad, text_h)
        line_h = font.size * 1.08 + stroke
        ty = SIZE - pad - len(lines) * line_h
        for line in lines:
            w = draw.textbbox((0, 0), line, font=font, stroke_width=stroke)[2]
            draw.text(((SIZE - w) / 2, ty), line, font=font, fill="white", stroke_width=stroke, stroke_fill="black")
            ty += line_h
    return canvas


def save_webp(img, path):
    for q in range(92, 30, -6):
        buf = io.BytesIO()
        img.save(buf, "WEBP", quality=q, method=6, exact=False)
        if buf.tell() <= MAX_BYTES:
            break
    path.write_bytes(buf.getvalue())
    return buf.tell(), q


# ---------------------------------------------------------------- main
def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("--count", type=int, default=3, help="number of stickers (default 3)")
    ap.add_argument("--lang", default="English", help="caption language (default English)")
    ap.add_argument("--text", action="append", help="use this caption instead of the LLM (repeatable)")
    ap.add_argument("--engine", choices=list(ENGINES), default="sdxl", help="stylize engine, see above (default sdxl)")
    ap.add_argument("--strength", type=float, default=0.55, help="img2img denoise 0.3 (close to photo) .. 0.8 (free); sdxl/photomaker only")
    ap.add_argument("--no-stylize", action="store_true", help="skip ComfyUI, only cut out the original")
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--out", type=Path, default=HERE / "out")
    args = ap.parse_args()

    src = ImageOps.exif_transpose(Image.open(args.image)).convert("RGB")
    args.out.mkdir(parents=True, exist_ok=True)
    seed = args.seed if args.seed is not None else random.randint(0, 2**31)

    # 1. captions
    subject, caps = "the subject", []
    if args.text:
        caps = args.text
    if not args.text or not args.no_stylize:
        if reachable(OLLAMA_URL):
            print(f"[caption] asking {VISION_MODEL} ...")
            subject, llm_caps = caption(src, args.count, args.lang)
            caps = caps or llm_caps
            print(f"[caption] subject: {subject}")
        else:
            print("[caption] Ollama not reachable - no captions")
    caps = (caps or [""]) * args.count
    count = len(args.text) if args.text else args.count

    stylize_on = not args.no_stylize
    if stylize_on and not reachable(COMFY_URL):
        print("[stylize] ComfyUI not running (start-comfyui.bat) - using original image")
        stylize_on = False

    stem = args.image.stem
    for i in range(count):
        base = src
        if stylize_on:
            print(f"[stylize] sticker {i + 1}/{count} (seed {seed + i}, engine {args.engine}) ...")
            base = stylize(src, subject, seed + i, args.strength, i, args.engine)
        print(f"[cutout ] sticker {i + 1}/{count} ...")
        cut = cut_out(base)
        sticker = compose(cut, caps[i])
        path = args.out / f"{stem}_sticker_{i + 1}.webp"
        size, q = save_webp(sticker, path)
        print(f"[done   ] {path}  \"{caps[i]}\"  ({size // 1024} KB, q={q})")


if __name__ == "__main__":
    sys.exit(main())
