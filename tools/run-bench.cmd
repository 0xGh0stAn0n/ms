@echo off
rem MapleSyrup live call bench: recorded questions into the call, timed to the first sound.
rem Four passes, one call each: MapleSyrup's own instructions; the variant in
rem variants\lang-focus.json (instructions only, MapleSyrup itself unchanged); the variant
rem in variants\lang-focus-quiet.json (and a tool to keep quiet when not spoken to); and
rem that one with turn detection less eager to answer at a pause.
rem Arguments go to bench_live.ps1 on every pass (-Mic, -Effort, -Questions, -UseRunning).
cd /d "%~dp0"
if not exist "%~dp0results" mkdir "%~dp0results"
set "EXTRA=%*"
echo MapleSyrup bench: four passes of about three minutes each...
call :pass base
call :pass lang-focus -Variant lang-focus.json
call :pass lang-focus-quiet -Variant lang-focus-quiet.json
call :pass lang-focus-quiet-vad-medium -Variant lang-focus-quiet.json -Eagerness medium
echo Done: the numbers are in the results folder.
timeout /t 10 >nul
goto :eof

:pass
set "LABEL=%~1"
shift
echo   pass: %LABEL%
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0bench_live.ps1" -Label "%LABEL%" %1 %2 %3 %4 %5 %6 %EXTRA% >> "%~dp0results\last-run.log" 2>&1
goto :eof
