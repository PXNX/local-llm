"""Backgrounds: simple drawn sets in the minimal freeonis look (white, a ground strip, faint line art)
or, for any other place ("the Oval Office", "map of the Black Sea"), a FLUX.1 drawing from ComfyUI,
cached in comedy/cache/bg/. Without ComfyUI a map is faked from noise, other places get the plain set.
"""
import hashlib
import math
import random
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).resolve().parent
CACHE = HERE / "cache" / "bg"
FONTS = Path("C:/Windows/Fonts")
DRAWN = ("plain", "room", "city", "field", "night")

BG_PROMPT = ("{desc}, empty background scene without any people or characters, wide flat 2D cartoon background, "
             "simple shapes, soft pastel colors, clean thin outlines, calm empty space in the middle, "
             "no text, no letters, no watermark")
MAP_PROMPT = ("simple flat cartoon map of {place}, top-down view, pale yellow land, light blue sea, clean dark "
              "coastlines, thin grey borders, no text, no labels, no letters, no watermark")


def font(size, name="ariblk.ttf"):
    for n in (name, "arialbd.ttf"):
        if (FONTS / n).exists():
            return ImageFont.truetype(str(FONTS / n), size)
    return ImageFont.load_default(size)


def layout(W, H):
    """(top of the ground strip, where the feet stand) as fractions of H."""
    return (0.70, 0.745) if H > W else (0.885, 0.93)


def is_map(kind):
    return kind.lower().startswith("map")


def ai_path(kind, W, H, seed):
    """Cache file of a FLUX background, or None for the drawn sets."""
    if kind.lower() in DRAWN:
        return None
    key = hashlib.sha1(f"{kind.lower()}|{W > H}|{seed}".encode()).hexdigest()[:12]
    return CACHE / f"{key}.png"


def generate(kind, W, H, seed, mc):
    """FLUX.1 background via caricatures/make_caricatures.draw, saved into the cache."""
    path = ai_path(kind, W, H, seed)
    if path is None or path.exists():
        return path
    place = kind[3:].strip().removeprefix("of ").strip() if is_map(kind) else kind
    prompt = MAP_PROMPT.format(place=place) if is_map(kind) else BG_PROMPT.format(desc=kind)
    print(f"[scene  ] drawing background: {kind} ...")
    img = mc.draw(prompt, seed, size=(1344, 768) if W >= H else (768, 1344))
    CACHE.mkdir(parents=True, exist_ok=True)
    img.save(path)
    return path


