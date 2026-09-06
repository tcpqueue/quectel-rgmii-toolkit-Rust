@echo off
chcp 65001 >nul
setlocal
if not exist "%~dp0SimpleAdmin-Setup.exe" goto cli
start "" "%~dp0SimpleAdmin-Setup.exe"
exit /b 0
:cli
call "%~dp0toolkit-cli.bat"
exit /b %ERRORLEVEL%
