@echo off
rem MapleSyrup live call bench: recorded questions into the call, timed to the first sound.
rem Two passes of the build beside this folder, as it is (its own instructions and tools):
rem a second pass says how much of a difference between builds is the day's noise.
rem To compare instructions without a build, pass -Variant <file in variants\> (see bench_live.ps1).
rem Arguments go to bench_live.ps1 on every pass (-Mic, -Effort, -Eagerness, -Variant, -UseRunning).
cd /d "%~dp0"
if not exist "%~dp0results" mkdir "%~dp0results"
set "EXTRA=%*"
echo MapleSyrup bench: two passes of about three minutes each...
call :pass build
call :pass build-again
echo Done: the numbers are in the results folder.
timeout /t 10 >nul
goto :eof

:pass
set "LABEL=%~1"
shift
echo   pass: %LABEL%
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0bench_live.ps1" -Label "%LABEL%" %1 %2 %3 %4 %5 %6 %EXTRA% >> "%~dp0results\last-run.log" 2>&1
goto :eof
