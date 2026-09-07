param([switch]$TestBuild)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$buildDir = Join-Path $env:TEMP ('simpleadmin-winui-build-' + [guid]::NewGuid())
$dotnet = $env:SIMPLEADMIN_DOTNET
if (-not $dotnet) {
    $localSdk = Join-Path $env:LOCALAPPDATA 'SimpleAdminBuild\dotnet\dotnet.exe'
    $dotnet = if (Test-Path -LiteralPath $localSdk) { $localSdk } else { 'dotnet.exe' }
}
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:DOTNET_NOLOGO = '1'
New-Item -ItemType Directory -Path $buildDir | Out-Null
try {
    $source = Join-Path $buildDir 'source'
    New-Item -ItemType Directory -Path $source | Out-Null
    Get-ChildItem -LiteralPath (Join-Path $repo 'installer') -File | Copy-Item -Destination $source
    $project = Join-Path $source 'SimpleAdmin.Installer.csproj'
    $publish = Join-Path $buildDir 'publish'
    $testMode = if ($TestBuild) { 'true' } else { 'false' }
    Push-Location $source
    try { & $dotnet publish $project -c Release -r win-x64 --self-contained true -o $publish "-p:GuiTest=$testMode" -p:RestoreLockedMode=true }
    finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw 'WinUI 3 publish failed. Install the .NET 8 SDK or set SIMPLEADMIN_DOTNET.' }
    if ($TestBuild) {
        $destination = Join-Path $repo 'work\winui-test'
        $expected = [IO.Path]::GetFullPath((Join-Path $repo 'work\winui-test'))
        if ([IO.Path]::GetFullPath($destination) -ne $expected) { throw 'Unexpected test output path.' }
        if (Test-Path -LiteralPath $destination) { Remove-Item -LiteralPath $destination -Recurse -Force }
        New-Item -ItemType Directory -Force -Path $destination | Out-Null
        Copy-Item -Path (Join-Path $publish '*') -Destination $destination -Recurse -Force
        Write-Host 'Built native WinUI 3 test application in work\winui-test.'
    } else {
        $licenses = Join-Path $publish 'licenses'
        New-Item -ItemType Directory -Path $licenses | Out-Null
        $assets = Get-Content -Raw -LiteralPath (Join-Path $source 'obj\project.assets.json') | ConvertFrom-Json
        $packageRoots = @($assets.packageFolders.PSObject.Properties.Name)
        foreach ($library in $assets.libraries.PSObject.Properties) {
            if ($library.Value.type -ne 'package') { continue }
            foreach ($packageRoot in $packageRoots) {
                $packagePath = Join-Path $packageRoot $library.Value.path
                if (-not (Test-Path -LiteralPath $packagePath)) { continue }
                foreach ($notice in (Get-ChildItem -LiteralPath $packagePath -File | Where-Object { $_.Name -match '(?i)license|third.party.notices' })) {
                    Copy-Item -LiteralPath $notice.FullName -Destination (Join-Path $licenses (($library.Name -replace '/', '-') + '-' + $notice.Name))
                }
            }
        }
        foreach ($name in @('adb.exe','AdbWinApi.dll','AdbWinUsbApi.dll','toolkit.ps1','development','LICENSE')) {
            Copy-Item -LiteralPath (Join-Path $repo $name) -Destination $publish -Recurse
        }
        $payload = Join-Path $buildDir 'payload.zip'
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        [IO.Compression.ZipFile]::CreateFromDirectory($publish, $payload, [IO.Compression.CompressionLevel]::Optimal, $false)
        $compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
        $output = Join-Path $buildDir 'SimpleAdmin-Setup.exe'
        & $compiler /nologo /target:winexe /platform:anycpu /optimize+ /utf8output /warnaserror+ "/out:$output" /reference:System.IO.Compression.dll "/resource:$payload,payload.zip" "/win32manifest:$source\app.manifest" "$source\Bootstrap.cs"
        if ($LASTEXITCODE -ne 0) { throw 'Single-file launcher compilation failed.' }
        Copy-Item -LiteralPath $output -Destination (Join-Path $repo 'SimpleAdmin-Setup.exe')
        Write-Host 'Built SimpleAdmin-Setup.exe: native WinUI 3, bundled Windows App SDK/.NET/ADB, offline x64.'
    }
} finally {
    $resolved = [IO.Path]::GetFullPath($buildDir)
    $tempRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
    if ($resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and (Split-Path $resolved -Leaf).StartsWith('simpleadmin-winui-build-')) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
