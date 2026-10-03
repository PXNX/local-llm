@echo off
rem Flow 8: objects and buildings (S-400, refinery, factory, Kremlin, sea mine, oil tanker ...) as
rem   flat 2D political-cartoon drawings in several views/states, optionally transparent (--cutout)
rem   and as SVG (--vectorize).
rem   Double-click: asks what to draw. Or pass options, e.g.:
rem     8-objects.bat --thing "S-400 air defense system" --thing "oil tanker ship" --cutout --vectorize
rem     8-objects.bat --thing "the Kremlin" --variant "at night with fireworks" --variant "in the snow"
rem   Result: caricatures\out\<thing>\
setlocal
cd /d "%~dp0"
call "%~dp0common\models-env.bat"
set "PY=%~dp0ComfyUI_windows_portable\python_embeded\python.exe"

set "ARGS=%*"
if "%~1"=="" (
  set /p "WHAT=What should be drawn (e.g. S-400 air defense system)? "
  set /p "VEC=Also cut out + trace to SVG? [Y/n] "
)
if "%~1"=="" (
  if not defined WHAT ( echo Nothing given. & pause & exit /b 1 )
  set ARGS=--thing "%WHAT%"
  if /i not "%VEC%"=="n" set ARGS=--thing "%WHAT%" --vectorize
)

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
