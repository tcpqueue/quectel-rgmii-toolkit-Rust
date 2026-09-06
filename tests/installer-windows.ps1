param([switch]$GuiTest)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$scratch = Join-Path $env:TEMP ('simpleadmin-installer-test-' + [guid]::NewGuid())
$server = $null
$oldSerial = $env:ANDROID_SERIAL
$oldCase = $env:SIMPLEADMIN_TEST_CASE
$oldDir = $env:SIMPLEADMIN_TEST_DIR
New-Item -ItemType Directory -Path $scratch | Out-Null

try {
    Copy-Item -LiteralPath (Join-Path $root 'toolkit.ps1') -Destination $scratch
    if ($GuiTest) { Copy-Item -LiteralPath (Join-Path $root 'work\SimpleAdmin-Setup.Test.exe') -Destination (Join-Path $scratch 'SimpleAdmin-Setup.exe') }
    New-Item -ItemType Directory -Path (Join-Path $scratch 'development') | Out-Null
    Set-Content -LiteralPath (Join-Path $scratch 'development/SHA256SUMS') -Value 'fixture'
    $env:SIMPLEADMIN_TEST_DIR = $scratch
    $env:ANDROID_SERIAL = ''
    $fakeSource = @'
using System;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Text;
public class FakeAdb {
    public static int Main(string[] args) {
        string dir = Environment.GetEnvironmentVariable("SIMPLEADMIN_TEST_DIR");
        if (args.Length == 1 && args[0] == "serve") {
            var listener = new TcpListener(IPAddress.Loopback, 0);
            listener.Start();
            File.WriteAllText(Path.Combine(dir, "port"), ((IPEndPoint)listener.LocalEndpoint).Port.ToString());
            while (true) {
                using (var client = listener.AcceptTcpClient()) {
                    client.ReceiveTimeout = 3000;
                    using (var stream = client.GetStream()) {
                        var reader = new StreamReader(stream);
                        string request = reader.ReadLine() ?? "";
                        while (!String.IsNullOrEmpty(reader.ReadLine())) {}
                        string mode = File.ReadAllText(Path.Combine(dir, "http-mode")).Trim();
                        string body = mode == "wrong-app" ? "factory web" : request.Contains("locales.js") ? "root.Lang" : request.Contains("login.html") ? "loginLanguage" : "SimpleAdminSpaMode";
                        string status = request.StartsWith("GET / HTTP/") && mode != "wrong-app" ? "303 See Other\r\nLocation: /login.html" : "200 OK";
                        byte[] bytes = Encoding.ASCII.GetBytes("HTTP/1.1 " + status + "\r\nConnection: close\r\nContent-Length: " + body.Length + "\r\n\r\n" + body);
                        stream.Write(bytes, 0, bytes.Length);
                    }
                }
            }
        }
        string modeName = Environment.GetEnvironmentVariable("SIMPLEADMIN_TEST_CASE") ?? "success";
        string call = String.Join(" ", args);
        File.AppendAllText(Path.Combine(dir, "calls"), call + "\n");
        if (call == "devices" || call == "devices -l") {
            Console.WriteLine("List of devices attached");
            if (modeName == "none") return 0;
            Console.WriteLine("FAKE1\t" + (modeName == "unauthorized" ? "unauthorized" : "device"));
            if (modeName == "multiple") Console.WriteLine("FAKE2\tdevice");
        } else if (call.Contains("push ") && modeName == "push-failed") {
            Console.Error.WriteLine("simulated push failure"); return 1;
        } else if (call.Contains("shell bash /tmp/development/install_simpleadmin_rust.sh")) {
            if (modeName == "streaming") {
                Console.WriteLine("STREAMING_PROBE"); Console.Out.Flush();
                for (int i = 0; i < 100 && !File.Exists(Path.Combine(dir, "streaming-observed")); i++) System.Threading.Thread.Sleep(50);
                if (!File.Exists(Path.Combine(dir, "streaming-observed"))) return 1;
            }
            if (modeName == "install-failed") { Console.Error.WriteLine("simulated installation failure"); return 1; }
            Console.WriteLine("simulated installation");
        } else if (call.Contains("cat /tmp/simpleadmin-install-result.env")) {
            Console.WriteLine(modeName == "missing-result" ? "REBOOT_REQUIRED=0" : "INSTALL_STATUS=OK");
        } else if (call.Contains("forward tcp:0 tcp:80")) {
            Console.WriteLine(File.ReadAllText(Path.Combine(dir, "port")));
        }
        return 0;
    }
}
'@
    Add-Type -TypeDefinition $fakeSource -OutputAssembly (Join-Path $scratch 'adb.exe') -OutputType ConsoleApplication
    Set-Content -LiteralPath (Join-Path $scratch 'http-mode') -Value 'ok'
    $server = Start-Process -FilePath (Join-Path $scratch 'adb.exe') -ArgumentList 'serve' -WindowStyle Hidden -PassThru
    for ($i = 0; $i -lt 50 -and -not (Test-Path (Join-Path $scratch 'port')); $i++) { Start-Sleep -Milliseconds 100 }
    if (-not (Test-Path (Join-Path $scratch 'port'))) { throw 'Fixture HTTP server failed to start.' }
    $count = 0
    foreach ($case in @('none', 'unauthorized', 'multiple', 'push-failed', 'install-failed', 'missing-result', 'success', 'wrong-app', 'diagnose', 'web')) {
        $env:SIMPLEADMIN_TEST_CASE = $case
        Set-Content -LiteralPath (Join-Path $scratch 'http-mode') -Value $(if ($case -eq 'wrong-app') { 'wrong-app' } else { 'ok' })
        Set-Content -LiteralPath (Join-Path $scratch 'calls') -Value ''
        $options = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $scratch 'toolkit.ps1'))
        if ($case -eq 'diagnose') { $options += '-DiagnoseOnly' }
        if ($case -eq 'web') { $options += '-OpenWebOnly' }
        $output = & powershell.exe @options 2>&1
        $code = $LASTEXITCODE
        $expected = if ($case -in @('success', 'diagnose', 'web')) { 0 } else { 1 }
        if ($code -ne $expected) { throw "${case}: exit $code expected $expected`n$($output -join "`n")" }
        $calls = Get-Content -LiteralPath (Join-Path $scratch 'calls') -Raw
        if ($case -in @('none', 'unauthorized', 'multiple') -and $calls -match 'shell|push') { throw "${case}: must not touch a device" }
        if ($case -eq 'diagnose' -and $calls -match 'install_simpleadmin|remount|reboot|AT\+|sms|passwd') { throw 'Diagnostic path changed device state.' }
        if ($case -eq 'web' -and $calls -match 'push|install_simpleadmin|remount|reboot|AT\+|sms|passwd') { throw 'Open web path changed device state.' }
        if ($case -notin @('success', 'diagnose', 'web') -and ($output -join "`n") -match 'Installation and HTTP checks passed') { throw 'False success reported.' }
        if (($output -join "`n") -notmatch "@@SIMPLEADMIN\|result\|$expected") { throw 'Structured final result missing.' }
        if ($case -eq 'wrong-app' -and $calls -notmatch 'forward --remove') { throw 'Failed HTTP tunnel was not removed.' }
        Write-Host "PASS Windows installer: $case"
        $count++
    }
    Write-Host "$count Windows installer checks passed"
    if ($GuiTest) {
        foreach ($case in @('none', 'unauthorized', 'multiple', 'push-failed', 'install-failed', 'missing-result', 'success', 'wrong-app', 'diagnose', 'streaming')) {
            $env:SIMPLEADMIN_TEST_CASE = $case
            Set-Content -LiteralPath (Join-Path $scratch 'http-mode') -Value $(if ($case -eq 'wrong-app') { 'wrong-app' } else { 'ok' })
            Set-Content -LiteralPath (Join-Path $scratch 'calls') -Value ''
            $guiResult = Join-Path $scratch ('gui-' + $case + '.txt')
            $operation = if ($case -eq 'diagnose') { 'diagnose' } else { 'install' }
            $gui = Start-Process -FilePath (Join-Path $scratch 'SimpleAdmin-Setup.exe') -ArgumentList @('--test-run', $operation, $guiResult) -WindowStyle Hidden -PassThru
            if (-not $gui.WaitForExit(30000)) { Stop-Process -Id $gui.Id -Force; throw "GUI timeout: $case" }
            if ($gui.ExitCode -ne 0 -or -not (Test-Path $guiResult)) { throw "GUI failed: $case" }
            $state = Get-Content -LiteralPath $guiResult -Raw -Encoding UTF8
            $calls = Get-Content -LiteralPath (Join-Path $scratch 'calls') -Raw
            if ($case -in @('none', 'unauthorized', 'multiple')) {
                if ($state -notmatch 'INSTALL=False' -or $calls -match 'shell|push') { throw "GUI unsafe device selection: $case" }
            } else {
                $expected = if ($case -in @('success','diagnose','streaming')) { 0 } else { 1 }
                if ($state -notmatch "RESULT=$expected" -or $state -notmatch 'REPORT=True' -or $state -notmatch 'BUSY=False' -or $state -notmatch 'INSTALL=True') { throw "GUI incorrect completion state: $case`n$state" }
            }
            Write-Host "PASS GUI workflow: $case"
        }
    }
} finally {
    if ($server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force }
    $env:ANDROID_SERIAL = $oldSerial
    $env:SIMPLEADMIN_TEST_CASE = $oldCase
    $env:SIMPLEADMIN_TEST_DIR = $oldDir
    # Only this test's GUID-named temporary directory may be removed.
    $resolved = [IO.Path]::GetFullPath($scratch)
    $tempRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
    if ($resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and (Split-Path $resolved -Leaf).StartsWith('simpleadmin-installer-test-')) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
