$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$scratch = Join-Path $env:TEMP ('simpleadmin-qualcomm-test-' + [guid]::NewGuid())
$dotnet = $env:SIMPLEADMIN_DOTNET
if (-not $dotnet) { $dotnet = Join-Path $env:LOCALAPPDATA 'SimpleAdminBuild\dotnet\dotnet.exe' }
New-Item -ItemType Directory -Path $scratch | Out-Null
try {
    Copy-Item -Path (Join-Path $repo 'tests\qualcomm\*') -Destination $scratch
    Copy-Item -LiteralPath (Join-Path $repo 'installer\Qualcomm.cs') -Destination $scratch
    Copy-Item -LiteralPath (Join-Path $repo 'installer\global.json') -Destination $scratch
    Push-Location $scratch
    try { & $dotnet run --project Qualcomm.Tests.csproj -c Release }
    finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw 'Qualcomm tests failed.' }
} finally {
    $resolved = [IO.Path]::GetFullPath($scratch)
    $tempRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
    if ($resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and (Split-Path $resolved -Leaf).StartsWith('simpleadmin-qualcomm-test-')) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}