@echo off
rem Flow 1: images and video in ComfyUI. Starts ComfyUI and opens the browser as soon as it is ready.
title ComfyUI
cd /d "%~dp0"

rem Free the VRAM: unload loaded Ollama models (they share the 6 GB with ComfyUI)
if exist "%LOCALAPPDATA%\Programs\Ollama\ollama.exe" (
  for /f "skip=1 tokens=1" %%M in ('call "%LOCALAPPDATA%\Programs\Ollama\ollama.exe" ps 2^>nul') do "%LOCALAPPDATA%\Programs\Ollama\ollama.exe" stop %%M >nul 2>&1
)

curl.exe -s -o nul http://127.0.0.1:8188/ && (
  echo ComfyUI is already running.
  start "" http://127.0.0.1:8188
  exit /b 0
)

rem Open the browser in the background once the server answers
start "" /min powershell -NoProfile -WindowStyle Hidden -Command "for($i=0;$i -lt 180;$i++){try{Invoke-WebRequest http://127.0.0.1:8188/ -UseBasicParsing -TimeoutSec 2 | Out-Null; Start-Process 'http://127.0.0.1:8188'; break}catch{Start-Sleep 2}}"

call "%~dp0start-comfyui.bat"
