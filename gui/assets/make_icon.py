"""Renders icon.svg into icon.ico (16-256 px) and icon.png (256 px).

Run with the ComfyUI Python (has Pillow + resvg-py), then commit the outputs:
    ComfyUI_windows_portable\\python_embeded\\python.exe gui\\assets\\make_icon.py
"""

import io
import re
from pathlib import Path

import resvg_py
from PIL import Image

HERE = Path(__file__).parent
SIZES = [256, 128, 64, 48, 32, 24, 16]


def render(svg: str, size: int) -> Image.Image:
    if size <= 24:
        # glow, shadow and dot only blur the tiny sizes
        svg = re.sub(r'<g id="detail">.*?</g>', "", svg, flags=re.S)
    png = resvg_py.svg_to_bytes(svg_string=svg, width=size, height=size)
    return Image.open(io.BytesIO(bytes(png))).convert("RGBA")


def main() -> None:
    svg = (HERE / "icon.svg").read_text(encoding="utf-8")
    images = [render(svg, s) for s in SIZES]
    images[0].save(HERE / "icon.png", optimize=True)
    images[0].save(HERE / "icon.ico", sizes=[(s, s) for s in SIZES], append_images=images[1:])
    print("wrote", HERE / "icon.png", "and", HERE / "icon.ico")


if __name__ == "__main__":
    main()
