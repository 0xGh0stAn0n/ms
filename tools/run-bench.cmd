@echo off
rem MapleSyrup live call bench: recorded questions into the call, timed to the first sound.
rem Three passes, one call each: MapleSyrup's own instructions; the variant in
rem variants\lang-focus.json (instructions only, MapleSyrup itself unchanged); and that
rem variant with turn detection less eager to answer at a pause.
rem Arguments go to bench_live.ps1 on every pass (-Mic, -Effort, -Questions, -UseRunning).
cd /d "%~dp0"
if not exist "%~dp0results" mkdir "%~dp0results"
set "EXTRA=%*"
echo MapleSyrup bench: three passes of about three minutes each...
call :pass base
call :pass lang-focus -Variant lang-focus.json
call :pass lang-focus-vad-medium -Variant lang-focus.json -Eagerness medium
echo Done: the numbers are in the results folder.
timeout /t 10 >nul
goto :eof

:pass
set "LABEL=%~1"
shift
echo   pass: %LABEL%
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0bench_live.ps1" -Label "%LABEL%" %1 %2 %3 %4 %5 %6 %EXTRA% >> "%~dp0results\last-run.log" 2>&1
goto :eof
