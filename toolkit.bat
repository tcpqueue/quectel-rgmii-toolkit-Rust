@echo off
chcp 65001 >nul
title SimpleAdmin Rust 安装工具
setlocal EnableExtensions EnableDelayedExpansion

set "ROOT=%~dp0"
set "ADB=%ROOT%adb.exe"
set "DEV_DIR=%ROOT%development"
set "ERR=0"
set "NEED_REBOOT=0"
set "INSTALL_STATUS="
set "INSTALL_ERROR="

if exist "%ADB%" goto adb_found
call :log 错误 "未找到 adb.exe: %ADB%"
call :wait_key
exit /b 1
:adb_found

if exist "%DEV_DIR%\install_simpleadmin_rust.sh" goto package_found
call :log 错误 "未找到安装包目录: %DEV_DIR%"
call :wait_key
exit /b 1
:package_found

call :log 信息 "正在等待 ADB 设备..."
"%ADB%" wait-for-device
if not errorlevel 1 goto adb_ready
set "ERR=1"
goto cleanup
:adb_ready

call :log 信息 "正在上传 SimpleAdmin Rust 安装包..."
"%ADB%" shell rm -rf /tmp/development
"%ADB%" push "%DEV_DIR%" /tmp/development
if not errorlevel 1 goto push_ok
set "ERR=1"
goto cleanup
:push_ok

call :log 信息 "正在运行一键安装脚本..."
"%ADB%" shell "rm -f /tmp/simpleadmin-reboot-required /tmp/simpleadmin-install-result.env; chmod +x /tmp/development/install_simpleadmin_rust.sh && bash /tmp/development/install_simpleadmin_rust.sh"
set "SCRIPT_ERR=!ERRORLEVEL!"

set "REBOOT_RESULT="
set "INSTALL_STATUS="
set "INSTALL_ERROR="
set "INSTALL_RESULT_FILE=%TEMP%\simpleadmin-install-result.env"
if not exist "!INSTALL_RESULT_FILE!" goto pull_result
 del /f /q "!INSTALL_RESULT_FILE!" >nul 2>nul
:pull_result
"%ADB%" pull /tmp/simpleadmin-install-result.env "!INSTALL_RESULT_FILE!" >nul 2>nul
if not exist "!INSTALL_RESULT_FILE!" goto result_loaded
for /f "usebackq tokens=1,* delims==" %%A in ("!INSTALL_RESULT_FILE!") do (
    if /I "%%A"=="INSTALL_STATUS" set "INSTALL_STATUS=%%B"
    if /I "%%A"=="INSTALL_ERROR" set "INSTALL_ERROR=%%B"
    if /I "%%A"=="REBOOT_REQUIRED" set "REBOOT_RESULT=%%B"
)
del /f /q "!INSTALL_RESULT_FILE!" >nul 2>nul
:result_loaded

if /I "!INSTALL_STATUS!"=="OK" goto result_ok
set "ERR=1"
if "!INSTALL_ERROR!"=="" goto cleanup
call :log 错误 "远端安装脚本返回失败: !INSTALL_ERROR!"
goto cleanup
:result_ok

if "!SCRIPT_ERR!"=="0" goto script_ok
set "ERR=1"
goto cleanup
:script_ok

call :log 信息 "正在检查 simpleadmin-httpd 服务或后台进程..."
set "RUN_CHECK_FILE=%TEMP%\simpleadmin-run-check.txt"
if not exist "!RUN_CHECK_FILE!" goto run_check
 del /f /q "!RUN_CHECK_FILE!" >nul 2>nul
:run_check
"%ADB%" shell "if systemctl is-active simpleadmin-httpd.service >/dev/null 2>&1 || ps | grep simpleadmin-httpd | grep -v grep >/dev/null 2>&1; then echo SIMPLEADMIN_RUNNING=1; else echo SIMPLEADMIN_RUNNING=0; fi" > "!RUN_CHECK_FILE!" 2>nul
findstr /C:"SIMPLEADMIN_RUNNING=1" "!RUN_CHECK_FILE!" >nul 2>nul
if not errorlevel 1 goto service_running
set "ERR=1"
del /f /q "!RUN_CHECK_FILE!" >nul 2>nul
goto cleanup
:service_running
del /f /q "!RUN_CHECK_FILE!" >nul 2>nul

if "!REBOOT_RESULT!"=="1" set "NEED_REBOOT=1"
if "!REBOOT_RESULT!"=="1" goto cleanup
"%ADB%" shell "rm -f /tmp/simpleadmin-reboot-required /tmp/simpleadmin-install-result.env" >nul 2>nul

:cleanup
call :log 信息 "正在清理临时文件..."
"%ADB%" shell "rm -rf /tmp/development; mount -o remount,ro / >/dev/null 2>&1 || true" >nul 2>nul

if "%ERR%"=="0" goto install_success
call :log 错误 "安装失败，请检查上方输出信息。"
goto finish

:install_success
call :log 成功 "SimpleAdmin Rust 二进制版本安装成功。"
if "%NEED_REBOOT%"=="1" goto reboot_now
call :log 成功 "请打开 http://设备IP/"
goto finish

:reboot_now
call :log 信息 "mobileap_cfg 已处理，需要重启后再访问管理页面。"
call :log 信息 "正在发送重启命令..."
"%ADB%" shell "sync; sleep 2; reboot" >nul 2>nul
call :log 成功 "已发送重启命令，请等待设备重启完成后再打开 http://设备IP/"
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
