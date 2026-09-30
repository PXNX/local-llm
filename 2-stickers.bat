@echo off
rem Flow 2: image -> transparent WebP stickers with short reaction text (Hi there!, Nope, ...).
rem   Drag & drop an image onto this file, or double-click it and pick an image.
rem   Extra options are passed through, e.g.:
rem     2-stickers.bat photo.jpg --count 5 --lang German
rem     2-stickers.bat photo.jpg --text "Monday mood" --no-stylize
rem     2-stickers.bat photo.jpg --no-text   (just the stylized image, no caption)
rem   Animate the results for Telegram afterwards with 11-animate-stickers.bat.
rem   Default engine is flux1 (FLUX.1 schnell draws the photographed animal/person as a near-
rem   photorealistic sticker acting out each reaction; one look + text style per batch) - SDXL is broken on this ComfyUI build (gray-square
rem   bug, see stickers\make_stickers.py). --engine photomaker needs models\photomaker\photomaker-v2.bin
rem   and only works once SDXL is fixed.
setlocal
cd /d "%~dp0"
set "PY=%~dp0ComfyUI_windows_portable\python_embeded\python.exe"
set "OLLAMA=%LOCALAPPDATA%\Programs\Ollama"

rem ---- pick the image
set "IMG=%~1"
if "%IMG%"=="" (
  for /f "delims=" %%F in ('powershell -NoProfile -STA -Command "Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.OpenFileDialog; $d.Title='Choose an image for stickers'; $d.Filter='Images|*.jpg;*.jpeg;*.png;*.webp;*.bmp'; if($d.ShowDialog() -eq 'OK'){$d.FileName}"') do set "IMG=%%F"
)
if "%IMG%"=="" ( echo No image selected. & pause & exit /b 1 )

rem ---- make sure Ollama is running (captions)
curl.exe -s -o nul http://127.0.0.1:11434/ || (
  echo Starting Ollama ...
  start "" "%OLLAMA%\ollama app.exe"
  call :wait http://127.0.0.1:11434/ 30 || echo Ollama did not start - continuing without captions.
)

rem ---- make sure ComfyUI is running (stylizing), unless --no-stylize
echo %* | find /i "--no-stylize" >nul || (
  curl.exe -s -o nul http://127.0.0.1:8188/ || (
    echo Starting ComfyUI in the background, this takes about 1 minute ...
    start "ComfyUI" /min cmd /c "%~dp0start-comfyui.bat"
    call :wait http://127.0.0.1:8188/ 150 || echo ComfyUI did not start - stickers will use the original image.
  )
)

rem ---- run
if "%~1"=="" (
  "%PY%" -s "%~dp0stickers\make_stickers.py" "%IMG%"
) else (
  "%PY%" -s "%~dp0stickers\make_stickers.py" %*
)
if errorlevel 1 ( echo. & echo Something went wrong, see the output above. & pause & exit /b 1 )

for %%I in ("%IMG%") do set "STEM=%%~nI"
start "" explorer "%~dp0stickers\out\%STEM%"
pause
exit /b 0

rem ---- :wait <url> <tries>  (2 s per try)
:wait
for /l %%i in (1,1,%2) do (
  curl.exe -s -o nul %1 && exit /b 0
  timeout /t 2 /nobreak >nul
)
exit /b 1
