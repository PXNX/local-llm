"""Today's top 5 funniest / cutest videos from Telegram channels, as one countdown video.

Pipeline: Telethon (your Telegram account) reads today's posts of the channels in
topvideos/channels.txt -> keeps short videos, drops reposts (same file / same duration+size) ->
pre-ranks them by engagement (views + reactions relative to the channel's usual views) and
downloads the best --max-candidates -> qwen3-vl:4b (Ollama) looks at a 2x2 frame grid + the
caption of each and scores "funny" and "cute" 0-10, names the subject and writes a short title ->
the topic (--topic, or auto: the one with the stronger top 5) decides, the 5 best clips that fit
it are ranked (AI score + engagement bonus) -> ffmpeg renders intro card, then #5 ... #1, each as
a title card + the clip (blurred fill background, rank badge, loudness normalized) -> one MP4.

Telegram login: create an app at https://my.telegram.org -> API development tools, put api_id
and api_hash (and the two-step verification password, if any) into the repo's .env (see
.env.example). The first run shows a QR code to scan in the Telegram app once, the session is
kept in topvideos/telegram.session (keep it private).
You must be subscribed to (or have opened) the channels so their numeric IDs can be resolved.

Usage (via 9-top5-videos.bat, which uses ComfyUI's embedded Python):
  9-top5-videos.bat [--topic funny|cute|auto] [--subject animals] [--hours 24] [options]
"""
import argparse
import base64
import datetime as dt
import io
import json
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

import av
import imageio_ffmpeg
from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).resolve().parent
ENV_FILE = HERE.parent / ".env"
OUT_DIR = HERE / "out"
DL_DIR = HERE / "downloads"
CACHE = HERE / "scores.json"
OLLAMA_URL = "http://127.0.0.1:11434"
VISION_MODEL = "qwen3-vl:4b"
FONTS = Path("C:/Windows/Fonts")
FFMPEG = imageio_ffmpeg.get_ffmpeg_exe()
FPS = 30
RANK_COLORS = {1: "#FFD400", 2: "#D9D9D9", 3: "#E0913A", 4: "#7FD1FF", 5: "#FF7FBF"}
TOPIC_WORDS = {"funny": "Funniest", "cute": "Cutest"}


# ---------------------------------------------------------------- helpers
def http_json(url, payload=None, timeout=600):
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def read_channels(path):
    chans = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            chans.append(line)
    return chans


def load_env(path):
    """KEY=value lines from the repo's .env (# comments, optional quotes); real env vars win."""
    import os

    env = {}
    if path.exists():
        for line in path.read_text(encoding="utf-8-sig").splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1)
                env[k.strip()] = v.strip().strip("'\"")
    env.update({k: v for k, v in os.environ.items() if k.startswith("TELEGRAM_")})
    return env


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
        + subj +
        '"subject": "<2-4 English words, e.g. \\"cat vs cucumber\\">", '
        f'"title": "<catchy title in {lang}, max 6 words, no hashtags, no emojis>"}}'
    )
    payload = {
        "model": VISION_MODEL,
        "messages": [{"role": "user", "content": prompt, "images": [base64.b64encode(buf.getvalue()).decode()]}],
        "format": "json", "stream": False, "think": False, "keep_alive": "5m",
        "options": {"temperature": 0.2},
    }
    try:
        res = http_json(f"{OLLAMA_URL}/api/chat", payload)
    except urllib.error.HTTPError:
        payload.pop("think")  # model without thinking support
        res = http_json(f"{OLLAMA_URL}/api/chat", payload)
    d = json.loads(res["message"]["content"])
    num = lambda k: max(0.0, min(10.0, float(d.get(k, 0) or 0)))
    return {
        "funny": num("funny"), "cute": num("cute"),
        "entertainment": bool(d.get("entertainment", True)),
        "fits_subject": bool(d.get("fits_subject", True)),
        "subject": str(d.get("subject", "")).strip(),
        "title": re.sub(r"[#\"]", "", str(d.get("title", "")).strip())[:60],
    }


def pick(cands, topic, subject, n):
    ok = [c for c in cands if c.get("ai") and c["ai"]["entertainment"] and (not subject or c["ai"]["fits_subject"])]
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


def card(path, W, H, big, sub, color, bg_img=None):
    """Title card: blurred frame (or gradient) background, huge headline, subtitle."""
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
    bg.save(path)


