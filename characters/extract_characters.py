"""Extract recurring characters from YouTube videos (e.g. a political cartoon channel) as
screenshots, one folder per character, keeping only distinct poses / facial expressions.

Pipeline: yt-dlp downloads the videos (video only, highest available resolution) -> frames are sampled every
--interval seconds (near-identical frames skipped) -> OWLv2 (open-vocabulary detector) finds
characters -> CLIP embeds each crop -> the new crops are clustered (one cluster ~ one
character) -> each cluster goes to the existing folder it looks very similar to, else to a
characters/names.txt folder if CLIP recognizes the person, else to a new character_NN
folder -> near-duplicates of what a folder already holds are dropped.

Naming: the folders from names.txt are created up front. Seed them once by moving/renaming
the character_NN folders of the first run (or by dropping a few screenshots in), later runs
then sort new screenshots into them. Delete wrong images so they don't attract more wrong
matches. out/_unsorted collects characters seen too rarely to form a folder.

Usage (via 6-characters.bat, which uses ComfyUI's embedded Python):
  6-characters.bat https://www.youtube.com/@freeonis/videos [--max-videos 10] [options]
  6-characters.bat video1.mp4 video2.mp4 ...      (local files work too)
"""
import argparse
import io
import re
import sys
from pathlib import Path

import av
import numpy as np
import torch
from PIL import Image
from sklearn.cluster import AgglomerativeClustering
from torchvision.ops import nms
from transformers import CLIPModel, CLIPProcessor, Owlv2ForObjectDetection, Owlv2Processor

HERE = Path(__file__).resolve().parent
DETECTOR = "google/owlv2-base-patch16-ensemble"
EMBEDDER = "openai/clip-vit-large-patch14"
DET_QUERIES = ["a cartoon character", "a cartoon person's face"]
# generic "nobody in the list" prompts for the zero-shot naming
OTHERS = ["a cartoon character", "a cartoon woman", "a cartoon man", "a cartoon soldier", "a drawing of a person"]
IMG_EXT = {".jpg", ".jpeg", ".png", ".webp"}
DEVICE = "cuda" if torch.cuda.is_available() else "cpu"
DTYPE = torch.float16 if DEVICE == "cuda" else torch.float32


# ---------------------------------------------------------------- download
def download(url, max_videos, max_duration, video_dir):
    import yt_dlp

    video_dir.mkdir(parents=True, exist_ok=True)

    def too_long(info, *, incomplete):
        d = info.get("duration")
        if max_duration and d and d > max_duration:
            return f"skip, longer than {max_duration}s (compilation?)"

    opts = {
        # video only (no ffmpeg needed for merging), highest resolution, H.264 preferred so PyAV decodes it everywhere
        "format": "bv*[vcodec^=avc1]/bv*/b",
        "outtmpl": str(video_dir / "%(id)s.%(ext)s"),
        "match_filter": too_long,
        "playlistend": max_videos,
        "ignoreerrors": True,
        "quiet": True,
        "no_warnings": True,
        "noprogress": True,
    }
    with yt_dlp.YoutubeDL(opts) as ydl:
        info = ydl.extract_info(url, download=True) or {}
    entries = info.get("entries", [info])
    ids = {e["id"] for e in entries if e and e.get("id")}
    return sorted(p for p in video_dir.iterdir() if p.stem in ids and p.suffix in {".mp4", ".webm", ".mkv"})


# ---------------------------------------------------------------- frames
def sample_frames(path, interval):
    """Yield (seconds, PIL image) every `interval` s, skipping frames nearly identical to the last kept one."""
    with av.open(str(path)) as c:
        stream = c.streams.video[0]
        stream.thread_type = "AUTO"
        next_t, last_thumb = 0.0, None
        for frame in c.decode(stream):
            t = float(frame.time or 0)
            if t < next_t:
                continue
            next_t = t + interval
            img = frame.to_image()
            thumb = np.asarray(img.convert("L").resize((32, 18)), dtype=np.float32)
            if last_thumb is not None and np.abs(thumb - last_thumb).mean() < 4:
                continue
            last_thumb = thumb
            yield t, img


def crop_box(img, box, pad=0.08):
    W, H = img.size
    x0, y0, x1, y1 = box
    px, py = (x1 - x0) * pad, (y1 - y0) * pad
    return img.crop((max(0, int(x0 - px)), max(0, int(y0 - py)), min(W, int(x1 + px)), min(H, int(y1 + py))))


