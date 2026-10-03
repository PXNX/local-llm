@echo off
rem Builds OpenCode 2 from source (bun) into %USERPROFILE%\.opencode2\bin\opencode.exe, used by opencode-wrap.
rem   install-opencode2.bat            latest v2.x tag
rem   install-opencode2.bat v2.0.22    a specific tag
setlocal
set "BUN=%USERPROFILE%\.bun\bin\bun.exe"
if not exist "%BUN%" set "BUN=bun"
"%BUN%" "%~dp0install-opencode2.ts" %* || ( pause & exit /b 1 )
