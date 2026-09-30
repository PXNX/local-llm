"""Political comedy cartoon sketches in the style of freeonis: an LLM writes the sketch, the other
flows provide the cast (caricatures from flow 7, objects from flow 8, optionally the screenshots of
flow 6, sounds of flows 5 and 9), Kokoro speaks the lines and everything is animated as a cutout
cartoon: squash & stretch while talking, cuts between wide shots and close-ups, entrances, jumps,
falls, drones hitting refineries, explosions, subtitles, an end card.

Pipeline: 1. write (LLM -> script.json, editable) 2. cast (existing drawings, missing ones drawn
with FLUX.1 in ComfyUI if it runs, else placeholders) 3. voices (Kokoro-82M, local) 4. timeline
5. sound mix 6. render (parallel, H.264) -> MP4 + thumbnail.

Usage (via 12-comedy.bat / 12-comedy-preview.bat, which use ComfyUI's embedded Python):
  12-comedy.bat --topic "Trump wants to buy Greenland" [--cast Trump --cast "Mette Frederiksen"]
  12-comedy-preview.bat --topic "..."           480p 30 fps, quick check of the timing
  12-comedy.bat --script comedy\\out\\<name>\\script.json   render an (edited) script in 1080p 60 fps
Result: comedy/out/<title>/<title>.mp4 (+ _preview.mp4, _thumbnail.jpg, script.json, script.txt)
"""
import argparse
import datetime as dt
import json
import os
import random
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(REPO))
from common import llm  # noqa: E402

import cast as cast_mod  # noqa: E402
import render  # noqa: E402
import scene as scene_mod  # noqa: E402
import sound  # noqa: E402
import timeline  # noqa: E402
import voice  # noqa: E402
import writer  # noqa: E402

QUALITY = {  # (short side, fps, x264 preset, crf)
    "final": (1080, 60, "medium", 18),
    "preview": (480, 30, "veryfast", 26),
}


