@echo off
rem Flow 9: today's top 5 funniest/cutest videos from Telegram channels -> one countdown video (#5 ... #1).
rem   Channels: topvideos\channels.txt. Login: TELEGRAM_* in .env (see .env.example),
rem   the first run shows a QR code to scan in the Telegram app once.
rem   Extra options are passed through, e.g.:
rem     9-top5-videos.bat --topic cute --subject animals
rem     9-top5-videos.bat --topic funny --hours 48 --format landscape
setlocal
cd /d "%~dp0"
set "PY=%~dp0ComfyUI_windows_portable\python_embeded\python.exe"
set "OLLAMA=%LOCALAPPDATA%\Programs\Ollama"

if not exist "%~dp0.env" (
  copy "%~dp0.env.example" "%~dp0.env" >nul
  echo Enter TELEGRAM_API_ID and TELEGRAM_API_HASH from https://my.telegram.org into .env, then run this again.
  start "" notepad "%~dp0.env"
  pause & exit /b 1
)

rem ---- make sure Ollama is running (qwen3-vl rates the clips)
curl.exe -s -o nul http://127.0.0.1:11434/ || (
  echo Starting Ollama ...
  start "" "%OLLAMA%\ollama app.exe"
  for /l %%i in (1,1,30) do (
    curl.exe -s -o nul http://127.0.0.1:11434/ && goto :ready
    timeout /t 2 /nobreak >nul
  )
  echo Ollama did not start. & pause & exit /b 1
)
:ready

"%PY%" -s "%~dp0topvideos\make_top5.py" %*
if errorlevel 1 ( echo. & echo Something went wrong, see the output above. & pause & exit /b 1 )

start "" explorer "%~dp0topvideos\out"
pause
exit /b 0
