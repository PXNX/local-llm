"""Microsoft Fluent Emoji (MIT) from the Iconify API, rendered with resvg.

The SVGs are downloaded once into topvideos/emoji_cache/ and work offline afterwards.
Names: https://icon-sets.iconify.design/fluent-emoji/
"""
import io
import urllib.request
from functools import lru_cache
from pathlib import Path

from PIL import Image

CACHE = Path(__file__).resolve().parent / "emoji_cache"
API = "https://api.iconify.design/fluent-emoji/{}.svg"


def _svg(name):
    path = CACHE / f"{name}.svg"
    if not path.exists():
        CACHE.mkdir(exist_ok=True)
        req = urllib.request.Request(API.format(name), headers={"User-Agent": "local-llm"})
        with urllib.request.urlopen(req, timeout=20) as r:
            data = r.read()
        if not data.startswith(b"<svg"):
            raise ValueError(f"unknown emoji {name!r}")
        path.write_bytes(data)
    return path.read_text(encoding="utf-8")


@lru_cache(maxsize=64)
def emoji(name, size):
    """RGBA image of the emoji, `size` px square, or None when it can't be fetched/rendered."""
    try:
        import resvg_py

        png = bytes(resvg_py.svg_to_bytes(svg_string=_svg(name), width=size, height=size, skip_system_fonts=True))
        return Image.open(io.BytesIO(png)).convert("RGBA")
    except Exception as e:
        print(f"[emoji] {name}: {e}")
        return None
