@echo off
rem Setup after cloning: installs Ollama + OpenCode, ComfyUI portable (+ GGUF node, rembg)
rem and the OpenCode config. Safe to re-run. Models are downloaded afterwards by 0-download-all.bat.
setlocal
cd /d "%~dp0"
call "%~dp0common\models-env.bat"
set "COMFY_VER=v0.37.0"
set "PY=%~dp0ComfyUI_windows_portable\python_embeded\python.exe"

echo === Ollama + Bun + Deno (winget)
winget list --id Ollama.Ollama -e >nul 2>&1 || winget install --id Ollama.Ollama -e --silent --accept-package-agreements --accept-source-agreements
rem Bun: installs and self-updates OpenCode (npm package opencode-ai), winget's own SST.opencode package lags behind upstream
winget list --id Oven-sh.Bun -e >nul 2>&1 || winget install --id Oven-sh.Bun -e --silent --accept-package-agreements --accept-source-agreements
rem Deno: JavaScript runtime yt-dlp needs for YouTube (6-characters.bat)
winget list --id DenoLand.Deno -e >nul 2>&1 || winget install --id DenoLand.Deno -e --silent --accept-package-agreements --accept-source-agreements

echo === OpenCode (bun global install, self-updating)
"%USERPROFILE%\.bun\bin\bun.exe" install -g opencode-ai

echo === OpenCode 2 for opencode-wrap (LLM_PROVIDER=opencode, built from source with bun, a few minutes once)
call "%~dp0opencode-wrap\install-opencode2.bat"

rem Large context with little VRAM (applies after Ollama restarts)
setx OLLAMA_CONTEXT_LENGTH 32768 >nul
setx OLLAMA_FLASH_ATTENTION 1 >nul
setx OLLAMA_KV_CACHE_TYPE q8_0 >nul
rem Ollama's models live in the one models folder (MODELS_DIR in .env, default models\), also for the tray app
setx OLLAMA_MODELS "%OLLAMA_MODELS%" >nul
echo Models folder: %MODELS_DIR%

echo === OpenCode config
if not exist "%USERPROFILE%\.config\opencode\opencode.json" (
  mkdir "%USERPROFILE%\.config\opencode" 2>nul
  copy "%~dp0config\opencode.json" "%USERPROFILE%\.config\opencode\opencode.json" >nul
  echo Installed %USERPROFILE%\.config\opencode\opencode.json
) else (
  echo Exists already, not overwritten. Template: config\opencode.json
)

echo === ComfyUI portable %COMFY_VER% (~2 GB)
if not exist "%PY%" (
  curl.exe -L --fail -C - --retry 5 -o ComfyUI_windows_portable_nvidia.7z https://github.com/Comfy-Org/ComfyUI/releases/download/%COMFY_VER%/ComfyUI_windows_portable_nvidia.7z || ( echo Download failed, re-run setup.bat. & pause & exit /b 1 )
  "%SystemRoot%\System32\tar.exe" -xf ComfyUI_windows_portable_nvidia.7z || ( echo Extract failed. & pause & exit /b 1 )
  del ComfyUI_windows_portable_nvidia.7z
)

echo === ComfyUI-GGUF node + sticker/vectorize/character/top-video dependencies
if not exist "ComfyUI_windows_portable\ComfyUI\custom_nodes\ComfyUI-GGUF" (
  git clone --depth 1 https://github.com/city96/ComfyUI-GGUF ComfyUI_windows_portable\ComfyUI\custom_nodes\ComfyUI-GGUF
)
"%PY%" -s -m pip install -q --no-warn-script-location -r ComfyUI_windows_portable\ComfyUI\custom_nodes\ComfyUI-GGUF\requirements.txt "rembg[cpu]" vtracer yt-dlp scikit-learn telethon qrcode imageio-ffmpeg resvg-py kokoro-onnx

if exist "%~dp0gui\target\release\local-llm.exe" (
  echo === Desktop shortcut local-llm.lnk
  set "LLM_EXE=%~dp0gui\target\release\local-llm.exe"
  set "LLM_DIR=%~dp0."
  powershell -NoProfile -ExecutionPolicy Bypass -Command "$l = Join-Path ([Environment]::GetFolderPath('Desktop')) 'local-llm.lnk'; $s = (New-Object -ComObject WScript.Shell).CreateShortcut($l); $s.TargetPath = $env:LLM_EXE; $s.WorkingDirectory = (Resolve-Path $env:LLM_DIR).Path; $s.IconLocation = $env:LLM_EXE + ',0'; $s.Save(); Write-Output $l"
)

echo.
echo Done. Next steps:
echo   1. Update the NVIDIA driver to ^>= 580 (needed by ComfyUI, see README).
echo   2. Run 0-download-all.bat to download the models.
echo   3. T3 Code: Settings ^> Providers ^> OpenCode, enable it (default model: free opencode/muse-spark-1.3-contributor-free).
pause
