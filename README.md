# Local AI: images, video (ComfyUI) and coding LLM (Ollama + OpenCode in T3 Code)

Hardware: RTX 2060 Laptop (6 GB VRAM), 16 GB RAM, i7-10750H.

## Setup (fresh clone)
Requires Windows 10/11, [Git for Windows](https://git-scm.com/download/win) and winget.
```bat
git clone https://github.com/PXNX/local-llm.git
cd local-llm
setup.bat          & rem Ollama, OpenCode, ComfyUI portable + GGUF node + rembg, OpenCode config
0-download-all.bat & rem all models (~100 GB, resumable)
```
Not in git (installed by the scripts): `ComfyUI_windows_portable/` and all models.
The OpenCode config template is `config/opencode.json`.

## Quick start - one .bat per flow
| File | What it does |
|---|---|
| `setup.bat` | One-time install of the tools (see above) |
| `0-download-all.bat` | Downloads/resumes all models (Ollama + ComfyUI). Safe to re-run. |
| `1-image-video-comfyui.bat` | Frees VRAM (stops Ollama models), starts ComfyUI, opens the browser |
| `2-stickers.bat` | Image -> 3 transparent WebP stickers with funny text. Drag & drop an image onto it or double-click and pick one. Starts Ollama + ComfyUI automatically. Result: `stickers\out\` |
| `3-coding-llm-t3code.bat` | Starts Ollama, preloads `qwen3:8b` and opens T3 Code. `3-coding-llm-t3code.bat gpt-oss:20b` for a different model, `... qwen3:8b cli` for OpenCode in the terminal |
| `4-vectorize.bat` | PNG/JPG -> SVG vector trace (like vectorizer.ai / Vector Magic). Drag & drop an image onto it or double-click and pick one. Fully local (`vtracer`), no Ollama/ComfyUI needed. Result: an `.svg` next to the input image |
| `5-soundfx.bat` | Generates game sound effects: mine/Shahed explosions, a Patriot rocket-motor launch sound, and a processed "Slava Ukraini" voice line. Fully local (numpy/scipy synthesis), no Ollama/ComfyUI needed. Result: `soundfx\out\` |

### Sticker options (`2-stickers.bat photo.jpg [options]`)
- `--count 5` number of stickers (each with a different caption and variant)
- `--lang German` caption language
- `--text "My text"` your own caption (repeatable, one per sticker)
- `--strength 0.4` closer to the original (0.3) ... freer cartoon (0.8), default 0.55
- `--no-stylize` no ComfyUI, only cut out the original (works without the driver update)

Pipeline: `qwen3-vl:4b` (Ollama) describes the image and writes captions, then ComfyUI SDXL
img2img (`stickers/sticker_workflow_api.json`, can also be dragged into ComfyUI) makes a cartoon sticker version,
then rembg removes the background, then white outline + Impact text, then 512x512 WebP < 100 KB (WhatsApp/Telegram).

### Vectorize options (`4-vectorize.bat photo.jpg [options]`)
- `--style photo` smooth curves for photos/gradients (default), `logo` crisp flat-color shapes, `sketch` preserves fine detail/lines
- `--mode color` full color (default), `bw` black & white silhouette
- `--out out.svg` output path (default: alongside the input, same name with `.svg`)

Pipeline: `vtracer` (Rust, pip-installed into ComfyUI's embedded Python) traces color regions and
fits curves/polygons to their outlines directly, no LLM or ComfyUI server involved.

### Sound effect options (`5-soundfx.bat [options]`)
- `--count 5` variations per explosion/rocket-motor sound (default 3), each with a different seed
- `--seed 42` base random seed (same seed + count reproduces the same set)
- `--out path\to\dir` output folder (default: `soundfx\out`)
- `--voice path\to\clip.wav` source recording for the "Slava Ukraini" voice line (16-bit PCM wav; default looks for `wav.wav` in Downloads)

Pipeline: `mine_explosion` and `shahed_impact` are layered noise/tone synthesis (crack transient +
pitched sub-bass thump + filtered rumble tail, numpy/scipy) meant to trigger on mine detonations
and Shahed/loitering-munition impacts; `patriot_launch` is an ignition transient plus a
flutter-modulated filtered-noise motor burn, for a Patriot interceptor launch. `slava_ukraini.wav`
is not synthesized - it's silence-trimmed and normalized from a real recorded clip you supply, meant
to play when a Flamingo cruise missile hits a Russian refinery. No AI model or ComfyUI/Ollama
server involved; add a generative audio model (e.g. Stable Audio Open in ComfyUI) later if more
organic variation is needed.

---------------------------------------------------------------------------
## A. Images & video - ComfyUI

### 1. Prerequisite: update the NVIDIA driver (required!)
ComfyUI's PyTorch is built for CUDA 13; driver 517 can't run it
(`torch.cuda.is_available()` = False). Install the latest GeForce driver (>= 580):
https://www.nvidia.com/Download/index.aspx  (GeForce > RTX 20 Series (Notebooks) > RTX 2060)
or via the NVIDIA App. Reboot, then check with `nvidia-smi`.

### 2. Models
`download-models.sh` (Git Bash) downloads everything into
`ComfyUI_windows_portable/ComfyUI/models`. It's safe to re-run: finished files are
skipped and partial files resume. Log: `download-models.log`.

| Tier | Purpose | Files |
|---|---|---|
| small  | Image SDXL Turbo | checkpoints/DreamShaperXL_Turbo_v2_1.safetensors |
| large  | Image FLUX.1 schnell / dev (GGUF Q4) | diffusion_models/flux1-schnell-Q4_K_S.gguf, flux1-dev-Q4_K_S.gguf; text_encoders/t5-v1_1-xxl-encoder-Q5_K_M.gguf, clip_l.safetensors; vae/ae.safetensors |
| small  | Video Wan 2.1 1.3B | diffusion_models/wan2.1_t2v_1.3B_fp16.safetensors; text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors; vae/wan_2.1_vae.safetensors |
| medium | Video/Image-to-Video Wan 2.2 5B | diffusion_models/wan2.2_ti2v_5B_fp16.safetensors; vae/wan2.2_vae.safetensors (+ umt5 above) |
| large  | Video Wan 2.2 14B (GGUF Q3) | diffusion_models/Wan2.2-T2V-A14B-HighNoise-Q3_K_M.gguf + LowNoise-Q3_K_M.gguf (+ umt5, wan_2.1_vae) |

GGUF files are loaded with the **ComfyUI-GGUF** custom node (already installed):
use "Unet Loader (GGUF)" / "CLIPLoader (GGUF)" instead of the normal loaders.

### 3. Start
Double-click `start-comfyui.bat` and open http://127.0.0.1:8188.
Templates: Menu **Workflow > Browse Templates**.

| Model | Template | 6 GB settings | Expectation |
|---|---|---|---|
| DreamShaper XL Turbo | Image Generation | 1024x1024, 6-8 steps, CFG 2, dpmpp_sde/karras | ~15-30 s |
| FLUX schnell Q4 | Flux Schnell (replace loaders with GGUF) | 1024x1024, 4 steps, CFG 1 | ~1-2 min |
| FLUX dev Q4 | Flux Dev (GGUF loaders) | 20 steps, guidance 3.5 | ~5+ min |
| Wan 2.1 1.3B | Wan 2.1 Text to Video | 832x480, 33 frames | several min |
| Wan 2.2 5B | Wan 2.2 5B Video Generation | 1280x704 is too much -> 832x480, 41 frames | 10-30 min |
| Wan 2.2 14B Q3 | Wan 2.2 14B T2V (GGUF loaders, high+low) | 480x480, 33 frames | very slow (30-60+ min), may hit RAM limits |

Large models spill into system RAM / the pagefile. Close other apps and keep the Windows pagefile on "system managed".

---------------------------------------------------------------------------
## B. Local coding LLM - Ollama + OpenCode in T3 Code

- **Ollama** (installed via winget) serves models at http://localhost:11434.
  It uses the RTX 2060 via **Vulkan** and works even with the old driver.
- User environment variables (set): `OLLAMA_CONTEXT_LENGTH=32768`,
  `OLLAMA_FLASH_ATTENTION=1`, `OLLAMA_KV_CACHE_TYPE=q8_0` (large context for agents
  with little VRAM). They apply after Ollama restarts (tray icon > Quit, then start it again).
- Models:
  | Model | Size | Behavior on this machine |
  |---|---|---|
  | qwen3:8b | ~5 GB | fits almost entirely on the GPU, fast (default) |
  | gpt-oss:20b | ~13 GB | GPU + CPU, usable |
  | qwen3-coder:30b | ~19 GB | MoE, mostly CPU/RAM, slow, pushes RAM to the limit |
  | qwen3.8-blend:27b | ~12.6 GB | [JetBrains Qwen3.8/3.6 27B blend](https://huggingface.co/collections/JetBrains/qwen38-36-27b-blend), IQ3_S from `hf.co/JetBrains/Qwen3.8-3.6-27B-blend-GGUF:IQ3_S`. Dense 27B, strongest coder here but slowest (~1-3 tokens/s, mostly CPU). Q4_K_M (16.8 GB) does not fit in 16 GB RAM |
- **OpenCode** (winget `SST.opencode`), config: `%USERPROFILE%\.config\opencode\opencode.json`
  (provider `ollama`, OpenAI-compatible endpoint `/v1`).
- **T3 Code**: OpenCode provider enabled with binaryPath (`providerInstances.opencode`) in
  `%USERPROFILE%\.t3\userdata\settings.json`. In T3 Code, pick the model under the
  OpenCode provider as `ollama/qwen3:8b` (or another one).

Note: ComfyUI and Ollama share the 6 GB of VRAM. Don't run both at the same time
(Ollama unloads models after 5 min idle; `ollama stop <model>` does it immediately).
