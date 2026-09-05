$ErrorActionPreference = 'Stop'
$repoDir = Split-Path $PSScriptRoot -Parent
$testDir = Join-Path ([IO.Path]::GetTempPath()) ('simpleadmin-rust-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $testDir | Out-Null
$binary = Join-Path $repoDir 'windows-test\bin\simpleadmin-httpd.exe'
$staticDir = Join-Path $repoDir 'development\simpleadmin\www'
$arguments = @('--mock', '--http', '127.0.0.1:18085', '--static', ('"' + $staticDir + '"'), '--auth-file', ('"' + (Join-Path $testDir 'auth') + '"'), '--ttl-file', ('"' + (Join-Path $testDir 'ttl') + '"'))
$process = Start-Process -FilePath $binary -ArgumentList $arguments -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $testDir 'stdout.log') -RedirectStandardError (Join-Path $testDir 'stderr.log')
try {
    $ready = $false
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        try {
            $response = Invoke-WebRequest 'http://127.0.0.1:18085/login.html' -UseBasicParsing
            $ready = $response.StatusCode -eq 200
            if ($ready) { break }
        } catch { Start-Sleep -Milliseconds 100 }
    }
    if (-not $ready) { throw 'Windows server did not start.' }
    $session = New-Object Microsoft.PowerShell.Commands.WebRequestSession
    $login = Invoke-RestMethod 'http://127.0.0.1:18085/api/login' -Method Post -Body @{username='admin'; password='admin'} -WebSession $session
    if (-not $login.ok) { throw 'Login failed.' }
    $dashboard = Invoke-RestMethod 'http://127.0.0.1:18085/api/dashboard_data' -WebSession $session
    if (-not $dashboard.lastUpdate) { throw 'Dashboard data missing.' }
    $saved = Invoke-RestMethod 'http://127.0.0.1:18085/api/set_password' -Method Post -Body @{current_password='admin'; new_password='windows-test'; confirm_password='windows-test'} -WebSession $session
    if (-not $saved.ok) { throw 'Password persistence failed.' }
    $login = Invoke-RestMethod 'http://127.0.0.1:18085/api/login' -Method Post -Body @{username='admin'; password='windows-test'} -WebSession $session
    if (-not $login.ok) { throw 'Changed password rejected.' }
    Write-Output 'Windows executable, HTTP, mock dashboard and password persistence passed.'
} finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    $process.Dispose()
}
