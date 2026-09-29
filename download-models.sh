#!/usr/bin/env bash
# Downloads all ComfyUI models into ComfyUI_windows_portable/ComfyUI/models.
# Safe to re-run: finished files are skipped, partial files are resumed.
cd "$(dirname "$0")/ComfyUI_windows_portable/ComfyUI/models" || exit 1

HF=https://huggingface.co
WAN21=$HF/Comfy-Org/Wan_2.1_ComfyUI_repackaged/resolve/main/split_files
WAN22=$HF/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files
WAN14=$HF/QuantStack/Wan2.2-T2V-A14B-GGUF/resolve/main

# target-folder  url
FILES=(
  # --- Images: SDXL (small, fast)
  "checkpoints      $HF/Lykon/dreamshaper-xl-v2-turbo/resolve/main/DreamShaperXL_Turbo_v2_1.safetensors"
  # --- Stickers: PhotoMaker (face-identity conditioning for the sdxl checkpoint above)
  "photomaker       $HF/TencentARC/PhotoMaker-V2/resolve/main/photomaker-v2.bin"
  # --- Images: FLUX (large, GGUF)
  "diffusion_models $HF/city96/FLUX.1-schnell-gguf/resolve/main/flux1-schnell-Q4_K_S.gguf"
  "diffusion_models $HF/city96/FLUX.1-dev-gguf/resolve/main/flux1-dev-Q4_K_S.gguf"
  "text_encoders    $HF/city96/t5-v1_1-xxl-encoder-gguf/resolve/main/t5-v1_1-xxl-encoder-Q5_K_M.gguf"
  "text_encoders    $HF/comfyanonymous/flux_text_encoders/resolve/main/clip_l.safetensors"
  "vae              $HF/Comfy-Org/Lumina_Image_2.0_Repackaged/resolve/main/split_files/vae/ae.safetensors"
  # --- Video: Wan 2.1 1.3B (small)
  "diffusion_models $WAN21/diffusion_models/wan2.1_t2v_1.3B_fp16.safetensors"
  "text_encoders    $WAN21/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors"
  "vae              $WAN21/vae/wan_2.1_vae.safetensors"
  # --- Video: Wan 2.2 5B (medium)
  "diffusion_models $WAN22/diffusion_models/wan2.2_ti2v_5B_fp16.safetensors"
  "vae              $WAN22/vae/wan2.2_vae.safetensors"
  # --- Video: Wan 2.2 14B (large, GGUF, two experts)
  "diffusion_models $WAN14/HighNoise/Wan2.2-T2V-A14B-HighNoise-Q3_K_M.gguf"
  "diffusion_models $WAN14/LowNoise/Wan2.2-T2V-A14B-LowNoise-Q3_K_M.gguf"
  # --- Stickers: --engine qwen-image21 / flux2-klein need more files not listed here (multi-GB
  # text encoders + a VAE each); see the --engine help in stickers/make_stickers.py for what's
  # missing and where it goes. Grab them manually once you know which exact checkpoint you want.
)

failed=0
for entry in "${FILES[@]}"; do
  read -r dir url <<<"$entry"
  name=$(basename "$url")
  mkdir -p "$dir"
  if [ -f "$dir/$name.done" ]; then echo "skip  $name"; continue; fi
  echo "get   $name"
  for attempt in 1 2 3 4 5 6 7 8 9 10; do
    if curl -L --fail -s -C - --retry 5 --retry-all-errors -o "$dir/$name" "$url"; then
      touch "$dir/$name.done"; break
    fi
    echo "      retry $attempt ($name)"; sleep 5
  done
  [ -f "$dir/$name.done" ] || { echo "FAIL  $name"; failed=1; }
done
[ $failed -eq 0 ] && echo ALLDONE || echo "SOME FAILED - re-run the script"
