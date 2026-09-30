@echo off
rem Flow 12: political comedy cartoon sketch (freeonis style) in 1080p 60 fps.
rem   Double-click: asks for a topic. Drag & drop a script.json (from an earlier run or the preview)
rem   onto it to render that script. Or pass options, e.g.:
rem     12-comedy.bat --topic "Trump wants to buy Greenland" --cast Trump --cast "Mette Frederiksen"
rem     12-comedy.bat --script comedy\out\<title>\script.json --handle @mychannel
rem   Quick 480p 30 fps check first: 12-comedy-preview.bat. Result: comedy\out\<title>\
setlocal
cd /d "%~dp0"
set "PY=%~dp0ComfyUI_windows_portable\python_embeded\python.exe"
set "MODE=%COMEDY_MODE%"
set "OLLAMA=%LOCALAPPDATA%\Programs\Ollama"

set "ARGS=%*"
if "%~1"=="" (
  set /p "TOPIC=What should the sketch be about? "
)
if "%~1"=="" (
  if not defined TOPIC ( echo Nothing given. & pause & exit /b 1 )
  set ARGS=--topic "%TOPIC%"
)
if /i "%~x1"==".json" set ARGS=--script %*

rem the LLM writes the sketch: OpenRouter (key in .env) or the local Ollama
curl.exe -s -o nul http://127.0.0.1:11434/ || (
  if exist "%OLLAMA%\ollama app.exe" start "" "%OLLAMA%\ollama app.exe"
)

rem missing caricatures/objects/backgrounds are drawn with FLUX.1 - the preview skips starting ComfyUI
if /i not "%MODE%"=="preview" (
  curl.exe -s -o nul http://127.0.0.1:8188/ || (
    echo Starting ComfyUI in the background, this takes about 1 minute ...
    start "ComfyUI" /min cmd /c "%~dp0start-comfyui.bat"
    call :wait http://127.0.0.1:8188/ 150 || echo ComfyUI did not start, missing drawings become placeholders.
  )
)

if /i "%MODE%"=="preview" (
  "%PY%" -s "%~dp0comedy\make_comedy.py" %ARGS% --preview
) else (
  "%PY%" -s "%~dp0comedy\make_comedy.py" %ARGS%
)
if errorlevel 1 ( echo. & echo Something went wrong, see the output above. & pause & exit /b 1 )

start "" explorer "%~dp0comedy\out"
pause
exit /b 0

rem ---- :wait <url> <tries>  (2 s per try)
:wait
for /l %%i in (1,1,%2) do (
  curl.exe -s -o nul %1 && exit /b 0
  timeout /t 2 /nobreak >nul
)
exit /b 1
