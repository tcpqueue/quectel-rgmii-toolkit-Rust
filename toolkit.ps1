param([switch]$DiagnoseOnly, [switch]$OpenWebOnly, [string]$HttpPort = '')
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$OutputEncoding = [Console]::OutputEncoding
$adb = Join-Path $PSScriptRoot 'adb.exe'
$development = Join-Path $PSScriptRoot 'development'
$serial = $null
$deviceChecked = $false
$stagingStarted = $false
$exitCode = 1
$logDir = Join-Path $PSScriptRoot 'logs'
if ($env:SIMPLEADMIN_REPORT_DIR) { $logDir = $env:SIMPLEADMIN_REPORT_DIR }
try { New-Item -ItemType Directory -Force -Path $logDir | Out-Null }
catch { $logDir = $env:TEMP }
$report = Join-Path $logDir ('simpleadmin-{0}-{1}.txt' -f (Get-Date -Format 'yyyyMMdd-HHmmss'), $PID)

function Event([string]$Kind, [string]$Value) {
    Write-Output ('@@SIMPLEADMIN|' + $Kind + '|' + $Value)
}

function Log([string]$Message) {
    [Console]::WriteLine($Message)
    [Console]::Out.Flush()
    Add-Content -LiteralPath $report -Value $Message -Encoding UTF8
}

function AdbCommand([string[]]$Arguments, [switch]$Quiet) {
    $ErrorActionPreference = 'Continue'
    $prefix = @()
    if ($serial) { $prefix = @('-s', $serial) }
    $lines = @(& $adb @prefix @Arguments 2>&1 | ForEach-Object {
        $line = $_.ToString()
        if (-not $Quiet) { Log $line }
        $line
    })
    [pscustomobject]@{ Code = $LASTEXITCODE; Lines = $lines }
}

function RequireAdb([string[]]$Arguments) {
    $result = AdbCommand $Arguments
    if ($result.Code -ne 0) { throw "ADB command failed: $($Arguments[0]) (exit $($result.Code))" }
}

function CheckDevice {
    Log '正在检查所选设备的系统、模块特征、权限及临时目录（只读检查）。'
    $probe = @'
echo SIMPLEADMIN_PREFLIGHT=1
echo SA_SYSTEM=$(uname -s)
echo SA_ARCH=$(uname -m)
echo SA_UID=$(id -u)
if [ -d /usrdata ] && { [ -c /dev/smd11 ] || [ -f /usrdata/etc/data/mobileap_cfg.xml ] || [ -f /etc/data/mobileap_cfg.xml ]; }; then echo SA_MODULE=1; else echo SA_MODULE=0; fi
if command -v bash >/dev/null 2>&1; then echo SA_BASH=1; else echo SA_BASH=0; fi
if [ -d /tmp ] && [ -w /tmp ]; then echo SA_TMP=1; else echo SA_TMP=0; fi
echo SIMPLEADMIN_PREFLIGHT_DONE=1
'@
    $probe = $probe.Replace("`r", '')
    $result = AdbCommand @('shell', $probe)
    $lines = @($result.Lines | ForEach-Object { $_.Trim() })
    if ($result.Code -ne 0 -or $lines -notcontains 'SIMPLEADMIN_PREFLIGHT_DONE=1') {
        throw '无法读取所选设备的系统信息，请检查 ADB 连接并重新选择模块。'
    }
    if ($lines -notcontains 'SA_SYSTEM=Linux' -or $lines -notcontains 'SA_ARCH=armv7l' -or $lines -notcontains 'SA_MODULE=1') {
        throw '所选 ADB 设备未通过模块兼容性检查。请连接 Quectel 模块，排除手机、模拟器或其他 ADB 设备；若通过端口转发连接，请确认转发目标是模块本身。'
    }
    if (-not $OpenWebOnly) {
        if ($lines -notcontains 'SA_UID=0') { throw '当前 ADB 没有 root 权限，无法安装或诊断模块。请使用模块提供的 root ADB 连接。' }
        if ($lines -notcontains 'SA_BASH=1') { throw '模块固件缺少 Bash，当前安装器无法在此固件上运行。' }
        if ($lines -notcontains 'SA_TMP=1') { throw '模块的 /tmp 不存在或不可写，尚未上传任何文件。请检查固件的临时目录挂载状态；根目录只读本身是正常的，不要直接解除根目录只读来绕过此检查。' }
    }
}

function ReadHttpPort {
    $probe = 'if [ -e /usrdata/simpleadmin/http_port ]; then cat /usrdata/simpleadmin/http_port; else echo 80; fi'
    $result = AdbCommand @('shell', $probe) -Quiet
    $value = ($result.Lines -join "`n").Trim()
    if ($result.Code -ne 0 -or $value -cnotmatch '^[1-9][0-9]{0,4}$' -or [int]$value -gt 65535) {
        throw '无法读取模块的 HTTP 端口配置，请在安装器中勾选修改端口后重新安装。'
    }
    Log "Device HTTP port: $value"
    return $value
}

