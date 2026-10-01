@echo off
rem One folder for every model, the same as the GUI and common\storage.py:
rem   MODELS_DIR from the environment, else from .env (relative = inside the repo), default <repo>\models.
rem   Sets MODELS_DIR, OLLAMA_MODELS, HF_HOME, TORCH_HOME and U2NET_HOME.  Usage: call "%~dp0common\models-env.bat"
for %%R in ("%~dp0..") do set "LLM_ROOT=%%~fR"
if defined MODELS_DIR goto :resolve
if exist "%LLM_ROOT%\.env" for /f "usebackq eol=# tokens=1,* delims==" %%A in ("%LLM_ROOT%\.env") do if /i "%%~A"=="MODELS_DIR" set "MODELS_DIR=%%~B"
if not defined MODELS_DIR set "MODELS_DIR=%LLM_ROOT%\models"
:resolve
pushd "%LLM_ROOT%"
for %%M in ("%MODELS_DIR%") do set "MODELS_DIR=%%~fM"
popd
rem a trailing backslash would escape the closing quote of "%MODELS_DIR%" arguments (D:\ -> D:\.)
if "%MODELS_DIR:~-1%"=="\" set "MODELS_DIR=%MODELS_DIR%."
if not exist "%MODELS_DIR%" mkdir "%MODELS_DIR%"
set "OLLAMA_MODELS=%MODELS_DIR%\ollama"
set "HF_HOME=%MODELS_DIR%\huggingface"
set "TORCH_HOME=%MODELS_DIR%\torch"
set "U2NET_HOME=%MODELS_DIR%\u2net"
set "LLM_ROOT="
