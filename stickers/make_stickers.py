"""Turn an input image into transparent WebP stickers with a short reaction caption.

Pipeline per sticker:
  1. Caption  - Ollama vision model picks a reaction, an expression/pose to draw and (optionally) a text
  2. Stylize  - ComfyUI draws the same animal/person as a cartoonish sticker acting out that
                reaction (optional); the photo only defines who to draw, not the composition
  3. Cut out  - rembg removes the background
  4. Compose  - white die-cut outline + text, one style for the whole batch (or no text at all),
                512x512 transparent WebP

Usage (via 2-stickers.bat, which uses ComfyUI's embedded Python):
  2-stickers.bat photo.jpg [--count 3] [--lang German] [--text "Custom text"] [--no-text]
                           [--engine sdxl] [--strength 0.55] [--no-stylize] [--seed 42]

Stylize engines (--engine):
  flux1         FLUX.1 [schnell] text-to-image from the vision model's look description (default). Needs
                models/diffusion_models/flux1-schnell-Q4_K_S.gguf, models/text_encoders/clip_l.safetensors
                and t5-v1_1-xxl-encoder-Q5_K_M.gguf, models/vae/ae.safetensors. SDXL is broken on this
                ComfyUI build (gray-square bug, upstream issue), FLUX.1 works instead.
  sdxl          DreamShaperXL Turbo img2img. Currently broken, see above - kept for when it's fixed.
  photomaker    Same SDXL checkpoint plus PhotoMaker face-identity conditioning. Needs
                models/photomaker/photomaker-v2.bin. Best resemblance-for-VRAM tradeoff, once SDXL works.
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

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont, ImageOps
from scipy import ndimage

HERE = Path(__file__).resolve().parent
COMFY_DIR = HERE.parent / "ComfyUI_windows_portable" / "ComfyUI"
COMFY_URL = "http://127.0.0.1:8188"
OLLAMA_URL = "http://127.0.0.1:11434"
VISION_MODEL = "qwen3-vl:4b"
FONTS = Path("C:/Windows/Fonts")
FREDOKA = HERE / "fonts" / "Fredoka.ttf"  # OFL, rounded bold like classic chat stickers

SIZE = 512          # sticker canvas (WhatsApp/Telegram spec)
BORDER = 10         # white die-cut outline in px
MAX_BYTES = 100_000  # WhatsApp limit for static stickers

# One style per input image (picked by the seed), so a batch reads as one sticker pack.
TEXT_STYLES = [
    {"font": FREDOKA, "fill": "#FFF3D6", "stroke": "#4A2A17", "placement": "bottom"},
    {"font": FREDOKA, "fill": "white", "stroke": "#D6336C", "placement": "bottom"},
    {"font": FREDOKA, "fill": "#FFD43B", "stroke": "#5C3A10", "placement": "bottom"},
    {"font": FREDOKA, "fill": "white", "stroke": "#1C6FB8", "placement": "bottom"},
]

ENGINES = {
    "flux1": "sticker_workflow_flux1_api.json",
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
def caption(img, count, lang, texts=None):
    buf = io.BytesIO()
    img.convert("RGB").resize((min(img.width, 768), int(img.height * min(img.width, 768) / img.width))).save(buf, "JPEG", quality=90)
    prompt = (
        "Look at this image. Reply with JSON only, in this exact shape: "
        '{"subject": "<English description of the main animal or person so an illustrator can draw the same '
        'individual again: species/breed, fur or hair colors and markings, eye color, distinctive features '
        '(beard, glasses, ...) - only the look, no pose, no facial expression or mood, no background, max 30 words>", '
        + (f'"stickers": [<one entry per text, in this order: {json.dumps(texts, ensure_ascii=False)}, each '
           if texts else f'"stickers": [<{count} different reaction-sticker ideas, each ') +
        '{"expression": "<a specific, exaggerated pose/face that unmistakably acts out that exact emotion for an '
        'image generator: always the face (eyes, mouth) AND what the body/paws/hands do - e.g. hugging a big red '
        'heart with eyes closed and a blissful smile for love, waving one paw with a big open-mouthed smile for hi, wide-mouthed mid-yawn for tired, one paw/hand covering the face for embarrassment, '
        'running mid-stride for on my way, big pleading eyes for please, eyes closed turned away for annoyed - '
        'optionally plus one small doodle that underlines it (floating pink hearts, question marks, zzz, sparkles, '
        'sweat drop, motion lines) - no words, no quotes, nothing written - max 20 words>", '
        + ('"text": "<that text, unchanged>"}>]}' if texts else
           f'"text": "<a short reaction-sticker text in {lang}, 1-3 words, like chat-sticker classics: '
           '\\"Hi there!\\", \\"Yes!\\", \\"Nope\\", \\"Thank youuuu\\", \\"Wait, what?\\", \\"Please?\\", \\"Oh no!\\", \\"Kisses!\\" '
           '- or an empty string if the pose already says it without text; no hashtags, no emojis>}>]}')
    )
    payload = {
        "model": VISION_MODEL,
        "messages": [{"role": "user", "content": prompt, "images": [base64.b64encode(buf.getvalue()).decode()]}],
        "format": "json",
        "stream": False,
        "think": False,
        "keep_alive": 0,  # free the VRAM right away for ComfyUI
        # Ollama defaults to a 32k context: its KV cache pushes half the model onto the CPU on 6 GB.
        # qwen3-vl still thinks despite think=False, so leave room for thinking + answer.
        "options": {"temperature": 0.9, "num_ctx": 6144, "num_predict": 3000},
    }
    try:
        res = http_json(f"{OLLAMA_URL}/api/chat", payload)
    except urllib.error.HTTPError:
        payload.pop("think")  # model without thinking support
        res = http_json(f"{OLLAMA_URL}/api/chat", payload)
    data = json.loads(res["message"]["content"])
    stickers = [
        {"expression": str(s.get("expression", "")).strip(), "text": str(s.get("text", "")).strip()}
        for s in data.get("stickers", []) if isinstance(s, dict)
    ]
    return str(data.get("subject", "the subject")).strip(), stickers


# ---------------------------------------------------------------- 2. stylize
def stylize(img, subject, expression, seed, strength, idx, engine):
    look = subject
    if expression:
        subject = f"{subject}, {expression}"
    # Pre-scale to ~1 megapixel, multiples of 64 (fits SDXL and the DiT edit models alike)
    scale = (1024 * 1024 / (img.width * img.height)) ** 0.5
    w, h = max(64, round(img.width * scale / 64) * 64), max(64, round(img.height * scale / 64) * 64)
    name = f"sticker_input_{idx}.png"
    img.convert("RGB").resize((w, h), Image.LANCZOS).save(COMFY_DIR / "input" / name)

    wf = json.loads((HERE / ENGINES[engine]).read_text())
    if engine == "flux1":
        text = wf["6"]["inputs"]["text"].replace("SUBJECT", look)
        wf["6"]["inputs"]["text"] = text.replace("EXPRESSION", expression or "smiling happily")
        wf["7"]["inputs"]["seed"] = seed
    elif engine == "sdxl":
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
    mask = np.array(rgba.getchannel("A")) > 16
    labeled, n = ndimage.label(mask)
    if n > 1:
        # drop specks, keep the subject plus doodles around it (hearts, question marks ...)
        sizes = ndimage.sum(mask, labeled, range(1, n + 1))
        mask = np.isin(labeled, np.flatnonzero(sizes >= sizes.max() * 0.003) + 1)
        rgba.putalpha(Image.fromarray(np.where(mask, np.array(rgba.getchannel("A")), 0).astype(np.uint8)))
    bbox = Image.fromarray(mask).getbbox()
    return rgba.crop(bbox) if bbox else rgba


# ---------------------------------------------------------------- 4. compose
def load_font(path, size):
    font = ImageFont.truetype(str(path), size)
    if path == FREDOKA:
        font.set_variation_by_name("Bold")
    return font


def fit_text(draw, text, font_path, has_stroke, max_w, max_h):
    """Largest size where the text (wrapped to <= 2 lines) fits max_w x max_h."""
    words = text.split()
    candidates = [[" ".join(words)]]
    for i in range(1, len(words)):
        candidates.append([" ".join(words[:i]), " ".join(words[i:])])
    for size in range(96, 17, -2):
        font = load_font(font_path, size)
        stroke = max(3, size // 9) if has_stroke else 0
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
    return load_font(font_path, 18), 3 if has_stroke else 0, [" ".join(words)]


def compose(cutout, text, style):
    canvas = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    pad = BORDER + 6
    text_h = int(SIZE * 0.26) if text else 0
    top = text and style["placement"] == "top"

    # Subject: fit into the area on the other side of the text band (may overlap it a bit)
    area_w, area_h = SIZE - 2 * pad, SIZE - 2 * pad - int(text_h * 0.6)
    subj = ImageOps.contain(cutout, (area_w, area_h), Image.LANCZOS)
    subj_top = pad + int(text_h * 0.6) if top else pad
    x, y = (SIZE - subj.width) // 2, subj_top + (area_h - subj.height) // 2

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
        font, stroke, lines = fit_text(draw, text, style["font"], style["stroke"] is not None, SIZE - 2 * pad, text_h)
        line_h = font.size * 1.08 + stroke
        ty = pad if top else SIZE - pad - len(lines) * line_h
        for line in lines:
            w = draw.textbbox((0, 0), line, font=font, stroke_width=stroke)[2]
            tx = (SIZE - w) / 2
            if style.get("badge"):
                draw.rounded_rectangle((tx - 14, ty - 6, tx + w + 14, ty + font.size * 1.08 + 6),
                                        radius=16, fill=style["badge"])
            # white outer rim around the colored stroke, like a die-cut text sticker
            draw.text((tx, ty), line, font=font, fill="white", stroke_width=stroke + 5, stroke_fill="white")
            draw.text((tx, ty), line, font=font, fill=style["fill"], stroke_width=stroke, stroke_fill=style["stroke"])
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
    ap.add_argument("--no-text", action="store_true", help="no text on any sticker, just the stylized image")
    ap.add_argument("--engine", choices=list(ENGINES), default="flux1", help="stylize engine, see above (default flux1)")
    ap.add_argument("--strength", type=float, default=0.55, help="img2img denoise 0.3 (close to photo) .. 0.8 (free); sdxl/photomaker only")
    ap.add_argument("--no-stylize", action="store_true", help="skip ComfyUI, only cut out the original")
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--out", type=Path, default=HERE / "out")
    args = ap.parse_args()

    src = ImageOps.exif_transpose(Image.open(args.image)).convert("RGB")
    args.out.mkdir(parents=True, exist_ok=True)
    seed = args.seed if args.seed is not None else random.randint(0, 2**31)

    # 1. captions - each sticker gets an "expression" (how the subject should pose to act out the
    # reaction, feeds the stylize prompt) and a "text" (may be empty, the pose can speak for itself)
    subject, stickers = "the subject", []
    if args.text:
        stickers = [{"expression": "", "text": t} for t in args.text]
    if not args.text or not args.no_stylize:
        if reachable(OLLAMA_URL):
            if reachable(COMFY_URL):
                # ComfyUI keeps FLUX in VRAM after a run; on 6 GB the vision model then crawls into a timeout
                req = urllib.request.Request(f"{COMFY_URL}/free", data=b'{"unload_models": true, "free_memory": true}',
                                             headers={"Content-Type": "application/json"})
                urllib.request.urlopen(req, timeout=30).read()
            print(f"[caption] asking {VISION_MODEL} ...")
            subject, llm_stickers = caption(src, args.count, args.lang, args.text)
            if args.text:  # keep the given texts, take the poses the model picked for them
                for s, l in zip(stickers, llm_stickers):
                    s["expression"] = l["expression"]
            else:
                stickers = llm_stickers
            print(f"[caption] subject: {subject}")
        else:
            print("[caption] Ollama not reachable - no captions")
    stickers = (stickers or [{"expression": "", "text": ""}]) * args.count
    count = len(args.text) if args.text else args.count
    if args.no_text:
        stickers = [{**s, "text": ""} for s in stickers]

    stylize_on = not args.no_stylize
    if stylize_on and not reachable(COMFY_URL):
        print("[stylize] ComfyUI not running (start-comfyui.bat) - using original image")
        stylize_on = False

    stem = args.image.stem
    for i in range(count):
        expression, text = stickers[i]["expression"], stickers[i]["text"]
        base = src
        if stylize_on:
            # same seed for the whole batch: FLUX then draws the same-looking character in the same style
            print(f"[stylize] sticker {i + 1}/{count} (seed {seed if expression else seed + i}, engine {args.engine}): {expression or '-'}")
            base = stylize(src, subject, expression, seed if expression else seed + i, args.strength, i, args.engine)
        print(f"[cutout ] sticker {i + 1}/{count} ...")
        cut = cut_out(base)
        sticker = compose(cut, text, TEXT_STYLES[seed % len(TEXT_STYLES)])
        path = args.out / f"{stem}_sticker_{i + 1}.webp"
        size, q = save_webp(sticker, path)
        print(f"[done   ] {path}  \"{text}\"  ({size // 1024} KB, q={q})")


if __name__ == "__main__":
    sys.exit(main())
