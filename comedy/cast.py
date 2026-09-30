"""Finds the drawings for the sketch, draws missing ones and falls back to placeholders.

Characters: caricatures/out/<Who>/ (flow 7, one file per expression) or, with --use-screenshots,
characters/out/<Name>/ (flow 6). Things: caricatures/out/<thing>/ (flow 8: side view, on fire,
damaged) and hand-made sets like caricatures/out/ww3/<thing>/cutout.png.
Missing drawings are made with caricatures/make_caricatures.py (FLUX.1 in ComfyUI) when it runs.
"""
import hashlib
import math
import re
import sys
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
CARICATURES = REPO / "caricatures" / "out"
SCREENSHOTS = REPO / "characters" / "out"
sys.path.insert(0, str(REPO / "caricatures"))
import make_caricatures as mc  # noqa: E402

# face -> the flow-7 expression that draws it (same order as mc.EXPRESSIONS)
FACE_PROMPTS = dict(zip(["neutral", "laughing", "angry", "shocked", "smug", "crying"], mc.EXPRESSIONS))
STATE_PROMPTS = {"normal": mc.VIEWS[0], "fire": mc.VIEWS[2], "damaged": mc.VIEWS[3]}
STATE_WORDS = {"normal": ("side_view", "three_quarter", "cutout"), "fire": ("fire", "burning"),
               "damaged": ("damaged", "broken", "destroyed")}
IMG_EXT = (".png", ".webp")


def _tokens(text):
    return set(re.findall(r"[a-z0-9]+", text.lower().replace("-", " "))) - {"the", "a", "an", "of", "cutout"}


def _variant_key(prompt):
    return mc.slug(prompt.split(",")[0])


def _cutouts(folder):
    """Transparent drawings in a folder: flow 7/8 *_cutout.png, or <sub>/cutout.png sets."""
    if not folder.is_dir():
        return []
    files = sorted(p for p in folder.iterdir() if p.suffix.lower() in IMG_EXT and "_cutout" in p.stem)
    if not files:
        files = sorted(p for p in folder.iterdir() if p.suffix.lower() in IMG_EXT and not p.name.startswith("_"))
    return files


