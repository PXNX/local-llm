"""The sketch: an LLM writes it as JSON (script.json), normalize() makes any script (LLM or hand
edited) safe for the renderer - unknown faces/actions/effects are mapped or dropped."""
import re

FACES = ["neutral", "laughing", "angry", "shocked", "smug", "crying"]
FACE_ALIASES = {
    "happy": "laughing", "laugh": "laughing", "joy": "laughing", "excited": "laughing", "proud": "smug",
    "confident": "smug", "grin": "smug", "sly": "smug", "smirk": "smug", "mad": "angry", "furious": "angry",
    "shouting": "angry", "annoyed": "angry", "surprised": "shocked", "scared": "shocked", "afraid": "shocked",
    "confused": "shocked", "sad": "crying", "upset": "crying", "cry": "crying", "calm": "neutral", "bored": "neutral",
}
ACTIONS = ["jump", "shake", "shrug", "nod", "tremble", "fall", "spin", "zoom",
           "enter_left", "enter_right", "exit_left", "exit_right"]
EVENTS = ["appear", "vanish", "fly_across", "explode", "hit"]
SHOTS = ["wide", "medium", "close"]
SFX = ["explosion", "big_explosion", "launch", "whoosh", "pop", "boing", "rimshot", "crickets",
       "sad_trombone", "ding", "slap"]
BACKGROUNDS = ["plain", "room", "city", "field", "night"]

EXAMPLE = """{
  "title": "Short funny title",
  "cast": [
    {"name": "Trump", "who": "Donald Trump", "gender": "male"},
    {"name": "Zelensky", "who": "Volodymyr Zelensky", "gender": "male"}
  ],
  "scenes": [
    {
      "background": "room",
      "label": "",
      "stage": ["Zelensky", "Trump"],
      "beats": [
        {"who": "Trump", "say": "Folks, I have a beautiful plan.", "face": "smug", "shot": "medium"},
        {"who": "Zelensky", "say": "Is it the same plan as last week?", "face": "neutral", "shot": "close"},
        {"pause": 1.5, "who": "Trump", "face": "shocked"},
        {"who": "Trump", "say": "It has a new cover!", "face": "laughing", "do": "shrug", "shot": "close"}
      ]
    },
    {
      "background": "map of the Black Sea",
      "label": "RUSSIA",
      "stage": ["oil refinery"],
      "beats": [
        {"thing": "Shahed drone", "event": "hit", "target": "oil refinery", "shot": "wide"},
        {"who": "Putin", "do": "enter_right", "say": "Just a small smoking accident.", "face": "crying"}
      ]
    }
  ]
}"""

PROMPT = """You write short political comedy sketches for an animated cartoon channel in the style of
"freeonis": simple flat 2D caricatures of world leaders, dry deadpan dialogue, absurd escalation,
awkward silent stares, frequent cuts to close-ups and one visual punchline at the end (a drone hits a
refinery, someone falls over, a thing explodes ...). It is satire of public political behaviour:
no slurs, no sexual content, no gore, people are never hurt (objects may explode).

Topic: {topic}
Dialogue language: {lang}
Length: about {seconds} seconds, roughly {words} spoken words in total, 1-3 scenes, {beats} beats.
{cast_hint}
Drawings that already exist (prefer these names): {known}

Reply with JSON only, exactly this structure:
{example}

Rules:
- cast: every character that speaks or appears; "name" short (used in the beats), "who" the full real
  name (used to draw the caricature), "gender" male or female
- scene "background": one of {backgrounds}, "map of <region>" or a short place ("the Kremlin throne
  room", "the Oval Office"); "label": optional short CAPS word shown on it (a country name on a map)
- scene "stage": left-to-right list of the characters AND things (objects/buildings, e.g. "oil refinery")
  standing in the scene at its start
- a line beat: "who", "say" (max 15 words), "face", optional "shot", optional "do", optional "react"
  ({{"OtherName": "shocked"}} - the listener's face)
- a pause beat: {{"pause": 1.5, "who": "X", "face": "..."}} - silent close-up stare, great for timing
- an action beat: "who" + "do" without "say" (e.g. somebody enters or falls over)
- an event beat: "thing" + "event" (+ "target" for hit): appear, vanish, fly_across (drone/missile
  over the sky), explode, hit (the thing flies into the target, which explodes and burns)
- "face": one of {faces}
- "do": one of {actions}
- "shot": wide (everybody), medium, close (speaker's face); cut a lot, like a real cartoon
- optional "sfx" on any beat: one of {sfx}
- the dialogue is in {lang}, all JSON keys and the values of face/do/shot/sfx/event stay English
"""


def write(topic, lang, seconds, cast, known, llm):
    words = int(seconds * 2.3)
    beats = f"{max(6, seconds // 6)}-{max(10, seconds // 3)}"
    cast_hint = f"Use these characters: {', '.join(cast)}." if cast else \
        "Pick the 2-3 world leaders that fit the topic best."
    prompt = PROMPT.format(topic=topic, lang=lang, seconds=seconds, words=words, beats=beats, cast_hint=cast_hint,
                           known=", ".join(known) or "none", example=EXAMPLE, backgrounds=", ".join(BACKGROUNDS),
                           faces=", ".join(FACES), actions=", ".join(ACTIONS), sfx=", ".join(SFX))
    return llm.text_json(prompt, temperature=0.9)


# ---------------------------------------------------------------- normalize
def _key(s):
    return re.sub(r"[^a-z0-9]+", " ", str(s).lower()).strip()


