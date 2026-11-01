@echo off
rem MapleSyrup live call bench: recorded questions into the call, timed to the first sound.
rem Arguments go to bench_live.ps1 (-Label, -Effort, -Eagerness, -Mic, -Questions).
cd /d "%~dp0"
if not exist "%~dp0results" mkdir "%~dp0results"
echo MapleSyrup bench: measuring the live call, about two minutes...
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0bench_live.ps1" %* > "%~dp0results\last-run.log" 2>&1
echo Done: the numbers are in the results folder.
timeout /t 10 >nul