class Cast:
    def __init__(self, generate=False, use_screenshots=False, seed=0):
        self.generate = generate
        self.use_screenshots = use_screenshots
        self.seed = seed
        self.made = 0

    # ------------------------------------------------------------ characters
    def character(self, member, faces):
        """{face: image path or None (placeholder)} for the faces the sketch uses."""
        who = member["who"]
        folders = [CARICATURES / mc.slug(who), CARICATURES / mc.slug(member["name"])]
        found = {}
        for folder in folders:
            for p in _cutouts(folder):
                for face, prompt in FACE_PROMPTS.items():
                    if p.stem.startswith(f"{folder.name}_{_variant_key(prompt)}_"):
                        found.setdefault(face, p)
        out = {}
        for face in faces:
            if face not in found and self.generate:
                found[face] = self._draw_person(member, face, folders[0])
            out[face] = found.get(face)
        missing = [f for f, p in out.items() if p is None]
        if missing:
            shots = self._screenshots(member) if self.use_screenshots else []
            other = found.get("neutral") or next(iter(found.values()), None)
            for i, face in enumerate(missing):
                out[face] = shots[(i * 7) % len(shots)] if shots else other
        return out

    def _screenshots(self, member):
        """Flow 6 cutouts: characters/out/<folder> whose name or names.txt entry matches."""
        names_txt = REPO / "characters" / "names.txt"
        aliases = {}
        if names_txt.exists():
            for line in names_txt.read_text(encoding="utf-8").splitlines():
                if "=" in line and not line.lstrip().startswith("#"):
                    folder, real = (s.strip() for s in line.split("=", 1))
                    aliases[folder] = real
        want = [_tokens(member["name"]), _tokens(member["who"])]
        for folder in sorted(SCREENSHOTS.iterdir()) if SCREENSHOTS.is_dir() else []:
            have = _tokens(folder.name) | _tokens(aliases.get(folder.name, ""))
            if any(w and w <= have for w in want):
                files = [p for p in _cutouts(folder) if p.suffix.lower() == ".png"]
                if files:
                    return files
        return []

    def _draw_person(self, member, face, folder):
        seed = self._seed(member["who"], face)
        subject = member["who"] + (f", {member['look']}" if member.get("look") else "")
        prompt = FACE_PROMPTS[face]
        print(f"[cast   ] drawing {member['who']}: {face} (seed {seed}) ...")
        return self._draw(mc.PERSON.format(subject=subject, variant=prompt), seed, folder,
                          f"{folder.name}_{_variant_key(prompt)}_{seed}")

    # ------------------------------------------------------------ things
    def thing(self, name, states):
        """{state: path or None} for states normal/fire/damaged."""
        folder, files = self._find_thing(name)
        found = {}
        for state in ("fire", "damaged", "normal"):
            for p in files:
                if any(w in p.stem.lower() for w in STATE_WORDS[state]) and p not in found.values():
                    found.setdefault(state, p)
                    break
        if folder is not None and folder.parent != CARICATURES and "damaged" not in found:
            destroyed = _cutouts(folder.parent / f"{folder.name}-destroyed")  # sets like ww3/tank + ww3/tank-destroyed
            if destroyed:
                found["damaged"] = destroyed[0]
        if files and "normal" not in found:
            rest = [p for p in files if p not in found.values()]
            found["normal"] = (rest or files)[0]
        out = {}
        for state in states:
            if state not in found and self.generate:
                target = folder if folder is not None and folder.parent == CARICATURES else CARICATURES / mc.slug(name)
                found[state] = self._draw_thing(name, state, target)
            out[state] = found.get(state)
        return out

    def _find_thing(self, name):
        """Best matching folder for a thing name: caricatures/out/<x>/ or caricatures/out/<set>/<x>/."""
        want = _tokens(name)
        best, best_score = None, 0.0
        if not CARICATURES.is_dir():
            return None, []
        for d in CARICATURES.iterdir():
            if not d.is_dir():
                continue
            cands = [d] + [s for s in d.iterdir() if s.is_dir() and (s / "cutout.png").exists()]
            for c in cands:
                have = _tokens(c.name)
                if not have or not _cutouts(c):
                    continue
                score = len(want & have) / len(want | have)
                if c.name.endswith("-destroyed"):
                    score -= 0.01
                if score > best_score:
                    best, best_score = c, score
        if best is None or best_score < 0.34:
            return None, []
        return best, _cutouts(best)

    def _draw_thing(self, name, state, folder):
        seed = self._seed(name, state)
        prompt = STATE_PROMPTS[state]
        print(f"[cast   ] drawing {name}: {state} (seed {seed}) ...")
        return self._draw(mc.THING.format(subject=name, variant=prompt), seed, folder,
                          f"{folder.name}_{_variant_key(prompt)}_{seed}")

    # ------------------------------------------------------------ shared
    def _seed(self, *parts):
        h = hashlib.sha1("|".join(parts).encode()).hexdigest()
        return (self.seed + int(h[:8], 16)) % 2**31

    def _draw(self, prompt, seed, folder, stem):
        folder.mkdir(parents=True, exist_ok=True)
        img = mc.draw(prompt, seed)
        img.save(folder / f"{stem}.png")
        path = folder / f"{stem}_cutout.png"
        mc.cut_out(img).save(path)
        self.made += 1
        return path


# ---------------------------------------------------------------- placeholders
SUIT = [(52, 62, 104), (70, 70, 74), (96, 60, 48), (40, 84, 70), (110, 40, 52), (60, 60, 120)]
SKIN = [(246, 196, 160), (236, 180, 140), (250, 210, 180)]