# ---------------------------------------------------------------- models
class Models:
    def __init__(self):
        print(f"[load ] {DETECTOR} + {EMBEDDER} on {DEVICE}")
        self.det_proc = Owlv2Processor.from_pretrained(DETECTOR)
        self.det = Owlv2ForObjectDetection.from_pretrained(DETECTOR, dtype=DTYPE).to(DEVICE).eval()
        self.clip_proc = CLIPProcessor.from_pretrained(EMBEDDER)
        self.clip = CLIPModel.from_pretrained(EMBEDDER, dtype=DTYPE).to(DEVICE).eval()
        self.dim = self.clip.config.projection_dim

    @torch.no_grad()
    def detect(self, img, threshold):
        inp = self.det_proc(text=[DET_QUERIES], images=img, return_tensors="pt").to(DEVICE)
        inp["pixel_values"] = inp["pixel_values"].to(DTYPE)
        out = self.det(**inp)
        side = max(img.size)  # OWLv2 pads to a square
        r = self.det_proc.post_process_grounded_object_detection(
            out, threshold=threshold, target_sizes=torch.tensor([[side, side]]))[0]
        boxes, scores = r["boxes"].float().cpu(), r["scores"].float().cpu()
        keep = nms(boxes, scores, 0.3)
        return [(boxes[i].tolist(), float(scores[i])) for i in keep]

    @staticmethod
    def _norm(f):
        f = getattr(f, "pooler_output", f)  # newer transformers return a model output
        return torch.nn.functional.normalize(f.float(), dim=-1).cpu().numpy()

    @torch.no_grad()
    def embed(self, imgs, batch=32):
        vecs = [np.zeros((0, self.dim), np.float32)]
        for i in range(0, len(imgs), batch):
            px = self.clip_proc(images=[im.convert("RGB") for im in imgs[i:i + batch]], return_tensors="pt")["pixel_values"]
            vecs.append(self._norm(self.clip.get_image_features(pixel_values=px.to(DEVICE, DTYPE))))
        return np.concatenate(vecs)

    @torch.no_grad()
    def embed_text(self, texts):
        t = self.clip_proc(text=texts, return_tensors="pt", padding=True).to(DEVICE)
        return self._norm(self.clip.get_text_features(**t))


# ---------------------------------------------------------------- character folders
class Library:
    """out/<name>/*.jpg plus a per-folder embedding cache (moves along when a folder is renamed)."""

    CACHE = ".embeddings.npz"

    def __init__(self, out_dir, models):
        self.out, self.models = out_dir, models
        self.folders = {}  # name -> (filenames, embeddings)
        for d in sorted(p for p in out_dir.iterdir() if p.is_dir() and not p.name.startswith(("_", "."))):
            self.folders[d.name] = self._load(d)
        seeded = [f"{n} ({len(f)})" for n, (f, _) in self.folders.items() if f]
        if seeded:
            print("[chars] " + ", ".join(seeded))

    def _load(self, d):
        cache = {}
        if (d / self.CACHE).exists():
            z = np.load(d / self.CACHE)
            cache = dict(zip(z["names"].tolist(), z["vecs"]))
        files = sorted(p.name for p in d.iterdir() if p.suffix.lower() in IMG_EXT)
        missing = [f for f in files if f not in cache]
        if missing:  # images the user dropped in (or a cache from an older run)
            cache.update(zip(missing, self.models.embed([Image.open(d / f) for f in missing])))
        vecs = np.stack([cache[f] for f in files]) if files else np.zeros((0, self.models.dim), np.float32)
        return files, vecs

    def match(self, members, min_sim, k=5):
        """Folder whose look is closest to the cluster, if close enough, else None.

        Compares the cluster's mean embedding with the mean of the folder's k images nearest to it
        (a folder can hold several looks, e.g. front and back view). In one uniform cartoon style,
        different characters easily reach 0.8, so this has to be strict - an unmatched cluster just
        becomes a new character_NN folder, which is quick to merge by hand.
        """
        centroid = np.mean([c["vec"] for c in members], axis=0)
        centroid /= np.linalg.norm(centroid)
        best, best_sim = None, min_sim
        for name, (files, vecs) in self.folders.items():
            if not files:
                continue
            near = vecs[np.argsort(vecs @ centroid)[-k:]].mean(0)
            sim = float(near @ centroid / np.linalg.norm(near))
            if sim >= best_sim:
                best, best_sim = name, sim
        return best

    def new_name(self):
        n = 1
        while (self.out / f"character_{n:02d}").exists():
            n += 1
        return f"character_{n:02d}"

    def add(self, name, crops, dedupe):
        """Save crops (best first) into out/name, skipping near-duplicates of what's already there."""
        d = self.out / name
        d.mkdir(exist_ok=True)
        files, vecs = self.folders.get(name, ([], np.zeros((0, self.models.dim), np.float32)))
        files, added = list(files), 0
        for c in crops:
            if len(vecs) and float((vecs @ c["vec"]).max()) >= dedupe:
                continue
            fn = f"{c['video']}_{c['t']:06.1f}s_{c['i']}.jpg"
            (d / fn).write_bytes(c["jpg"])
            files.append(fn)
            vecs = np.vstack([vecs, c["vec"][None]])
            added += 1
        if not name.startswith("_"):
            self.folders[name] = (files, vecs)
            np.savez(d / self.CACHE, names=np.array(files), vecs=vecs)
        return added


