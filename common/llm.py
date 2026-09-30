"""Vision-LLM calls shared by all flows: OpenRouter (free cloud models) or a local Ollama model.

Configured in the repo's .env (real environment variables win):
  LLM_PROVIDER        openrouter (default when OPENROUTER_API_KEY is set) or ollama
  OPENROUTER_API_KEY  https://openrouter.ai/keys
  OPENROUTER_MODEL    a vision-capable model, default qwen/qwen3.8-27b:free
  OPENROUTER_FALLBACKS up to 3 more models (comma separated), tried in order when one fails
  LLM_LOCAL_FALLBACK  1 (default): use the local Ollama model when all OpenRouter models fail
  OLLAMA_MODEL        local vision model, default qwen3-vl:4b
  OLLAMA_TEXT_MODEL   local model for text-only prompts (text_json), default OLLAMA_MODEL
"""
import base64
import json
import os
import re
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ENV_FILE = REPO / ".env"
OLLAMA_URL = "http://127.0.0.1:11434"
OPENROUTER_URL = "https://openrouter.ai/api/v1/chat/completions"
DEFAULT_OPENROUTER_MODEL = "qwen/qwen3.8-27b:free"
DEFAULT_OLLAMA_MODEL = "qwen3-vl:4b"
DEFAULT_FALLBACKS = "google/gemma-4-31b-it:free,dots-studio/dots-3-note-preview:free,nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free"
MAX_FALLBACKS = 3


def load_env(path=ENV_FILE):
    """KEY=value lines from .env (# comments, optional quotes); real environment variables win."""
    env = {}
    if path.exists():
        for line in path.read_text(encoding="utf-8-sig").splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1)
                env[k.strip()] = v.strip().strip("'\"")
    env.update({k: v for k, v in os.environ.items() if k in env or k.startswith(("TELEGRAM_", "OPENROUTER_", "LLM_", "OLLAMA_"))})
    return env


_ENV = load_env()


def provider():
    p = _ENV.get("LLM_PROVIDER", "").strip().lower()
    if p in ("openrouter", "ollama"):
        return p
    return "openrouter" if _ENV.get("OPENROUTER_API_KEY") else "ollama"


def model():
    if provider() == "openrouter":
        return _ENV.get("OPENROUTER_MODEL") or DEFAULT_OPENROUTER_MODEL
    return ollama_model()


def ollama_model():
    return _ENV.get("OLLAMA_MODEL") or DEFAULT_OLLAMA_MODEL


def openrouter_models():
    """The configured model, then up to MAX_FALLBACKS fallbacks (no duplicates)."""
    fallbacks = _ENV.get("OPENROUTER_FALLBACKS")
    fallbacks = DEFAULT_FALLBACKS if fallbacks is None else fallbacks
    names = [model()] + [m.strip() for m in fallbacks.split(",") if m.strip()][:MAX_FALLBACKS]
    return list(dict.fromkeys(names))


def local_fallback():
    return _ENV.get("LLM_LOCAL_FALLBACK", "1").strip() not in ("0", "false", "no", "")


def _ollama_up():
    try:
        urllib.request.urlopen(OLLAMA_URL, timeout=3)
        return True
    except Exception:
        return False


def label():
    return f"{model()} via {provider()}"


def is_local():
    return provider() == "ollama"


def available():
    if provider() == "openrouter" and _ENV.get("OPENROUTER_API_KEY"):
        return True
    if provider() == "openrouter" and not local_fallback():
        return False
    return _ollama_up()


def unload():
    """Free the local model's VRAM (no-op for OpenRouter)."""
    if is_local():
        try:
            _post(f"{OLLAMA_URL}/api/generate", {"model": ollama_model(), "keep_alive": 0}, timeout=30)
        except Exception:
            pass


def _post(url, payload, headers=None, timeout=600):
    req = urllib.request.Request(url, data=json.dumps(payload).encode(),
                                 headers={"Content-Type": "application/json", **(headers or {})})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def _parse_json(text):
    text = re.sub(r"<think>.*?</think>", "", text or "", flags=re.S).strip()
    text = re.sub(r"^```(?:json)?\s*|\s*```$", "", text).strip()
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        m = re.search(r"\{.*\}", text, re.S)
        if not m:
            raise ValueError(f"model reply is not JSON: {text[:200]!r}")
        return json.loads(m.group(0))


