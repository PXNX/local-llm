@echo off
rem Flow 3: coding LLM in T3 Code (OpenCode provider).
rem   Default: free OpenCode Zen cloud model opencode/muse-spark-1.3-contributor-free (no key, no GPU memory)
rem   Other Zen model:    3-coding-llm-t3code.bat opencode/mimo-v2.6-flash-free
rem   Local Ollama model: 3-coding-llm-t3code.bat qwen3:8b     (or gpt-oss:20b, qwen3-coder:30b, qwen3.8-blend:27b)
rem   Terminal instead of T3 Code:  3-coding-llm-t3code.bat qwen3:8b cli
setlocal
call "%~dp0common\models-env.bat"
set "MODEL=%~1"
if "%MODEL%"=="" set "MODEL=opencode/muse-spark-1.3-contributor-free"
set "OLLAMA=%LOCALAPPDATA%\Programs\Ollama"

rem Zen models run in the cloud: nothing to start or load
if /i "%MODEL:~0,9%"=="opencode/" (
  set "FULL=%MODEL%"
  goto :open
)
set "FULL=ollama/%MODEL%"

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

"%OLLAMA%\ollama.exe" list | find /i "%MODEL%" >nul || (
  echo Model %MODEL% is missing, downloading ...
  "%OLLAMA%\ollama.exe" pull %MODEL% || ( pause & exit /b 1 )
)

echo Loading %MODEL% into memory ...
powershell -NoProfile -Command "Invoke-RestMethod http://127.0.0.1:11434/api/generate -Method Post -Body '{\"model\":\"%MODEL%\",\"keep_alive\":\"30m\"}' | Out-Null"
"%OLLAMA%\ollama.exe" ps

:open
if /i "%~2"=="cli" (
  "%USERPROFILE%\.bun\bin\opencode.exe" -m %FULL%
  exit /b 0
)

start "" "%LOCALAPPDATA%\Programs\t3code\T3 Code (Alpha).exe"
echo.
echo T3 Code is opening: in the model picker choose provider OpenCode, then %FULL%.
timeout /t 8 >nul
