"""The library: characters and backgrounds that are kept between sketches, so a character looks and
sounds the same in every video and a place is drawn once.

comedy/library/characters.json   {"characters": [{"name", "who", "gender", "image", "personality", "look",
                                 "voice", "voice_lang", "pitch", "speed"}, ...]} - written by --save-cast and by
                                 the GUI (Comedy sketch > Saved characters); edit it by hand if you like
comedy/library/scenes/<place>.png (+ <place>_vertical.png)   one background per place, used by every sketch
                                 with that background; put your own picture there to use it instead
"""
import json
import os
import re
import shutil
from pathlib import Path

HERE = Path(__file__).resolve().parent
LIB = HERE / "library"
CHARACTERS = LIB / "characters.json"
SCENES = LIB / "scenes"
FIELDS = ("image", "personality", "look", "voice", "pitch", "speed")
SCENE_EXT = (".png", ".webp", ".jpg", ".jpeg")


def _key(s):
    return re.sub(r"[^a-z0-9]+", " ", str(s).lower()).strip()


# ---------------------------------------------------------------- characters
def characters():
    """Saved characters; an unreadable file counts as empty (and is never overwritten blindly, see remember)."""
    try:
        data = json.loads(CHARACTERS.read_text(encoding="utf-8"))
        return [c for c in data.get("characters", []) if isinstance(c, dict) and c.get("name")]
    except FileNotFoundError:
        return []
    except (OSError, ValueError, AttributeError) as e:
        print(f"[library] {CHARACTERS} is unreadable ({str(e)[:100]}), ignoring the saved characters")
        return None


def find(saved, name, who=None):
    """The saved character for a name: same name or full name, else the same last name."""
    keys = [k for k in (_key(name), _key(who or "")) if k]
    for c in saved:
        if {_key(c["name"]), _key(c.get("who", ""))} & set(keys):
            return c
    last = {k.split()[-1] for k in keys}
    return next((c for c in saved if _key(c["name"]).split()[-1] in last), None)


def fill(cast, lang):
    """Gives cast members what the library knows and they don't have yet (the script and the command line win)."""
    saved = characters() or []
    for m in cast:
        c = find(saved, m["name"], m.get("who"))
        if c is None:
            continue
        got = []
        for f in FIELDS:
            v = c.get(f)
            if v in (None, "") or f in m:
                continue
            if f == "image" and not Path(v).exists():
                print(f"[library] {m['name']}: saved picture is gone ({v})")
                continue
            if f == "voice" and _key(c.get("voice_lang", "")) != _key(lang):
                continue  # a voice only fits the language it was picked for
            m[f] = v
            got.append(f)
        if c.get("who") and _key(m.get("who", "")) == _key(m["name"]):
            m["who"] = c["who"]
        if got:
            print(f"[library] {m['name']}: saved {', '.join(got)}")


def personalities(names):
    """{name: personality} of the saved characters among names (for the writer's prompt)."""
    saved = characters() or []
    out = {}
    for n in names:
        c = find(saved, n)
        if c and c.get("personality"):
            out[n] = c["personality"]
    return out


def remember(cast, lang):
    """--save-cast: stores the sketch's characters (picture, personality, look, voice) for the next sketches."""
    saved = characters()
    if saved is None:
        print("[library] not saving the characters: fix or delete the unreadable file first")
        return
    for m in cast:
        c = find(saved, m["name"], m.get("who"))
        if c is None:
            c = {"name": m["name"]}
            saved.append(c)
        c.update({k: m[k] for k in ("who", "gender", *FIELDS) if m.get(k) not in (None, "")})
        if m.get("voice"):
            c["voice_lang"] = lang
    LIB.mkdir(parents=True, exist_ok=True)
    tmp = CHARACTERS.with_suffix(".json.tmp")
    tmp.write_text(json.dumps({"characters": saved}, indent=2, ensure_ascii=False), encoding="utf-8")
    os.replace(tmp, CHARACTERS)
    print(f"[library] saved {len(cast)} character(s): {CHARACTERS}")


# ---------------------------------------------------------------- scenes
def _scene_stem(kind, landscape):
    words = [w for w in _key(kind).split() if w not in ("the", "a", "an")]
    return "-".join(words)[:80] + ("" if landscape else "_vertical")


def scene(kind, landscape):
    """The saved background of a place, or None."""
    stem = _scene_stem(kind, landscape)
    return next((SCENES / (stem + e) for e in SCENE_EXT if (SCENES / (stem + e)).exists()), None) if stem else None


def keep_scene(kind, landscape, path):
    """Copies a freshly drawn background into the library; returns the library file."""
    stem = _scene_stem(kind, landscape)
    if not stem:
        return path
    SCENES.mkdir(parents=True, exist_ok=True)
    for e in SCENE_EXT:  # a redrawn place replaces the old picture, whatever its type
        (SCENES / (stem + e)).unlink(missing_ok=True)
    target = SCENES / (stem + Path(path).suffix.lower())
    tmp = target.with_name(target.name + ".tmp")
    shutil.copyfile(path, tmp)
    os.replace(tmp, target)
    return target


def places(landscape=True):
    """Names of the saved places in this format (for the writer's prompt)."""
    if not SCENES.is_dir():
        return []
    stems = {p.stem for p in SCENES.iterdir() if p.suffix.lower() in SCENE_EXT}
    return sorted(s.removesuffix("_vertical").replace("-", " ") for s in stems if s.endswith("_vertical") != landscape)
