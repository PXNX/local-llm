@echo off
rem Flow 5: generate procedural game sound effects (mine/Shahed explosions, Patriot rocket motor)
rem   and process the "Slava Ukraini" voice line into soundfx\out\. Fully local (numpy/scipy),
rem   no Ollama/ComfyUI needed. Extra options are passed through, e.g.:
rem     5-soundfx.bat --count 5 --seed 42
rem     5-soundfx.bat --voice "C:\path\to\voice.wav"
setlocal
cd /d "%~dp0"
set "PY=%~dp0ComfyUI_windows_portable\python_embeded\python.exe"

"%PY%" -s "%~dp0soundfx\generate_sfx.py" %*
if errorlevel 1 ( echo. & echo Something went wrong, see the output above. & pause & exit /b 1 )

start "" explorer "%~dp0soundfx\out"
pause
exit /b 0
