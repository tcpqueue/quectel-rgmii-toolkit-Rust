@echo off
chcp 65001 >nul
title SimpleAdmin Rust 卸载工具
setlocal EnableExtensions

set "ROOT=%~dp0"
set "ADB=%ROOT%adb.exe"
set "SCRIPT=%ROOT%development\uninstall_simpleadmin_rust.sh"
set "ERR=0"

if exist "%ADB%" goto adb_found
call :log 错误 "未找到 adb.exe: %ADB%"
call :wait_key
exit /b 1
:adb_found

if exist "%SCRIPT%" goto script_found
call :log 错误 "未找到卸载脚本: %SCRIPT%"
call :wait_key
exit /b 1
:script_found

call :log 信息 "正在等待 ADB 设备..."
"%ADB%" wait-for-device
if not errorlevel 1 goto adb_ready
set "ERR=1"
goto cleanup
:adb_ready

call :log 信息 "正在上传 SimpleAdmin Rust 卸载脚本..."
"%ADB%" shell "rm -f /tmp/uninstall_simpleadmin_rust.sh"
"%ADB%" push "%SCRIPT%" /tmp/uninstall_simpleadmin_rust.sh
if not errorlevel 1 goto push_ok
set "ERR=1"
goto cleanup
:push_ok

call :log 信息 "正在运行卸载脚本..."
"%ADB%" shell "chmod +x /tmp/uninstall_simpleadmin_rust.sh && bash /tmp/uninstall_simpleadmin_rust.sh"
set "SCRIPT_ERR=%ERRORLEVEL%"
if "%SCRIPT_ERR%"=="0" goto script_ok
set "ERR=1"
goto cleanup
:script_ok

call :log 信息 "正在检查文件删除和进程停止状态..."
"%ADB%" shell "test ! -e /usrdata/simpleadmin/simpleadmin-httpd && ! ps | grep simpleadmin-httpd | grep -v grep >/dev/null 2>&1"
if not errorlevel 1 goto cleanup
set "ERR=1"

:cleanup
call :log 信息 "正在清理临时文件..."
"%ADB%" shell "rm -f /tmp/uninstall_simpleadmin_rust.sh; mount -o remount,ro / >/dev/null 2>&1 || true" >nul 2>nul

if "%ERR%"=="0" goto uninstall_success
call :log 错误 "卸载失败，请检查上方输出信息。"
goto finish

:uninstall_success
call :log 成功 "SimpleAdmin Rust 二进制版本卸载成功。"
goto finish

:finish
call :wait_key
exit /b %ERR%

:log
echo [%~1] %~2
exit /b 0

:wait_key
call :log 信息 "请按任意键继续 . . ."
pause >nul
exit /b 0
