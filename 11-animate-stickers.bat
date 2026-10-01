@echo off
rem Flow 11: finished stickers (Flow 2) -> animated Telegram video stickers (WebM VP9 with transparency,
rem   512 px, max 3 s, max 256 KB). Each sticker gets a looping <name>.webm next to it.
rem   Drag & drop a sticker folder (stickers\out\<image>) or .webp files onto this file,
rem   or double-click it and pick a folder. Extra options are passed through, e.g.:
rem     11-animate-stickers.bat stickers\out\cat          (each character acts out its pose: waves, hugs, dances ...)
rem     11-animate-stickers.bat stickers\out\cat\cat_sticker_1.webp --prompt "the cat waves its raised paw"
rem     11-animate-stickers.bat stickers\out\cat --mode loop   (quick simple loop, no GPU)
rem   Wan 2.2 in ComfyUI animates each sticker, roughly 5-15 minutes per sticker on the RTX 2060.
setlocal
cd /d "%~dp0"
call "%~dp0commonmodels-env.bat"
set "PY=%~dp0ComfyUI_windows_portable\python_embeded\python.exe"

rem ---- pick the sticker folder
set "IN=%~1"
if "%IN%"=="" (
  for /f "delims=" %%F in ('powershell -NoProfile -STA -Command "Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description='Choose a sticker folder (stickers\out\...)'; $d.SelectedPath='%~dp0stickers\out'; if($d.ShowDialog() -eq 'OK'){$d.SelectedPath}"') do set "IN=%%F"
)
if "%IN%"=="" ( echo No folder selected. & pause & exit /b 1 )

rem ---- make sure ComfyUI is running, unless --mode loop
echo %* | find /i "--mode loop" >nul || (
  curl.exe -s -o nul http://127.0.0.1:8188/ || (
    echo Starting ComfyUI in the background, this takes about 1 minute ...
    start "ComfyUI" /min cmd /c "%~dp0start-comfyui.bat"
    call :wait http://127.0.0.1:8188/ 150 || ( echo ComfyUI did not start. & pause & exit /b 1 )
  )
)

rem ---- run
if "%~1"=="" (
  "%PY%" -s "%~dp0stickers\animate_stickers.py" "%IN%"
) else (
  "%PY%" -s "%~dp0stickers\animate_stickers.py" %*
)
if errorlevel 1 ( echo. & echo Something went wrong, see the output above. & pause & exit /b 1 )

if exist "%IN%\" ( start "" explorer "%IN%" ) else ( for %%I in ("%IN%") do start "" explorer "%%~dpI" )
pause
exit /b 0

rem ---- :wait <url> <tries>  (2 s per try)
:wait
for /l %%i in (1,1,%2) do (
  curl.exe -s -o nul %1 && exit /b 0
  timeout /t 2 /nobreak >nul
)
exit /b 1