def _ground(d, w, h, top, rng, fill=(191, 199, 174), line=(169, 178, 150), tick=(150, 160, 130)):
    y = int(h * top)
    d.rectangle((0, y, w, h), fill=fill)
    d.line((0, y, w, y), fill=line, width=max(2, h // 300))
    for _ in range(int(w / h * 30)):
        x, yy = rng.uniform(0, w), rng.uniform(y + h * 0.015, h * 0.99)
        s = h * 0.008
        d.line((x, yy, x - s * 0.4, yy - s), fill=tick, width=max(1, h // 500))
        d.line((x + s * 0.6, yy, x + s * 0.4, yy - s * 0.9), fill=tick, width=max(1, h // 500))


def _room(d, w, h, top):
    c, lw = (226, 226, 226), max(2, h // 280)
    u = min(w, h)
    y = h * top
    # window with a cross, framed picture, floor lamp, plant, a skirting line
    wx, wy = w * 0.12, h * 0.16
    d.rounded_rectangle((wx, wy, wx + u * 0.3, wy + u * 0.36), radius=u // 60, outline=c, width=lw)
    d.line((wx + u * 0.15, wy, wx + u * 0.15, wy + u * 0.36), fill=c, width=lw)
    d.line((wx, wy + u * 0.18, wx + u * 0.3, wy + u * 0.18), fill=c, width=lw)
    px, py = w * 0.70, h * 0.18
    d.rectangle((px, py, px + u * 0.22, py + u * 0.16), outline=c, width=lw)
    d.polygon([(px + u * 0.03, py + u * 0.13), (px + u * 0.09, py + u * 0.05), (px + u * 0.13, py + u * 0.1),
               (px + u * 0.16, py + u * 0.07), (px + u * 0.19, py + u * 0.13)], outline=c, width=lw)
    lx = w * 0.9
    d.line((lx, y, lx, y - u * 0.55), fill=c, width=lw)
    d.polygon([(lx - u * 0.06, y - u * 0.55), (lx + u * 0.06, y - u * 0.55), (lx + u * 0.04, y - u * 0.66),
               (lx - u * 0.04, y - u * 0.66)], outline=c, width=lw)
    d.line((lx - u * 0.05, y, lx + u * 0.05, y), fill=c, width=lw)
    tx = w * 0.04
    d.polygon([(tx, y), (tx + u * 0.1, y), (tx + u * 0.085, y - u * 0.1), (tx + u * 0.015, y - u * 0.1)], outline=c, width=lw)
    for a in (-0.5, 0, 0.5):
        d.arc((tx + u * (0.02 + a * 0.06), y - u * 0.28, tx + u * (0.08 + a * 0.06), y - u * 0.08), 180, 360, fill=c, width=lw)
    d.rectangle((0, y, w, h), fill=(244, 244, 242))
    d.line((0, y, w, y), fill=c, width=lw)


def _city(d, w, h, top, rng, color=(201, 197, 221)):
    y = h * top
    x = -rng.uniform(0, h * 0.05)
    while x < w:
        bw = rng.uniform(h * 0.05, h * 0.12)
        bh = rng.uniform(h * 0.12, h * 0.34)
        d.rectangle((x, y - bh, x + bw, y), fill=color)
        if rng.random() < 0.3:
            d.rectangle((x + bw * 0.4, y - bh - h * 0.04, x + bw * 0.5, y - bh), fill=color)
        x += bw + rng.uniform(0, h * 0.01)


def _field(d, w, h, top, rng):
    c, lw = (170, 170, 170), max(2, h // 300)
    y = h * top
    tx = w * rng.uniform(0.72, 0.85)
    d.line((tx, y, tx, y - h * 0.32), fill=c, width=lw * 3)
    for a, l in ((-0.8, 0.12), (-0.3, 0.1), (0.5, 0.13), (1.0, 0.08)):
        yy = y - h * (0.18 + 0.1 * abs(a))
        d.line((tx, yy, tx + math.cos(a - math.pi / 2) * h * l, yy + math.sin(a - math.pi / 2) * h * l), fill=c, width=lw * 2)
    sx = w * rng.uniform(0.18, 0.3)
    d.line((sx, y, sx, y - h * 0.14), fill=c, width=lw * 2)
    d.polygon([(sx - h * 0.02, y - h * 0.14), (sx + h * 0.09, y - h * 0.14), (sx + h * 0.11, y - h * 0.125),
               (sx + h * 0.09, y - h * 0.11), (sx - h * 0.02, y - h * 0.11)], fill=(250, 250, 250), outline=c, width=lw)


def _night(img, d, w, h, top, rng):
    for y in range(int(h * top)):
        k = y / (h * top)
        d.line((0, y, w, y), fill=(int(27 + 20 * k), int(36 + 24 * k), int(64 + 40 * k)))
    for _ in range(int(w / h * 60)):
        x, y, r = rng.uniform(0, w), rng.uniform(0, h * top * 0.8), rng.uniform(h * 0.001, h * 0.003)
        d.ellipse((x - r, y - r, x + r, y + r), fill=(255, 250, 220))
    mx, my, mr = w * 0.8, h * 0.18, h * 0.07
    d.ellipse((mx - mr, my - mr, mx + mr, my + mr), fill=(250, 240, 200))
    d.ellipse((mx - mr * 0.5, my - mr * 1.1, mx + mr * 1.3, my + mr * 0.7), fill=(35, 46, 80))
    _city(d, w, h, top, rng, color=(22, 28, 50))


def fake_map(w, h, seed):
    """Pastel land/sea map from smooth noise (no labels) - stands in for a FLUX map."""
    rng = np.random.default_rng(seed)
    gw, gh = 9, max(5, int(9 * h / w))
    grid = rng.random((gh, gw)).astype(np.float32)
    field = np.asarray(Image.fromarray((grid * 255).astype(np.uint8)).resize((w // 4, h // 4), Image.BICUBIC),
                       dtype=np.float32) / 255
    fine = rng.random((gh * 4, gw * 4)).astype(np.float32)
    field += 0.18 * np.asarray(Image.fromarray((fine * 255).astype(np.uint8)).resize((w // 4, h // 4), Image.BICUBIC),
                               dtype=np.float32) / 255
    land = field > np.quantile(field, 0.45)
    mask = Image.fromarray((land * 255).astype(np.uint8)).resize((w, h), Image.BILINEAR).filter(ImageFilter.GaussianBlur(3))
    mask = mask.point(lambda v: 255 if v > 127 else 0)
    img = Image.new("RGB", (w, h), (160, 212, 230))
    img.paste((246, 231, 176), (0, 0), mask)
    edge = mask.filter(ImageFilter.MaxFilter(5)).point(lambda v: v)
    edge = Image.fromarray(((np.asarray(edge) > 0) & ~(np.asarray(mask.filter(ImageFilter.MinFilter(5))) > 0)).astype(np.uint8) * 255)
    img.paste((110, 143, 160), (0, 0), edge)
    return img


def label(img, text):
    """Big spaced caps at the top, like the country names on freeonis maps."""
    if not text:
        return img
    d = ImageDraw.Draw(img)
    w, h = img.size
    f = font(int(min(w, h) * 0.06))
    spaced = "  ".join(text.upper())
    tw = d.textlength(spaced, font=f)
    d.text(((w - tw) / 2, h * 0.07), spaced, font=f, fill=(58, 58, 58))
    return img


def background(kind, text, W, H, scale, seed, path=None):
    """RGB background at scale x output size. path: FLUX drawing (from generate) if there is one."""
    w, h = int(W * scale), int(H * scale)
    top, _ = layout(W, H)
    rng = random.Random(seed)
    k = kind.lower()
    if path is not None and Path(path).exists():
        src = Image.open(path).convert("RGB")
        s = max(w / src.width, h / src.height)
        src = src.resize((int(src.width * s) + 1, int(src.height * s) + 1), Image.LANCZOS)
        img = src.crop(((src.width - w) // 2, (src.height - h) // 2, (src.width - w) // 2 + w, (src.height - h) // 2 + h))
        return label(img, text)
    if is_map(k):
        return label(fake_map(w, h, seed), text)
    img = Image.new("RGB", (w, h), (255, 255, 255))
    d = ImageDraw.Draw(img)
    if k == "room":
        _room(d, w, h, top)
    elif k == "city":
        img.paste((250, 251, 255), (0, 0, w, int(h * top)))
        _city(d, w, h, top, rng)
        _ground(d, w, h, top, rng)
    elif k == "field":
        img.paste((246, 248, 250), (0, 0, w, int(h * top)))
        _field(d, w, h, top, rng)
        _ground(d, w, h, top, rng, fill=(213, 220, 185), line=(190, 198, 160), tick=(170, 180, 140))
    elif k == "night":
        _night(img, d, w, h, top, rng)
        _ground(d, w, h, top, rng, fill=(52, 66, 58), line=(40, 52, 45), tick=(70, 88, 76))
    else:
        _ground(d, w, h, top, rng)
    return label(img, text)
