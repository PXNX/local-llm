"""Turns the script + voice lines + drawings into a plan: plain data (picklable for the render
workers) that says, for every moment, where everything is, which face it makes, what the camera
frames, which line is spoken and what explodes."""
import numpy as np
from PIL import Image

import cast as cast_mod
import scene as scene_mod

ACTION_SECONDS = {"jump": 0.6, "shake": 0.6, "shrug": 0.6, "nod": 0.7, "fall": 0.6, "spin": 0.7, "zoom": 0.25,
                  "enter_left": 0.9, "enter_right": 0.9, "exit_left": 0.9, "exit_right": 0.9}
EVENT_SECONDS = {"appear": 0.9, "vanish": 0.7, "fly_across": 2.0, "explode": 1.8, "hit": 2.8}
FLIGHT = 1.3
STINGS = {"rimshot": 1.3, "sad_trombone": 2.2, "crickets": 2.0, "ding": 0.8}  # played after the line
GAP = 0.3
INTRO = 0.6


def _aspect(path, kind):
    if path is None:
        return 0.62 if kind == "char" else 1.3
    with Image.open(path) as im:
        return im.width / im.height


def _envelope(audio, fps, sr=44100):
    """Talk amplitude 0..1 per video frame (drives the squash & stretch)."""
    hop = sr / fps
    n = int(len(audio) / hop) + 1
    rms = np.array([np.sqrt(np.mean(audio[int(i * hop):int((i + 1) * hop)] ** 2) + 1e-12) for i in range(n)])
    rms = rms / max(1e-6, np.quantile(rms, 0.9))
    out, v = [], 0.0
    for x in np.clip(rms, 0, 1.2):
        v += (x - v) * (0.6 if x > v else 0.25)
        out.append(round(float(min(1.0, v)), 3))
    return out


