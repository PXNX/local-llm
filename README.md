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
| `5-soundfx.bat` | Generates game sound effects: mine/Shahed explosions, a Patriot rocket-motor launch sound, and a processed "Slava Ukraini" voice line; `--speech clip.mp3` strips a recording down to speech only. Fully local (numpy/scipy synthesis, Demucs for speech), no Ollama/ComfyUI needed. Result: `soundfx\out\` |
| `6-characters.bat` | YouTube channel/playlist/video -> screenshots of the recurring characters in distinct poses/expressions, one folder per character. Double-click and enter a URL (default `@freeonis`). Fully local (yt-dlp + OWLv2 + CLIP). Result: `characters\out\<name>\` |
| `7-caricatures.bat` | Famous people (`--who "Emmanuel Macron"`) or yourself (drag & drop a photo) as original flat 2D political-cartoon caricatures in 6 expressions, optionally transparent + SVG. Starts ComfyUI (+ Ollama for photos). Result: `caricatures\out\<name>\` |
| `8-objects.bat` | Objects/buildings (`--thing "S-400 air defense system"`, refinery, Kremlin, sea mine, oil tanker ...) in the same flat cartoon look, 4 views/states (side, 3/4, on fire, damaged), optionally transparent + SVG. Result: `caricatures\out\<thing>\` |
| `9-top5-videos.bat` | Today's videos from Telegram channels (`topvideos\channels.txt`, e.g. `1482614635`) -> one countdown video "Top 5 Funniest/Cutest Videos of Today" from #5 to #1. Needs a Telegram API key (`.env`) and Ollama (`qwen3-vl:4b` rates the clips). Result: `topvideos\out\top5_<topic>_<date>.mp4` + a credits `.txt` |
| `10-image-to-video.bat` | Image -> short animated MP4 clip (the image is the first frame). Drag & drop an image onto it or double-click and pick one. `qwen3-vl:4b` writes the motion prompt unless you give `--prompt`. Wan 2.2 TI2V 5B in ComfyUI (already downloaded), roughly 10-30 min per clip. Starts Ollama + ComfyUI automatically. Result: `img2video\out\` |

### Sticker options (`2-stickers.bat photo.jpg [options]`)
- `--count 5` number of stickers (each with a different reaction, pose and text style)
- `--lang German` caption language
- `--text "My text"` your own caption (repeatable, one per sticker), skips the LLM for text
- `--no-text` no text on any sticker, just the stylized image acting out the reaction
- `--engine flux1` stylize engine, see below, default `flux1`
- `--strength 0.4` closer to the original (0.3) ... freer cartoon (0.8), default 0.55 (`flux1`/`sdxl`/`photomaker` only)
- `--no-stylize` no ComfyUI, only cut out the original (works without the driver update)

Pipeline: `qwen3-vl:4b` (Ollama) picks a reaction per sticker - a pose/expression for the subject to
act it out (e.g. "waving, big cheerful smile" for a greeting) plus a short text or none at all
("Hi there!", "Nope", "Thank youuuu" ...) - then ComfyUI draws the subject acting out that reaction
(`--engine`, below), then rembg removes the background, then a white outline plus the text (font,
color and placement cycle through a few styles across the batch, see `TEXT_STYLES` in
`stickers/make_stickers.py`), then 512x512 WebP < 100 KB (WhatsApp/Telegram).

Stylize engines (`stickers/sticker_workflow_*_api.json`, can also be dragged into ComfyUI):
| engine | needs | notes |
| --- | --- | --- |
| `flux1` (default) | `flux1-schnell-Q4_K_S.gguf` + `clip_l.safetensors` + `t5-v1_1-xxl-encoder-Q5_K_M.gguf` + `ae.safetensors` (all already downloaded) | FLUX.1 schnell img2img, GGUF-quantized, fits the RTX 2060's 6 GB VRAM - SDXL is broken on this ComfyUI build (see Known issue below), FLUX.1 works |
| `sdxl` | `DreamShaperXL_Turbo_v2_1.safetensors` (already downloaded) | Fast SDXL img2img, but currently broken - flat gray output, see Known issue below |
| `photomaker` | same checkpoint + `models/photomaker/photomaker-v2.bin` (`0-download-all.bat` grabs it) | SDXL + PhotoMaker face-identity conditioning - best resemblance for the VRAM it costs, once SDXL is fixed |
| `qwen-image21` | `models/diffusion_models/qwen-image-2.1-UC-fp8.safetensors` (downloaded) + a Qwen3-VL-8B text encoder + the Qwen-Image 2.1 VAE (not downloaded yet, several more GB) | Native reference-image editing, best quality ceiling, but slow on 6 GB VRAM |
| `flux2-klein` | `models/diffusion_models/flux2-klein-base-9b-fp8.safetensors` + `models/vae/flux2_vae.safetensors` (downloaded) + a Flux.2 Klein Qwen3 text encoder (not downloaded yet) | FLUX.2's distilled edit model, also slow on 6 GB VRAM |

`qwen-image21` and `flux2-klein` are wired up but need the missing text-encoder (and for
`qwen-image21`, VAE) files placed by hand before they'll run - see the `--engine` help in
`stickers/make_stickers.py --help` for exact filenames/paths.

### Vectorize options (`4-vectorize.bat photo.jpg [options]`)
- `--style photo` smooth curves for photos/gradients (default), `logo` crisp flat-color shapes, `sketch` preserves fine detail/lines
- `--mode color` full color (default), `bw` black & white silhouette
- `--out out.svg` output path (default: alongside the input, same name with `.svg`)

Pipeline: `vtracer` (Rust, pip-installed into ComfyUI's embedded Python) traces color regions and
fits curves/polygons to their outlines directly, no LLM or ComfyUI server involved.

### Character options (`6-characters.bat <url or video files> [options]`)
- `--max-videos 20` latest N videos per channel/playlist (default 10), `--max-duration 600` skips longer videos (compilations)
- `--interval 0.5` seconds between sampled frames (default 1), `--min-size 0.15` ignore small background characters
- `--dedupe 0.95` keep more similar poses (default 0.93), `--full-frame` save the whole frame instead of the crop
- `--only-known` only fill existing folders, `--reprocess` redo videos that were already processed

Pipeline: yt-dlp downloads the video stream (<=720p, no ffmpeg needed) into `characters\videos\`, frames
are sampled (near-identical frames skipped), OWLv2 detects characters, CLIP embeds the crops, crops
are clustered into characters and near-duplicate poses are dropped.

Naming: `characters\names.txt` lists the wanted characters (Zelensky, Trump, Macron, Xi Jinping, Putin,
Kim Jong Un, Merz, Khamenei); a folder is created for each. Only very recognizable caricatures are
auto-named (Trump is; the others usually are not). Everything else lands in `character_NN` folders:
**after a run, move/rename those into the named folders** (and delete misfits). In one uniform cartoon
style different characters look very similar to CLIP, so later runs only add a new cluster to an
existing folder when it is nearly identical (`--match 0.9`); otherwise you get a new `character_NN` to
merge by hand. Lowering `--match` merges more but files wrong characters (e.g. a blonde woman under Putin).

### Caricature / object options (`7-caricatures.bat` / `8-objects.bat`)
- `--who "Name"` / `--thing "object"` (repeatable), `--photo me.jpg --name Felix` caricature from a photo
  (qwen3-vl describes the look, SDXL img2img), `--features "grey hair, glasses"` extra look hints
- `--variant "..."` own expressions/poses or views/states instead of the built-in list, `--count 3` fewer images
- `--strength 0.75` photo img2img: 0.5 close to the photo ... 0.9 free cartoon
- `--cutout` transparent PNG (rembg), `--vectorize` SVG via vtracer ("logo" preset, of the cutout if given)

The look is described in the prompt (flat 2D satire cartoon, big round head, thick outlines, pastel
colors, `caricatures/caricature_workflow_api.json`, can be dragged into ComfyUI); it draws new caricatures
and does not copy any channel's character designs.

**Known issue:** SDXL returns flat gray images in this ComfyUI install - the UNet's noise prediction
stays near zero at every sampling step, regardless of ComfyUI version, PyTorch version (tested
2.6.0 through 2.14.0), CUDA build, or attention backend; matches unresolved upstream issues
Comfy-Org/ComfyUI#13116 and #15137. FLUX.1/2 and Qwen-Image are unaffected (same GPU, same install),
so `2-stickers.bat` now defaults to the `flux1` engine instead. Flows 7/8 still use SDXL and need
either that upstream bug fixed or switching to a FLUX-based workflow like `stickers` did.

### Image-to-video options (`10-image-to-video.bat photo.jpg [options]`)
- `--prompt "the dog wags its tail, slow zoom in"` what should move and how the camera moves (English works best, repeatable, one per clip); without it `qwen3-vl:4b` looks at the image and writes one
- `--count 3` number of clips, each with its own prompt and seed
- `--seconds 3` clip length at 24 fps (default 3; ~5 s is the practical limit on 6 GB VRAM)
- `--size 832` long side in pixels, the aspect ratio follows the image (default 832 -> 832x480 for 16:9; `640` is faster)
- `--steps 20` sampling steps (12-15 is faster but blurrier), `--cfg 5` prompt strength, `--seed 42`

Pipeline: `qwen3-vl:4b` (Ollama, unloaded right after) describes the scene + one motion and camera move
-> the image is resized to the video size and encoded as the first latent frame (`Wan22ImageToVideoLatent`)
-> Wan 2.2 TI2V 5B generates the remaining frames (uni_pc, 20 steps, CFG 5, shift 8) -> H.264 MP4 at 24 fps,
plus a `.txt` with the prompt next to it. The workflow is `img2video/img2video_workflow_api.json` (can be
dragged into ComfyUI); the 5B fp16 model is bigger than the 6 GB VRAM and gets offloaded to RAM (`--lowvram`).

### Top-5 video options (`9-top5-videos.bat [options]`)
- `--topic funny` / `cute` / `auto` (default: whichever theme has the stronger top 5 today), `--subject animals` only clips about that
- `--hours 48` look back N hours instead of "since midnight" (with fewer than 5 videos today it widens to 24 h by itself)
- `--channel @name` (repeatable) instead of `channels.txt`, `--max-candidates 25` clips downloaded + rated
- `--clip-seconds 30` max length per clip, `--format landscape` 1280x720 instead of vertical 720x1280, `--lang German` titles
- `--headline "My text"` own intro text, `--no-title-bar` only the rank badge, `--nvenc` GPU encoding (new driver needed)

Setup once: create an app at https://my.telegram.org (API development tools) and put `TELEGRAM_API_ID`/`TELEGRAM_API_HASH`
(plus `TELEGRAM_PASSWORD` if two-step verification is on) into `.env` in the repo root - not in git, the .bat
creates it from `.env.example` and opens Notepad. The first
run shows a QR code: scan it in the Telegram app under Settings > Devices > Link Desktop Device
(`--login code` for phone number + login code instead; that code arrives as a message in the "Telegram"
chat of your app, rarely by SMS, and the script says where it was sent); the session is stored in `topvideos\telegram.session`
(treat it like a password, it is in `.gitignore`). Numeric channel IDs only resolve for channels your account has joined.

Pipeline: Telethon reads today's posts, keeps videos of 3-180 s and drops reposts (same file or same
duration+size) -> pre-ranks by engagement (views, reactions, forwards relative to the channel's median views)
and downloads the top candidates to `topvideos\downloads\` -> `qwen3-vl:4b` sees a 2x2 grid of frames + the post
caption and scores funny/cute 0-10, flags non-entertainment (news, ads, text slides) and writes a short title
(cached in `topvideos\scores.json`) -> the 5 best clips of one topic are ranked by AI score + up to 2 points
engagement bonus -> ffmpeg (bundled `imageio-ffmpeg`) renders intro, then per place a title card and the clip
(blurred fill background, rank badge, loudness-normalized audio) and joins everything into one MP4.
The clips belong to the channels/creators: check the rights before you publish the result.

### Sound effect options (`5-soundfx.bat [options]`)
- `--count 5` variations per explosion/rocket-motor sound (default 3), each with a different seed
- `--seed 42` base random seed (same seed + count reproduces the same set)
- `--out path\to\dir` output folder (default: `soundfx\out`)
- `--voice path\to\clip.wav` source recording for the "Slava Ukraini" voice line (16-bit PCM wav; default looks for `wav.wav` in Downloads)
- `--speech path\to\clip.mp3` removes background noise/music from a clip (mp3/wav/m4a/...) and keeps only the speech, written as `<name>_speech.wav`; repeatable. With `--speech`, only those clips are processed (no explosions etc.)
- `--denoise 1.5` strength of the noise gate after vocal separation (0 = separation only, default 1; higher is more aggressive but sounds more "underwater")

Pipeline: `mine_explosion` and `shahed_impact` are layered noise/tone synthesis (crack transient +
pitched sub-bass thump + filtered rumble tail, numpy/scipy) meant to trigger on mine detonations
and Shahed/loitering-munition impacts; `patriot_launch` is an ignition transient plus a
flutter-modulated filtered-noise motor burn, for a Patriot interceptor launch. `slava_ukraini.wav`
is not synthesized - it's silence-trimmed and normalized from a real recorded clip you supply, meant
to play when a Flamingo cruise missile hits a Russian refinery. `--speech` runs the clip through
HDemucs vocal separation (torchaudio, ~320 MB model downloaded once to the torch cache, GPU if
available), then an 80 Hz high-pass against rumble/hum and a spectral noise gate whose noise
profile is estimated from the quietest frames of each frequency band, then silence-trim + normalize.
Apart from Demucs, no AI model or ComfyUI/Ollama server involved; add a generative audio model (e.g. Stable Audio Open in ComfyUI) later if more
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
