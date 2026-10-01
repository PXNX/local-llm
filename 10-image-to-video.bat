@echo off
rem Flow 10: image -> short animated MP4 clip (Wan 2.2 TI2V 5B in ComfyUI, the image is the first frame).
rem   Drag & drop an image onto this file, or double-click it and pick one.
rem   Without --prompt, qwen3-vl (Ollama) looks at the image and writes the motion prompt.
rem   Extra options are passed through, e.g.:
rem     10-image-to-video.bat photo.jpg --prompt "the cat yawns and stretches, slow zoom in"
rem     10-image-to-video.bat photo.jpg --count 3 --seconds 4
rem     10-image-to-video.bat photo.jpg --size 640 --steps 15   (faster, lower quality)
rem   Takes roughly 10-30 minutes per clip on the RTX 2060. Result: img2video\out\
setlocal
cd /d "%~dp0"
call "%~dp0commonmodels-env.bat"
set "PY=%~dp0ComfyUI_windows_portable\python_embeded\python.exe"
set "OLLAMA=%LOCALAPPDATA%\Programs\Ollama"

rem ---- pick the image
set "IMG=%~1"
if "%IMG%"=="" (
  for /f "delims=" %%F in ('powershell -NoProfile -STA -Command "Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.OpenFileDialog; $d.Title='Choose an image to animate'; $d.Filter='Images|*.jpg;*.jpeg;*.png;*.webp;*.bmp'; if($d.ShowDialog() -eq 'OK'){$d.FileName}"') do set "IMG=%%F"
)
if "%IMG%"=="" ( echo No image selected. & pause & exit /b 1 )

rem ---- make sure Ollama is running (motion prompt), unless --prompt is given
echo %* | find /i "--prompt" >nul || (
  curl.exe -s -o nul http://127.0.0.1:11434/ || (
    echo Starting Ollama ...
    start "" "%OLLAMA%\ollama app.exe"
    call :wait http://127.0.0.1:11434/ 30 || echo Ollama did not start - using a generic motion prompt.
  )
)

rem ---- make sure ComfyUI is running
curl.exe -s -o nul http://127.0.0.1:8188/ || (
  echo Starting ComfyUI in the background, this takes about 1 minute ...
  start "ComfyUI" /min cmd /c "%~dp0start-comfyui.bat"
  call :wait http://127.0.0.1:8188/ 150 || ( echo ComfyUI did not start. & pause & exit /b 1 )
)

rem ---- run
if "%~1"=="" (
  "%PY%" -s "%~dp0img2video\make_video.py" "%IMG%"
) else (
  "%PY%" -s "%~dp0img2video\make_video.py" %*
)
if errorlevel 1 ( echo. & echo Something went wrong, see the output above. & pause & exit /b 1 )

start "" explorer "%~dp0img2video\out"
pause
exit /b 0

rem ---- :wait <url> <tries>  (2 s per try)
:wait
for /l %%i in (1,1,%2) do (
  curl.exe -s -o nul %1 && exit /b 0
  timeout /t 2 /nobreak >nul
)
exit /b 1