def vectorize_all(out_dir):
    """Cut out (rembg, the screenshots have scene backgrounds) and trace every screenshot without
    an SVG yet: <name>.jpg -> <name>_cutout.png + <name>_cutout.svg. Run at the end so it also
    covers folders that were renamed/merged by hand."""
    sys.path.insert(0, str(HERE.parent / "caricatures"))
    from make_caricatures import cut_out, vectorize

    todo = [p for p in sorted(out_dir.rglob("*.jpg")) if not p.with_name(p.stem + "_cutout.svg").exists()]
    for n, jpg in enumerate(todo, 1):
        png = jpg.with_name(jpg.stem + "_cutout.png")
        cut_out(Image.open(jpg).convert("RGB"), plain_background=False).save(png)
        vectorize(png, png.with_suffix(".svg"))
        print(f"[trace] {n}/{len(todo)} {png.with_suffix('.svg').relative_to(out_dir)}")
    print(f"[trace] {len(todo)} traced to SVG")


def read_names(path):
    names = {}
    if path.exists():
        for line in path.read_text(encoding="utf-8").splitlines():
            if "=" in line and not line.lstrip().startswith("#"):
                folder, real = (s.strip() for s in line.split("=", 1))
                names[folder] = real
    return names


def zero_shot(models, names):
    """Returns f(members) -> folder name if CLIP recognizes the cluster as one of `names`."""
    if not names:
        return lambda members, conf: None
    labels = list(names) + [None] * len(OTHERS)
    T = models.embed_text([f"a cartoon caricature of {v}" for v in names.values()] + OTHERS)

    def name_of(members, conf):
        logits = 100 * np.stack([c["vec"] for c in members]) @ T.T
        p = np.exp(logits - logits.max(1, keepdims=True))
        p = (p / p.sum(1, keepdims=True)).mean(0)
        return labels[p.argmax()] if p.max() >= conf else None

    return name_of


