@echo off
rem MapleSyrup live call bench: recorded questions into the call, timed to the first sound,
rem at each of three ways of thinking (low, medium, high), one call each.
rem Arguments go to bench_live.ps1 on every pass (-Mic, -Eagerness, -Questions, -UseRunning).
cd /d "%~dp0"
if not exist "%~dp0results" mkdir "%~dp0results"
echo MapleSyrup bench: three passes of about four minutes each...
for %%E in (low medium high) do (
  echo   pass: thinking %%E
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0bench_live.ps1" -Effort %%E -Label "think-%%E" %* >> "%~dp0results\last-run.log" 2>&1
)
echo Done: the numbers are in the results folder.
timeout /t 10 >nul