def known_names():
    names = []
    for folder in (cast_mod.CARICATURES, cast_mod.SCREENSHOTS):
        if folder.is_dir():
            for d in sorted(folder.iterdir()):
                if d.is_dir() and not d.name.startswith(("_", "character_")):
                    names += [s.name for s in d.iterdir() if s.is_dir() and (s / "cutout.png").exists()] or [d.name]
    return sorted(set(n.replace("_", " ") for n in names))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--topic", help="what the sketch is about, e.g. \"Putin explains the refinery fires\"")
    ap.add_argument("--script", type=Path, help="render this script.json (written earlier, maybe edited) instead of writing a new one")
    ap.add_argument("--cast", action="append", default=[], help="character to use (repeatable), e.g. --cast Trump")
    ap.add_argument("--lang", default="English", help="dialogue language (default English; Kokoro: English, Spanish, "
                    "French, Italian, Portuguese, Hindi, Japanese, Chinese - others use the Windows voices)")
    ap.add_argument("--seconds", type=int, default=60, help="rough length of the sketch (default 60)")
    ap.add_argument("--preview", action="store_true", help="quick 480p 30 fps render (default: 1080p 60 fps)")
    ap.add_argument("--format", choices=["landscape", "vertical"], default="landscape",
                    help="landscape 16:9 (default) or vertical 9:16 for Shorts")
    ap.add_argument("--write-only", action="store_true", help="only write script.json/.txt (to edit it), no video")
    ap.add_argument("--no-generate", action="store_true", help="never draw missing characters/things/backgrounds "
                    "with ComfyUI, use placeholders (default: draw them when ComfyUI runs)")
    ap.add_argument("--use-screenshots", action="store_true", help="use flow 6 screenshots (characters/out) for "
                    "characters without caricatures - those are the channel's own designs, not for publishing")
    ap.add_argument("--voice-engine", choices=["auto", "kokoro", "sapi"], default="auto")
    ap.add_argument("--music", choices=["calm", "funny", "none"], default="calm", help="quiet music bed (default calm)")
    ap.add_argument("--no-subtitles", action="store_true")
    ap.add_argument("--no-end-card", action="store_true")
    ap.add_argument("--handle", default="", help="channel name shown top right, e.g. @mychannel")
    ap.add_argument("--workers", type=int, default=max(1, min(6, (os.cpu_count() or 2) // 2)),
                    help="parallel render processes (default: half the CPU threads, max 6)")
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--out", type=Path, default=HERE / "out")
    args = ap.parse_args()
    if not args.topic and not args.script:
        ap.error("give --topic \"...\" or --script path\\to\\script.json")
    started = time.time()

    # ---- 1. write
    if args.script:
        raw = json.loads(args.script.read_text(encoding="utf-8"))
        script = writer.normalize(raw)
        folder = args.script.resolve().parent
    else:
        print(f"[write  ] {llm.text_label()} writes the sketch ...")
        if not llm.available():
            sys.exit("No LLM available: set OPENROUTER_API_KEY in .env or start Ollama.")
        for attempt in range(3):
            try:
                script = writer.normalize(writer.write(args.topic, args.lang, args.seconds, args.cast, known_names(), llm))
                break
            except (ValueError, RuntimeError, KeyError, TypeError) as e:
                print(f"[write  ] unusable script ({str(e)[:150]}), trying again ...")
        else:
            sys.exit("The LLM did not write a usable script, try again or another --topic.")
        llm.unload()
        folder = args.out / f"{cast_mod.mc.slug(script['title'])[:60]}_{dt.date.today():%Y%m%d}"
    script.setdefault("seed", args.seed if args.seed is not None else random.randint(0, 2**31))
    script["lang"] = args.lang if not args.script else raw.get("lang", args.lang)
    folder.mkdir(parents=True, exist_ok=True)
    print(writer.as_text(script))

    # ---- 3a. voices first: they don't need the GPU and fix the cast's voices in script.json
    voices = voice.Voices(script["lang"], args.voice_engine)
    voices.assign(script["cast"])
    (folder / "script.json").write_text(json.dumps(script, indent=2, ensure_ascii=False), encoding="utf-8")
    (folder / "script.txt").write_text(writer.as_text(script), encoding="utf-8")
    print(f"[write  ] {folder / 'script.json'}")
    if args.write_only:
        return 0

    # ---- 2. cast
    comfy = cast_mod.mc.reachable(cast_mod.mc.COMFY_URL)
    generate = comfy and not args.no_generate
    if not generate:
        print("[cast   ] " + ("drawing disabled" if args.no_generate else "ComfyUI is not running") +
              ": missing drawings become placeholders")
    cs = cast_mod.Cast(generate=generate, use_screenshots=args.use_screenshots, seed=script["seed"])
    if args.use_screenshots:
        print("[cast   ] note: flow-6 screenshots are the channel's own character designs - fine for testing, "
              "don't publish them")
    sprites = {}
    for m in script["cast"]:
        faces = {"neutral"} | {b.get("face") for sc in script["scenes"] for b in sc["beats"] if b.get("who") == m["name"]}
        faces |= {f for sc in script["scenes"] for b in sc["beats"] for n, f in (b.get("react") or {}).items() if n == m["name"]}
        faces |= {"shocked"} if any(b.get("target") == m["name"] for sc in script["scenes"] for b in sc["beats"]) else set()
        sprites[m["name"]] = cs.character(m, sorted(f for f in faces if f))
    burned = {b.get("target") for sc in script["scenes"] for b in sc["beats"] if b.get("event") == "hit"} | \
             {b.get("thing") for sc in script["scenes"] for b in sc["beats"] if b.get("event") == "explode"}
    for thing in writer.things(script):
        sprites[thing] = cs.thing(thing, ["normal", "fire"] if thing in burned else ["normal"])
    W_, H_ = (16, 9) if args.format == "landscape" else (9, 16)
    bg_paths = {}
    for si, sc in enumerate(script["scenes"]):
        path = scene_mod.ai_path(sc["background"], W_, H_, script["seed"] + si)
        if path is not None and not path.exists() and generate:
            scene_mod.generate(sc["background"], W_, H_, script["seed"] + si, cast_mod.mc)
        bg_paths[si] = path if path is not None and path.exists() else None
    for name, states in sprites.items():
        missing = [s for s, p in states.items() if p is None]
        if missing:
            print(f"[cast   ] {name}: placeholder for {', '.join(missing)}")

    # ---- 3b + 4. voices and timeline
    short, fps, preset, crf = QUALITY["preview" if args.preview else "final"]
    W, H = (round(short * 16 / 9 / 2) * 2, short) if args.format == "landscape" else (short, round(short * 16 / 9 / 2) * 2)
    print(f"[voice  ] {voices.label()} speaks {sum('say' in b for sc in script['scenes'] for b in sc['beats'])} lines ...")
    plan, dialogue, effects = timeline.build(
        script, {k: {s: (str(p) if p else None) for s, p in v.items()} for k, v in sprites.items()},
        voices.say, W, H, fps, end_card=not args.no_end_card, subtitles=not args.no_subtitles,
        handle=args.handle, seed=script["seed"], bg_paths={k: (str(v) if v else None) for k, v in bg_paths.items()})
    print(f"[plan   ] {plan['total']:.1f} s, {len(plan['scenes'])} scenes, {W}x{H} @ {fps} fps")

    # ---- 5. sound
    tmp = folder / ("_tmp_preview" if args.preview else "_tmp")
    tmp.mkdir(parents=True, exist_ok=True)
    wav = sound.mix(tmp / "audio.wav", plan["total"], dialogue, effects, args.music, script["seed"])

    # ---- 6. render
    name = cast_mod.mc.slug(script["title"])[:60] + ("_preview" if args.preview else "")
    print(f"[render ] {args.workers} workers ...")
    video = render.encode(plan, wav, folder / f"{name}.mp4", args.workers, preset, crf, tmp)
    thumb = render.thumbnail(plan, folder / f"{name}_thumbnail.jpg")
    out = folder / f"{name}.mp4"
    render.top5.mux(video, out, cover=thumb, audio=wav)
    for p in tmp.iterdir():
        p.unlink()
    tmp.rmdir()
    print(f"[done   ] {out}  ({plan['total']:.0f} s video in {(time.time() - started) / 60:.1f} min)")
    if args.preview:
        print(f"[done   ] happy with it? Edit {folder / 'script.json'} if needed, then render it in 1080p 60 fps:\n"
              f"          12-comedy.bat --script \"{folder / 'script.json'}\"")
    return 0


if __name__ == "__main__":
    sys.exit(main())
