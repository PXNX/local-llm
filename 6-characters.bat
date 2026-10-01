@echo off
rem Flow 6: extract recurring characters from YouTube videos as screenshots, one folder per character.
rem   Double-click: asks for a channel/playlist/video URL. Or pass URL(s)/video files and options, e.g.:
rem     6-characters.bat https://www.youtube.com/@freeonis/videos --max-videos 20
rem   Characters to collect: characters\names.txt. Result: characters\out\<name>\
setlocal
cd /d "%~dp0"
call "%~dp0commonmodels-env.bat"
set "PY=%~dp0ComfyUI_windows_portable\python_embeded\python.exe"

if "%~1"=="" (
  set /p "URL=YouTube channel/playlist/video URL [https://www.youtube.com/@freeonis/videos]: "
  if not defined URL set "URL=https://www.youtube.com/@freeonis/videos"
)

rem ComfyUI shares the 6 GB VRAM - the detector runs much faster with it closed
curl.exe -s -o nul http://127.0.0.1:8188/ && echo Note: ComfyUI is running, close it if this runs out of VRAM.

if "%~1"=="" (
  "%PY%" -s "%~dp0characters\extract_characters.py" "%URL%" --vectorize
) else (
  "%PY%" -s "%~dp0characters\extract_characters.py" %* --vectorize
)
if errorlevel 1 ( echo. & echo Something went wrong, see the output above. & pause & exit /b 1 )

start "" explorer "%~dp0characters\out"
pause
exit /b 0
