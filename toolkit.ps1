param([switch]$DiagnoseOnly, [switch]$OpenWebOnly)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$OutputEncoding = [Console]::OutputEncoding
$adb = Join-Path $PSScriptRoot 'adb.exe'
$development = Join-Path $PSScriptRoot 'development'
$serial = $null
$exitCode = 1
$logDir = Join-Path $PSScriptRoot 'logs'
try { New-Item -ItemType Directory -Force -Path $logDir | Out-Null }
catch { $logDir = $env:TEMP }
$report = Join-Path $logDir ('simpleadmin-{0}-{1}.txt' -f (Get-Date -Format 'yyyyMMdd-HHmmss'), $PID)

function Event([string]$Kind, [string]$Value) {
    Write-Output ('@@SIMPLEADMIN|' + $Kind + '|' + $Value)
}

function Log([string]$Message) {
    Write-Host $Message
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

function ProbeWeb([switch]$KeepTunnel) {
    $forward = AdbCommand @('forward', 'tcp:0', 'tcp:80') -Quiet
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
        RequireAdb @('shell', 'rm -rf /tmp/development; rm -f /tmp/simpleadmin-install-result.env')
        RequireAdb @('push', $development, '/tmp/development')
        Event 'stage' 'install'
        $install = AdbCommand @('shell', 'bash /tmp/development/install_simpleadmin_rust.sh')
        $result = AdbCommand @('shell', 'cat /tmp/simpleadmin-install-result.env')
        if ($install.Code -ne 0 -or $result.Code -ne 0 -or $result.Lines -notcontains 'INSTALL_STATUS=OK') {
            throw 'Installation did not pass. See the error and diagnostics in this report.'
        }
        Event 'stage' 'verify'
        ProbeWeb -KeepTunnel
        $null = AdbCommand @('shell', 'ip -4 addr show')
        Log 'Installation and HTTP checks passed. Use http:// (not https://) with a reachable module IPv4 address.'
        if ($result.Lines -contains 'REBOOT_REQUIRED=1') {
            Log 'Network configuration changed. Restart the module manually, then check its current IP address.'
            Event 'reboot' 'required'
        }
        $exitCode = 0
    }
} catch {
    Log "ERROR: $($_.Exception.Message)"
    Event 'failure' 'operation'
    if ($serial -and -not $DiagnoseOnly -and -not $OpenWebOnly) {
        try { Diagnose } catch { Log "Diagnostics incomplete: $($_.Exception.Message)" }
    }
} finally {
    if ($serial -and -not $DiagnoseOnly -and -not $OpenWebOnly) {
        $null = AdbCommand @('shell', 'rm -rf /tmp/development') -Quiet
    }
    Log "Report saved: $report"
    Event 'result' ([string]$exitCode)
}
exit $exitCode
