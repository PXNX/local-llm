"""Vision-LLM calls shared by all flows: OpenRouter (free cloud models), OpenCode Zen (free cloud
models via opencode-wrap, no key), the llama.cpp router the GUI starts (Coding page) or a local
Ollama model.

Configured in the repo's .env (real environment variables win):
  LLM_PROVIDER        openrouter (default when OPENROUTER_API_KEY is set), opencode, llamacpp or ollama
  OPENROUTER_API_KEY  https://openrouter.ai/keys
  OPENROUTER_MODEL    a vision-capable model, default qwen/qwen3.8-27b:free
  OPENROUTER_FALLBACKS up to 3 more models (comma separated), tried in order when one fails
  LLM_LOCAL_FALLBACK  1 (default): use the local Ollama model when all OpenRouter models fail
  OPENCODE_MODEL      Zen model for LLM_PROVIDER=opencode, default opencode/deepseek-v4.1-flash-free
  OPENCODE_WRAP_URL   opencode-wrap (opencode-wrap/server.ts, started with bun when down), default http://127.0.0.1:8000/v1
  LLM_LOCAL_FALLBACK  also applies to opencode and llamacpp: local Ollama model when that model fails
  LLAMA_CPP_URL       llama.cpp router base URL, default http://127.0.0.1:8080 (started by the GUI)
  LLAMA_CPP_MODEL     model the router loads, default nvidia/NVIDIA-Nemotron-3.5-Bolt-E4B-4GB:Q4_K_M
  OLLAMA_MODEL        local vision model, default qwen3-vl:4b
  LLM_LOCAL_PROFILE   speed (default), quality or long (200k context) - the GUI's toggle: which local model answers text-only
                      prompts when OLLAMA_TEXT_MODEL isn't set (LLM_CODING_SPEED_MODEL, default qwen3:8b, or
                      LLM_CODING_QUALITY_MODEL, default qwen3.8-blend:27b - much better, ~5x slower here)
  OLLAMA_TEXT_MODEL   local model for text-only prompts (text_json), overrides the profile - qwen3-vl:4b
                      struggles with long structured prompts (e.g. the comedy sketch writer): it can
                      burn its entire num_predict budget "thinking" and return empty content, never
                      reaching the answer, however high num_predict is set
"""
import base64
import json
import os
import re
import shutil
import subprocess
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
DEFAULT_OPENCODE_MODEL = "opencode/deepseek-v4.1-flash-free"
DEFAULT_LLAMACPP_URL = "http://127.0.0.1:8080"
DEFAULT_LLAMACPP_MODEL = "nvidia/NVIDIA-Nemotron-3.5-Bolt-E4B-4GB:Q4_K_M"
DEFAULT_WRAP_URL = "http://127.0.0.1:8000/v1"
DEFAULT_OLLAMA_TEXT_MODEL = "qwen3:8b"
DEFAULT_QUALITY_TEXT_MODEL = "qwen3.8-blend:27b"
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
    env.update({k: v for k, v in os.environ.items() if k in env or k.startswith(("TELEGRAM_", "OPENROUTER_", "OPENCODE_", "LLM_", "OLLAMA_", "LLAMA_CPP_"))})
    return env


_ENV = load_env()


def provider():
    p = _ENV.get("LLM_PROVIDER", "").strip().lower()
    if p in ("llamacpp", "llama-cpp", "llama.cpp"):
        return "llamacpp"
    if p in ("openrouter", "opencode", "ollama"):
        return p
    return "openrouter" if _ENV.get("OPENROUTER_API_KEY") else "ollama"


def provider_name():
    return {"opencode": "OpenCode Zen", "llamacpp": "llama.cpp"}.get(provider(), provider())


def model():
    if provider() == "openrouter":
        return _ENV.get("OPENROUTER_MODEL") or DEFAULT_OPENROUTER_MODEL
    if provider() == "opencode":
        return _ENV.get("OPENCODE_MODEL") or DEFAULT_OPENCODE_MODEL
    if provider() == "llamacpp":
        return _ENV.get("LLAMA_CPP_MODEL") or DEFAULT_LLAMACPP_MODEL
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


def wrap_url():
    return (_ENV.get("OPENCODE_WRAP_URL") or DEFAULT_WRAP_URL).rstrip("/")


