@echo off
rem Flow 4: trace a PNG/JPG into an SVG vector (like vectorizer.ai / Vector Magic), fully local (vtracer).
rem   Drag & drop an image onto this file, or double-click it and pick one.
rem   Extra options are passed through, e.g.:
rem     4-vectorize.bat logo.png --style logo --mode bw
setlocal
cd /d "%~dp0"
set "PY=%~dp0ComfyUI_windows_portable\python_embeded\python.exe"

rem ---- pick the image
set "IMG=%~1"
if "%IMG%"=="" (
  for /f "delims=" %%F in ('powershell -NoProfile -STA -Command "Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.OpenFileDialog; $d.Title='Choose an image to vectorize'; $d.Filter='Images|*.jpg;*.jpeg;*.png;*.webp;*.bmp'; if($d.ShowDialog() -eq 'OK'){$d.FileName}"') do set "IMG=%%F"
)
if "%IMG%"=="" ( echo No image selected. & pause & exit /b 1 )

rem ---- run
if "%~1"=="" (
  "%PY%" -s "%~dp0vectorize\trace_svg.py" "%IMG%"
) else (
  "%PY%" -s "%~dp0vectorize\trace_svg.py" %*
)
if errorlevel 1 ( echo. & echo Something went wrong, see the output above. & pause & exit /b 1 )

for %%I in ("%IMG%") do start "" explorer "%%~dpI"
pause
exit /b 0