def build(script, sprites, voice_fn, W, H, fps, end_card=True, subtitles=True, handle="", seed=0, bg_paths=None):
    """sprites: {name: {state: path|None}} (faces for characters, normal/fire/damaged for things).
    voice_fn(text, member) -> mono audio. Returns (plan, dialogue [(t, audio, pan)], effects [(t, name)])."""
    names = {m["name"]: m for m in script["cast"]}
    top, feet = scene_mod.layout(W, H)
    vertical = H > W
    char_h = H * (0.40 if vertical else 0.62)
    plan = {"W": W, "H": H, "fps": fps, "title": script["title"], "handle": handle, "subtitles": subtitles,
            "scenes": [], "lines": [], "end": None, "sprites": sprites}
    dialogue, effects = [], []
    t = 0.0
    scenes = list(script["scenes"])
    if end_card:
        chars = [m["name"] for m in script["cast"]][:4]
        scenes.append({"background": "plain", "label": "", "stage": chars, "end": True,
                       "beats": [{"who": c, "do": "jump"} for c in chars] + [{"pause": 1.6}]})
    for si, sc in enumerate(scenes):
        kind = sc["background"]
        # ---- layout: stage items left to right, all feet on the ground
        items = []
        for name in sc["stage"]:
            is_char = name in names
            states = sprites.get(name, {})
            ref = states.get("neutral") if is_char else states.get("normal")
            ref = ref or next((p for p in states.values() if p), None)
            if is_char:
                h = char_h * (0.68 if sc.get("end") else 1)
            elif cast_mod.missile_like(name):
                h = H * 0.18
            else:
                h = H * (0.36 if vertical else 0.50)
            items.append({"name": name, "type": "char" if is_char else "thing", "h": h,
                          "w": h * _aspect(ref, "char" if is_char else "thing"),
                          "faces": [(0.0, "neutral")], "states": [(0.0, "normal")], "visible": [(0.0, True)],
                          "actions": []})
        total_w = sum(i["w"] for i in items)
        room = W * 0.94
        if total_w > room * 0.9:
            k = room * 0.9 / total_w
            for i in items:
                i["h"] *= k
                i["w"] *= k
            total_w *= k
        gap = min((W - total_w) / (len(items) + 1), W * 0.14) if items else 0
        x = (W - total_w - gap * (len(items) - 1)) / 2
        for i in items:
            i["x"], i["y"] = x + i["w"] / 2, H * feet
            x += i["w"] + gap
        by = {i["name"]: i for i in items}
        n_chars = sum(1 for i in items if i["type"] == "char")
        # hidden until they enter / appear
        for b in sc["beats"]:
            if b.get("do", "").startswith("enter_") and b.get("who") in by:
                by[b["who"]]["visible"] = [(0.0, False)]
            if b.get("event") == "appear" and b.get("thing") in by:
                by[b["thing"]]["visible"] = [(0.0, False)]
        for name, it in by.items():
            first = next((b for b in sc["beats"] if b.get("who") == name or b.get("thing") == name), None)
            if first and ((first.get("do") or "").startswith("enter_") or first.get("event") == "appear"):
                continue
            it["visible"] = [(0.0, True)]
        t0 = t
        shots = [(t, "wide", None)]
        flights, booms = [], []
        t += INTRO
        for b in sc["beats"]:
            bt = t
            who = b.get("who")
            it = by.get(who)
            act = b.get("do")
            face = b.get("face")
            shot = b.get("shot")
            if it is not None and face:
                it["faces"].append((bt, face))
            for other, f in (b.get("react") or {}).items():
                if other in by:
                    by[other]["faces"].append((bt + 0.3, f))
            speak_at = bt + 0.1
            if it is not None and act:
                d = ACTION_SECONDS.get(act, 0.6)
                it["actions"].append((bt, act, d))
                if act.startswith("enter_"):
                    it["visible"].append((bt, True))
                    speak_at = bt + d
                elif act.startswith("exit_"):
                    it["visible"].append((bt + d, False))
            end = bt + (ACTION_SECONDS.get(act, 0.6) + GAP if act else 0)
            if "say" in b and who in names:
                audio = voice_fn(b["say"], names[who])
                length = len(audio) / 44100
                pan = ((it["x"] / W) * 2 - 1) if it else 0.0
                dialogue.append((speak_at, audio, pan))
                plan["lines"].append({"t0": speak_at, "t1": speak_at + length, "who": who, "text": b["say"],
                                      "env": _envelope(audio, fps)})
                end = max(end, speak_at + length + GAP)
                if act == "tremble":
                    it["actions"][-1] = (bt, "tremble", end - bt)
                shot = shot or ("close" if n_chars >= 2 else "medium")
            elif "pause" in b:
                end = max(end, bt + b["pause"])
                shot = shot or ("close" if who else None)
            if b.get("event"):
                ev, thing = b["event"], b["thing"]
                end = max(end, bt + EVENT_SECONDS[ev])
                target = by.get(thing)
                if ev == "appear" and target:
                    target["visible"].append((bt, True))
                    target["actions"].append((bt, "pop_in", 0.35))
                    effects.append((bt, b.get("sfx", "pop")))
                elif ev == "vanish" and target:
                    target["actions"].append((bt, "pop_out", 0.25))
                    target["visible"].append((bt + 0.25, False))
                    effects.append((bt, b.get("sfx", "whoosh")))
                elif ev == "explode" and target:
                    booms.append({"t": bt + 0.1, "x": target["x"], "y": target["y"] - target["h"] * 0.45,
                                  "size": max(target["h"], H * 0.25)})
                    target["states"].append((bt + 0.15, "burning"))
                    effects.append((bt + 0.1, b.get("sfx", "explosion")))
                elif ev == "fly_across":
                    flights.append({"thing": thing, "t0": bt, "t1": bt + 1.8, "from": (-W * 0.15, H * 0.24),
                                    "to": (W * 1.15, H * 0.18), "arc": -H * 0.04, "h": H * 0.15})
                    effects.append((bt + 0.4, b.get("sfx", "whoosh")))
                elif ev == "hit":
                    tgt = by.get(b["target"])
                    if tgt is None:
                        continue
                    aim = (tgt["x"], tgt["y"] - tgt["h"] * 0.5)
                    start_x = -W * 0.15 if tgt["x"] > W * 0.4 else W * 1.15
                    flights.append({"thing": thing, "t0": bt, "t1": bt + FLIGHT, "from": (start_x, H * 0.12),
                                    "to": aim, "arc": -H * 0.08, "h": H * 0.15})
                    effects.append((bt, "launch" if "missile" in thing.lower() or "rocket" in thing.lower() else "whoosh"))
                    booms.append({"t": bt + FLIGHT, "x": aim[0], "y": aim[1], "size": max(tgt["h"] * 0.9, H * 0.3)})
                    effects.append((bt + FLIGHT, b.get("sfx", "big_explosion")))
                    if tgt["type"] == "thing":
                        tgt["states"].append((bt + FLIGHT + 0.05, "burning"))
                    else:
                        tgt["faces"].append((bt + FLIGHT + 0.05, "shocked"))
                        tgt["actions"].append((bt + FLIGHT, "tremble", 1.2))
                shot = shot or "wide"
            elif b.get("sfx"):
                if b["sfx"] in STINGS and "say" in b:
                    effects.append((end - GAP + 0.1, b["sfx"]))
                    end += STINGS[b["sfx"]]
                else:
                    effects.append((bt, b["sfx"]))
            if act == "zoom" and it is not None:
                shots.append((bt, "zoom", who))
            elif shot and (shot == "wide" or who in by or b.get("target") in by):
                focus = who if who in by else b.get("target") if b.get("target") in by else None
                shots.append((bt, shot if focus or shot == "wide" else "wide", focus))
            elif not shot and act and it is not None:
                shots.append((bt, "wide" if act.startswith(("enter_", "exit_", "fall")) else "medium",
                              None if act.startswith(("enter_", "exit_")) else who))
            t = end
        t += 0.3
        if sc.get("end"):
            shots = [(t0, "wide", None)]
        path = (bg_paths or {}).get(si)
        plan["scenes"].append({"t0": t0, "t1": t, "kind": kind, "label": sc.get("label", ""), "bg_path": path,
                               "seed": seed + si, "items": items, "shots": shots, "flights": flights,
                               "booms": booms, "ground": top, "end": bool(sc.get("end"))})
        if sc.get("end"):
            plan["end"] = (t0, t)
    plan["total"] = t
    return plan, dialogue, effects
