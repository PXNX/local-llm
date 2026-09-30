@echo off
rem Flow 12 preview: the same sketch as 12-comedy.bat, but quick in 480p 30 fps, to check the jokes
rem   and the timing. Doesn't start ComfyUI (missing drawings become placeholders unless it already
rem   runs). Afterwards edit comedy\out\<title>\script.json if needed and drag it onto 12-comedy.bat.
setlocal
set "COMEDY_MODE=preview"
call "%~dp012-comedy.bat" %*
