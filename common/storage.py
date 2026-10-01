"""One folder for every model and download cache, shared with the GUI and the .bat files (common/models-env.bat).

MODELS_DIR (real env var, else the repo's .env; relative = inside the repo; default <repo>/models) holds
ComfyUI's model folders directly and the other tools' stores in subfolders:
  ollama/       OLLAMA_MODELS (Ollama started by the GUI / .bat files)
  huggingface/  HF_HOME     (transformers: OWLv2 + CLIP in characters/)
  torch/        TORCH_HOME  (torch hub: Demucs in soundfx/)
  u2net/        U2NET_HOME  (rembg cutouts)
  kokoro/       Kokoro TTS voices (comedy/)
`import common` calls route(), so the libraries download there instead of the user profile.
"""
import os
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SUBDIRS = {"OLLAMA_MODELS": "ollama", "HF_HOME": "huggingface", "TORCH_HOME": "torch", "U2NET_HOME": "u2net"}


def _env_file_value(key, path=REPO / ".env"):
    value = ""
    try:
        lines = path.read_text(encoding="utf-8-sig").splitlines()
    except OSError:
        return value
    for line in lines:
        line = line.strip()
        if line and not line.startswith("#") and "=" in line:
            k, v = line.split("=", 1)
            if k.strip() == key:
                value = v.strip().strip("'\"")
    return value


def models_dir():
    value = (os.environ.get("MODELS_DIR") or _env_file_value("MODELS_DIR")).strip()
    return (REPO / value).resolve() if value else REPO / "models"


def path(sub):
    """A subfolder of the models folder, e.g. path("kokoro")."""
    return models_dir() / sub


def route():
    """Points every library that downloads models at the models folder (before they are imported)."""
    root = models_dir()
    os.environ["MODELS_DIR"] = str(root)
    for key, sub in SUBDIRS.items():
        os.environ[key] = str(root / sub)
    return root
