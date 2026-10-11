"""Console status lines for the GUI's log view: info stays the plain "[tag] message" convention;
warn/error add a level word right after the tag so the GUI colors them without guessing from wording.
"""
import sys


def info(tag: str, msg: str) -> None:
    print(f"[{tag}] {msg}", flush=True)


def warn(tag: str, msg: str) -> None:
    print(f"[{tag}] WARNING: {msg}", flush=True)


def error(tag: str, msg: str) -> None:
    print(f"[{tag}] ERROR: {msg}", file=sys.stderr, flush=True)
