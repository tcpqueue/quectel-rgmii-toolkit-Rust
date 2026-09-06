@echo off
chcp 65001 >nul
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0toolkit.ps1"
set "RESULT=%ERRORLEVEL%"
pause
exit /b %RESULT%