def badge(path, W, H, rank, title):
    """Transparent overlay for the clip: rank circle top-left, title bar at the bottom."""
    im = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    unit = min(W, H)
    r = int(unit * 0.09)
    cx, cy = int(unit * 0.04) + r, int(unit * 0.04) + r
    d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=RANK_COLORS[rank], outline="black", width=max(3, unit // 150))
    f = font(int(r * 1.2))
    t = f"#{rank}"
    tw = d.textlength(t, font=f)
    d.text((cx - tw / 2, cy - f.size * 0.62), t, font=f, fill="black")
    if title:
        f2, lines = fit_lines(d, title, "ariblk.ttf", W * 0.88, int(unit * 0.06), 22)
        bar_h = int(len(lines) * f2.size * 1.12 + unit * 0.04)
        y0 = H - bar_h - int(H * 0.06)
        d.rounded_rectangle((W * 0.04, y0, W * 0.96, y0 + bar_h), radius=unit // 40, fill=(0, 0, 0, 150))
        centered(d, lines, f2, y0 + int(unit * 0.02), W, "white", stroke=0)
    im.save(path)


# ---------------------------------------------------------------- 4. video
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
    ffmpeg(["-loop", "1", "-framerate", str(FPS), "-t", f"{seconds}", "-i", str(png),
            "-f", "lavfi", "-t", f"{seconds}", "-i", "anullsrc=r=44100:cl=stereo",
            "-vf", f"fade=in:0:{FPS // 3},fade=out:st={seconds - 0.3}:d=0.3,format=yuv420p",
            *venc(args), *AENC, "-r", str(FPS), "-shortest", str(out)])


def render_clip(src, overlay_png, start, length, has_audio, W, H, out, args):
    fo = max(0.0, length - 0.35)
    vf = (f"[0:v]split[a][b];"
          f"[a]scale={W}:{H}:force_original_aspect_ratio=increase,crop={W}:{H},boxblur=25:3,eq=brightness=-0.12[bg];"
          f"[b]scale={W}:{H}:force_original_aspect_ratio=decrease[fg];"
          f"[bg][fg]overlay=(W-w)/2:(H-h)/2,setsar=1,fps={FPS}[v0];"
          f"[v0][1:v]overlay=0:0,fade=in:0:6,fade=out:st={fo}:d=0.35,format=yuv420p[v]")
    inputs = ["-ss", f"{start}", "-t", f"{length}", "-i", str(src), "-i", str(overlay_png)]
    if has_audio:
        af = (f"[0:a]aresample=44100,aformat=channel_layouts=stereo,loudnorm=I=-16:TP=-1.5:LRA=11,"
              f"afade=in:d=0.2,afade=out:st={fo}:d=0.35[a]")
    else:
        inputs += ["-f", "lavfi", "-t", f"{length}", "-i", "anullsrc=r=44100:cl=stereo"]
        af = "[2:a]anull[a]"
    ffmpeg([*inputs, "-filter_complex", vf + ";" + af, "-map", "[v]", "-map", "[a]",
            *venc(args), *AENC, "-r", str(FPS), "-t", f"{length}", str(out)])


def concat(parts, out):
    lst = out.with_suffix(".txt")
    lst.write_text("".join(f"file '{p.as_posix()}'\n" for p in parts), encoding="utf-8")
    ffmpeg(["-f", "concat", "-safe", "0", "-i", str(lst), "-c", "copy", "-movflags", "+faststart", str(out)])
    lst.unlink()


# ---------------------------------------------------------------- main
async def run(args):
    from telethon import TelegramClient

    env = load_env(ENV_FILE)
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
        cands = cands[:args.max_candidates]
        print(f"[dl   ] downloading {len(cands)} candidate(s) ...")
        for c in cands:
            path = DL_DIR / f"{c['key']}.mp4"
            if not path.exists() or path.stat().st_size == 0:
                tmp = path.with_suffix(".part")
                await client.download_media(c["_msg"], file=str(tmp))
                tmp.replace(path)
            c["path"] = path

    # Ollama for scoring
    try:
        urllib.request.urlopen(OLLAMA_URL, timeout=3)
    except Exception:
        sys.exit("Ollama is not running (needed to rate the clips).")
    for i, c in enumerate(cands, 1):
        try:
            c["file_dur"], c["has_audio"] = probe(c["path"])
            ck = f"{c['key']}|{args.subject or ''}|{args.lang}"
            if ck not in cache:
                cache[ck] = score(frame_grid(c["path"], c["file_dur"] or c["duration"]), c["text"], args.subject, args.lang)
                CACHE.write_text(json.dumps(cache, indent=1, ensure_ascii=False), encoding="utf-8")
            c["ai"] = cache[ck]
            a = c["ai"]
            print(f"[rate ] {i}/{len(cands)} {c['channel'][:20]:20} funny {a['funny']:4.1f} cute {a['cute']:4.1f} "
                  f"{'' if a['entertainment'] else '(not fun) '}{a['subject']}")
        except Exception as e:
            print(f"[rate ] {c['key']}: skipped ({e})")
    # unload the VLM so the GPU is free for encoding
    try:
        http_json(f"{OLLAMA_URL}/api/generate", {"model": VISION_MODEL, "keep_alive": 0}, timeout=30)
    except Exception:
        pass

    topic, top = pick(cands, args.topic, args.subject, args.count)
    if len(top) < 2:
        sys.exit(f"Only {len(top)} usable clip(s) - not enough for a ranking. Try --hours 48 or more channels.")
    word = TOPIC_WORDS[topic]
    print(f"[pick ] topic: {topic}")
    n = len(top)
    for rank, c in enumerate(top, 1):
        c["rank"] = rank
        print(f"   #{rank}  {c['final']:5.2f}  {c['ai']['title'] or c['ai']['subject']}  ({c['channel']}, msg {c['msg_id']})")

    # ---- render
    stamp = f"{now:%Y-%m-%d}"
    work = OUT_DIR / f"_work_{topic}_{stamp}"
    work.mkdir(exist_ok=True)
    headline = args.headline or f"Top {n} {word} Videos of Today"
    parts = []
    card(work / "intro.png", W, H, headline, f"{now:%d.%m.%Y}", "#FFD400")
    render_card(work / "intro.png", args.intro_seconds, work / "00_intro.mp4", args)
    parts.append(work / "00_intro.mp4")
    for c in sorted(top, key=lambda c: -c["rank"]):  # countdown: #5 first
        r = c["rank"]
        title = c["ai"]["title"] or c["ai"]["subject"]
        length = min(args.clip_seconds, c["file_dur"] or c["duration"])
        with av.open(str(c["path"])) as con:
            s = con.streams.video[0]
            fr = next(con.decode(s))
            still = fr.to_image()
        card(work / f"card_{r}.png", W, H, f"#{r}", title, RANK_COLORS[r], still)
        badge(work / f"badge_{r}.png", W, H, r, "" if args.no_title_bar else title)
        print(f"[video] #{r}: {length:.1f}s")
        render_card(work / f"card_{r}.png", args.card_seconds, work / f"{10 - r:02d}a_card.mp4", args)
        render_clip(c["path"], work / f"badge_{r}.png", 0.0, length, c["has_audio"], W, H,
                    work / f"{10 - r:02d}b_clip.mp4", args)
        parts += [work / f"{10 - r:02d}a_card.mp4", work / f"{10 - r:02d}b_clip.mp4"]

    out = OUT_DIR / f"top{n}_{topic}_{stamp}.mp4"
    concat(parts, out)
    credits = [f"#{c['rank']}: {c['channel']} (t.me/c/{c['chan_id']}/{c['msg_id']})" for c in top]
    out.with_suffix(".txt").write_text(f"{headline} - {stamp}\n\n" + "\n".join(credits) + "\n", encoding="utf-8")
    if not args.keep_work:
        for p in work.iterdir():
            p.unlink()
        work.rmdir()
    print(f"[done ] {out}")
    print(f"        sources/credits: {out.with_suffix('.txt').name}")


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
    ap.add_argument("--clip-seconds", type=float, default=30, help="cut each clip to this length (default 30 s)")
    ap.add_argument("--card-seconds", type=float, default=2.0)
    ap.add_argument("--intro-seconds", type=float, default=3.0)
    ap.add_argument("--format", choices=["vertical", "landscape"], default="vertical",
                    help="vertical 720x1280 (Shorts/Reels/TikTok, default) or landscape 1280x720")
    ap.add_argument("--lang", default="English", help="language of the titles (default English)")
    ap.add_argument("--headline", help='own intro text, default "Top 5 Funniest/Cutest Videos of Today"')
    ap.add_argument("--no-title-bar", action="store_true", help="only the rank badge on the clips")
    ap.add_argument("--nvenc", action="store_true", help="encode on the GPU (needs NVIDIA driver >= 570)")
    ap.add_argument("--keep-work", action="store_true", help="keep the intermediate cards/clips")
    args = ap.parse_args()
    # channel names/captions contain emojis, which the Windows console codepage can't print
    sys.stdout.reconfigure(errors="replace")
    args.count = max(2, min(5, args.count))

    import asyncio
    asyncio.run(run(args))


if __name__ == "__main__":
    main()
