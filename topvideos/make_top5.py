"""Today's top 5 funniest / cutest videos from Telegram channels, as one countdown video.

Pipeline: Telethon (your Telegram account) reads today's posts of the channels in
topvideos/channels.txt -> keeps short videos, drops reposts (same file / same duration+size) ->
pre-ranks them by engagement (views + reactions relative to the channel's usual views) and
downloads the best --max-candidates -> qwen3-vl:4b (Ollama) looks at a 2x2 frame grid + the
caption of each and scores "funny" and "cute" 0-10, names the subject and writes a short title ->
the topic (--topic, or auto: the one with the stronger top 5) decides, the 5 best clips that fit
it are ranked (AI score + engagement bonus) -> ffmpeg renders #5 ... #1, each as
a title card + the clip (blurred fill background, rank badge, loudness normalized), an animated
subscribe prompt half-way through -> one MP4 that loops seamlessly as a Short (#1 flows into #5).

Telegram login: create an app at https://my.telegram.org -> API development tools, put api_id
and api_hash (and the two-step verification password, if any) into the repo's .env (see
.env.example). The first run shows a QR code to scan in the Telegram app once, the session is
kept in topvideos/telegram.session (keep it private).
You must be subscribed to (or have opened) the channels so their numeric IDs can be resolved.

Usage (via 9-top5-videos.bat, which uses ComfyUI's embedded Python):
  9-top5-videos.bat [--topic funny|cute|auto] [--subject animals] [--hours 24] [options]
"""
import argparse
import datetime as dt
import io
import json
import math
import re
import shutil
import subprocess
import sys
from pathlib import Path

import av
import imageio_ffmpeg
from PIL import Image, ImageColor, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).resolve().parent
ENV_FILE = HERE.parent / ".env"
OUT_DIR = HERE / "out"
DL_DIR = HERE / "downloads"
CACHE = HERE / "scores.json"
sys.path.insert(0, str(HERE.parent))
sys.path.insert(0, str(HERE))  # embedded Python doesn't add the script folder (soundtrack.py)
from common import llm  # noqa: E402

FONTS = Path("C:/Windows/Fonts")
FFMPEG = imageio_ffmpeg.get_ffmpeg_exe()
FPS = 30
RANK_COLORS = {1: "#FFD400", 2: "#D9D9D9", 3: "#E0913A", 4: "#7FD1FF", 5: "#FF7FBF"}
TOPIC_WORDS = {"funny": "Funniest", "cute": "Cutest"}


# ---------------------------------------------------------------- helpers
def read_channels(path):
    chans = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            chans.append(line)
    return chans


def load_cache():
    return json.loads(CACHE.read_text(encoding="utf-8")) if CACHE.exists() else {}


# ---------------------------------------------------------------- 1. Telegram
CODE_WHERE = {
    "SentCodeTypeApp": "as a message in your Telegram app (chat 'Telegram' with the blue tick, on phone or desktop)",
    "SentCodeTypeSms": "by SMS",
    "SentCodeTypeFragmentSms": "by SMS via Fragment",
    "SentCodeTypeFirebaseSms": "by SMS",
    "SentCodeTypeCall": "by phone call",
    "SentCodeTypeFlashCall": "by a missed call (the code is the last digits of the calling number)",
    "SentCodeTypeMissedCall": "by a missed call (the code is the last digits of the calling number)",
    "SentCodeTypeEmailCode": "by e-mail (the address set up in Telegram)",
    "SentCodeTypeSetUpEmailRequired": "nowhere yet - Telegram wants a login e-mail set up first, use --login qr",
}


async def finish_2fa(client, password=None):
    import getpass

    from telethon.errors import PasswordHashInvalidError

    if password:
        try:
            await client.sign_in(password=password)
            print("[login] two-step verification: used TELEGRAM_PASSWORD from .env")
            return
        except PasswordHashInvalidError:
            print("[login] TELEGRAM_PASSWORD in .env is wrong.")
    print("\n[login] Two-step verification is on: type your Telegram cloud password and press Enter.")
    print("        (the characters are not shown while typing)")
    while True:
        try:
            await client.sign_in(password=getpass.getpass("Password: "))
            return
        except PasswordHashInvalidError:
            print("[login] wrong password, try again.")


async def login(client, mode, password=None):
    """One-time login. qr: scan with the Telegram app, no code needed. code: phone number + login code."""
    from telethon import functions
    from telethon.errors import SessionPasswordNeededError

    try:
        await client(functions.updates.GetStateRequest())
    except SessionPasswordNeededError:
        # QR/code was already accepted in an earlier run, only the 2FA password is missing
        await finish_2fa(client, password)
        mode = None
    except Exception:
        pass
    if mode is None:
        pass
    elif mode == "qr":
        import os
        import qrcode

        png = HERE / "login_qr.png"
        print("[login] On your phone: Telegram > Settings > Devices > Link Desktop Device, then scan the QR code.")
        qr = await client.qr_login()
        try:
            while True:
                qrcode.make(qr.url).save(png)
                os.startfile(png)
                try:
                    await qr.wait(timeout=qr.expires.timestamp() - dt.datetime.now().timestamp() - 1)
                    break
                except TimeoutError:
                    print("[login] QR code expired, showing a new one ...")
                    await qr.recreate()
        except SessionPasswordNeededError:
            await finish_2fa(client, password)
        finally:
            png.unlink(missing_ok=True)
    else:
        phone = input("Phone number (international, e.g. +49...): ").strip()
        sent = await client.send_code_request(phone)
        kind = type(sent.type).__name__
        print(f"[login] Telegram sent the code {CODE_WHERE.get(kind, f'({kind})')}.")
        try:
            await client.sign_in(phone, input("Login code: ").strip(), phone_code_hash=sent.phone_code_hash)
        except SessionPasswordNeededError:
            await finish_2fa(client, password)
    me = await client.get_me()
    print(f"[login] logged in as {me.first_name} - session saved, no login needed next time.")


async def resolve(client, ref):
    from telethon.tl.types import PeerChannel

    if re.fullmatch(r"-?\d+", ref):
        cid = int(ref)
        if str(cid).startswith("-100"):
            cid = int(str(cid)[4:])
        try:
            return await client.get_entity(PeerChannel(abs(cid)))
        except ValueError:
            # not in the session cache yet: load the dialog list once, then retry
            await client.get_dialogs()
            return await client.get_entity(PeerChannel(abs(cid)))
    return await client.get_entity(ref)


def engagement(msg):
    reactions = 0
    if msg.reactions and msg.reactions.results:
        reactions = sum(r.count for r in msg.reactions.results)
    return (msg.views or 0), reactions, (msg.forwards or 0)


