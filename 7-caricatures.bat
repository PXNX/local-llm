@echo off
rem Flow 7: famous people (or yourself, from a photo) as flat 2D political-cartoon caricatures
rem   in several expressions, optionally transparent (--cutout) and as SVG (--vectorize).
rem   Double-click: asks who to draw. Or pass options, e.g.:
rem     7-caricatures.bat --who "Emmanuel Macron" --who "Friedrich Merz" --vectorize
rem     7-caricatures.bat --photo me.jpg --name Felix --cutout --vectorize
rem   Drag & drop a photo onto this file to caricature that person.
rem   Result: caricatures\out\<name>\
setlocal
cd /d "%~dp0"
set "PY=%~dp0ComfyUI_windows_portable\python_embeded\python.exe"
set "OLLAMA=%LOCALAPPDATA%\Programs\Ollama"

rem ---- arguments: none -> ask, a single image file -> --photo, else pass through
set "ARGS=%*"
if "%~1"=="" (
  set /p "WHO=Who should be drawn (e.g. Emmanuel Macron)? "
  set /p "VEC=Also vectorize to SVG? [y/N] "
)
if "%~1"=="" (
  if not defined WHO ( echo Nobody given. & pause & exit /b 1 )
  set ARGS=--who "%WHO%"
  if /i "%VEC%"=="y" set ARGS=--who "%WHO%" --cutout --vectorize
)
if exist "%~1" set ARGS=--photo %*

rem ---- Ollama describes the look for --photo
echo %ARGS% | find /i "--photo" >nul && (
  curl.exe -s -o nul http://127.0.0.1:11434/ || (
    echo Starting Ollama ...
    start "" "%OLLAMA%\ollama app.exe"
    call :wait http://127.0.0.1:11434/ 30 || echo Ollama did not start - continuing without a look description.
  )
)

rem ---- ComfyUI draws
curl.exe -s -o nul http://127.0.0.1:8188/ || (
  echo Starting ComfyUI in the background, this takes about 1 minute ...
  start "ComfyUI" /min cmd /c "%~dp0start-comfyui.bat"
  call :wait http://127.0.0.1:8188/ 150 || ( echo ComfyUI did not start. & pause & exit /b 1 )
)

"%PY%" -s "%~dp0caricatures\make_caricatures.py" %ARGS%
if errorlevel 1 ( echo. & echo Something went wrong, see the output above. & pause & exit /b 1 )

start "" explorer "%~dp0caricatures\out"
pause
exit /b 0

rem ---- :wait <url> <tries>  (2 s per try)
:wait
for /l %%i in (1,1,%2) do (
  curl.exe -s -o nul %1 && exit /b 0
  timeout /t 2 /nobreak >nul
)
exit /b 1