def placeholder_person(name, face, h=900):
    """Simple stand-in caricature (big round head, small suit body) so a sketch renders without ComfyUI."""
    k = int(hashlib.sha1(name.encode()).hexdigest()[:6], 16)
    w = int(h * 0.62)
    im = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    lw = max(4, h // 110)
    suit, skin = SUIT[k % len(SUIT)], SKIN[(k // 7) % len(SKIN)]
    cx, head_r = w / 2, h * 0.25
    head_cy = head_r + lw
    body_top, body_bot = head_cy + head_r * 0.8, h * 0.86
    d.rounded_rectangle((cx - w * 0.30, body_top, cx + w * 0.30, body_bot), radius=int(w * 0.14), fill=suit,
                        outline="black", width=lw)
    d.polygon([(cx - w * 0.07, body_top + 6), (cx + w * 0.07, body_top + 6), (cx, body_top + h * 0.13)], fill="white")
    d.polygon([(cx - w * 0.025, body_top + 10), (cx + w * 0.025, body_top + 10), (cx, body_top + h * 0.12)],
              fill=(220, 60, 60))
    for sx in (-1, 1):
        d.rectangle((cx + sx * w * 0.12 - w * 0.05, body_bot - 4, cx + sx * w * 0.12 + w * 0.05, h - lw * 2),
                    fill=(40, 40, 44), outline="black", width=lw)
    d.ellipse((cx - head_r, head_cy - head_r, cx + head_r, head_cy + head_r), fill=skin, outline="black", width=lw)
    hair = [(250, 214, 110), (70, 50, 40), (140, 140, 140), (30, 30, 30)][(k // 3) % 4]
    d.chord((cx - head_r, head_cy - head_r, cx + head_r, head_cy + head_r * 0.2), 180, 360, fill=hair,
            outline="black", width=lw)
    ey, er = head_cy + head_r * 0.12, head_r * 0.07
    for sx in (-1, 1):
        ex = cx + sx * head_r * 0.35
        if face == "laughing":
            d.arc((ex - er * 2, ey - er, ex + er * 2, ey + er * 2), 200, 340, fill="black", width=lw)
        else:
            r = er * (1.8 if face == "shocked" else 1)
            d.ellipse((ex - r, ey - r, ex + r, ey + r), fill="black")
        if face == "angry":
            d.line((ex - sx * er * 3, ey - er * 4, ex + sx * er * 3, ey - er * 2), fill="black", width=lw)
    my, mw = head_cy + head_r * 0.55, head_r * 0.35
    if face in ("laughing", "shocked", "angry"):
        d.ellipse((cx - mw * (0.6 if face == "shocked" else 1), my - mw * 0.3, cx + mw * (0.6 if face == "shocked" else 1),
                   my + mw * 0.6), fill=(90, 30, 30), outline="black", width=lw)
    elif face == "crying":
        d.arc((cx - mw, my, cx + mw, my + mw), 200, 340, fill="black", width=lw)
        for sx in (-1, 1):
            d.ellipse((cx + sx * head_r * 0.4 - er, ey + er * 2, cx + sx * head_r * 0.4 + er, ey + er * 5),
                      fill=(110, 180, 255))
    elif face == "smug":
        d.arc((cx - mw, my - mw * 0.6, cx + mw * 1.2, my + mw * 0.3), 20, 150, fill="black", width=lw)
    else:
        d.line((cx - mw * 0.7, my, cx + mw * 0.7, my), fill="black", width=lw)
    return im


def placeholder_thing(name, state, h=700):
    """Stand-in object: a rounded box with the name, orange when burning, grey and tilted when damaged."""
    words = name.upper()
    w = int(h * 1.3)
    im = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    lw = max(4, h // 90)
    fill = {"fire": (240, 150, 60), "damaged": (130, 130, 130)}.get(state, (170, 190, 215))
    d.rounded_rectangle((lw, h * 0.25, w - lw, h - lw), radius=h // 8, fill=fill, outline="black", width=lw)
    d.rectangle((w * 0.65, h * 0.05, w * 0.78, h * 0.3), fill=fill, outline="black", width=lw)
    from PIL import ImageFont
    try:
        f = ImageFont.truetype("arialbd.ttf", max(20, int(h * 0.1)))
    except OSError:
        f = ImageFont.load_default()
    lines, cur = [], ""
    for word in words.split():
        if d.textlength((cur + " " + word).strip(), font=f) > w * 0.85 and cur:
            lines.append(cur)
            cur = word
        else:
            cur = (cur + " " + word).strip()
    lines.append(cur)
    y = h * 0.62 - len(lines) * f.size * 0.6
    for line in lines:
        d.text((w / 2 - d.textlength(line, font=f) / 2, y), line, font=f, fill="black")
        y += f.size * 1.15
    if state == "damaged":
        for i in range(4):
            x = w * (0.2 + 0.2 * i)
            d.line((x, h * 0.3, x + h * 0.05, h * 0.5, x - h * 0.03, h * 0.7), fill="black", width=lw)
        im = im.rotate(-8, resample=Image.BICUBIC, expand=True)
    return im


def missile_like(name):
    return bool(_tokens(name) & {"missile", "drone", "shahed", "rocket", "bomb", "flamingo", "neptune", "storm",
                                 "shadow", "taurus", "atacms", "himars", "jet", "plane", "f16", "kamikaze", "uav"})


def placeholder_projectile(h=260):
    w = int(h * 3)
    im = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    lw = max(3, h // 30)
    d.polygon([(w * 0.05, h * 0.2), (w * 0.25, h * 0.45), (w * 0.25, h * 0.55), (w * 0.05, h * 0.8)],
              fill=(120, 130, 140), outline="black", width=lw)
    d.rounded_rectangle((w * 0.1, h * 0.36, w * 0.85, h * 0.64), radius=int(h * 0.14), fill=(200, 205, 210),
                        outline="black", width=lw)
    d.pieslice((w * 0.72, h * 0.36, w * 0.98, h * 0.64), 270, 90, fill=(220, 70, 60), outline="black", width=lw)
    return im


def star_points(cx, cy, r, points, inner=0.6, phase=0.0):
    pts = []
    for i in range(points * 2):
        a = math.pi * i / points + phase
        rr = r if i % 2 == 0 else r * inner
        pts.append((cx + rr * math.cos(a), cy + rr * math.sin(a)))
    return pts