# ---------------------------------------------------------------- main
def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("sources", nargs="+", help="YouTube channel/playlist/video URL(s) and/or local video files")
    ap.add_argument("--max-videos", type=int, default=10, help="latest N videos per channel/playlist (default 10)")
    ap.add_argument("--max-duration", type=int, default=600,
                    help="skip videos longer than this many seconds, e.g. compilations (default 600, 0 = no limit)")
    ap.add_argument("--interval", type=float, default=1.0, help="seconds between sampled frames (default 1.0)")
    ap.add_argument("--min-score", type=float, default=0.2, help="detector confidence (default 0.2)")
    ap.add_argument("--min-size", type=float, default=0.15,
                    help="ignore characters smaller than this fraction of the frame height (default 0.15)")
    ap.add_argument("--cluster", type=float, default=0.2,
                    help="max cosine distance within one character (default 0.2, lower = more, purer folders)")
    ap.add_argument("--match", type=float, default=0.9,
                    help="similarity for a new cluster to join an existing folder (default 0.9, lower = merges more, "
                         "but also wrongly)")
    ap.add_argument("--name-conf", type=float, default=0.8,
                    help="CLIP confidence to auto-name a new character from names.txt (default 0.8, 1 = never)")
    ap.add_argument("--min-count", type=int, default=3,
                    help="a new character needs this many sightings for its own folder, else -> _unsorted (default 3)")
    ap.add_argument("--dedupe", type=float, default=0.93,
                    help="drop a crop this similar to one already in its folder (default 0.93, higher = keep more)")
    ap.add_argument("--only-known", action="store_true",
                    help="only fill existing folders, don't create character_NN / _unsorted")
    ap.add_argument("--full-frame", action="store_true", help="save the whole frame instead of the character crop")
    ap.add_argument("--names", type=Path, default=HERE / "names.txt", help="character list (default characters/names.txt)")
    ap.add_argument("--out", type=Path, default=HERE / "out", help="output folder (default characters/out)")
    ap.add_argument("--reprocess", action="store_true", help="also process videos that were already processed")
    ap.add_argument("--vectorize", action="store_true",
                    help="afterwards cut out and trace all screenshots in the output folder to SVG (rembg + vtracer)")
    args = ap.parse_args()

    videos = []
    for s in args.sources:
        if re.match(r"https?://", s):
            print(f"[yt   ] {s} (latest {args.max_videos})")
            videos += download(s, args.max_videos, args.max_duration, HERE / "videos")
        elif Path(s).is_file():
            videos.append(Path(s))
        else:
            sys.exit(f"Not a URL or file: {s}")

    args.out.mkdir(parents=True, exist_ok=True)
    names = read_names(args.names)
    for folder in names:
        (args.out / folder).mkdir(exist_ok=True)
    done_file = args.out / ".processed.txt"
    done = set(done_file.read_text().split()) if done_file.exists() else set()
    todo = [v for v in dict.fromkeys(videos) if args.reprocess or v.stem not in done]
    if not todo:
        print("[done ] no new videos (use --reprocess to redo)")
        if args.vectorize:
            vectorize_all(args.out)
        return 0

    models = Models()
    lib = Library(args.out, models)
    name_of = zero_shot(models, names)

    crops = []
    for n, path in enumerate(todo, 1):
        found = 0
        for t, img in sample_frames(path, args.interval):
            for i, (box, score) in enumerate(models.detect(img, args.min_score)):
                if box[3] - box[1] < args.min_size * img.height:
                    continue
                crop = crop_box(img, box)
                buf = io.BytesIO()
                (img if args.full_frame else crop).save(buf, "JPEG", quality=92)
                crops.append(dict(video=path.stem, t=t, i=i, score=score, crop=crop, jpg=buf.getvalue()))
                found += 1
        print(f"[scan ] {n}/{len(todo)} {path.name}: {found} character crops")
    if not crops:
        print("[done ] no characters found")
        if args.vectorize:
            vectorize_all(args.out)
        return 0

    print(f"[embed] {len(crops)} crops")
    for c, v in zip(crops, models.embed([c["crop"] for c in crops])):
        c["vec"] = v
        del c["crop"]

    # cluster the new crops, one cluster ~ one character in similar looks
    if len(crops) >= 2:
        labels = AgglomerativeClustering(n_clusters=None, metric="cosine", linkage="average",
                                         distance_threshold=args.cluster).fit_predict(np.stack([c["vec"] for c in crops]))
    else:
        labels = [0]
    clusters = {}
    for c, l in zip(crops, labels):
        clusters.setdefault(l, []).append(c)

    groups = {}
    for members in sorted(clusters.values(), key=len, reverse=True):
        name = lib.match(members, args.match) or name_of(members, args.name_conf)
        if not name and not args.only_known:
            name = lib.new_name() if len(members) >= args.min_count else "_unsorted"
            if name != "_unsorted":
                (args.out / name).mkdir()
        if name:
            groups.setdefault(name, []).extend(members)

    for name, members in sorted(groups.items()):
        added = lib.add(name, sorted(members, key=lambda c: -c["score"]), args.dedupe)
        print(f"[save ] {name}: +{added} (of {len(members)} sightings)")

    with done_file.open("a") as f:
        f.writelines(v.stem + "\n" for v in todo)
    print(f"[done ] {args.out}")
    print("        Move/rename character_NN folders into the names.txt folders, later runs sort into them.")
    if args.vectorize:
        vectorize_all(args.out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