def _wrap_up():
    try:
        urllib.request.urlopen(f"{wrap_url()}/health", timeout=3)
        return True
    except Exception:
        return False


def _start_wrap():
    """Start opencode-wrap (bun, it starts `opencode serve` itself) unless it is up; False if that fails."""
    if _wrap_up():
        return True
    bun = shutil.which("bun") or str(Path.home() / ".bun" / "bin" / "bun.exe")
    if not Path(bun).exists() and not shutil.which("bun"):
        print("[llm  ] bun is missing (setup.bat installs it), can't start opencode-wrap")
        return False
    print("[llm  ] starting opencode-wrap (OpenCode Zen) ...")
    port = wrap_url().rsplit(":", 1)[-1].split("/")[0]
    subprocess.Popen([bun, str(REPO / "opencode-wrap" / "server.ts")], cwd=REPO,
                     env={**os.environ, **({"WRAP_PORT": port} if port.isdigit() else {})},
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL,
                     creationflags=getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0) | getattr(subprocess, "CREATE_NO_WINDOW", 0))
    for _ in range(90):  # opencode serve loads its model catalog first
        time.sleep(1)
        if _wrap_up():
            return True
    print("[llm  ] opencode-wrap did not start (run `bun opencode-wrap/server.ts` to see why)")
    return False


def label():
    return f"{model()} via {provider_name()}"


def ollama_text_model():
    """OLLAMA_TEXT_MODEL, else the model of the speed/quality profile."""
    if _ENV.get("OLLAMA_TEXT_MODEL"):
        return _ENV["OLLAMA_TEXT_MODEL"]
    # the 200k profile only differs in context size, which text prompts don't need
    if _ENV.get("LLM_LOCAL_PROFILE", "").strip().lower() in ("quality", "long"):
        return _ENV.get("LLM_CODING_QUALITY_MODEL") or DEFAULT_QUALITY_TEXT_MODEL
    return _ENV.get("LLM_CODING_SPEED_MODEL") or DEFAULT_OLLAMA_TEXT_MODEL


def text_label():
    if provider() == "ollama":
        return f"{ollama_text_model()} via ollama"
    return label()


def is_local():
    return provider() == "ollama"


def llamacpp_base():
    """The router's OpenAI-compatible base URL (`/v1` appended unless the setting has it)."""
    url = (_ENV.get("LLAMA_CPP_URL") or DEFAULT_LLAMACPP_URL).strip().rstrip("/")
    return url if url.endswith("/v1") else f"{url}/v1"


def _llamacpp_up():
    try:
        urllib.request.urlopen(f"{llamacpp_base()}/models", timeout=3)
        return True
    except Exception:
        return False


def available():
    if provider() == "openrouter" and _ENV.get("OPENROUTER_API_KEY"):
        return True
    if provider() == "openrouter" and not local_fallback():
        return False
    if provider() == "opencode":
        return _start_wrap() or (local_fallback() and _ollama_up())
    if provider() == "llamacpp":
        return _llamacpp_up() or (local_fallback() and _ollama_up())
    return _ollama_up()


def unload():
    """Free the local model's VRAM (no-op for the cloud providers)."""
    if provider() == "ollama":
        try:
            _post(f"{OLLAMA_URL}/api/generate", {"model": ollama_model(), "keep_alive": 0}, timeout=30)
        except Exception:
            pass
    elif provider() == "llamacpp":
        try:
            _post(f"{llamacpp_base().removesuffix('/v1')}/models/unload", {"model": model()}, timeout=30)
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
    if provider() == "llamacpp":
        try:
            return _llamacpp(prompt, b64, temperature)
        except Exception as e:
            if local_fallback() and _ollama_up():
                print(f"[llm  ] WARNING: {model()} failed ({_short(e)}), using the local model {ollama_model()}")
                return _ollama(prompt, b64, temperature, keep_alive, ollama_options)
            raise RuntimeError(f"llama.cpp ({model()}): {e}") from None
    if provider() == "opencode":
        try:
            if not _start_wrap():
                raise RuntimeError("opencode-wrap is not running")
            return _opencode(prompt, b64, temperature)
        except Exception as e:
            if local_fallback() and _ollama_up():
                print(f"[llm  ] WARNING: {model()} failed ({_short(e)}), using the local model {ollama_model()}")
                return _ollama(prompt, b64, temperature, keep_alive, ollama_options)
            raise RuntimeError(f"OpenCode Zen ({model()}): {e}") from None
    if provider() == "openrouter":
        errors = []
        if _ENV.get("OPENROUTER_API_KEY"):
            for name in openrouter_models():
                try:
                    return _openrouter(name, prompt, b64, temperature)
                except Exception as e:  # rate limit, model gone, bad JSON ... -> next model
                    errors.append(f"{name}: {e}")
                    print(f"[llm  ] WARNING: {name} failed ({_short(e)}), trying the next model ...")
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
        "model": ollama_model() if b64 else ollama_text_model(),
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


