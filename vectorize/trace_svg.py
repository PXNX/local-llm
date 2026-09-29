"""Trace a PNG/JPG raster image into an SVG vector - color regions with curve-fitted
outlines, similar to vectorizer.ai / Vector Magic. Runs fully locally via vtracer
(no ComfyUI/Ollama needed).

Usage (via 4-vectorize.bat, which uses ComfyUI's embedded Python):
  4-vectorize.bat photo.jpg [--style photo|logo|sketch] [--mode color|bw] [--out out.svg]
"""
import argparse
import sys
from pathlib import Path

import vtracer

# style -> vtracer curve-fitting knobs
PRESETS = {
    "photo":  dict(mode="spline",  filter_speckle=4, color_precision=6, corner_threshold=60, layer_difference=16),
    "logo":   dict(mode="polygon", filter_speckle=8, color_precision=8, corner_threshold=30, layer_difference=8),
    "sketch": dict(mode="spline",  filter_speckle=1, color_precision=8, corner_threshold=20, layer_difference=4),
}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("--style", choices=PRESETS, default="photo",
                     help="photo = smooth curves for photos (default), logo = crisp flat-color shapes, sketch = fine detail/lines")
    ap.add_argument("--mode", choices=["color", "bw"], default="color",
                     help="color = full color (default), bw = black & white silhouette")
    ap.add_argument("--out", type=Path, default=None, help="output .svg (default: alongside the input)")
    args = ap.parse_args()

    if not args.image.exists():
        sys.exit(f"Not found: {args.image}")
    out = args.out or args.image.with_suffix(".svg")

    print(f"[trace] {args.image.name} -> {out.name}  (style={args.style}, mode={args.mode})")
    vtracer.convert_image_to_svg_py(
        str(args.image), str(out),
        colormode="binary" if args.mode == "bw" else "color",
        hierarchical="stacked",
        path_precision=8,
        **PRESETS[args.style],
    )
    print(f"[done ] {out}  ({out.stat().st_size // 1024} KB)")


if __name__ == "__main__":
    sys.exit(main())