def _pick(value, allowed, aliases=None, default=None):
    v = _key(value).replace(" ", "_")
    if v in allowed:
        return v
    if aliases and v in aliases:
        return aliases[v]
    for a in allowed:  # "angry and shouting" -> angry, "close up" -> close
        if a in v or v.startswith(a):
            return a
    return default


def normalize(script):
    """Returns a clean copy: known keys only, names resolved to the cast, every scene with a stage."""
    cast, by_key = [], {}

    def member(name, who=None, gender=None):
        name = str(name).strip()
        k = _key(name)
        if not k:
            return None
        for m in cast:  # "Donald Trump" and "Trump" are the same person
            if k in (_key(m["name"]), _key(m["who"])) or k.split()[-1] == _key(m["name"]).split()[-1]:
                by_key[k] = m
                return m
        m = {"name": name, "who": str(who or name).strip(), "gender": "female" if str(gender).lower().startswith("f") else "male"}
        cast.append(m)
        by_key[k] = m
        return m

    for c in script.get("cast") or []:
        if isinstance(c, dict) and c.get("name"):
            m = member(c["name"], c.get("who"), c.get("gender"))
            for extra in ("voice", "pitch", "speed", "look"):
                if c.get(extra) not in (None, ""):
                    m[extra] = c[extra]
        elif isinstance(c, str):
            member(c)

    def char(name):
        k = _key(name)
        return (by_key.get(k) or member(name))["name"] if k else None

    def is_char(name):
        return _key(name) in by_key

    scenes = []
    for sc in script.get("scenes") or []:
        if not isinstance(sc, dict):
            continue
        stage = []
        for s in sc.get("stage") or sc.get("cast") or []:
            s = str(s).strip()
            if s and s not in stage:
                stage.append(char(s) if is_char(s) else s)
        beats = []
        for b in sc.get("beats") or []:
            if not isinstance(b, dict):
                continue
            nb = {}
            if b.get("who"):
                nb["who"] = char(b["who"])
            if b.get("say") and nb.get("who"):
                nb["say"] = str(b["say"]).strip()
            if b.get("pause") is not None:
                try:
                    nb["pause"] = min(4.0, max(0.3, float(b["pause"])))
                except (TypeError, ValueError):
                    nb["pause"] = 1.2
            if b.get("thing") and b.get("event"):
                ev = _pick(b["event"], EVENTS)
                if ev:
                    nb["thing"], nb["event"] = str(b["thing"]).strip(), ev
                    if ev == "hit":
                        tgt = str(b.get("target") or "").strip()
                        if not tgt:
                            continue
                        nb["target"] = char(tgt) if is_char(tgt) else tgt
            for key, allowed, aliases in (("face", FACES, FACE_ALIASES), ("do", ACTIONS, None),
                                          ("shot", SHOTS, None), ("sfx", SFX, None)):
                if b.get(key):
                    v = _pick(b[key], allowed, aliases)
                    if v:
                        nb[key] = v
            if isinstance(b.get("react"), dict):
                react = {char(k): _pick(v, FACES, FACE_ALIASES, "neutral") for k, v in b["react"].items() if _key(k)}
                if react:
                    nb["react"] = react
            if not ({"say", "pause", "event", "do"} & nb.keys()) and "sfx" not in nb:
                continue
            beats.append(nb)
            # everybody who acts must stand on the stage (entering characters are added hidden)
            for name in [nb.get("who"), nb.get("target") if nb.get("event") == "hit" else None]:
                if name and name not in stage:
                    stage.append(name)
            if nb.get("event") in ("appear", "explode", "vanish") and nb["thing"] not in stage:
                stage.append(nb["thing"])
        if not beats:
            continue
        bg = str(sc.get("background") or "plain").strip()
        scenes.append({"background": bg if _key(bg) else "plain", "label": str(sc.get("label") or "").strip()[:24],
                       "stage": stage, "beats": beats})
    if not scenes:
        raise ValueError("the script has no scenes with beats")
    used = {n for sc in scenes for n in sc["stage"]} | {b.get("who") for sc in scenes for b in sc["beats"]}
    return {"title": str(script.get("title") or "Untitled").strip(), "cast": [m for m in cast if m["name"] in used],
            "scenes": scenes, **({"seed": script["seed"]} if isinstance(script.get("seed"), int) else {})}


def things(script):
    """Names of all objects (not characters) in the script."""
    names = {m["name"] for m in script["cast"]}
    out = []
    for sc in script["scenes"]:
        for n in sc["stage"] + [b.get("thing") for b in sc["beats"]] + [b.get("target") for b in sc["beats"]]:
            if n and n not in names and n not in out:
                out.append(n)
    return out


def as_text(script):
    lines = [f"== {script['title']} =="]
    for i, sc in enumerate(script["scenes"], 1):
        lines.append(f"\n[scene {i}: {sc['background']}{' - ' + sc['label'] if sc['label'] else ''}] "
                     f"stage: {', '.join(sc['stage'])}")
        for b in sc["beats"]:
            extra = ", ".join(f"{k}={b[k]}" for k in ("face", "do", "shot", "sfx") if k in b)
            if "say" in b:
                lines.append(f"  {b['who']}: {b['say']}" + (f"   ({extra})" if extra else ""))
            elif "pause" in b:
                lines.append(f"  ... {b.get('who', '')} {b['pause']:.1f} s silence" + (f"   ({extra})" if extra else ""))
            elif "event" in b:
                lines.append(f"  * {b['thing']} {b['event']}" + (f" -> {b['target']}" if "target" in b else ""))
            else:
                lines.append(f"  * {b.get('who', '')} {b.get('do', b.get('sfx', ''))}")
    return "\n".join(lines)