def _llamacpp(prompt, b64, temperature):
    """The router's OpenAI-compatible chat endpoint (llama serve). Text-only prompts send a plain
    string, images go as an OpenAI content list - a text-only GGUF rejects them, then Ollama falls back."""
    if b64:
        content = [{"type": "text", "text": prompt},
                   {"type": "image_url", "image_url": {"url": f"data:image/jpeg;base64,{b64}"}}]
    else:
        content = prompt
    payload = {
        "model": model(),
        "messages": [{"role": "user", "content": content}],
        "temperature": temperature,
        "response_format": {"type": "json_object"},
    }
    url = f"{llamacpp_base()}/chat/completions"
    try:
        res = _post(url, payload, timeout=600)
    except urllib.error.HTTPError as e:
        if e.code != 400:
            raise RuntimeError(f"llama.cpp {e.code}: {_http_error(e)}") from None
        payload.pop("response_format")  # the router may not implement JSON mode, the prompt asks for JSON
        try:
            res = _post(url, payload, timeout=600)
        except urllib.error.HTTPError as again:
            raise RuntimeError(f"llama.cpp {again.code}: {_http_error(again)}") from None
    if "error" in res:
        raise RuntimeError(f"llama.cpp: {_error_text(res['error'])}")
    return _parse_json(res["choices"][0]["message"].get("content"))


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
            body = _http_error(e)
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
            raise RuntimeError(f"OpenRouter: {_error_text(res['error'])}")
        return _parse_json(res["choices"][0]["message"].get("content"))
    raise RuntimeError("OpenRouter: no answer after retries")


def _error_text(err):
    """The readable part of an OpenAI-style error object: the provider's own message (OpenRouter puts
    it, often as JSON text, into metadata.raw) rather than a generic "Provider returned error"."""
    if not isinstance(err, dict):
        return str(err)
    msg = str(err.get("message") or "").strip()
    meta = err.get("metadata") or {}
    raw = meta.get("raw")
    if isinstance(raw, str):
        try:
            inner = json.loads(raw)
            if isinstance(inner, dict):
                inner = inner.get("error", inner)
                raw = inner.get("message") or inner.get("msg") or raw if isinstance(inner, dict) else raw
        except ValueError:
            pass
        raw = " ".join(str(raw).split())
        if raw and raw not in msg:
            msg = raw if msg in ("", "Provider returned error") else f"{msg}: {raw}"
    provider = meta.get("provider_name")
    return f"{provider}: {msg}" if provider and msg else msg or str(err)


def _http_error(e):
    """An HTTPError's body as a readable message (JSON error bodies are unpacked)."""
    body = e.read().decode(errors="replace")
    try:
        data = json.loads(body)
        if isinstance(data, dict) and "error" in data:
            return _error_text(data["error"])
    except ValueError:
        pass
    return " ".join(body.split())[:300]


def _short(e, limit=220):
    text = str(e)
    return text if len(text) <= limit else text[:limit - 3] + "..."


def _opencode(prompt, b64, temperature):
    """OpenCode Zen through opencode-wrap (OpenAI-compatible, retries rate limits itself)."""
    content = [{"type": "text", "text": prompt}]
    if b64:
        content.append({"type": "image_url", "image_url": {"url": f"data:image/jpeg;base64,{b64}"}})
    payload = {
        "model": model(),
        "messages": [{"role": "user", "content": content}],
        "temperature": temperature,
        "response_format": {"type": "json_object"},
    }
    try:
        res = _post(f"{wrap_url()}/chat/completions", payload, timeout=600)
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"opencode-wrap {e.code}: {_http_error(e)}") from None
    return _parse_json(res["choices"][0]["message"].get("content"))
