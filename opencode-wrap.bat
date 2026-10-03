@echo off
rem opencode-wrap: OpenAI-compatible API for the free OpenCode Zen models (default muse-spark-1.3-contributor-free)
rem at http://127.0.0.1:8000/v1, in front of OpenCode 2 (opencode-wrap\install-opencode2.bat builds it).
rem The flows start it by themselves with LLM_PROVIDER=opencode; this window is for other tools / to watch the log.
rem   opencode-wrap.bat                 default model
rem   set WRAP_MODEL=mimo-v2.6-flash-free & opencode-wrap.bat
setlocal
set "BUN=%USERPROFILE%\.bun\bin\bun.exe"
if not exist "%BUN%" set "BUN=bun"
if not exist "%USERPROFILE%\.opencode2\bin\opencode.exe" call "%~dp0opencode-wrap\install-opencode2.bat" || exit /b 1
"%BUN%" "%~dp0opencode-wrap\server.ts"
pause
