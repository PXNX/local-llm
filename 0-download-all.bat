@echo off
rem Setup: downloads/resumes all models (ComfyUI ~57 GB + Ollama ~53 GB). Safe to re-run.
setlocal
cd /d "%~dp0"
set "OLLAMA=%LOCALAPPDATA%\Programs\Ollama"

curl.exe -s -o nul http://127.0.0.1:11434/ || start "" "%OLLAMA%\ollama app.exe"
timeout /t 5 /nobreak >nul

echo === Ollama models
for %%M in (qwen3:8b qwen3-vl:4b gpt-oss:20b qwen3-coder:30b) do (
  echo --- %%M
  "%OLLAMA%\ollama.exe" pull %%M
)
rem JetBrains Qwen3.8/3.6 27B blend (coding), from Hugging Face, with a short alias
set "JB=hf.co/JetBrains/Qwen3.8-3.6-27B-blend-GGUF:IQ3_S"
echo --- %JB%
"%OLLAMA%\ollama.exe" pull %JB% && "%OLLAMA%\ollama.exe" cp %JB% qwen3.8-blend:27b

echo === ComfyUI models
"%ProgramFiles%\Git\bin\bash.exe" "%~dp0download-models.sh"
pause
