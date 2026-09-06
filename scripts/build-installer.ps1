param([switch]$TestBuild)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$buildDir = Join-Path $env:TEMP ('simpleadmin-gui-build-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $buildDir | Out-Null
try {
    $framework = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319'
    $compiler = Join-Path $framework 'csc.exe'
    if (-not (Test-Path -LiteralPath $compiler)) { throw '.NET Framework C# compiler is unavailable.' }
    Copy-Item -LiteralPath (Join-Path $repo 'installer\Program.cs') -Destination $buildDir
    Copy-Item -LiteralPath (Join-Path $repo 'installer\MainWindow.xaml') -Destination $buildDir
    Copy-Item -LiteralPath (Join-Path $repo 'installer\app.manifest') -Destination $buildDir
    $references = @('WindowsBase', 'PresentationCore', 'PresentationFramework') | ForEach-Object { '/reference:' + (Join-Path $framework ('WPF\' + $_ + '.dll')) }
    $output = Join-Path $buildDir 'SimpleAdmin-Setup.exe'
    $defines = @()
    if ($TestBuild) { $defines = @('/define:GUI_TEST_HARNESS') }
    & $compiler /nologo /target:winexe /platform:anycpu /optimize+ /utf8output /warnaserror+ "/out:$output" @references @defines /reference:System.Xaml.dll "/resource:$buildDir\MainWindow.xaml,MainWindow.xaml" "/win32manifest:$buildDir\app.manifest" "$buildDir\Program.cs"
    if ($LASTEXITCODE -ne 0) { throw 'GUI compilation failed.' }
    if ($TestBuild) {
        New-Item -ItemType Directory -Force -Path (Join-Path $repo 'work') | Out-Null
        Copy-Item -LiteralPath $output -Destination (Join-Path $repo 'work\SimpleAdmin-Setup.Test.exe')
    } else {
        Copy-Item -LiteralPath $output -Destination (Join-Path $repo 'SimpleAdmin-Setup.exe')
    }
    Write-Host 'Built SimpleAdmin-Setup.exe (WPF / Windows .NET Framework, no downloaded runtime).'
} finally {
    $resolved = [IO.Path]::GetFullPath($buildDir)
    $tempRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
    if ($resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and (Split-Path $resolved -Leaf).StartsWith('simpleadmin-gui-build-')) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