async def collect(client, channels, since, args):
    """Today's video posts of all channels, deduplicated, with an engagement score."""
    found, seen = [], set()
    for ref in channels:
        try:
            ent = await resolve(client, ref)
        except Exception as e:
            print(f"[chan ] {ref}: cannot open ({e}) - are you subscribed to it?")
            continue
        name = getattr(ent, "title", None) or getattr(ent, "username", ref)
        username = getattr(ent, "username", None)
        vids, all_views = [], []
        async for msg in client.iter_messages(ent, limit=args.scan_limit):
            if msg.date < since:
                break
            all_views.append(msg.views or 0)
            v = msg.video
            if not v or msg.video_note or msg.gif:
                continue
            dur = next((a.duration for a in v.attributes if hasattr(a, "duration")), 0) or 0
            if not (args.min_duration <= dur <= args.max_duration) or v.size > args.max_mb * 1024 * 1024:
                continue
            keys = {("doc", v.id), ("dur", round(dur), v.size // 50_000)}
            if keys & seen:
                continue  # repost of a video we already have
            seen |= keys
            vids.append((msg, dur))
        base = max(1.0, sorted(all_views)[len(all_views) // 2]) if all_views else 1.0  # median views
        for msg, dur in vids:
            views, reacts, fwd = engagement(msg)
            found.append({
                "key": f"{ent.id}_{msg.id}", "channel": name, "chan_id": ent.id, "msg_id": msg.id,
                # public channels: t.me/<name>/<id> opens for everyone; t.me/c/... only for members
                "link": f"https://t.me/{username}/{msg.id}" if username else f"https://t.me/c/{ent.id}/{msg.id}",
                "date": msg.date.isoformat(), "duration": float(dur), "text": (msg.message or "")[:400],
                "views": views, "reactions": reacts, "forwards": fwd,
                "eng": views / base + 20 * reacts / base + 10 * fwd / base,
                "_msg": msg,
            })
        print(f"[chan ] {name}: {len(vids)} video(s) since {since:%Y-%m-%d %H:%M}")
    return found


# ---------------------------------------------------------------- 2. frames + scoring
def probe(path):
    with av.open(str(path)) as c:
        dur = float(c.duration / av.time_base) if c.duration else 0.0
        return dur, bool(c.streams.audio)


def frame_grid(path, duration, n=4, cell=384):
    """2x2 grid of frames at 15/38/62/85 % of the clip, so the VLM sees what happens."""
    times = [duration * f for f in (0.15, 0.38, 0.62, 0.85)[:n]]
    frames = []
    with av.open(str(path)) as c:
        s = c.streams.video[0]
        for t in times:
            c.seek(int(t / s.time_base), stream=s, any_frame=False, backward=True)
            for fr in c.decode(s):
                if fr.time is None or fr.time >= t - 0.05:
                    frames.append(fr.to_image())
                    break
    if not frames:
        raise RuntimeError("no frames decoded")
    w, h = frames[0].size
    cw, ch = (cell, int(cell * h / w)) if w >= h else (int(cell * w / h), cell)
    grid = Image.new("RGB", (cw * 2, ch * 2))
    for i, f in enumerate(frames):
        grid.paste(f.convert("RGB").resize((cw, ch)), ((i % 2) * cw, (i // 2) * ch))
    return grid


def score(grid, caption, subject, lang):
    buf = io.BytesIO()
    grid.save(buf, "JPEG", quality=88)
    subj = (f'"fits_subject": <true if the video is about {subject}, else false>, ' if subject else "")
    prompt = (
        "These are 4 frames (left to right, top to bottom) of a short video from a Telegram channel. "
        f"The post caption was: \"{caption or '(none)'}\"\n"
        "Rate it for a 'top videos of today' compilation. Reply with JSON only, in this exact shape: "
        '{"funny": <0-10, how funny/hilarious it is (fails, pranks, absurd moments)>, '
        '"cute": <0-10, how cute/wholesome it is (babies, puppies, kittens, sweet moments)>, '
        '"entertainment": <true if it is a fun/cute clip, false for news, war, ads, promos, talking heads, text slides>, '
        '"political": <true if it touches politics, politicians or other real public figures (also as caricatures '
        'or memes), war, military, soldiers, weapons, protests, elections, national conflicts, propaganda or news>, '
        '"wholesome": <0-10, how wholesome and family-friendly it is (10 = kind, feel-good, fine for kids; '
        '0 = mean, gross, violent, sexual, mocking people)>, '
        + subj +
        '"subject": "<2-4 English words, e.g. \\"cat vs cucumber\\">", '
        f'"title": "<catchy title in {lang}, max 6 words, no hashtags, no emojis>"}}'
    )
    d = llm.vision_json(prompt, buf.getvalue(), temperature=0.2, keep_alive="5m")
    num = lambda k: max(0.0, min(10.0, float(d.get(k, 0) or 0)))
    return {
        "funny": num("funny"), "cute": num("cute"),
        "entertainment": bool(d.get("entertainment", True)),
        "political": d.get("political") not in (False, "false", "no", 0),  # unsure counts as political
        "wholesome": num("wholesome"),
        "fits_subject": bool(d.get("fits_subject", True)),
        "subject": str(d.get("subject", "")).strip(),
        "title": re.sub(r"[#\"]", "", str(d.get("title", "")).strip())[:60],
    }


# second line of defence next to the VLM's "political" flag (caption, title and subject are checked)
POLITICAL = re.compile(
    r"\b(polit\w*|putin\w*|russ\w*|kreml\w*|kremlin|ukrain\w*|selenskyj?|zelensk\w*|trump\w*|biden|harris|merz|"
    r"scholz|habeck|baerbock|weidel|afd|cdu|spd|nato|election\w*|wahlen|wahlkampf|vote|warfare|krieg\w*|"
    r"soldier\w*|soldat\w*|army|armee|milit\w*|drone\w*|drohne\w*|missile\w*|rakete\w*|panzer\w*|weapon\w*|"
    r"waffe\w*|protest\w*|propaganda|fake news|nachrichten|israel\w*|gaza|hamas|iran\w*|china|"
    r"communis\w*|kommunis\w*|nazi\w*|faschis\w*|fascis\w*|kanzler\w*|president\w*|pr[äa]sident\w*|minister\w*|"
    r"regierung\w*|government|parliament|bundestag)\b", re.I)


def clean(c, min_wholesome):
    """Wholesome and non-political only."""
    a = c["ai"]
    text = " ".join([c.get("text", ""), a.get("title", ""), a.get("subject", "")])
    return (not a.get("political", True) and a.get("wholesome", 0) >= min_wholesome
            and not POLITICAL.search(text))


def pick(cands, topic, subject, n, min_wholesome=6):
    ok = [c for c in cands if c.get("ai") and c["ai"]["entertainment"] and (not subject or c["ai"]["fits_subject"])
          and clean(c, min_wholesome)]
    if not ok:
        return topic if topic != "auto" else "funny", []
    # engagement bonus: 0..2 points by rank among the candidates, so the AI score dominates
    order = sorted(ok, key=lambda c: c["eng"])
    for i, c in enumerate(order):
        c["eng_bonus"] = 2.0 * i / max(1, len(order) - 1)
    topics = ["funny", "cute"] if topic == "auto" else [topic]
    best = None
    for t in topics:
        # a clip counts for a topic only if that is its stronger side (keeps the 5 on one theme)
        pool = [c for c in ok if c["ai"][t] >= 5 and c["ai"][t] >= c["ai"]["cute" if t == "funny" else "funny"] - 1]
        if len(pool) < n:
            pool = sorted(ok, key=lambda c: -c["ai"][t])[:max(n, len(pool))]
        ranked = sorted(pool, key=lambda c: -(c["ai"][t] + c["eng_bonus"]))[:n]
        total = sum(c["ai"][t] for c in ranked)
        if best is None or total > best[0]:
            best = (total, t, ranked)
    for c in best[2]:
        c["final"] = round(c["ai"][best[1]] + c["eng_bonus"], 2)
    return best[1], best[2]


# ---------------------------------------------------------------- 3. graphics
def font(size, name="impact.ttf"):
    for n in (name, "ariblk.ttf", "arialbd.ttf"):
        if (FONTS / n).exists():
            return ImageFont.truetype(str(FONTS / n), size)
    return ImageFont.load_default(size)


def fit_lines(draw, text, fnt_name, max_w, start, min_size=28):
    size = start
    while size >= min_size:
        f = font(size, fnt_name)
        words, lines, cur = text.split(), [], ""
        for w in words:
            t = (cur + " " + w).strip()
            if draw.textlength(t, font=f) <= max_w or not cur:
                cur = t
            else:
                lines.append(cur)
                cur = w
        lines.append(cur)
        if len(lines) <= 3 and all(draw.textlength(l, font=f) <= max_w for l in lines):
            return f, lines
        size -= 4
    return font(min_size, fnt_name), [text]


def centered(draw, lines, f, y, W, fill, stroke=6, stroke_fill="black"):
    for l in lines:
        w = draw.textlength(l, font=f)
        draw.text(((W - w) / 2, y), l, font=f, fill=fill, stroke_width=stroke, stroke_fill=stroke_fill)
        y += int(f.size * 1.12)
    return y


def card(path, W, H, big, sub, color, bg_img=None, top=None):
    """Title card: blurred frame (or gradient) background, huge headline, subtitle, optional small top line."""
    if bg_img is not None:
        bg = bg_img.convert("RGB")
        s = max(W / bg.width, H / bg.height)
        bg = bg.resize((int(bg.width * s) + 1, int(bg.height * s) + 1))
        bg = bg.crop(((bg.width - W) // 2, (bg.height - H) // 2, (bg.width - W) // 2 + W, (bg.height - H) // 2 + H))
        bg = bg.filter(ImageFilter.GaussianBlur(28))
        bg = Image.blend(bg, Image.new("RGB", (W, H), "black"), 0.45)
    else:
        bg = Image.new("RGB", (W, H))
        px = ImageDraw.Draw(bg)
        for y in range(H):
            k = y / H
            px.line([(0, y), (W, y)], fill=(int(40 + 90 * k), int(20 + 30 * k), int(90 + 60 * (1 - k))))
    d = ImageDraw.Draw(bg)
    unit = min(W, H)
    f_big, big_lines = fit_lines(d, big, "impact.ttf", W * 0.9, int(unit * 0.30))
    f_sub, sub_lines = fit_lines(d, sub, "ariblk.ttf", W * 0.86, int(unit * 0.075))
    total = len(big_lines) * f_big.size * 1.12 + 30 + len(sub_lines) * f_sub.size * 1.12
    y = centered(d, big_lines, f_big, int((H - total) / 2), W, color, stroke=max(4, unit // 90))
    centered(d, sub_lines, f_sub, y + 30, W, "white", stroke=max(3, unit // 160))
    if top:
        f_top, top_lines = fit_lines(d, top.upper(), "impact.ttf", W * 0.86, int(unit * 0.085))
        centered(d, top_lines, f_top, int(H * 0.07), W, "#FFD400", stroke=max(3, unit // 160))
    bg.save(path)


def rgb(c):
    return ImageColor.getrgb(c)


def ease_back(k):
    k = min(1.0, max(0.0, k)) - 1
    return 1 + 2.70158 * k ** 3 + 1.70158 * k ** 2


def fancy_text(text, f, top="#FFFFFF", bottom="#FFD400", stroke=8, shadow=10):
    """Text with a vertical gradient fill, thick black outline and a soft drop shadow (RGBA image)."""
    probe_d = ImageDraw.Draw(Image.new("L", (1, 1)))
    l, t, r, b = probe_d.textbbox((0, 0), text, font=f, stroke_width=stroke)
    w, h = r - l + shadow * 2, b - t + shadow * 2
    pos = (-l + shadow // 2, -t + shadow // 2)
    fill_m, line_m = Image.new("L", (w, h)), Image.new("L", (w, h))
    ImageDraw.Draw(fill_m).text(pos, text, font=f, fill=255)
    ImageDraw.Draw(line_m).text(pos, text, font=f, fill=255, stroke_width=stroke, stroke_fill=255)
    grad = Image.new("RGB", (w, h))
    c1, c2 = rgb(top), rgb(bottom)
    gd = ImageDraw.Draw(grad)
    for y in range(h):
        k = y / max(1, h - 1)
        gd.line([(0, y), (w, y)], fill=tuple(int(c1[i] + (c2[i] - c1[i]) * k) for i in range(3)))
    out = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    sh = Image.new("L", (w, h))
    sh.paste(line_m.crop((0, 0, w - shadow // 2, h - shadow // 2)), (shadow // 2, shadow // 2))
    out.paste((0, 0, 0, 255), (0, 0), sh.filter(ImageFilter.GaussianBlur(shadow / 2)).point(lambda a: int(a * 0.6)))
    out.paste((0, 0, 0, 255), (0, 0), line_m)
    out.paste(grad, (0, 0), fill_m)
    return out


TOPIC_EMOJI = {"funny": ("face-with-tears-of-joy", "rolling-on-the-floor-laughing"),
               "cute": ("smiling-cat-with-heart-eyes", "smiling-face-with-hearts")}
RANK_EMOJI = {1: "1st-place-medal", 2: "2nd-place-medal", 3: "3rd-place-medal"}


def emoji(name, size, char=None):
    """Fluent Emoji `name` as RGBA; falls back to the Segoe UI Emoji glyph `char` when offline."""
    from fluent_emoji import emoji as fluent

    im = fluent(name, size)
    if im is not None or char is None:
        return im
    try:
        f = ImageFont.truetype(str(FONTS / "seguiemj.ttf"), 109)
        im = Image.new("RGBA", (160, 160), (0, 0, 0, 0))
        ImageDraw.Draw(im).text((10, 10), char, font=f, embedded_color=True)
        box = im.getbbox()
        return im.crop(box).resize((size, size), Image.LANCZOS) if box else None
    except Exception:
        return None


def paste_center(canvas, im, cx, cy, scale=1.0, angle=0.0, alpha=1.0):
    if scale != 1.0:
        im = im.resize((max(1, int(im.width * scale)), max(1, int(im.height * scale))), Image.BICUBIC)
    if angle:
        im = im.rotate(angle, resample=Image.BICUBIC, expand=True)
    if alpha < 1.0:
        im = im.copy()
        im.putalpha(im.getchannel("A").point(lambda a: int(a * alpha)))
    canvas.alpha_composite(im, (int(cx - im.width / 2), int(cy - im.height / 2)))


def polaroid(img, max_w, max_h, angle):
    """Photo in a white frame with a soft shadow, slightly tilted."""
    img = img.convert("RGB")
    s = min(max_w / img.width, max_h / img.height)
    img = img.resize((max(1, int(img.width * s)), max(1, int(img.height * s))), Image.LANCZOS)
    m = max(6, int(min(img.size) * 0.04))
    fr = Image.new("RGBA", (img.width + 2 * m, img.height + 3 * m), "white")
    fr.paste(img, (m, m))
    pad = m * 3
    out = Image.new("RGBA", (fr.width + 2 * pad, fr.height + 2 * pad), (0, 0, 0, 0))
    sh = Image.new("L", out.size)
    ImageDraw.Draw(sh).rectangle((pad + m, pad + m, pad + fr.width + m, pad + fr.height + m), fill=150)
    out.paste((0, 0, 0, 255), (0, 0), sh.filter(ImageFilter.GaussianBlur(m)))
    out.alpha_composite(fr, (pad, pad))
    return out.rotate(angle, resample=Image.BICUBIC, expand=True)


def backdrop(W, H, color, still=None, dark=0.55):
    """Colourful background: rank-colour glow over a deep gradient, optionally a hint of the blurred clip."""
    c = rgb(color)
    top, bottom = tuple(int(v * 0.35) for v in c), (18, 8, 40)
    bg = Image.new("RGB", (W, H))
    d = ImageDraw.Draw(bg)
    for y in range(H):
        k = y / H
        d.line([(0, y), (W, y)], fill=tuple(int(top[i] + (bottom[i] - top[i]) * k) for i in range(3)))
    if still is not None:
        s = still.convert("RGB")
        sc = max(W / s.width, H / s.height)
        s = s.resize((int(s.width * sc) + 1, int(s.height * sc) + 1))
        s = s.crop(((s.width - W) // 2, (s.height - H) // 2, (s.width - W) // 2 + W, (s.height - H) // 2 + H))
        bg = Image.blend(bg, s.filter(ImageFilter.GaussianBlur(30)), 1 - dark)
    size = int(max(W, H) * 1.1)
    glow = Image.radial_gradient("L").resize((size, size)).point(lambda a: int(max(0, 255 - a * 1.4) * 0.8))
    bg.paste(Image.new("RGB", glow.size, c), ((W - size) // 2, int(H * 0.40) - size // 2), glow)
    return bg.convert("RGBA")


def rays(canvas, cx, cy, angle, color=(255, 255, 255), alpha=34, n=16):
    """Sunburst rays over a copy of the canvas, turned by `angle` degrees."""
    over = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(over)
    R = max(canvas.size) * 1.5
    for i in range(n):
        a0 = math.radians(angle) + 2 * math.pi * i / n
        a1 = a0 + math.pi / n
        d.polygon([(cx, cy), (cx + R * math.cos(a0), cy + R * math.sin(a0)),
                   (cx + R * math.cos(a1), cy + R * math.sin(a1))], fill=(*color, alpha))
    out = canvas.copy()
    out.alpha_composite(over)
    return out


def ribbon(text, f, fill="#FFD400", fg="black", angle=-3):
    d0 = ImageDraw.Draw(Image.new("L", (1, 1)))
    l, t, r, b = d0.textbbox((0, 0), text, font=f)
    px, py = int(f.size * 0.45), int(f.size * 0.22)
    im = Image.new("RGBA", (r - l + 2 * px, b - t + 2 * py), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle((0, 0, im.width - 1, im.height - 1), radius=im.height // 4, fill=fill,
                        outline="black", width=max(3, f.size // 14))
    d.text((px - l, py - t), text, font=f, fill=fg)
    return im.rotate(angle, resample=Image.BICUBIC, expand=True) if angle else im


def starburst(r, color, points=14):
    im = Image.new("RGBA", (2 * r + 8, 2 * r + 8), (0, 0, 0, 0))
    c = r + 4
    pts = []
    for i in range(points * 2):
        a = math.pi * i / points - math.pi / 2
        rr = r if i % 2 == 0 else r * 0.80
        pts.append((c + rr * math.cos(a), c + rr * math.sin(a)))
    ImageDraw.Draw(im).polygon(pts, fill=color, outline="black", width=max(3, r // 18))
    return im


def fit_width(im, max_w):
    return im.resize((int(max_w), int(im.height * max_w / im.width)), Image.LANCZOS) if im.width > max_w else im


def card_frames(folder, W, H, rank, title, color, still, headline, seconds, topic="funny"):
    """Animated rank card as a PNG sequence: rotating sunburst, #N pops in, the clip's still as a tilted
    polaroid, title pill slides up, the headline as a ribbon at the top (same on every card -> loops)."""
    folder.mkdir(exist_ok=True)
    unit = min(W, H)
    vertical = H > W
    base = backdrop(W, H, color, still)
    num = fancy_text(f"#{rank}", font(int(unit * 0.34)), "#FFFFFF", color, stroke=max(6, unit // 70))
    pol = polaroid(still, W * (0.70 if vertical else 0.42), H * (0.36 if vertical else 0.62),
                   -4 if rank % 2 else 4) if still is not None else None
    d0 = ImageDraw.Draw(Image.new("L", (1, 1)))
    f_t, lines = fit_lines(d0, title, "ariblk.ttf", W * (0.80 if vertical else 0.42), int(unit * 0.07), 24)
    pill = Image.new("RGBA", (int(W * (0.88 if vertical else 0.48)), int(len(lines) * f_t.size * 1.15 + unit * 0.05)),
                     (0, 0, 0, 0))
    pd = ImageDraw.Draw(pill)
    pd.rounded_rectangle((0, 0, pill.width - 1, pill.height - 1), radius=unit // 30, fill=(0, 0, 0, 170),
                         outline=color, width=max(3, unit // 150))
    centered(pd, lines, f_t, int(unit * 0.025), pill.width, "white", stroke=0)
    rib = fit_width(ribbon(headline.upper(), font(int(unit * 0.07))), W * 0.94) if headline else None
    # medal for the podium, the topic's emoji for the other places; pops in after the number
    emo = emoji(RANK_EMOJI.get(rank) or TOPIC_EMOJI.get(topic, TOPIC_EMOJI["funny"])[rank % 2], int(unit * 0.16))
    if vertical:
        num_c, pol_c, pill_c = (W / 2, H * 0.25), (W / 2, H * 0.58), (W / 2, H * 0.86)
    else:
        num_c, pol_c, pill_c = (W * 0.26, H * 0.46), (W * 0.70, H * 0.50), (W * 0.26, H * 0.80)
    for i in range(int(round(seconds * FPS))):
        t = i / FPS
        fr = rays(base, *num_c, angle=t * 25)
        if pol is not None:
            k = ease_back(t / 0.45)
            paste_center(fr, pol, pol_c[0], pol_c[1] + (1 - min(1, t / 0.35)) * H * 0.08, scale=0.85 + 0.15 * k)
        k = ease_back(t / 0.35)
        pulse = 1 + 0.03 * math.sin(max(0.0, t - 0.35) * 8)
        paste_center(fr, num, *num_c, scale=max(0.05, (0.2 + 0.8 * k) * pulse), angle=6 * (1 - min(1, t / 0.35)))
        if emo is not None and t > 0.2:
            ke = ease_back((t - 0.2) / 0.35)
            paste_center(fr, emo, num_c[0] + unit * 0.22, num_c[1] - unit * 0.10, scale=max(0.05, ke),
                         angle=12 * math.sin(t * 3))
        a = min(1.0, max(0.0, (t - 0.15) / 0.25))
        if a > 0:
            paste_center(fr, pill, pill_c[0], pill_c[1] + (1 - a) * unit * 0.08, alpha=a)
        if rib is not None:
            paste_center(fr, rib, W / 2, H * (0.075 if vertical else 0.09))
        fr.convert("RGB").save(folder / f"f_{i:03d}.png", compress_level=1)
    return folder / "f_%03d.png"


def badge(path, W, H, rank, title, headline=None):
    """Transparent overlay for the clip: rank starburst top-left, headline ribbon next to it,
    soft rank-colour edges, optional title bar at the bottom."""
    im = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    unit = min(W, H)
    c = rgb(RANK_COLORS[rank])
    edge = int(H * 0.10)
    gd = ImageDraw.Draw(im)
    for y in range(edge):
        a = int(110 * (1 - y / edge) ** 2)
        gd.line([(0, y), (W, y)], fill=(*c, a))
        gd.line([(0, H - 1 - y), (W, H - 1 - y)], fill=(*c, a))
    r = int(unit * 0.10)
    cx, cy = int(unit * 0.035) + r, int(unit * 0.035) + r
    paste_center(im, starburst(r, RANK_COLORS[rank]), cx, cy, angle=-8)
    paste_center(im, fancy_text(f"#{rank}", font(int(r * 1.05)), "#FFFFFF", "#FFFFFF",
                                stroke=max(3, unit // 150), shadow=4), cx, cy, angle=-8)
    if headline:
        rib = fit_width(ribbon(headline.upper(), font(int(unit * 0.042)), angle=0), W - (cx + r) - unit * 0.06)
        paste_center(im, rib, cx + r + unit * 0.02 + rib.width / 2, cy)
    if title:
        d = ImageDraw.Draw(im)
        f2, lines = fit_lines(d, title, "ariblk.ttf", W * 0.88, int(unit * 0.06), 22)
        bar_h = int(len(lines) * f2.size * 1.12 + unit * 0.04)
        y0 = H - bar_h - int(H * 0.06)
        d.rounded_rectangle((W * 0.04, y0, W * 0.96, y0 + bar_h), radius=unit // 40, fill=(0, 0, 0, 150))
        centered(d, lines, f2, y0 + int(unit * 0.02), W, "white", stroke=0)
    im.save(path)


def thumbnail(path, W, H, top, word, topic):
    """Cover image: #1 big as a polaroid, #2/#3 behind it, "TOP 5 <word>" in big gradient letters + emoji."""
    unit = min(W, H)
    v = H > W
    col = "#FF9F1C" if topic == "funny" else "#FF6FB5"
    im = rays(backdrop(W, H, col), W / 2, H * 0.55, 0, alpha=45, n=20)
    by_rank = {c["rank"]: c for c in top}
    pw, ph = (W * 0.46, H * 0.25) if v else (W * 0.26, H * 0.46)
    side = ((3, W * 0.24, H * 0.38, 11), (2, W * 0.76, H * 0.38, -11)) if v else \
        ((3, W * 0.14, H * 0.60, 10), (2, W * 0.86, H * 0.60, -10))
    for r, x, y, a in side:
        if by_rank.get(r, {}).get("still") is not None:
            paste_center(im, polaroid(by_rank[r]["still"], pw, ph, a), x, y)
    if by_rank.get(1, {}).get("still") is not None:
        paste_center(im, polaroid(by_rank[1]["still"], W * (0.74 if v else 0.42), H * (0.40 if v else 0.60), -3),
                     W / 2, H * (0.60 if v else 0.58))
        sx, sy = W * (0.80 if v else 0.68), H * (0.43 if v else 0.30)
        paste_center(im, starburst(int(unit * 0.11), RANK_COLORS[1]), sx, sy, angle=-8)
        paste_center(im, fancy_text("#1", font(int(unit * 0.12)), "#FFFFFF", "#FFFFFF", stroke=4, shadow=4),
                     sx, sy, angle=-8)
    t1 = fit_width(fancy_text(f"TOP {len(top)}", font(int(unit * (0.30 if v else 0.20))), "#FFFFFF", "#FFD400",
                              stroke=max(8, unit // 55), shadow=14), W * 0.94)
    t2 = fit_width(fancy_text(word.upper(), font(int(unit * (0.19 if v else 0.15))), "#FFF3B0",
                              "#FF3B3B" if topic == "funny" else "#FF4FA3", stroke=max(8, unit // 60), shadow=14),
                   W * 0.94)
    paste_center(im, t1, W / 2, H * (0.10 if v else 0.14), angle=-3)
    paste_center(im, t2, W / 2, H * (0.21 if v else 0.31), angle=-3)
    paste_center(im, ribbon("VIDEOS OF TODAY", font(int(unit * 0.075)), angle=2), W / 2, H * 0.88)
    left, right = TOPIC_EMOJI.get(topic, TOPIC_EMOJI["funny"])
    for name, char, x, a in ((left, "\U0001F602", 0.14, 12), (right, "\U0001F63B", 0.86, -12)):
        e = emoji(name, int(unit * 0.22), char)
        if e is not None:
            paste_center(im, e, W * x, H * 0.80, angle=a)
    trophy = emoji("trophy", int(unit * 0.12))
    if trophy is not None and by_rank.get(1, {}).get("still") is not None:
        paste_center(im, trophy, W * (0.22 if v else 0.30), H * (0.43 if v else 0.30), angle=10)
    im.convert("RGB").save(path, quality=92)


SUB_SECONDS = 3.6
SUB_CLICK = 1.7  # when the cursor clicks SUBSCRIBE (seconds into the prompt)


def subscribe_frames(folder, W, H, text="Please subscribe for more!"):
    """PNG sequence (sub_000.png ...) of an animated call-to-action: text + red SUBSCRIBE button pop in,
    the button pulses, a cursor clicks it -> grey SUBSCRIBED + wiggling bell, then everything pops out."""
    folder.mkdir(exist_ok=True)
    unit = min(W, H)
    f_txt, lines = fit_lines(ImageDraw.Draw(Image.new("RGB", (1, 1))), text, "impact.ttf", W * 0.84, int(unit * 0.10))
    f_btn = font(int(unit * 0.065), "ariblk.ttf")
    click = SUB_CLICK

    def panel(subscribed, press, bell_angle):
        pw, ph = int(W * 0.9), int(len(lines) * f_txt.size * 1.12 + unit * 0.24)
        im = Image.new("RGBA", (pw, ph), (0, 0, 0, 0))
        d = ImageDraw.Draw(im)
        y = centered(d, lines, f_txt, 0, pw, "white", stroke=max(4, unit // 110))
        label = "SUBSCRIBED" if subscribed else "SUBSCRIBE"
        bw, bh = int(d.textlength(label, font=f_btn) + unit * 0.24), int(unit * 0.13)
        bw, bh = int(bw * press), int(bh * press)
        bx, by = (pw - bw) // 2, y + int(unit * 0.04) + (int(unit * 0.13) - bh) // 2
        d.rounded_rectangle((bx, by, bx + bw, by + bh), radius=bh // 2,
                            fill="#5A5A5A" if subscribed else "#FF0000", outline="white", width=max(3, unit // 180))
        lw = d.textlength(label, font=f_btn)
        tx = bx + (bw - lw) / 2 - (unit * 0.04 if subscribed else 0)
        d.text((tx, by + (bh - f_btn.size) / 2 - f_btn.size * 0.12), label, font=f_btn, fill="white")
        if subscribed:  # bell next to the label
            s = int(bh * 0.5)
            bell = Image.new("RGBA", (s, s), (0, 0, 0, 0))
            b = ImageDraw.Draw(bell)
            b.pieslice((s * 0.15, s * 0.05, s * 0.85, s * 0.95), 180, 360, fill="white")
            b.rectangle((s * 0.15, s * 0.5, s * 0.85, s * 0.75), fill="white")
            b.rectangle((s * 0.05, s * 0.72, s * 0.95, s * 0.8), fill="white")
            b.ellipse((s * 0.4, s * 0.8, s * 0.6, s), fill="white")
            bell = bell.rotate(bell_angle, resample=Image.BICUBIC)
            im.alpha_composite(bell, (int(tx + lw + unit * 0.02), by + (bh - s) // 2))
        return im, (bx + bw // 2, by + bh // 2)

    def cursor(size):
        c = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        pts = [(0, 0), (0, 0.78), (0.2, 0.6), (0.34, 0.92), (0.48, 0.86), (0.34, 0.55), (0.58, 0.55)]
        ImageDraw.Draw(c).polygon([(x * size * 0.95 + 2, y * size * 0.95 + 2) for x, y in pts],
                                  fill="white", outline="black", width=max(2, size // 20))
        return c

    cur = cursor(int(unit * 0.12))
    n = int(SUB_SECONDS * FPS)
    for i in range(n):
        t = i / FPS
        if t < 0.45:  # pop in, ease-out-back
            k = t / 0.45
            scale, alpha = 0.3 + 0.7 * (1 + 2.7 * (k - 1) ** 3 + 1.7 * (k - 1) ** 2), k
        elif t > SUB_SECONDS - 0.35:  # pop out
            k = (SUB_SECONDS - t) / 0.35
            scale, alpha = 0.6 + 0.4 * k, k
        else:
            scale, alpha = 1.0, 1.0
        subscribed = t >= click
        press = 0.9 if click - 0.08 <= t < click + 0.08 else (1 + 0.04 * math.sin(t * 9) if not subscribed else 1.0)
        bell = 18 * math.sin((t - click) * 22) * max(0.0, 1 - (t - click) / 1.0) if subscribed else 0
        p, (bcx, bcy) = panel(subscribed, press, bell)
        # fixed-size canvas (room for the cursor below), so the panel never jumps
        im = Image.new("RGBA", (p.width, p.height + int(unit * 0.25)), (0, 0, 0, 0))
        im.alpha_composite(p)
        if 0.8 <= t < click + 0.6:  # cursor glides onto the button, then rests there
            k = min(1.0, (t - 0.8) / (click - 0.8 - 0.1))
            k = 1 - (1 - k) ** 3
            x0, y0 = p.width * 0.95, im.height - cur.height
            im.alpha_composite(cur, (int(x0 + (bcx + unit * 0.02 - x0) * k), int(y0 + (bcy - y0) * k)))
        if scale != 1.0:
            im = im.resize((max(1, int(im.width * scale)), max(1, int(im.height * scale))), Image.BICUBIC)
        if alpha < 1.0:
            im.putalpha(im.getchannel("A").point(lambda a: int(a * alpha)))
        frame = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        cy = int(H * 0.70)
        frame.alpha_composite(im, ((W - im.width) // 2, cy - im.height // 2))
        frame.save(folder / f"sub_{i:03d}.png", compress_level=1)
    return folder / "sub_%03d.png"


# ---------------------------------------------------------------- 4. video
def best_window(path, dur, length, has_audio):
    """Start of the liveliest `length` seconds: the loudest stretch of the clip's own audio (reactions,
    laughter, the punchline), nudged a little towards the end where memes land their joke.
    Only used to choose the cut - the original sound itself is not kept unless --audio original."""
    if dur <= length + 0.5:
        return 0.0
    if not has_audio:
        return round((dur - length) / 2, 2)
    import numpy as np

    step = 0.25
    buckets = np.zeros(int(dur / step) + 2)
    try:
        with av.open(str(path)) as con:
            for fr in con.decode(con.streams.audio[0]):
                if fr.time is not None:
                    a = fr.to_ndarray().astype(np.float32)
                    buckets[min(len(buckets) - 1, int(fr.time / step))] += float((a * a).mean())
    except Exception:
        return round((dur - length) / 2, 2)
    win = max(1, int(length / step))
    sums = np.convolve(buckets, np.ones(win), "valid")
    starts = np.arange(len(sums)) * step
    sums *= 1 + 0.15 * starts / max(starts.max(), 1e-6)
    return round(float(min(starts[int(sums.argmax())], dur - length)), 2)


def still_at(path, t):
    """A frame from about t seconds in (the first frame is often black or a title)."""
    with av.open(str(path)) as con:
        s = con.streams.video[0]
        if t > 0.5:
            con.seek(int(t / s.time_base), stream=s)
        for fr in con.decode(s):
            return fr.to_image()


def ffmpeg(args):
    cmd = [FFMPEG, "-hide_banner", "-loglevel", "error", "-y", *args]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode:
        raise RuntimeError(f"ffmpeg failed:\n{r.stderr[-2000:]}")


def venc(args):
    if args.nvenc:
        return ["-c:v", "h264_nvenc", "-preset", "p5", "-cq", "23", "-pix_fmt", "yuv420p"]
    return ["-c:v", "libx264", "-preset", "veryfast", "-crf", "21", "-pix_fmt", "yuv420p"]


AENC = ["-c:a", "aac", "-b:a", "160k", "-ar", "44100", "-ac", "2"]


def render_card(png, seconds, out, args):
    """Still image or PNG sequence (pattern with %03d) -> clip with fades and silent audio."""
    src = ["-framerate", str(FPS), "-i", str(png)] if "%" in str(png) else \
        ["-loop", "1", "-framerate", str(FPS), "-t", f"{seconds}", "-i", str(png)]
    ffmpeg([*src, "-f", "lavfi", "-t", f"{seconds}", "-i", "anullsrc=r=44100:cl=stereo",
            "-vf", f"fade=in:0:{FPS // 3},fade=out:st={seconds - 0.3}:d=0.3,format=yuv420p",
            *venc(args), *AENC, "-r", str(FPS), "-shortest", str(out)])


def render_clip(src, overlay_png, start, length, has_audio, W, H, out, args, extra=None):
    """extra: (png sequence pattern, seconds into the clip) of an animated overlay, e.g. the subscribe prompt."""
    fo = max(0.0, length - 0.35)
    vf = (f"[0:v]split[a][b];"
          f"[a]scale={W}:{H}:force_original_aspect_ratio=increase,crop={W}:{H},boxblur=25:3,"
          f"eq=brightness=-0.05:saturation=1.6[bg];"
          f"[b]scale={W - 16}:{H - 16}:force_original_aspect_ratio=decrease,pad=iw+12:ih+12:6:6:white[fg];"
          f"[bg][fg]overlay=(W-w)/2:(H-h)/2,setsar=1,fps={FPS}[v0];"
          f"[v0][1:v]overlay=0:0[v1];")
    inputs = ["-ss", f"{start}", "-t", f"{length}", "-i", str(src), "-i", str(overlay_png)]
    if extra:
        pattern, at = extra
        inputs += ["-framerate", str(FPS), "-i", str(pattern)]
        vf += (f"[2:v]format=rgba,setpts=PTS+{at:.3f}/TB[s];"
               f"[v1][s]overlay=0:0:eof_action=pass[v2];")
        last = "[v2]"
    else:
        last = "[v1]"
    vf += f"{last}fade=in:0:6,fade=out:st={fo}:d=0.35,format=yuv420p[v]"
    if has_audio:
        af = (f"[0:a]aresample=44100,aformat=channel_layouts=stereo,loudnorm=I=-16:TP=-1.5:LRA=11,"
              f"afade=in:d=0.2,afade=out:st={fo}:d=0.35[a]")
    else:
        inputs += ["-f", "lavfi", "-t", f"{length}", "-i", "anullsrc=r=44100:cl=stereo"]
        af = f"[{3 if extra else 2}:a]anull[a]"
    ffmpeg([*inputs, "-filter_complex", vf + ";" + af, "-map", "[v]", "-map", "[a]",
            *venc(args), *AENC, "-r", str(FPS), "-t", f"{length}", str(out)])


def concat(parts, out):
    lst = out.with_suffix(".txt")
    lst.write_text("".join(f"file '{p.as_posix()}'\n" for p in parts), encoding="utf-8")
    ffmpeg(["-f", "concat", "-safe", "0", "-i", str(lst), "-c", "copy", str(out)])
    lst.unlink()


def mux(video, out, cover=None, audio=None):
    """Final MP4: video copied, sound replaced by `audio` (WAV, loudness-normalized) if given,
    cover embedded as the thumbnail."""
    ins, maps = ["-i", str(video)], ["-map", "0:v:0"]
    if audio:
        ins += ["-i", str(audio)]
        maps += ["-map", "1:a:0"]
        acodec = [*AENC, "-af", "loudnorm=I=-14:TP=-1.5:LRA=11"]  # no -shortest: the 1-frame cover would cut it
    else:
        maps += ["-map", "0:a:0"]
        acodec = ["-c:a", "copy"]
    if cover:
        maps += ["-map", f"{len(ins) // 2}:v:0", "-disposition:v:1", "attached_pic"]
        ins += ["-i", str(cover)]
    ffmpeg([*ins, *maps, "-c:v", "copy", *acodec, "-movflags", "+faststart", str(out)])


async def run(args):
    from telethon import TelegramClient

    env = llm.load_env(ENV_FILE)
    if not env.get("TELEGRAM_API_ID") or not env.get("TELEGRAM_API_HASH"):
        sys.exit(f"Missing TELEGRAM_API_ID / TELEGRAM_API_HASH in {ENV_FILE}. Copy .env.example to .env and fill "
                 "them in from https://my.telegram.org (API development tools).")
    api_id, api_hash = int(env["TELEGRAM_API_ID"]), env["TELEGRAM_API_HASH"]
    args.password = env.get("TELEGRAM_PASSWORD") or None
    channels = args.channel or read_channels(HERE / "channels.txt")
    if not channels:
        sys.exit("No channels: add IDs/usernames to topvideos/channels.txt or pass --channel.")

    now = dt.datetime.now().astimezone()
    since = now - dt.timedelta(hours=args.hours) if args.hours else now.replace(hour=0, minute=0, second=0, microsecond=0)
    W, H = (720, 1280) if args.format == "vertical" else (1280, 720)
    DL_DIR.mkdir(parents=True, exist_ok=True)
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    cache = load_cache()

    import sqlite3

    client = TelegramClient(str(HERE / "telegram"), api_id, api_hash)
    try:
        await client.connect()
    except sqlite3.OperationalError as e:
        if "locked" not in str(e):
            raise
        sys.exit("The Telegram login (topvideos\\telegram.session) is in use by another run of this script. "
                 "Wait until it has finished (or close its window) and start again.")
    if not await client.is_user_authorized():
        await login(client, args.login, args.password)
    async with client:
        cands = await collect(client, channels, since, args)
        if len(cands) < args.count and not args.hours:
            print(f"[info ] only {len(cands)} video(s) since midnight, widening to the last 24 h")
            since = now - dt.timedelta(hours=24)
            cands = await collect(client, channels, since, args)
        if not cands:
            sys.exit("No videos found in that time window.")

        cands.sort(key=lambda c: -c["eng"])
        if args.cached_only:
            rated = {k.split("|")[0] for k, v in cache.items() if k.endswith(f"|{args.subject or ''}|{args.lang}")}
            cands = [c for c in cands if c["key"] in rated and (DL_DIR / f"{c['key']}.mp4").exists()]
            print(f"[info ] --cached-only: {len(cands)} clip(s) already downloaded and rated")
        cands = cands[:args.max_candidates]
        print(f"[dl   ] downloading {len(cands)} candidate(s) ...")
        for i, c in enumerate(cands, 1):
            path = DL_DIR / f"{c['key']}.mp4"
            print(f"[dl   ] {i}/{len(cands)} {c['channel'][:20]:20} {c['link']}")
            if not path.exists() or path.stat().st_size == 0:
                tmp = path.with_suffix(".part")
                await client.download_media(c["_msg"], file=str(tmp))
                tmp.replace(path)
            c["path"] = path

    if not llm.available():
        sys.exit("Vision LLM not available (needed to rate the clips): set OPENROUTER_API_KEY in .env "
                 "or start Ollama with LLM_PROVIDER=ollama.")
    print(f"[rate ] rating with {llm.label()}")
    for i, c in enumerate(cands, 1):
        try:
            c["file_dur"], c["has_audio"] = probe(c["path"])
            ck = f"{c['key']}|{args.subject or ''}|{args.lang}"
            if ck not in cache or "wholesome" not in cache[ck]:
                cache[ck] = score(frame_grid(c["path"], c["file_dur"] or c["duration"]), c["text"], args.subject, args.lang)
                CACHE.write_text(json.dumps(cache, indent=1, ensure_ascii=False), encoding="utf-8")
            c["ai"] = cache[ck]
            a = c["ai"]
            print(f"[rate ] {i}/{len(cands)} {c['channel'][:20]:20} funny {a['funny']:4.1f} cute {a['cute']:4.1f} "
                  f"{'' if a['entertainment'] else '(not fun) '}{a['subject']}  {c['link']}")
        except Exception as e:
            print(f"[rate ] {c['key']}: skipped ({e})")
    llm.unload()  # frees the GPU for encoding

    pool = [c for c in cands if c.get("ai")]
    made = []
    for v in range(1, args.batch + 1):
        topic, top = pick(pool, args.topic, args.subject, args.count, args.min_wholesome)
        if len(top) < 2:
            if not made:
                sys.exit(f"Only {len(top)} usable clip(s) - not enough for a ranking. "
                         "Try --hours 48 or more channels.")
            print(f"[batch] only {len(top)} usable clip(s) left, stopping after {len(made)} video(s)")
            break
        if args.batch > 1:
            print(f"[batch] video {v}/{args.batch}")
        made.append(render_video(top, topic, args, now, W, H, suffix=f"_{v}" if v > 1 else ""))
        used = {c["key"] for c in top}
        pool = [c for c in pool if c["key"] not in used]  # every clip is used in one video only
    if len(made) > 1:
        print(f"[done ] {len(made)} videos:")
        for p in made:
            print(f"        {p}")


def render_video(top, topic, args, now, W, H, suffix=""):
    word = TOPIC_WORDS[topic]
    print(f"[pick ] topic: {topic}")
    n = len(top)
    for rank, c in enumerate(top, 1):
        c["rank"] = rank
        print(f"   #{rank}  {c['final']:5.2f}  {c['ai']['title'] or c['ai']['subject']}  ({c['channel']}) {c['link']}")

    stamp = f"{now:%Y-%m-%d}"
    work = OUT_DIR / f"_work_{topic}_{stamp}{suffix}"
    work.mkdir(exist_ok=True)
    headline = args.headline or f"Top {n} {word} Videos of Today"
    order = sorted(top, key=lambda c: -c["rank"])  # countdown: #5 first
    for c in order:
        full = c["file_dur"] or c["duration"]
        c["len"] = min(args.clip_seconds, full)
        c["start"] = best_window(c["path"], full, c["len"], c["has_audio"])

    # subscribe prompt in the middle: the clip closest to the half-way mark that is long enough for it
    t, spans = (args.intro_seconds if args.intro else 0.0), []
    for c in order:
        t += args.card_seconds
        spans.append((c, t))
        t += c["len"]
    sub = {}
    events = [(0.0, "whoosh")] if args.intro else []
    for c, s0 in spans:  # whoosh + pop as each rank card flies in
        events += [(s0 - args.card_seconds, "whoosh"), (s0 - args.card_seconds + 0.12, "pop")]
    if not args.no_subscribe:
        for c, s0 in sorted(spans, key=lambda x: abs(x[1] + x[0]["len"] / 2 - t / 2)):
            if c["len"] >= SUB_SECONDS + 0.6:
                at = min(max(t / 2 - s0, 0.3), c["len"] - SUB_SECONDS - 0.3)
                sub[c["rank"]] = (subscribe_frames(work / "subscribe", W, H, args.subscribe_text), at)
                events.append((s0 + at + SUB_CLICK, "click"))
                print(f"[video] subscribe prompt at {s0 + at:.1f}s of {t:.1f}s (in #{c['rank']})")
                break

    # No separate intro by default: the headline sits on top of every rank card, so when a Short loops,
    # #1 flows into #5 exactly like #2 flows into #1 and viewers don't notice the restart.
    parts = []
    if args.intro:
        card(work / "intro.png", W, H, headline, f"{now:%d.%m.%Y}", "#FFD400")
        render_card(work / "intro.png", args.intro_seconds, work / "00_intro.mp4", args)
        parts.append(work / "00_intro.mp4")
    for c in order:
        r = c["rank"]
        title = c["ai"]["title"] or c["ai"]["subject"]
        c["still"] = still_at(c["path"], c["start"] + c["len"] * 0.4)
        card_src = card_frames(work / f"card_{r}", W, H, r, title, RANK_COLORS[r], c["still"],
                               None if args.intro else headline, args.card_seconds, topic)
        badge(work / f"badge_{r}.png", W, H, r, title if args.title_bar else "", None if args.intro else headline)
        print(f"[video] #{r}: {c['len']:.1f}s")
        render_card(card_src, args.card_seconds, work / f"{10 - r:02d}a_card.mp4", args)
        # the clips' own sound (often copyrighted music) is only kept with --audio original
        render_clip(c["path"], work / f"badge_{r}.png", c["start"], c["len"], c["has_audio"] and args.audio == "original",
                    W, H, work / f"{10 - r:02d}b_clip.mp4", args, extra=sub.get(r))
        parts += [work / f"{10 - r:02d}a_card.mp4", work / f"{10 - r:02d}b_clip.mp4"]

    out = OUT_DIR / f"top{n}_{topic}_{stamp}{suffix}.mp4"
    thumb = out.with_name(out.stem + "_thumbnail.jpg")
    thumbnail(thumb, W, H, top, word, topic)
    joined = work / "joined.mp4"
    concat(parts, joined)
    music = None
    if args.audio == "music":
        import soundtrack

        total = probe(joined)[0]
        seed = int(f"{now:%Y%m%d}") * 100 + int(suffix[1:] or 1) * 2 + (topic == "cute")  # new tune per video
        music = soundtrack.soundtrack(work / "music.wav", total, events, topic, seed)
        print(f"[audio] own soundtrack ({total:.1f}s, loops with the video)")
    mux(joined, out, thumb, music)
    credits = [f"#{c['rank']}: {c['channel']} ({c['link']})" for c in top]
    out.with_suffix(".txt").write_text(f"{headline} - {stamp}\n\n" + "\n".join(credits) + "\n", encoding="utf-8")
    if not args.keep_work:
        shutil.rmtree(work)
    print(f"[done ] {out}")
    print(f"        thumbnail: {thumb.name}, sources/credits: {out.with_suffix('.txt').name}")
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--topic", choices=["auto", "funny", "cute"], default="auto",
                    help="auto = whichever theme has the stronger top 5 today (default)")
    ap.add_argument("--subject", help='only clips about this, e.g. "animals", "cats", "babies"')
    ap.add_argument("--channel", action="append", help="channel ID or @username (repeatable, overrides channels.txt)")
    ap.add_argument("--login", choices=["qr", "code"], default="qr",
                    help="first login: qr = scan a QR code with the Telegram app (default), code = phone + login code")
    ap.add_argument("--hours", type=float, help="look back this many hours instead of 'since midnight'")
    ap.add_argument("--count", type=int, default=5, help="places in the ranking (default 5)")
    ap.add_argument("--max-candidates", type=int, default=25, help="download/rate at most this many clips (default 25)")
    ap.add_argument("--scan-limit", type=int, default=300, help="posts read per channel at most (default 300)")
    ap.add_argument("--min-duration", type=float, default=3)
    ap.add_argument("--max-duration", type=float, default=180, help="skip longer videos (default 180 s)")
    ap.add_argument("--max-mb", type=float, default=150, help="skip larger files (default 150 MB)")
    ap.add_argument("--clip-seconds", type=float, default=10,
                    help="max length per clip, the liveliest part is used (default 10 s: 5 places stay under 60 s)")
    ap.add_argument("--card-seconds", type=float, default=1.2)
    ap.add_argument("--min-wholesome", type=float, default=6,
                    help="only clips the VLM rates at least this wholesome, 0-10 (default 6); political clips are "
                         "always left out")
    ap.add_argument("--intro", action="store_true",
                    help="separate headline card at the start (default: headline on the rank cards, loops seamlessly)")
    ap.add_argument("--intro-seconds", type=float, default=3.0)
    ap.add_argument("--format", choices=["vertical", "landscape"], default="vertical",
                    help="vertical 720x1280 (Shorts/Reels/TikTok, default) or landscape 1280x720")
    ap.add_argument("--lang", default="English", help="language of the titles (default English)")
    ap.add_argument("--headline", help='own intro text, default "Top 5 Funniest/Cutest Videos of Today"')
    ap.add_argument("--title-bar", action="store_true", help="also show the title as a bar on the clips")
    ap.add_argument("--audio", choices=["music", "original", "none"], default="music",
                    help="music = own generated soundtrack, copyright-free (default); original = the clips' "
                         "own sound (may contain copyrighted music); none = silent")
    ap.add_argument("--batch", type=int, default=1,
                    help="make up to N videos in one run, each with different clips (default 1)")
    ap.add_argument("--no-subscribe", action="store_true", help="no subscribe prompt in the middle")
    ap.add_argument("--subscribe-text", default="Please subscribe for more!")
    ap.add_argument("--nvenc", action="store_true", help="encode on the GPU (needs NVIDIA driver >= 570)")
    ap.add_argument("--keep-work", action="store_true", help="keep the intermediate cards/clips")
    ap.add_argument("--cached-only", action="store_true",
                    help="only use clips that are already downloaded and rated (no new downloads; only ratings from "
                         "before the wholesome/political check are redone)")
    args = ap.parse_args()
    # channel names/captions contain emojis, which the Windows console codepage can't print
    sys.stdout.reconfigure(errors="replace")
    args.count = max(2, min(5, args.count))

    import asyncio
    asyncio.run(run(args))


if __name__ == "__main__":
    main()
