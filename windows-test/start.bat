@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0"
if not exist state mkdir state
bin\simpleadmin-httpd.exe --mock --http 127.0.0.1:8080 --static ..\development\simpleadmin\www --auth-file state\simpleadmin.auth --ttl-file state\ttlvalue
pause
