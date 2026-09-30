"""Draws the plan frame by frame (a pure function of time) and encodes it in parallel: the timeline
is split into chunks, each worker process pipes raw frames into its own ffmpeg (H.264), the chunks
are joined without re-encoding and the mixed audio is added."""
import math
import random
import subprocess
import sys
from collections import OrderedDict
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont, ImageOps

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
sys.path.insert(0, str(REPO / "topvideos"))
import make_top5 as top5  # noqa: E402

import cast as cast_mod  # noqa: E402
import scene as scene_mod  # noqa: E402

FONT = REPO / "stickers" / "fonts" / "Fredoka.ttf"
ZOOM_MAX = 3.2


def ease_out(k):
    k = min(1.0, max(0.0, k))
    return 1 - (1 - k) ** 3


def ease_in(k):
    k = min(1.0, max(0.0, k))
    return k * k


def at(changes, t, default=None):
    """Value of the last (time, value) change at or before t."""
    v = default
    for ct, cv in changes:
        if ct <= t:
            v = cv
        else:
            break
    return v


def sub_font(size):
    try:
        f = ImageFont.truetype(str(FONT), size)
        try:
            f.set_variation_by_axes([600])
        except Exception:
            pass
        return f
    except OSError:
        return scene_mod.font(size)


# ---------------------------------------------------------------- sprites
class Sprite:
    """A drawing kept at the largest size it is ever shown at, plus halved mip levels; any size is
    then one cheap bilinear resize away."""

    def __init__(self, img, max_h):
        img = img.convert("RGBA")
        box = img.getchannel("A").getbbox()
        if box:
            img = img.crop(box)
        h = max(8, int(min(max_h, 2600)))
        img = img.resize((max(1, round(img.width * h / img.height)), h), Image.LANCZOS)
        self.mips = [img]
        while self.mips[-1].height > 64:
            m = self.mips[-1]
            self.mips.append(m.resize((max(1, m.width // 2), max(1, m.height // 2)), Image.BILINEAR))
        self.aspect = img.width / img.height
        self.cache = OrderedDict()

    def get(self, w, h):
        w, h = max(1, int(w)), max(1, int(h))
        key = (w, h)
        if key in self.cache:
            self.cache.move_to_end(key)
            return self.cache[key]
        src = self.mips[0]
        for m in self.mips:
            if m.height >= h:
                src = m
        im = src.resize((w, h), Image.BILINEAR)
        self.cache[key] = im
        if len(self.cache) > 12:
            self.cache.popitem(last=False)
        return im


class Renderer:
    def __init__(self, plan):
        self.p = plan
        self.W, self.H = plan["W"], plan["H"]
        self.sprites = {}
        self.bgs = {}
        self.bg_view = (None, None)
        self.subs = {}
        self.unit = min(self.W, self.H)
        self.handle_img = None
        if plan.get("handle"):
            f = scene_mod.font(int(self.unit * 0.035))
            d = ImageDraw.Draw(Image.new("L", (1, 1)))
            w = int(d.textlength(plan["handle"], font=f)) + 8
            im = Image.new("RGBA", (w, int(f.size * 1.4)), (0, 0, 0, 0))
            ImageDraw.Draw(im).text((4, 0), plan["handle"], font=f, fill=(190, 190, 190, 210))
            self.handle_img = im

    # ------------------------------------------------------------ assets
    def sprite(self, name, state, kind, h):
        key = (name, state)
        if key not in self.sprites:
            path = self.p["sprites"].get(name, {}).get(state)
            if path:
                img = Image.open(path)
            elif kind == "char":
                img = cast_mod.placeholder_person(name, state)
            elif kind == "flyer" and cast_mod.missile_like(name):
                img = cast_mod.placeholder_projectile()
            else:
                img = cast_mod.placeholder_thing(name, state)
            self.sprites[key] = Sprite(img, h * ZOOM_MAX)
        return self.sprites[key]

    def background(self, si):
        if si not in self.bgs:
            sc = self.p["scenes"][si]
            self.bgs = {si: scene_mod.background(sc["kind"], sc["label"], self.W, self.H, 2, sc["seed"], sc["bg_path"])}
            self.bg_view = (None, None)
        return self.bgs[si]

    # ------------------------------------------------------------ camera
    def camera(self, sc, t):
        W, H = self.W, self.H
        shot_t, kind, focus = sc["shots"][0]
        zoom_punch = None
        for st, k, f in sc["shots"]:
            if st > t:
                break
            if k == "zoom":
                zoom_punch = st
            else:
                shot_t, kind, focus, zoom_punch = st, k, f, None
        items = {i["name"]: i for i in sc["items"]}
        it = items.get(focus)
        if kind == "wide" or it is None:
            cx, cy, z = W / 2, H / 2, 1.0
        else:
            h = it["h"]
            part = 0.58 if kind == "close" else 0.95
            z = min(ZOOM_MAX, max(1.15, 0.92 * H / (part * h)))
            top = it["y"] - h
            cx, cy = it["x"], top + part * h / 2 + 0.03 * h
            z *= 1 + 0.03 * min(1.0, (t - shot_t) / 4)  # slow push-in
        if zoom_punch is not None:
            z *= 1 + 0.3 * ease_out((t - zoom_punch) / 0.2)
        # screen shake after explosions
        for b in sc["booms"]:
            k = t - b["t"]
            if 0 <= k < 0.6:
                a = self.unit * 0.018 * (1 - k / 0.6)
                cx += a * math.sin(k * 97) / z
                cy += a * math.cos(k * 71) / z
        hw, hh = W / (2 * z), H / (2 * z)
        cx = min(W - hw, max(hw, cx))
        cy = min(H - hh, max(hh, cy))
        return cx, cy, z

    # ------------------------------------------------------------ frame
    def frame(self, t):
        p = self.p
        si = next((i for i, s in enumerate(p["scenes"]) if s["t0"] <= t < s["t1"]), len(p["scenes"]) - 1)
        sc = p["scenes"][si]
        cx, cy, z = self.camera(sc, t)
        W, H = self.W, self.H
        box = tuple(round(v * 2, 1) for v in (cx - W / (2 * z), cy - H / (2 * z), cx + W / (2 * z), cy + H / (2 * z)))
        if self.bg_view[0] != (si, box):
            self.bg_view = ((si, box), self.background(si).resize((W, H), Image.BILINEAR, box=box))
        canvas = self.bg_view[1].copy()

        def to_screen(x, y):
            return (x - cx) * z + W / 2, (y - cy) * z + H / 2

        order = sorted(sc["items"], key=lambda i: i["type"] == "char")  # things behind characters
        for it in order:
            self.draw_item(canvas, it, t, to_screen, z)
        for fl in sc["flights"]:
            self.draw_flight(canvas, fl, t, to_screen, z)
        for b in sc["booms"]:
            self.draw_boom(canvas, b, t, to_screen, z)
        if sc["end"]:
            self.draw_end(canvas, t - sc["t0"])
        if p["subtitles"]:
            self.draw_subtitle(canvas, t)
        if self.handle_img is not None:
            canvas.paste(self.handle_img, (int(W - self.handle_img.width - self.unit * 0.03), int(self.unit * 0.025)),
                         self.handle_img)
        return canvas

    def pose(self, it, t):
        """(dx, dy, angle, sx, sy, alpha-scale) from actions, talking and idle breathing."""
        h = it["h"]
        dx = dy = ang = 0.0
        sx = sy = 1.0
        scale = 1.0
        phase = (hash(it["name"]) % 100) / 16
        sy *= 1 + 0.008 * math.sin(t * 2 * math.pi * 0.45 + phase)
        if it["type"] == "char":
            for ln in self.p["lines"]:
                if ln["who"] == it["name"] and ln["t0"] <= t < ln["t1"]:
                    env = ln["env"]
                    a = env[min(len(env) - 1, int((t - ln["t0"]) * self.p["fps"]))]
                    a = round(a * 8) / 8
                    sy *= 1 + 0.045 * a
                    sx *= 1 - 0.02 * a
                    ang += 1.2 * a * math.sin(t * 7 + phase)
                    break
        fallen = 0.0
        for st, kind, d in it["actions"]:
            k = (t - st) / d if d else 1
            if kind == "fall" and t >= st:
                fallen = 88 * ease_in(k / 1)
                continue
            if not 0 <= k < 1:
                if kind.startswith("exit_") and k >= 1:
                    dx += (-1 if kind == "exit_left" else 1) * (self.W + it["w"])
                continue
            tt = t - st
            if kind == "jump":
                dy -= 0.22 * h * math.sin(math.pi * k)
                if k > 0.8:
                    sy *= 1 - 0.1 * math.sin(math.pi * (k - 0.8) / 0.2)
            elif kind == "shake":
                dx += 0.03 * h * math.sin(tt * 2 * math.pi * 9) * (1 - k)
            elif kind == "shrug":
                sy *= 1 + 0.06 * math.sin(2 * math.pi * k * 2)
                dy -= 0.02 * h * abs(math.sin(2 * math.pi * k))
            elif kind == "nod":
                ang += 5 * math.sin(2 * math.pi * 2 * k) * (1 - k)
            elif kind == "tremble":
                dx += 0.008 * h * math.sin(tt * 2 * math.pi * 23)
            elif kind == "spin":
                sx *= math.cos(2 * math.pi * k)
                dy -= 0.08 * h * math.sin(math.pi * k)
            elif kind.startswith("enter_"):
                side = -1 if kind == "enter_left" else 1
                off = (it["x"] + it["w"]) if side < 0 else (self.W - it["x"] + it["w"])
                dx += side * off * (1 - ease_out(k))
                dy -= 0.05 * h * abs(math.sin(3 * math.pi * k))
            elif kind.startswith("exit_"):
                side = -1 if kind == "exit_left" else 1
                off = (it["x"] + it["w"]) if side < 0 else (self.W - it["x"] + it["w"])
                dx += side * off * ease_in(k)
                dy -= 0.05 * h * abs(math.sin(3 * math.pi * k))
            elif kind == "pop_in":
                scale *= max(0.02, top5.ease_back(k))
            elif kind == "pop_out":
                scale *= max(0.02, 1 - k)
        if fallen:
            ang += fallen * (1 if it["x"] < self.W / 2 else -1)
        return dx, dy, ang, sx, sy, scale

    def draw_item(self, canvas, it, t, to_screen, z):
        if not at(it["visible"], t, True):
            return
        dx, dy, ang, sx, sy, scale = self.pose(it, t)
        if it["type"] == "char":
            state = at(it["faces"], t, "neutral")
        else:
            burning = at(it["states"], t, "normal") == "burning"
            paths = self.p["sprites"].get(it["name"], {})
            state = next((s for s in ("fire", "damaged") if paths.get(s)), "normal") if burning else "normal"
        spr = self.sprite(it["name"], state, it["type"], it["h"])
        h = it["h"] * z * sy * scale
        w = h * spr.aspect * abs(sx) / sy
        if h < 2 or w < 2:
            return
        fx, fy = to_screen(it["x"] + dx, it["y"] + dy)
        self.paste(canvas, spr.get(w, h), fx, fy, ang, mirror=sx < 0)
        if it["type"] == "thing" and at(it["states"], t, "normal") == "burning":
            since = t - next(st for st, s in it["states"] if s == "burning")
            self.flames(canvas, fx, fy - h, w, h, t, since)

    def paste(self, canvas, im, fx, fy, ang=0.0, mirror=False):
        """Paste with the bottom-centre (feet) at (fx, fy), rotated around the feet."""
        if mirror:
            im = ImageOps.mirror(im)
        ang = round(ang * 2) / 2
        if abs(ang) >= 0.5:
            h = im.height
            r = im.rotate(ang, resample=Image.BICUBIC, expand=True)
            th = math.radians(ang)
            vx, vy = h / 2 * math.sin(th), h / 2 * math.cos(th)
            cxr, cyr = fx - vx, fy - vy
            canvas.paste(r, (int(cxr - r.width / 2), int(cyr - r.height / 2)), r)
        else:
            canvas.paste(im, (int(fx - im.width / 2), int(fy - im.height)), im)

    def flames(self, canvas, x, top, w, h, t, since):
        d = ImageDraw.Draw(canvas)
        grow = ease_out(since / 0.4)
        lw = max(2, int(h / 90))
        for i in range(4):
            fxp = x + (i - 1.5) * w * 0.18
            fh = h * 0.32 * grow * (0.8 + 0.2 * math.sin(t * 11 + i * 1.7))
            fw = w * 0.13
            base = top + h * 0.2
            for col, s in (((255, 138, 31), 1.0), ((255, 217, 59), 0.55)):
                d.polygon([(fxp - fw * s, base), (fxp - fw * 0.6 * s, base - fh * s * 0.5),
                           (fxp + fw * 0.15 * math.sin(t * 9 + i), base - fh * s),
                           (fxp + fw * 0.6 * s, base - fh * s * 0.5), (fxp + fw * s, base)],
                          fill=col, outline="black" if s == 1.0 else None, width=lw)
        rng = random.Random(int(x))
        for k in range(5):
            age = (t - k * 0.5) % 2.5
            px = x + rng.uniform(-0.2, 0.2) * w + age * w * 0.1
            py = top - age * h * 0.35
            r = h * (0.05 + 0.05 * age) * grow
            g = int(120 + 40 * age)
            d.ellipse((px - r, py - r, px + r, py + r), fill=(g, g, g), outline=(70, 70, 70), width=lw)

    def draw_flight(self, canvas, fl, t, to_screen, z):
        k = (t - fl["t0"]) / (fl["t1"] - fl["t0"])
        if not 0 <= k < 1:
            return
        (x0, y0), (x1, y1) = fl["from"], fl["to"]
        x = x0 + (x1 - x0) * k
        y = y0 + (y1 - y0) * k + fl["arc"] * math.sin(math.pi * k) + fl["h"] * 0.05 * math.sin(t * 9)
        vx = x1 - x0
        vy = (y1 - y0) + fl["arc"] * math.pi * math.cos(math.pi * k)
        spr = self.sprite(fl["thing"], "normal", "flyer", fl["h"])
        h = fl["h"] * z
        im = spr.get(h * spr.aspect, h)
        left = vx < 0
        ang = -math.degrees(math.atan2(vy, abs(vx)))
        sx, sy = to_screen(x, y)
        self.paste(canvas, im, sx, sy + h / 2, ang if not left else -ang, mirror=left)

    def draw_boom(self, canvas, b, t, to_screen, z):
        k = t - b["t"]
        if not 0 <= k < 1.3:
            return
        x, y = to_screen(b["x"], b["y"])
        s = b["size"] * z * 0.6
        d = ImageDraw.Draw(canvas)
        lw = max(2, int(s / 60))
        if k < 0.07:
            canvas.paste(Image.blend(canvas, Image.new("RGB", canvas.size, "white"), 0.6))
            d = ImageDraw.Draw(canvas)
        rng = random.Random(int(b["t"] * 1000))
        # smoke puffs drifting out
        if k > 0.2:
            for i in range(7):
                a = 2 * math.pi * i / 7 + rng.uniform(-0.3, 0.3)
                kk = (k - 0.2) / 1.1
                dist = s * (0.4 + 0.6 * ease_out(kk))
                r = s * 0.28 * (1 - kk) + 1
                g = int(110 + 90 * kk)
                px, py = x + math.cos(a) * dist, y + math.sin(a) * dist * 0.7 - s * 0.3 * kk
                d.ellipse((px - r, py - r, px + r, py + r), fill=(g, g, g), outline=(60, 60, 60), width=lw)
        # debris
        for i in range(8):
            a = rng.uniform(0, 2 * math.pi)
            v = rng.uniform(0.8, 1.6) * s
            px = x + math.cos(a) * v * k
            py = y + math.sin(a) * v * k - v * 0.8 * k + 1.8 * s * k * k
            r = s * 0.04
            d.polygon([(px - r, py - r), (px + r, py - r * 0.5), (px + r * 0.5, py + r), (px - r * 0.8, py + r * 0.6)],
                      fill=(50, 45, 40))
        # fireball: three stacked starbursts, pops in and shrinks away
        if k < 0.75:
            g = top5.ease_back(k / 0.22) if k < 0.22 else 1 - ease_in((k - 0.22) / 0.53)
            for col, rr, pts in (((255, 90, 30), 1.0, 12), ((255, 170, 30), 0.72, 10), ((255, 236, 110), 0.42, 8)):
                r = max(1.0, s * rr * g)
                d.polygon(cast_mod.star_points(x, y, r, pts, 0.62, k * 1.5 + rr), fill=col,
                          outline="black" if rr == 1.0 else None, width=lw)

    def draw_subtitle(self, canvas, t):
        ln = next((l for l in self.p["lines"] if l["t0"] <= t < l["t1"] + 0.15), None)
        if ln is None:
            return
        words = ln["text"].split()
        per = 12 if self.W > self.H else 7
        chunks = [words[i:i + per] for i in range(0, len(words), per)] or [[]]
        # split into roughly equal chunks, each shown for its share of the line
        n = len(chunks)
        size = len(words) / n
        chunks = [words[round(i * size):round((i + 1) * size)] for i in range(n)]
        k = min(n - 1, int((t - ln["t0"]) / max(0.01, ln["t1"] - ln["t0"]) * n))
        key = (id(ln), k)
        if key not in self.subs:
            self.subs.clear()
            self.subs[key] = self.subtitle_img(" ".join(chunks[k]))
        im = self.subs[key]
        y = self.H * (0.955 if self.W > self.H else 0.86) - im.height
        canvas.paste(im, (int((self.W - im.width) / 2), int(y)), im)

    def subtitle_img(self, text):
        f = sub_font(int(self.unit * (0.058 if self.W > self.H else 0.05)))
        d = ImageDraw.Draw(Image.new("L", (1, 1)))
        max_w = self.W * 0.86
        lines, cur = [], ""
        for w in text.split():
            test = (cur + " " + w).strip()
            if d.textlength(test, font=f) > max_w and cur:
                lines.append(cur)
                cur = w
            else:
                cur = test
        lines.append(cur)
        stroke = max(2, f.size // 9)
        lh = int(f.size * 1.2)
        width = int(max(d.textlength(l, font=f) for l in lines)) + stroke * 4
        im = Image.new("RGBA", (width, lh * len(lines) + stroke * 4), (0, 0, 0, 0))
        di = ImageDraw.Draw(im)
        for i, l in enumerate(lines):
            lw = d.textlength(l, font=f)
            di.text(((width - lw) / 2, stroke + i * lh), l, font=f, fill="white", stroke_width=stroke, stroke_fill="black")
        return im

    def draw_end(self, canvas, k):
        if not hasattr(self, "_end"):
            f = top5.font(int(self.unit * 0.11))
            title = top5.fancy_text(self.p["title"].upper(), f, "#FFFFFF", "#FFD400", stroke=max(4, self.unit // 110),
                                    shadow=max(4, self.unit // 100))
            self._end_title = top5.fit_width(title, self.W * 0.9)
            rib = top5.ribbon("SUBSCRIBE FOR MORE!", top5.font(int(self.unit * 0.07)), fill="#FF3B3B", fg="white")
            self._end_rib = top5.fit_width(rib, self.W * 0.8)
            self._end = True
        kk = top5.ease_back(k / 0.4)
        im = self._end_title
        s = max(0.05, kk)
        im = im.resize((max(1, int(im.width * s)), max(1, int(im.height * s))), Image.BILINEAR)
        canvas.paste(im, (int((self.W - im.width) / 2), int(self.H * 0.16 - im.height / 2)), im)
        if k > 0.5:
            rib = self._end_rib
            pulse = 1 + 0.04 * math.sin((k - 0.5) * 7)
            rib = rib.resize((int(rib.width * pulse), int(rib.height * pulse)), Image.BILINEAR)
            canvas.paste(rib, (int((self.W - rib.width) / 2), int(self.H * 0.16 + self._end_title.height * 0.6)), rib)


# ---------------------------------------------------------------- encoding
_worker = None


def _init(plan):
    global _worker
    _worker = Renderer(plan)


def _chunk(job):
    a, b, path, venc = job
    p = _worker.p
    W, H, fps = p["W"], p["H"], p["fps"]
    cmd = [top5.FFMPEG, "-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24",
           "-s", f"{W}x{H}", "-r", str(fps), "-i", "-",
           "-vf", "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p", *venc,
           "-g", str(fps * 2), "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709",
           "-r", str(fps), str(path)]
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        for i in range(a, b):
            proc.stdin.write(_worker.frame(i / fps).tobytes())
        proc.stdin.close()
    except BrokenPipeError:
        pass
    err = proc.stderr.read().decode(errors="replace")
    if proc.wait():
        raise RuntimeError(f"ffmpeg failed: {err[-1500:]}")
    return b - a


def encode(plan, audio_wav, out, workers, preset, crf, tmp):
    """Renders all frames into out (MP4, H.264 + AAC)."""
    import multiprocessing as mp

    fps = plan["fps"]
    frames = int(math.ceil(plan["total"] * fps))
    n = max(1, min(frames // (fps * 2), workers * 3))
    bounds = [round(i * frames / n) for i in range(n + 1)]
    venc = ["-c:v", "libx264", "-preset", preset, "-crf", str(crf)]
    tmp.mkdir(parents=True, exist_ok=True)
    jobs = [(bounds[i], bounds[i + 1], tmp / f"chunk_{i:03d}.mp4", venc) for i in range(n)]
    done = 0
    if workers <= 1:
        _init(plan)
        for j in jobs:
            done += _chunk(j)
            print(f"[render ] {done}/{frames} frames", flush=True)
    else:
        with mp.get_context("spawn").Pool(workers, initializer=_init, initargs=(plan,)) as pool:
            for k in pool.imap(_chunk, jobs):
                done += k
                print(f"[render ] {done}/{frames} frames ({100 * done // frames}%)", flush=True)
    video = tmp / "video.mp4"
    top5.concat([j[2] for j in jobs], video)
    for j in jobs:
        j[2].unlink()
    return video


def thumbnail(plan, path):
    """Cover: a close-up moment with the title on it."""
    r = Renderer(plan)
    t = next((l["t0"] + 0.5 for l in plan["lines"]), plan["total"] / 3)
    im = r.frame(t).convert("RGBA")
    f = top5.font(int(r.unit * 0.13))
    title = top5.fit_width(top5.fancy_text(plan["title"].upper(), f, "#FFFFFF", "#FFD400",
                                           stroke=max(5, r.unit // 90), shadow=max(5, r.unit // 90)), r.W * 0.92)
    im.alpha_composite(title, (int((r.W - title.width) / 2), int(r.H * 0.04)))
    im.convert("RGB").save(path, quality=92)
    return path
