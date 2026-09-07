using System;
using System.Diagnostics;
using System.IO;
using System.IO.Compression;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text.RegularExpressions;

[assembly: AssemblyTitle("移远高通系列5G模块配置与维护")]
[assembly: AssemblyVersion("1.3.0.0")]
static class Bootstrap
{
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern int MessageBoxW(IntPtr hwnd, string text, string caption, uint type);

    static string Quote(string value)
    {
        return "\"" + Regex.Replace(value, @"(\\*)(""|$)", m => new string('\\', m.Groups[1].Length * 2) + (m.Groups[2].Value == "\"" ? "\\\"" : "")) + "\"";
    }

    [STAThread]
    static int Main(string[] args)
    {
        string folder = null;
        try {
            if (!Environment.Is64BitOperatingSystem || Environment.OSVersion.Version.Build < 19041)
                throw new NotSupportedException("原生 WinUI 3 设备助手需要 Windows 10 2004（19041）或更新的 64 位系统。");
            folder = Path.Combine(Path.GetTempPath(), "SimpleAdmin-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(folder);
            using (var stream = Assembly.GetExecutingAssembly().GetManifestResourceStream("payload.zip"))
            using (var zip = new ZipArchive(stream, ZipArchiveMode.Read)) {
                foreach (var entry in zip.Entries) {
                    string target = Path.GetFullPath(Path.Combine(folder, entry.FullName));
                    if (!target.StartsWith(folder + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase))
                        throw new InvalidDataException("安装包资源路径无效。");
                    if (String.IsNullOrEmpty(entry.Name)) { Directory.CreateDirectory(target); continue; }
                    Directory.CreateDirectory(Path.GetDirectoryName(target));
                    using (var input = entry.Open()) using (var output = File.Create(target)) input.CopyTo(output);
                }
            }
            string arguments = String.Join(" ", Array.ConvertAll(args, Quote));
            var info = new ProcessStartInfo(Path.Combine(folder, "SimpleAdmin.Installer.exe"), arguments) {
                WorkingDirectory = folder, UseShellExecute = false, CreateNoWindow = true
            };
            using (var process = Process.Start(info)) { process.WaitForExit(); return process.ExitCode; }
        } catch (Exception error) {
            string reportDir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "SimpleAdmin", "Reports");
            Directory.CreateDirectory(reportDir);
            string report = Path.Combine(reportDir, "launcher-" + DateTime.Now.ToString("yyyyMMdd-HHmmss") + ".txt");
            File.WriteAllText(report, error.ToString());
            if (args.Length == 0) MessageBoxW(IntPtr.Zero, "无法启动设备助手：" + error.Message + "\n报告：" + report, "SimpleAdmin", 0x10);
            return 1;
        } finally {
            // Do not stop a shared ADB server just to remove its executable.
            if (folder != null) {
                try { Directory.Delete(folder, true); }
                catch (IOException) { }
                catch (UnauthorizedAccessException) { }
            }
        }
    }
}