function ProbeWeb([switch]$KeepTunnel) {
    $devicePort = ReadHttpPort
    Event 'http-port' $devicePort
    $forward = AdbCommand @('forward', 'tcp:0', "tcp:$devicePort") -Quiet
    $port = ($forward.Lines | Where-Object { $_ -match '^\d+$' } | Select-Object -Last 1)
    if ($forward.Code -ne 0 -or -not $port) { throw 'Unable to create an ADB HTTP tunnel.' }
    $success = $false
    try {
        foreach ($page in @(@('/', 'SimpleAdminSpaMode'), @('/login.html', 'loginLanguage'), @('/js/locales.js', 'root.Lang'))) {
            $request = [System.Net.HttpWebRequest]::Create("http://127.0.0.1:$port$($page[0])")
            $request.Proxy = $null
            $request.Timeout = 5000
            $request.ReadWriteTimeout = 5000
            $request.AllowAutoRedirect = $false
            $response = $request.GetResponse()
            try {
                if ($page[0] -eq '/' -and [int]$response.StatusCode -eq 303 -and $response.Headers['Location'] -eq '/login.html') { continue }
                $reader = New-Object System.IO.StreamReader($response.GetResponseStream())
                try { $body = $reader.ReadToEnd() } finally { $reader.Dispose() }
                if ([int]$response.StatusCode -ne 200 -or -not $body.Contains($page[1])) {
                    throw "Unexpected HTTP response for $($page[0])."
                }
            } finally { $response.Dispose() }
        }
        Log 'PC_HTTP_CHECK=OK (ADB tunnel; LAN reachability must be checked separately)'
        $success = $true
        if ($KeepTunnel) {
            Log "Open on this PC: http://127.0.0.1:$port/ (available while ADB is connected)"
            Event 'url' "http://127.0.0.1:$port/"
        }
    } finally {
        if (-not $KeepTunnel -or -not $success) {
            $null = AdbCommand @('forward', '--remove', "tcp:$port") -Quiet
        }
    }
}

function Diagnose {
    Log 'Collecting diagnostics. No login, SMS, AT commands or configuration changes.'
    $source = Join-Path $development 'diagnose_simpleadmin.sh'
    $remote = "/tmp/simpleadmin-diagnose-$PID.sh"
    try {
        RequireAdb @('push', $source, $remote)
        RequireAdb @('shell', "bash $remote")
    } finally {
        $null = AdbCommand @('shell', "rm -f $remote") -Quiet
    }
}

try {
    Event 'report' $report
    Event 'stage' 'device'
    if ($HttpPort -ne '' -and ($HttpPort -cnotmatch '^[1-9][0-9]{0,4}$' -or [int]$HttpPort -gt 65535)) {
        throw 'HTTP 端口必须是 1–65535 的整数。'
    }
    Log ('SimpleAdmin {0} - {1}' -f $(if ($DiagnoseOnly) { 'diagnostics' } else { 'installer' }), (Get-Date -Format o))
    if (-not (Test-Path -LiteralPath $adb)) { throw 'adb.exe is missing. Extract the entire offline ZIP first.' }
    $devices = AdbCommand @('devices')
    if ($devices.Code -ne 0) { throw 'ADB device enumeration failed.' }
    $ready = @($devices.Lines | Where-Object { $_ -match '^\S+\s+device$' } | ForEach-Object { ($_ -split '\s+')[0] })
    if ($env:ANDROID_SERIAL) {
        if ($ready -notcontains $env:ANDROID_SERIAL) { throw 'ANDROID_SERIAL is not a connected, authorized device.' }
        $serial = $env:ANDROID_SERIAL
    } elseif ($ready.Count -eq 1) {
        $serial = $ready[0]
    } else { throw 'Connect exactly one authorized ADB device, or set ANDROID_SERIAL to select one.' }
    Log "Selected device: $serial"
    CheckDevice
    $deviceChecked = $true
    if ($OpenWebOnly) {
        Event 'stage' 'verify'
        ProbeWeb -KeepTunnel
        $exitCode = 0
    } elseif ($DiagnoseOnly) {
        Event 'stage' 'diagnose'
        Diagnose
        Event 'stage' 'verify'
        ProbeWeb
        $exitCode = 0
    } else {
        if (-not (Test-Path (Join-Path $development 'SHA256SUMS'))) { throw 'Package checksums are missing. Extract the entire offline ZIP.' }
        Event 'stage' 'upload'
        $stagingStarted = $true
        RequireAdb @('shell', 'rm -rf /tmp/development; rm -f /tmp/simpleadmin-install-result.env')
        RequireAdb @('push', $development, '/tmp/development')
        Event 'stage' 'install'
        $installCommand = 'bash /tmp/development/install_simpleadmin_rust.sh'
        if ($HttpPort -ne '') { $installCommand = "SIMPLEADMIN_HTTP_PORT=$HttpPort $installCommand" }
        $install = AdbCommand @('shell', $installCommand)
        $result = AdbCommand @('shell', 'cat /tmp/simpleadmin-install-result.env')
        if ($install.Code -ne 0 -or $result.Code -ne 0 -or $result.Lines -notcontains 'INSTALL_STATUS=OK') {
            throw 'Installation did not pass. See the error and diagnostics in this report.'
        }
        Event 'stage' 'verify'
        ProbeWeb -KeepTunnel
        $null = AdbCommand @('shell', 'ip -4 addr show')
        Log 'Installation and HTTP checks passed. Use http:// with a reachable module IPv4 address and the device HTTP port shown above.'
        if ($result.Lines -contains 'REBOOT_REQUIRED=1') {
            Log 'Network configuration changed. Restart the module manually, then check its current IP address.'
            Event 'reboot' 'required'
        }
        $exitCode = 0
    }
} catch {
    Log "ERROR: $($_.Exception.Message)"
    Event 'error' ($_.Exception.Message -replace '[\r\n]+', ' ')
    Event 'failure' 'operation'
    if ($deviceChecked -and $serial -and -not $DiagnoseOnly -and -not $OpenWebOnly) {
        try { Diagnose } catch { Log "Diagnostics incomplete: $($_.Exception.Message)" }
    }
} finally {
    if ($stagingStarted -and $serial -and -not $DiagnoseOnly -and -not $OpenWebOnly) {
        $null = AdbCommand @('shell', 'rm -rf /tmp/development') -Quiet
    }
    Log "Report saved: $report"
    Event 'result' ([string]$exitCode)
}
exit $exitCode