def vision_json(prompt, jpeg, temperature=0.2, keep_alive=0, ollama_options=None):
    """Ask the vision model about one JPEG image, return its JSON reply as a dict."""
    return _ask(prompt, base64.b64encode(jpeg).decode(), temperature, keep_alive, ollama_options)


def text_json(prompt, temperature=0.8, keep_alive=0, ollama_options=None):
    """Text-only prompt (no image), return the JSON reply as a dict. Locally OLLAMA_TEXT_MODEL
    (e.g. qwen3:8b) is used when set, else the vision model."""
    return _ask(prompt, None, temperature, keep_alive, ollama_options)


def _ask(prompt, b64, temperature, keep_alive, ollama_options):
    if provider() == "openrouter":
        errors = []
        if _ENV.get("OPENROUTER_API_KEY"):
            for name in openrouter_models():
                try:
                    return _openrouter(name, prompt, b64, temperature)
                except Exception as e:  # rate limit, model gone, bad JSON ... -> next model
                    errors.append(f"{name}: {e}")
                    print(f"[llm  ] {name} failed ({str(e)[:120]}), trying the next model ...")
        if local_fallback() and _ollama_up():
            print(f"[llm  ] OpenRouter failed, using the local model {ollama_model()}")
            return _ollama(prompt, b64, temperature, keep_alive, ollama_options)
        raise RuntimeError("all LLM models failed: " + "; ".join(errors or ["OPENROUTER_API_KEY is not set"]))
    return _ollama(prompt, b64, temperature, keep_alive, ollama_options)


# optional callable run right before every local Ollama call, also the fallback after OpenRouter failed
# (make_stickers.py frees ComfyUI's VRAM with it - on 6 GB the vision model otherwise times out)
before_local = None


def _ollama(prompt, b64, temperature, keep_alive, ollama_options):
    if before_local:
        before_local()
    msg = {"role": "user", "content": prompt}
    if b64:
        msg["images"] = [b64]
    payload = {
        "model": ollama_model() if b64 else (_ENV.get("OLLAMA_TEXT_MODEL") or ollama_model()),
        "messages": [msg],
        "format": "json",
        "stream": False,
        "think": False,
        "keep_alive": keep_alive,
        "options": {"temperature": temperature, **(ollama_options or {})},
    }
    try:
        res = _post(f"{OLLAMA_URL}/api/chat", payload)
    except urllib.error.HTTPError:
        payload.pop("think")  # model without thinking support
        res = _post(f"{OLLAMA_URL}/api/chat", payload)
    return _parse_json(res["message"]["content"])


def _openrouter(name, prompt, b64, temperature):
    key = _ENV.get("OPENROUTER_API_KEY")
    content = [{"type": "text", "text": prompt}]
    if b64:
        content.append({"type": "image_url", "image_url": {"url": f"data:image/jpeg;base64,{b64}"}})
    payload = {
        "model": name,
        "messages": [{"role": "user", "content": content}],
        "temperature": temperature,
        "response_format": {"type": "json_object"},
    }
    headers = {"Authorization": f"Bearer {key}", "X-Title": "local-llm"}
    for attempt in range(3):  # a few quick retries per model, then the next fallback
        try:
            res = _post(OPENROUTER_URL, payload, headers, timeout=300)
        except urllib.error.HTTPError as e:
            body = e.read().decode(errors="replace")[:300]
            if e.code == 400 and "response_format" in payload:
                payload.pop("response_format")  # model without JSON mode, the prompt asks for JSON anyway
                continue
            if e.code in (429, 502, 503) and attempt < 2:
                wait = 3 * 2 ** attempt  # free models are rate limited
                print(f"[llm  ] OpenRouter {e.code}, retrying in {wait} s ...")
                time.sleep(wait)
                continue
            raise RuntimeError(f"OpenRouter {e.code}: {body}") from None
        if "error" in res:
            raise RuntimeError(f"OpenRouter: {res['error']}")
        return _parse_json(res["choices"][0]["message"].get("content"))
    raise RuntimeError("OpenRouter: no answer after retries")
